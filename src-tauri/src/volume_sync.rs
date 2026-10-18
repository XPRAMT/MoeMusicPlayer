//! "音量同步": keeping several playback devices' real Windows volumes in proportion (like the reference channel
//! mapper). When one device's volume changes, the other chosen devices move so that their ratio to it stays what it was
//! when the sync started. It runs here, in the backend, because it must work all the time: from the settings page, the
//! quick-settings drawer, the Windows volume mixer or a headset button, whether or not any settings UI is open.
//!
//! Volumes are whole percents (0 to 100). The ratios are remembered per pair, so going down to 0 and coming back
//! restores the proportions instead of leaving every device at the same level.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Ratios within this band of 1 snap to exactly 1 (two devices set to "about the same" move together).
const SNAP_LOW: f64 = 0.9;
const SNAP_HIGH: f64 = 1.1;
/// A device the player just wrote to is not taken to have been changed from outside for this long: the write comes
/// back as a change notification, and the player's own sync writes must not start another round.
pub(crate) const ECHO_HOLD: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub(crate) struct VolumeSync {
    enabled: bool,
    /// The last volume known for each device.
    known: HashMap<String, u8>,
    /// `"{reference}>{id}"` is id's volume divided by the reference's, learned the first time and remembered.
    scales: HashMap<String, f64>,
    /// When the player last wrote each device.
    written: HashMap<String, Instant>,
    /// The devices that took part in the last round (the ratios are forgotten when this changes).
    participants: Vec<String>,
}

impl VolumeSync {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Self::default()
        }
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.scales.clear();
        }
    }

    /// Remember volumes that were read (not changed): the starting point for the ratios.
    pub(crate) fn learn(&mut self, volumes: impl IntoIterator<Item = (String, u8)>) {
        for (id, percent) in volumes {
            self.known.insert(id, percent);
        }
    }

    fn ratio_to(&mut self, id: &str, reference: &str, id_volume: u8, reference_volume: u8) -> f64 {
        let key = format!("{reference}>{id}");
        if let Some(known) = self.scales.get(&key) {
            return *known;
        }
        let mut ratio = if reference_volume > 0 {
            f64::from(id_volume) / f64::from(reference_volume)
        } else {
            1.0
        };
        if ratio > SNAP_LOW && ratio < SNAP_HIGH {
            ratio = 1.0;
        }
        self.scales.insert(key, ratio);
        // The same pair seen from the other side (a device at 0 cannot be divided by, so it falls back to 1).
        self.scales
            .insert(format!("{id}>{reference}"), if ratio > 0.0 { 1.0 / ratio } else { 1.0 });
        ratio
    }

    /// A device's volume is now `percent`. `local` means the player itself set it (the settings UI): that is never
    /// an echo. Returns the volumes the other devices should be set to now (and records them as written).
    pub(crate) fn changed(
        &mut self,
        id: &str,
        percent: u8,
        local: bool,
        now: Instant,
        participants: &[String],
    ) -> Vec<(String, u8)> {
        let held = !local
            && self
                .written
                .get(id)
                .is_some_and(|at| now.duration_since(*at) < ECHO_HOLD);
        if local {
            self.written.insert(id.to_owned(), now);
        }
        let previous = self.known.insert(id.to_owned(), percent);
        if held || !self.enabled {
            return Vec::new();
        }
        let Some(previous) = previous.filter(|previous| *previous != percent) else {
            return Vec::new();
        };
        if !participants.iter().any(|participant| participant == id) {
            return Vec::new();
        }
        if self.participants != participants {
            self.scales.clear();
            self.participants = participants.to_vec();
        }
        let mut updates = Vec::new();
        for other in participants {
            if other == id {
                continue;
            }
            let Some(other_volume) = self.known.get(other).copied() else {
                continue;
            };
            let ratio = self.ratio_to(other, id, other_volume, previous);
            let next = (f64::from(percent) * ratio).round().clamp(0.0, 100.0) as u8;
            updates.push((other.clone(), next));
        }
        for (other, next) in &updates {
            self.known.insert(other.clone(), *next);
            self.written.insert(other.clone(), now);
        }
        updates
    }
}

/// Sent whenever Windows reports a volume or mute change of a watched playback device, for the settings UI to show.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) struct DeviceVolumeEvent {
    pub(crate) id: String,
    /// 0 to 100.
    pub(crate) volume_percent: f32,
    pub(crate) muted: bool,
}

/// The always-on part (Windows): watches every playback device's volume through Windows' own notifications and keeps
/// the chosen devices in proportion when the sync is on.
#[cfg(target_os = "windows")]
pub(crate) mod service {
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Instant;

    use player_platform_windows::endpoint_volume::{
        read_endpoint_volume, set_endpoint_muted, set_endpoint_volume, VolumeWatcher,
    };
    use tauri::{AppHandle, Emitter};

    use super::{DeviceVolumeEvent, VolumeSync};
    use crate::settings::SettingsStore;

    enum Job {
        /// Windows reported a change (the settings UI, the mixer, a headset button, or an echo of our own write).
        Changed { id: String, percent: u8 },
        /// The settings UI set this device through the player.
        Local { id: String, percent: u8 },
        /// The list of devices changed (or at start): watch them all and read their volumes.
        Refresh,
    }

    static SYNC: Mutex<Option<VolumeSync>> = Mutex::new(None);
    static JOBS: OnceLock<mpsc::Sender<Job>> = OnceLock::new();

    fn with_sync<T>(action: impl FnOnce(&mut VolumeSync) -> T) -> T {
        let mut guard = SYNC.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        action(guard.get_or_insert_with(|| VolumeSync::new(false)))
    }

    pub(crate) fn set_enabled(enabled: bool) {
        with_sync(|sync| sync.set_enabled(enabled));
    }

    /// The devices that move together: the chosen ones, when two or more are playing at once (exclusive mode plays on
    /// one device and has nothing to keep in proportion).
    fn participants(settings: &SettingsStore) -> Vec<String> {
        let Ok(snapshot) = settings.snapshot() else {
            return Vec::new();
        };
        if snapshot.output_exclusive {
            return Vec::new();
        }
        let mut ids: Vec<String> = Vec::new();
        for entry in snapshot.output_devices.iter().filter(|entry| entry.enabled) {
            if let Some(id) = entry.id.as_ref() {
                if !ids.contains(id) {
                    ids.push(id.clone());
                }
            }
        }
        if ids.len() < 2 {
            ids.clear();
        }
        ids
    }

    fn write(id: &str, percent: u8) {
        let volume = f32::from(percent) / 100.0;
        let _ = set_endpoint_volume(Some(id), volume);
        // Like the reference mapper: volume 0 also mutes, anything above unmutes.
        let _ = set_endpoint_muted(Some(id), percent == 0);
    }

    /// Start watching. `enabled` is the saved state of the "音量同步" switch.
    pub(crate) fn start(app: AppHandle, settings: Arc<SettingsStore>, enabled: bool) {
        set_enabled(enabled);
        let (jobs, queue) = mpsc::channel::<Job>();
        if JOBS.set(jobs.clone()).is_err() {
            return;
        }
        let sink_jobs = jobs.clone();
        let sink_app = app.clone();
        let watcher = VolumeWatcher::start(move |change| {
            let percent = (change.volume * 100.0).round().clamp(0.0, 100.0) as u8;
            let _ = sink_app.emit(
                "device-volume-changed",
                DeviceVolumeEvent {
                    id: change.id.clone(),
                    volume_percent: change.volume * 100.0,
                    muted: change.muted,
                },
            );
            // Windows calls this on its own thread, where it must not change other endpoints' volumes.
            let _ = sink_jobs.send(Job::Changed {
                id: change.id,
                percent,
            });
        });
        let _ = std::thread::Builder::new()
            .name("moe-volume-sync".to_owned())
            .spawn(move || {
                for job in queue {
                    match job {
                        Job::Refresh => {
                            let devices = player_audio_windows::output_devices::list_output_devices();
                            watcher.watch(devices.iter().map(|device| device.id.clone()).collect());
                            let volumes = devices.iter().filter_map(|device| {
                                let volume = read_endpoint_volume(Some(&device.id)).ok()?;
                                Some((device.id.clone(), (volume.volume * 100.0).round() as u8))
                            });
                            let volumes: Vec<(String, u8)> = volumes.collect();
                            with_sync(|sync| sync.learn(volumes));
                        }
                        Job::Changed { id, percent } => handle(&settings, &id, percent, false),
                        Job::Local { id, percent } => handle(&settings, &id, percent, true),
                    }
                }
            });
        let _ = jobs.send(Job::Refresh);
    }

    fn handle(settings: &SettingsStore, id: &str, percent: u8, local: bool) {
        let participants = participants(settings);
        let writes = with_sync(|sync| sync.changed(id, percent, local, Instant::now(), &participants));
        for (other, value) in writes {
            write(&other, value);
        }
    }

    /// The device list changed: watch the new devices and read their volumes.
    pub(crate) fn refresh() {
        if let Some(jobs) = JOBS.get() {
            let _ = jobs.send(Job::Refresh);
        }
    }

    /// The settings UI set this device's volume through the player.
    pub(crate) fn local_change(id: String, percent: u8) {
        if let Some(jobs) = JOBS.get() {
            let _ = jobs.send(Job::Local { id, percent });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    fn sync_with(volumes: &[(&str, u8)]) -> VolumeSync {
        let mut sync = VolumeSync::new(true);
        sync.learn(volumes.iter().map(|(id, percent)| ((*id).to_owned(), *percent)));
        sync
    }

    #[test]
    fn other_devices_keep_their_ratio_to_the_one_that_moved() {
        let mut sync = sync_with(&[("a", 50), ("b", 25), ("c", 100)]);
        let updates = sync.changed("a", 80, true, Instant::now(), &ids(&["a", "b", "c"]));
        assert_eq!(updates, [("b".to_owned(), 40), ("c".to_owned(), 100)], "b stays half of a, c is clamped to 100");
    }

    #[test]
    fn ratios_within_ten_percent_snap_to_equal_and_are_remembered_through_zero() {
        let mut sync = sync_with(&[("a", 50), ("b", 53)]);
        let both = ids(&["a", "b"]);
        let now = Instant::now();
        assert_eq!(sync.changed("a", 30, true, now, &both), [("b".to_owned(), 30)], "about equal snaps to the same level");
        assert_eq!(sync.changed("a", 0, true, now, &both), [("b".to_owned(), 0)]);
        assert_eq!(
            sync.changed("a", 60, true, now, &both),
            [("b".to_owned(), 60)],
            "the remembered ratio is used even though both were at 0"
        );
    }

    #[test]
    fn the_other_device_moving_uses_the_same_pair_ratio_seen_from_its_side() {
        let mut sync = sync_with(&[("a", 40), ("b", 20)]);
        let both = ids(&["a", "b"]);
        let now = Instant::now();
        assert_eq!(sync.changed("a", 80, true, now, &both), [("b".to_owned(), 40)]);
        assert_eq!(sync.changed("b", 10, true, now, &both), [("a".to_owned(), 20)], "b is half of a");
    }

    #[test]
    fn devices_outside_the_sync_or_without_a_known_volume_are_left_alone() {
        let mut sync = sync_with(&[("a", 100), ("b", 100), ("d", 40)]);
        let updates = sync.changed("a", 50, true, Instant::now(), &ids(&["a", "b", "c"]));
        assert_eq!(updates, [("b".to_owned(), 50)], "c is unknown, d does not take part");
        assert!(sync
            .changed("d", 10, true, Instant::now(), &ids(&["a", "b", "c"]))
            .is_empty());
    }

    #[test]
    fn nothing_happens_while_the_sync_is_off_but_the_volumes_are_still_tracked() {
        let mut sync = sync_with(&[("a", 50), ("b", 25)]);
        sync.set_enabled(false);
        assert!(sync.changed("a", 80, true, Instant::now(), &ids(&["a", "b"])).is_empty());
        sync.set_enabled(true);
        // The ratio is taken from the volumes at the first change after switching on: a=80, b=25.
        let updates = sync.changed("a", 40, true, Instant::now(), &ids(&["a", "b"]));
        assert_eq!(updates, [("b".to_owned(), 13)]);
    }

    #[test]
    fn a_change_that_is_the_echo_of_the_players_own_write_does_not_start_another_round() {
        let mut sync = sync_with(&[("a", 50), ("b", 50)]);
        let both = ids(&["a", "b"]);
        let start = Instant::now();
        let updates = sync.changed("a", 60, true, start, &both);
        assert_eq!(updates, [("b".to_owned(), 60)]);
        // Windows reports both writes back a moment later: they are echoes.
        assert!(sync.changed("a", 60, false, start + Duration::from_millis(30), &both).is_empty());
        assert!(sync.changed("b", 60, false, start + Duration::from_millis(40), &both).is_empty());
        // A change from outside after the hold moves the other device.
        let later = start + ECHO_HOLD + Duration::from_millis(10);
        assert_eq!(sync.changed("b", 30, false, later, &both), [("a".to_owned(), 30)]);
    }

    #[test]
    fn a_change_from_outside_moves_the_others_and_a_new_set_of_devices_forgets_old_ratios() {
        let mut sync = sync_with(&[("a", 50), ("b", 25), ("c", 50)]);
        let now = Instant::now();
        assert_eq!(sync.changed("a", 100, false, now, &ids(&["a", "b"])), [("b".to_owned(), 50)]);
        // c joins: the pair ratios are learned again from the current volumes.
        let later = now + ECHO_HOLD * 2;
        let updates = sync.changed("a", 50, false, later, &ids(&["a", "b", "c"]));
        assert_eq!(updates, [("b".to_owned(), 25), ("c".to_owned(), 25)]);
    }
}
