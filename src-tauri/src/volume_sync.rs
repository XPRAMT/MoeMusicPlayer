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
/// A change notification that repeats a value the player wrote to that device within this long is the echo of that
/// write (Windows reports every write back) and is ignored: it must neither start another round nor overwrite what
/// the player knows. A different value in that time is a real change from outside.
pub(crate) const ECHO_HOLD: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub(crate) struct VolumeSync {
    enabled: bool,
    /// The last volume known for each device.
    known: HashMap<String, u8>,
    /// `"{reference}>{id}"` is id's volume divided by the reference's, learned the first time and remembered.
    scales: HashMap<String, f64>,
    /// What the player wrote to each device within the last [`ECHO_HOLD`], and when (several during a drag).
    written: HashMap<String, Vec<(Instant, u8)>>,
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

    /// Note that the player writes `percent` to `id` now (its echo will be recognised).
    fn record_write(&mut self, id: &str, percent: u8, now: Instant) {
        let writes = self.written.entry(id.to_owned()).or_default();
        writes.retain(|(at, _)| now.saturating_duration_since(*at) < ECHO_HOLD);
        writes.push((now, percent));
    }

    /// Whether a reported `percent` for `id` is the echo of one of the player's own recent writes.
    fn is_echo(&self, id: &str, percent: u8, now: Instant) -> bool {
        self.written.get(id).is_some_and(|writes| {
            writes
                .iter()
                .any(|(at, value)| *value == percent && now.saturating_duration_since(*at) < ECHO_HOLD)
        })
    }

    /// A device's volume is now `percent`. `local` means the player itself sets it (the settings UI; the caller then
    /// writes it): that is never an echo and is recorded as written. A reported change (`local` false) that repeats a
    /// recent write is an echo: ignored, `known` untouched. Returns the volumes the other devices should be set to now
    /// (only those that differ from what is known; they are recorded as written).
    pub(crate) fn changed(
        &mut self,
        id: &str,
        percent: u8,
        local: bool,
        now: Instant,
        participants: &[String],
    ) -> Vec<(String, u8)> {
        if local {
            self.record_write(id, percent, now);
        } else if self.is_echo(id, percent, now) {
            return Vec::new();
        }
        let previous = self.known.insert(id.to_owned(), percent);
        if !self.enabled {
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
            // Already there: writing it again would only bring another notification.
            if next != other_volume {
                updates.push((other.clone(), next));
            }
        }
        for (other, next) in &updates {
            self.known.insert(other.clone(), *next);
            self.record_write(other, *next, now);
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
        /// The settings UI sets this device through the player: this thread writes it (then the others), so the write
        /// and the sync happen in order, before Windows' notification of the write is looked at. The result of writing
        /// this device goes back to the caller.
        Local {
            id: String,
            percent: u8,
            reply: mpsc::Sender<Result<(), String>>,
        },
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

    /// Set a device's real volume. Like the reference mapper, volume 0 also mutes and anything above unmutes.
    pub(crate) fn write(id: &str, percent: u8) -> Result<(), String> {
        let volume = f32::from(percent) / 100.0;
        set_endpoint_volume(Some(id), volume)?;
        set_endpoint_muted(Some(id), percent == 0)?;
        Ok(())
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
                        Job::Changed { id, percent } => {
                            // Nothing to answer: errors writing the followers are already logged.
                            let _ = handle(&settings, &id, percent, false);
                        }
                        Job::Local { id, percent, reply } => {
                            let _ = reply.send(handle(&settings, &id, percent, true));
                        }
                    }
                }
            });
        let _ = jobs.send(Job::Refresh);
    }

    /// One change, in order: a local one writes its own device first (its error is the result), then the devices
    /// that follow it.
    fn handle(settings: &SettingsStore, id: &str, percent: u8, local: bool) -> Result<(), String> {
        let participants = participants(settings);
        let writes = with_sync(|sync| sync.changed(id, percent, local, Instant::now(), &participants));
        let own = if local { write(id, percent) } else { Ok(()) };
        for (other, value) in writes {
            if let Err(error) = write(&other, value) {
                eprintln!("[volume-sync] could not set {other} to {value}%: {error}");
            }
        }
        own
    }

    /// The device list changed: watch the new devices and read their volumes.
    pub(crate) fn refresh() {
        if let Some(jobs) = JOBS.get() {
            let _ = jobs.send(Job::Refresh);
        }
    }

    /// The settings UI sets this device's volume: written (with the sync) on the sync thread, in order with the
    /// notifications. Blocks until it is written; without the sync thread the device is written here.
    pub(crate) fn set_local(id: String, percent: u8) -> Result<(), String> {
        let Some(jobs) = JOBS.get() else {
            return write(&id, percent);
        };
        let (reply, answer) = mpsc::channel();
        jobs.send(Job::Local {
            id: id.clone(),
            percent,
            reply,
        })
        .map_err(|_| "音量同步服務已停止".to_owned())?;
        answer
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| "設定音量逾時".to_owned())?
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
        assert_eq!(updates, [("b".to_owned(), 40)], "b stays half of a; c, clamped to 100, is already there");
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
    fn a_drag_keeps_the_others_in_step_even_with_echoes_of_earlier_steps_in_between() {
        let mut sync = sync_with(&[("a", 50), ("b", 25)]);
        let both = ids(&["a", "b"]);
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        assert_eq!(sync.changed("a", 52, true, at(0), &both), [("b".to_owned(), 26)]);
        // Windows reports the writes back (the dragged device and the follower) between the steps.
        assert!(sync.changed("a", 52, false, at(20), &both).is_empty());
        assert!(sync.changed("b", 26, false, at(25), &both).is_empty());
        assert_eq!(sync.changed("a", 56, true, at(60), &both), [("b".to_owned(), 28)]);
        // An echo of an older step arriving late is still an echo, and does not undo the newer value.
        assert!(sync.changed("a", 52, false, at(80), &both).is_empty());
        assert_eq!(sync.changed("a", 60, true, at(120), &both), [("b".to_owned(), 30)]);
        assert_eq!(sync.known.get("a"), Some(&60));
    }

    #[test]
    fn an_echo_does_not_change_what_is_known() {
        let mut sync = sync_with(&[("a", 50), ("b", 50)]);
        let both = ids(&["a", "b"]);
        let now = Instant::now();
        sync.changed("a", 70, true, now, &both);
        sync.changed("a", 70, false, now + Duration::from_millis(10), &both);
        assert_eq!(sync.known.get("a"), Some(&70));
        assert_eq!(sync.known.get("b"), Some(&70));
    }

    #[test]
    fn a_different_value_right_after_a_write_is_a_change_from_outside() {
        let mut sync = sync_with(&[("a", 50), ("b", 50)]);
        let both = ids(&["a", "b"]);
        let now = Instant::now();
        sync.changed("a", 60, true, now, &both);
        // A headset button moved a within the hold: not the value written, so it is followed.
        assert_eq!(sync.changed("a", 40, false, now + Duration::from_millis(100), &both), [("b".to_owned(), 40)]);
    }

    #[test]
    fn several_devices_settle_after_one_round_without_writing_each_other_back_and_forth() {
        let mut sync = sync_with(&[("a", 50), ("b", 25), ("c", 100)]);
        let all = ids(&["a", "b", "c"]);
        let start = Instant::now();
        let mut pending: Vec<(String, u8)> = sync.changed("a", 80, true, start, &all);
        let mut writes = pending.len();
        // Feed every write back as Windows would (each one a notification), several times over.
        for round in 1..5 {
            let echoes = std::mem::take(&mut pending);
            for (id, percent) in echoes.iter().chain([("a".to_owned(), 80)].iter()) {
                let more = sync.changed(id, *percent, false, start + Duration::from_millis(10 * round), &all);
                writes += more.len();
                pending.extend(more);
            }
        }
        assert_eq!(writes, 1, "only b moves (c is already at 100); the echoes start nothing");
        assert_eq!(sync.known.get("b"), Some(&40));
        assert_eq!(sync.known.get("c"), Some(&100));
    }

    #[test]
    fn a_device_already_at_its_target_is_not_written_again() {
        let mut sync = sync_with(&[("a", 50), ("b", 50)]);
        let both = ids(&["a", "b"]);
        let now = Instant::now();
        assert_eq!(sync.changed("a", 60, true, now, &both), [("b".to_owned(), 60)]);
        // b is moved to 60 from outside; a then follows to 60 — and b, already at a's level, is left alone.
        let later = now + ECHO_HOLD * 2;
        assert_eq!(sync.changed("b", 61, false, later, &both), [("a".to_owned(), 61)]);
        assert!(sync.changed("a", 61, true, later + ECHO_HOLD * 2, &both).is_empty());
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
