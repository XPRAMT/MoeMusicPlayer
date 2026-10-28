//! Rodio/CPAL (WASAPI shared mode) backend.
//!
//! Every method here runs on the audio worker thread and must return without
//! waiting for CPAL's render callback. When a WASAPI endpoint is invalidated
//! (unplugged headphones, Bluetooth disconnect, default-device switch that
//! disables the old endpoint) CPAL reports the error once and its render
//! thread exits, so the mixer is never pulled again. Rodio APIs such as
//! `Player::append` after `Player::stop` (`sleep_until_end`) and
//! `Player::try_seek` block until that callback runs, which would park the
//! worker forever.
//!
//! - Load never reuses a stopped `Player`; each load connects a fresh one.
//! - Seek on a healthy stream uses `Player::try_seek` on a helper thread with
//!   a deadline. If it times out, or the stream is already known to be dead,
//!   the decoder is reopened and seeked on this thread, attached to a fresh
//!   `Player`, and the output is reported lost so the worker rebuilds it.
//! - Abandoned helper threads own only an `Arc<Player>`; no lock is held
//!   while they wait and nothing ever joins them.
//!
//! Sample-rate handling follows [`ResamplingMode`]:
//!
//! - High quality: the stream runs at the endpoint's shared-mode mix rate and
//!   every track whose rate differs is wrapped in [`Resampled`], so Rodio's
//!   mixer never converts.
//! - Windows built-in: before a track is attached the stream is reopened
//!   strictly at the track's rate (only when it differs), and the Windows
//!   audio engine converts to the mix rate. If that open fails, the stream
//!   stays on or returns to the mix rate, the track is converted in-app, and
//!   the fallback is reported in [`ResamplingInfo`].

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::cpal::StreamError;
use rodio::mixer::Mixer;
use rodio::{
    ChannelCount, Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Sample, SampleRate, Source,
};

use crate::mirror::{
    DelayLine, MirrorControl, MirrorHub, MirrorSource, MixSource, Ring, StallWatch, Tee,
    JITTER_MARGIN_MS,
};
use crate::output_devices::{
    active_devices, find_output_device, ChannelMixer, ChannelRoute, DeviceState,
    OutputDeviceSetting, OutputDeviceStatus,
};
use crate::resample::{needs_resampling, Resampled};
use crate::{
    AudioBackend, AudioError, DsdOptions, DsdSettings, DseeHxState, OutputStatus, ResamplingConfig,
    ResamplingConversion, ResamplingInfo, ResamplingMode,
};

/// Upper bound for opening a WASAPI endpoint. A driver that hangs inside
/// Activate/Initialize must not take the worker with it.
const OUTPUT_OPEN_TIMEOUT: Duration = Duration::from_secs(3);
/// How often the worker compares the opened endpoint with the system default.
/// Device notifications (`device_notify`) trigger the check at once; this slow poll only covers a missed one.
pub(crate) const DEFAULT_DEVICE_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// A running shared-mode stream pulls the mixer every few milliseconds; no
/// pull for this long means the render thread is gone without an error.
const OUTPUT_STALL_TIMEOUT: Duration = Duration::from_secs(3);
/// The heartbeat source publishes its sample count in batches.
const HEARTBEAT_BATCH: u32 = 1024;
/// Deadline for `Player::try_seek`, which normally completes within ~5 ms.
const SEEK_TIMEOUT: Duration = Duration::from_secs(1);

pub(super) struct RodioBackend {
    // Dropped on a disposal thread; see `Drop`.
    output: Option<OpenOutput>,
    track: TrackPlayer,
    last_device_check: Instant,
    seen_device_generation: u64,
    last_heartbeat: (u64, Instant),
    /// Shared with the backend factory so a rebuilt output starts in the
    /// current mode and, in Windows-builtin mode, at the current track's rate.
    config: Arc<ResamplingConfig>,
    mode: ResamplingMode,
    dsee_enabled: bool,
    /// Windows-builtin only: the track rate the endpoint refused, and why.
    fallback: Option<OutputFallback>,
    /// The device the main output was opened for (`None`: the system default).
    opened_for: Option<String>,
    /// Copies of the track for the other chosen devices.
    hub: Arc<MirrorHub>,
    mirrors: Vec<MirrorOutput>,
    pending: Vec<PendingMirror>,
    /// Devices that failed to open or died, with the reason and when to try again.
    failed: Vec<(OutputDeviceSetting, String, Instant)>,
    master_volume: f32,
    reported_status: Vec<OutputDeviceStatus>,
    /// Plays the tracks exclusive mode could not: it must not ask for a rebuild because exclusive mode is on.
    as_fallback: bool,
}

/// A device that plays a copy of the track next to the main output.
struct MirrorOutput {
    setting: OutputDeviceSetting,
    // Owns CPAL's stream; dropped on a disposal thread.
    sink: Option<MixerDeviceSink>,
    ring: Arc<Ring>,
    hub: Arc<MirrorHub>,
    control: Arc<MirrorControl>,
    signals: Arc<OutputSignals>,
    /// Notices a stream that stops pulling audio without an error (a Bluetooth device that powered off).
    watch: StallWatch,
    rate_hz: u32,
    channels: u16,
}

impl Drop for MirrorOutput {
    fn drop(&mut self) {
        self.hub.remove_ring(&self.ring);
    }
}

struct PendingMirror {
    setting: OutputDeviceSetting,
    result: mpsc::Receiver<Result<MirrorOutput, String>>,
}

/// A mirror stream that pulled nothing for this long is treated as gone.
const MIRROR_STALL_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a device that failed to open waits before the next attempt.
const MIRROR_RETRY_INTERVAL: Duration = Duration::from_secs(5);

struct OpenOutput {
    // The device sink owns CPAL's WASAPI stream and must outlive the player.
    sink: MixerDeviceSink,
    device_id: Option<String>,
    signals: Arc<OutputSignals>,
    /// The rate the stream actually runs at (the mixer's rate).
    sample_rate: SampleRate,
    /// The endpoint's shared-mode mix rate reported when the stream opened.
    device_rate: SampleRate,
    /// Opened with the device's default format rather than a requested rate.
    mix: bool,
}

#[derive(Clone, Debug)]
struct OutputFallback {
    rate: SampleRate,
    reason: String,
}

/// Output change needed before a track at some rate is attached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputPlan {
    Keep,
    /// Reopen strictly at this rate (Windows-builtin).
    OpenAt(SampleRate),
    /// Reopen with the device's default (mix) format.
    OpenMix,
}

/// Decide whether the stream must be reopened for a track at `source_rate`.
/// `refused_rate` is a rate the current endpoint already failed to open at,
/// which is not retried for every track.
fn plan_output(
    mode: ResamplingMode,
    source_rate: SampleRate,
    output_rate: SampleRate,
    output_is_mix: bool,
    refused_rate: Option<SampleRate>,
) -> OutputPlan {
    match mode {
        ResamplingMode::HighQuality if output_is_mix => OutputPlan::Keep,
        ResamplingMode::HighQuality => OutputPlan::OpenMix,
        ResamplingMode::WindowsBuiltin if output_rate == source_rate => OutputPlan::Keep,
        ResamplingMode::WindowsBuiltin if refused_rate == Some(source_rate) => {
            if output_is_mix {
                OutputPlan::Keep
            } else {
                OutputPlan::OpenMix
            }
        }
        ResamplingMode::WindowsBuiltin => OutputPlan::OpenAt(source_rate),
    }
}

/// What the current track goes through on its way to the endpoint.
fn conversion_for(
    source_rate: Option<SampleRate>,
    output_rate: SampleRate,
    device_rate: SampleRate,
    in_app: bool,
) -> ResamplingConversion {
    match source_rate {
        None => ResamplingConversion::None,
        Some(rate) if rate != output_rate && in_app => ResamplingConversion::HighQuality,
        Some(rate) if rate != output_rate => ResamplingConversion::Basic,
        Some(_) if output_rate != device_rate => ResamplingConversion::Windows,
        Some(_) => ResamplingConversion::None,
    }
}

#[derive(Default)]
struct OutputSignals {
    failure: Mutex<Option<String>>,
    heartbeat: AtomicU64,
}

impl RodioBackend {
    /// True unless the stream reported an error or stopped pulling audio.
    fn output_alive(&self) -> bool {
        let Some(output) = self.output.as_ref() else {
            return false;
        };
        if output
            .signals
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
        {
            return false;
        }
        let beats = output.signals.heartbeat.load(Ordering::Relaxed);
        beats != self.last_heartbeat.0 || self.last_heartbeat.1.elapsed() < OUTPUT_STALL_TIMEOUT
    }

    fn mark_output_failed(&self, message: &str) {
        if let Some(output) = self.output.as_ref() {
            let mut failure = output
                .signals
                .failure
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if failure.is_none() {
                *failure = Some(message.to_owned());
            }
        }
    }

    /// Open the current default output endpoint. The open runs on a helper
    /// thread and is abandoned after `OUTPUT_OPEN_TIMEOUT`. In Windows-builtin
    /// mode the stream opens directly at the last track's rate when known; if
    /// the endpoint refuses it, the mix rate is used and the fallback noted.
    /// Failing both is an error so the worker's recovery retries.
    pub(super) fn new(config: Arc<ResamplingConfig>) -> Result<Self, AudioError> {
        let mode = config.mode();
        let requested = match mode {
            ResamplingMode::WindowsBuiltin => config.track_rate_hint().and_then(SampleRate::new),
            ResamplingMode::HighQuality => None,
        };
        let mut fallback = None;
        let device = config.primary_device();
        let output = match requested {
            Some(rate) => {
                match open_output_with_timeout(Some(rate), OUTPUT_OPEN_TIMEOUT, &device) {
                    Ok(output) => output,
                    Err(error) => {
                        let reason = error.to_string();
                        log_refused_rate(rate, &reason);
                        let output = open_output_with_timeout(None, OUTPUT_OPEN_TIMEOUT, &device)?;
                        fallback = Some(OutputFallback { rate, reason });
                        output
                    }
                }
            }
            None => open_output_with_timeout(None, OUTPUT_OPEN_TIMEOUT, &device)?,
        };
        let mut track = TrackPlayer::new(output.sink.mixer().clone(), output.sample_rate);
        track.dsee_enabled = config.dsee_hx();
        track.dsd = Arc::clone(&config.dsd);
        track.device_channels = usize::from(output.sink.config().channel_count().get());
        let hub = MirrorHub::new();
        track.hub = Arc::clone(&hub);
        let now = Instant::now();
        let mut backend = Self {
            output: Some(output),
            track,
            last_device_check: now,
            seen_device_generation: crate::device_notify::device_change_generation(),
            last_heartbeat: (0, now),
            config: Arc::clone(&config),
            mode,
            dsee_enabled: config.dsee_hx(),
            fallback,
            opened_for: device,
            hub,
            mirrors: Vec::new(),
            pending: Vec::new(),
            failed: Vec::new(),
            master_volume: 1.0,
            reported_status: Vec::new(),
            as_fallback: false,
        };
        let _ = backend.apply_devices();
        Ok(backend)
    }

    /// The shared-mode backend that plays a track while exclusive mode is on but could not take it.
    pub(super) fn new_fallback(config: Arc<ResamplingConfig>) -> Result<Self, AudioError> {
        let mut backend = Self::new(config)?;
        backend.as_fallback = true;
        Ok(backend)
    }

    /// Bring the main output and the mirrors in line with the chosen devices. A change of the main device or of its
    /// channel map restarts the loaded track at its position; everything else is applied live.
    fn apply_devices(&mut self) -> Result<(), AudioError> {
        let active = active_devices(&self.config.devices());
        let Some(primary) = active.first().cloned() else {
            return Ok(());
        };
        let mirrors_wanted = active.len() > 1;
        let margin = if mirrors_wanted { JITTER_MARGIN_MS } else { 0 };
        // The delay only exists to line devices up with each other; a lone device plays without it.
        let primary_delay = if mirrors_wanted { primary.delay_ms } else { 0 };
        self.track
            .delay_ms
            .store(margin + primary_delay, Ordering::Relaxed);
        let map_changed = self.track.map != primary.channel_map;
        self.track.map = primary.channel_map.clone();

        let mut restart = map_changed && self.track.path.is_some();
        if self.opened_for != primary.id {
            let resume = self.track.resume_point();
            match open_output_with_timeout(None, OUTPUT_OPEN_TIMEOUT, &primary.id) {
                Ok(output) => {
                    self.replace_output(output);
                    self.opened_for = primary.id.clone();
                    self.fallback = None;
                    if let Some(rate) = self.track.playback_rate.or(self.track.source_rate) {
                        if resume.is_some() {
                            self.prepare_output_for(rate);
                        }
                    }
                    if let Some((path, position)) = resume {
                        self.track.seek_by_reopening(&path, position)?;
                        restart = false;
                    }
                }
                Err(error) => {
                    eprintln!("[audio] could not open the chosen output device: {error}");
                    self.failed
                        .retain(|(setting, _, _)| setting.id != primary.id);
                }
            }
        }
        if restart {
            if let Some((path, position)) = self.track.resume_point() {
                self.track.seek_by_reopening(&path, position)?;
            }
        }

        // Mirrors: keep the ones still wanted (updating their settings), close the others, open the new ones.
        let wanted: Vec<OutputDeviceSetting> = active.into_iter().skip(1).collect();
        let mut kept = Vec::new();
        for mirror in self.mirrors.drain(..) {
            if wanted.iter().any(|setting| setting.id == mirror.setting.id) {
                kept.push(mirror);
            } else {
                dispose_mirror(mirror);
            }
        }
        self.mirrors = kept;
        self.pending.retain(|pending| {
            wanted
                .iter()
                .any(|setting| setting.id == pending.setting.id)
        });
        self.failed
            .retain(|(setting, _, _)| wanted.iter().any(|wanted| wanted.id == setting.id));
        for setting in &wanted {
            if let Some(mirror) = self
                .mirrors
                .iter_mut()
                .find(|mirror| mirror.setting.id == setting.id)
            {
                mirror
                    .control
                    .delay_ms
                    .store(setting.delay_ms, Ordering::Relaxed);
                mirror.control.set_map(setting.channel_map.clone());
                mirror.control.set_gain(self.master_volume);
                mirror.setting = setting.clone();
                continue;
            }
            if let Some(pending) = self
                .pending
                .iter_mut()
                .find(|pending| pending.setting.id == setting.id)
            {
                pending.setting = setting.clone();
                continue;
            }
            if let Some(failed) = self
                .failed
                .iter_mut()
                .find(|(failed, _, _)| failed.id == setting.id)
            {
                // Retry right away; a changed setting is a reason to look again.
                failed.0 = setting.clone();
                failed.2 = Instant::now() - MIRROR_RETRY_INTERVAL;
                continue;
            }
            self.start_mirror(setting.clone());
        }
        self.publish_status();
        Ok(())
    }

    fn start_mirror(&mut self, setting: OutputDeviceSetting) {
        let (tx, rx) = mpsc::sync_channel(1);
        let hub = Arc::clone(&self.hub);
        let gain = self.master_volume;
        let opening = setting.clone();
        let spawned = thread::Builder::new()
            .name("moe-audio-mirror-open".to_owned())
            .spawn(move || {
                let _ = tx.send(open_mirror(opening, hub, gain));
            });
        match spawned {
            Ok(_) => self.pending.push(PendingMirror {
                setting,
                result: rx,
            }),
            Err(error) => self
                .failed
                .push((setting, error.to_string(), Instant::now())),
        }
    }

    /// Install mirrors that finished opening, drop ones whose stream died and retry the failed ones.
    fn poll_mirrors(&mut self) {
        let mut index = 0;
        while index < self.pending.len() {
            let outcome = match self.pending[index].result.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "the output device thread stopped unexpectedly".to_owned(),
                )),
            };
            let Some(outcome) = outcome else {
                index += 1;
                continue;
            };
            let pending = self.pending.remove(index);
            match outcome {
                Ok(mirror) => {
                    mirror.control.set_gain(self.master_volume);
                    self.mirrors.push(mirror);
                }
                Err(reason) => self.failed.push((pending.setting, reason, Instant::now())),
            }
        }
        let mut index = 0;
        while index < self.mirrors.len() {
            let failure = self.mirrors[index]
                .signals
                .failure
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let stalled = failure.is_none() && {
                let beats = self.mirrors[index]
                    .signals
                    .heartbeat
                    .load(Ordering::Relaxed);
                self.mirrors[index]
                    .watch
                    .observe(beats, Instant::now(), MIRROR_STALL_TIMEOUT)
            };
            let failure = failure.or_else(|| {
                stalled.then(|| {
                    "the device stopped requesting audio (powered off or out of range?)".to_owned()
                })
            });
            match failure {
                Some(reason) => {
                    let mirror = self.mirrors.remove(index);
                    self.failed
                        .push((mirror.setting.clone(), reason, Instant::now()));
                    dispose_mirror(mirror);
                }
                None => index += 1,
            }
        }
        let due: Vec<OutputDeviceSetting> = self
            .failed
            .iter()
            .filter(|(_, _, since)| since.elapsed() >= MIRROR_RETRY_INTERVAL)
            .map(|(setting, _, _)| setting.clone())
            .collect();
        for setting in due {
            self.failed.retain(|(failed, _, _)| failed.id != setting.id);
            self.start_mirror(setting);
        }
        self.publish_status();
    }

    /// Publish what every chosen device is doing, for the settings page.
    fn publish_status(&mut self) {
        let mut status: Vec<OutputDeviceStatus> = Vec::new();
        let wanted_primary = self.config.primary_device();
        if let Some(output) = self.output.as_ref() {
            let missing = wanted_primary
                .as_deref()
                .is_some_and(|wanted| output.device_id.as_deref() != Some(wanted));
            status.push(OutputDeviceStatus {
                id: wanted_primary.clone(),
                state: DeviceState::Playing,
                message: missing.then(|| "找不到這個裝置，目前改由系統預設裝置播放。".to_owned()),
                device_rate: Some(output.device_rate.get()),
                channels: Some(output.sink.config().channel_count().get()),
                converted: false,
            });
        }
        let (_, track_rate) = self.hub.format();
        for mirror in &self.mirrors {
            status.push(OutputDeviceStatus {
                id: mirror.setting.id.clone(),
                state: DeviceState::Playing,
                message: None,
                device_rate: Some(mirror.rate_hz),
                channels: Some(mirror.channels),
                converted: mirror.rate_hz != track_rate,
            });
        }
        for pending in &self.pending {
            status.push(OutputDeviceStatus {
                id: pending.setting.id.clone(),
                state: DeviceState::Opening,
                message: None,
                device_rate: None,
                channels: None,
                converted: false,
            });
        }
        for (setting, reason, _) in &self.failed {
            status.push(OutputDeviceStatus {
                id: setting.id.clone(),
                state: DeviceState::Lost,
                message: Some(reason.clone()),
                device_rate: None,
                channels: None,
                converted: false,
            });
        }
        if status != self.reported_status {
            self.config.set_device_status(status.clone());
            self.reported_status = status;
        }
    }

    /// Bring the stream to the rate the current mode wants for a track at
    /// `source_rate`. Returns true when the output (and its mixer) was
    /// replaced; the track's previous player is gone in that case.
    fn prepare_output_for(&mut self, source_rate: SampleRate) -> bool {
        self.config.set_track_rate_hint(source_rate.get());
        let Some(output) = self.output.as_ref() else {
            return false;
        };
        let refused_rate = self.fallback.as_ref().map(|fallback| fallback.rate);
        let plan = plan_output(
            self.mode,
            source_rate,
            output.sample_rate,
            output.mix,
            refused_rate,
        );
        let output_is_mix = output.mix;
        if self.mode == ResamplingMode::WindowsBuiltin && refused_rate != Some(source_rate) {
            // Either the stream already runs at this rate or a fresh attempt
            // follows; an older refusal no longer describes this track.
            self.fallback = None;
        }
        match plan {
            OutputPlan::Keep => false,
            OutputPlan::OpenAt(rate) => {
                match open_output_with_timeout(
                    Some(rate),
                    OUTPUT_OPEN_TIMEOUT,
                    &self.config.primary_device(),
                ) {
                    Ok(output) => {
                        self.replace_output(output);
                        true
                    }
                    Err(error) => {
                        let reason = error.to_string();
                        log_refused_rate(rate, &reason);
                        self.fallback = Some(OutputFallback { rate, reason });
                        !output_is_mix && self.reopen_mix()
                    }
                }
            }
            OutputPlan::OpenMix => self.reopen_mix(),
        }
    }

    /// Reopen with the device's mix format. On failure the current stream is
    /// kept; it still works, just at a different rate than preferred.
    fn reopen_mix(&mut self) -> bool {
        match open_output_with_timeout(None, OUTPUT_OPEN_TIMEOUT, &self.config.primary_device()) {
            Ok(output) => {
                self.replace_output(output);
                true
            }
            Err(error) => {
                eprintln!("[audio] reopening the output at the mix rate failed: {error}");
                false
            }
        }
    }

    fn replace_output(&mut self, output: OpenOutput) {
        self.track.device_channels = usize::from(output.sink.config().channel_count().get());
        let old_player = self
            .track
            .attach(output.sink.mixer().clone(), output.sample_rate);
        let old_output = self.output.replace(output);
        dispose_off_worker(old_output, old_player);
        let now = Instant::now();
        self.last_heartbeat = (0, now);
        self.last_device_check = now;
    }

    fn plan_for(&mut self, path: &Path, decoder: &AudioDecoder) -> crate::dsee::DseePlan {
        crate::dsee::plan(
            self.dsee_enabled,
            path,
            &track_signal(decoder),
            &mut self.track.dsee_probe,
        )
    }
}

fn log_refused_rate(rate: SampleRate, reason: &str) {
    eprintln!(
        "[audio] warning: the output device could not open a shared-mode stream at {} Hz \
         ({reason}); using the mix rate with high-quality in-app conversion",
        rate.get()
    );
}

/// Dropping a CPAL stream joins its render thread. If a driver call is stuck
/// there, the join would block; hand teardown to a disposable thread so the
/// audio worker always stays responsive.
fn dispose_off_worker(output: Option<OpenOutput>, player: Option<Arc<Player>>) {
    if output.is_none() && player.is_none() {
        return;
    }
    let _ = thread::Builder::new()
        .name("moe-audio-output-dispose".to_owned())
        .spawn(move || {
            drop(player);
            drop(output);
        });
}

/// Closing a mirror's stream joins its render thread; do that off the worker like the main output.
fn dispose_mirror(mut mirror: MirrorOutput) {
    let sink = mirror.sink.take();
    let _ = thread::Builder::new()
        .name("moe-audio-mirror-dispose".to_owned())
        .spawn(move || {
            drop(mirror);
            drop(sink);
        });
}

impl Drop for RodioBackend {
    fn drop(&mut self) {
        for mirror in self.mirrors.drain(..) {
            dispose_mirror(mirror);
        }
        dispose_off_worker(self.output.take(), self.track.player.take());
    }
}

impl AudioBackend for RodioBackend {
    fn probe(&self) -> Option<crate::TrackProbe> {
        Some(decoder_probe(Arc::clone(&self.config.dsd)))
    }

    fn load(&mut self, path: &Path) -> Result<Option<Duration>, AudioError> {
        let decoder = open_decoder_with(path, &self.config.dsd)?;
        // A new load is a new track: its header is read again.
        self.track.dsee_probe.clear();
        let plan = self.plan_for(path, &decoder);
        self.prepare_output_for(plan_rate(&decoder, plan.output_rate));
        self.track.load_decoder(path, decoder, plan.engagement)
    }

    fn play(&mut self) -> Result<(), AudioError> {
        self.track.play()
    }

    fn pause(&mut self) -> Result<(), AudioError> {
        self.track.pause();
        // The mirrors would otherwise play out what they buffered after the main output stopped.
        self.hub.flush_mirrors();
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        self.track.stop();
        self.hub.flush();
        Ok(())
    }

    fn set_output_devices(&mut self, devices: Vec<OutputDeviceSetting>) -> Result<(), AudioError> {
        self.config.set_devices(devices);
        self.apply_devices()
    }

    fn seek(&mut self, position: Duration) -> Result<Duration, AudioError> {
        let output_alive = self.output_alive();
        let outcome = self.track.seek(position, output_alive, SEEK_TIMEOUT)?;
        if outcome.timed_out {
            self.mark_output_failed("seeking did not complete; the output stream is unresponsive");
        }
        Ok(outcome.position)
    }

    fn load_at(
        &mut self,
        path: &Path,
        position: Duration,
    ) -> Result<(Option<Duration>, Duration), AudioError> {
        let decoder = open_decoder_with(path, &self.config.dsd)?;
        let plan = self.plan_for(path, &decoder);
        self.prepare_output_for(plan_rate(&decoder, plan.output_rate));
        self.track
            .load_decoder_at(path, decoder, position, plan.engagement)
    }

    fn set_resampling_mode(&mut self, mode: ResamplingMode) -> Result<(), AudioError> {
        self.config.set_mode(mode);
        if mode == self.mode {
            return Ok(());
        }
        self.mode = mode;
        self.fallback = None;
        // Capture the live position before the output (and player) changes.
        let resume = self.track.resume_point();
        let rate = self.track.playback_rate.or(self.track.source_rate);
        let replaced = match rate {
            Some(rate) if resume.is_some() => self.prepare_output_for(rate),
            // Nothing is playing; HQ only needs the stream back at the mix
            // rate, and Windows-builtin reopens when the next track loads.
            _ => match (mode, self.output.as_ref()) {
                (ResamplingMode::HighQuality, Some(output)) if !output.mix => self.reopen_mix(),
                _ => false,
            },
        };
        if replaced {
            if let Some((path, position)) = resume {
                self.track.seek_by_reopening(&path, position)?;
            }
        }
        Ok(())
    }

    fn set_dsee_hx(&mut self, enabled: bool) -> Result<(), AudioError> {
        self.config.set_dsee_hx(enabled);
        self.dsee_enabled = enabled;
        self.track.dsee_enabled = enabled;
        let Some((path, position)) = self.track.resume_point() else {
            return Ok(());
        };
        let was_playing = !self.track.paused;
        let decoder = open_decoder_with(&path, &self.config.dsd)?;
        let plan = self.plan_for(&path, &decoder);
        self.prepare_output_for(plan_rate(&decoder, plan.output_rate));
        self.track
            .load_decoder_at(&path, decoder, position, plan.engagement)?;
        if was_playing {
            self.track.play()?;
        }
        Ok(())
    }

    fn set_dsd_settings(&mut self, settings: DsdSettings) -> Result<(), AudioError> {
        let before = self.config.dsd.settings();
        self.config.dsd.set(settings);
        // The gain is followed live by the sources that are playing; only a new output rate needs a reload.
        if before.output_rate == self.config.dsd.settings().output_rate {
            return Ok(());
        }
        let Some((path, position)) = self.track.resume_point() else {
            return Ok(());
        };
        if !crate::dsd::is_dsd_path(&path) {
            return Ok(());
        }
        let was_playing = !self.track.paused;
        let decoder = open_decoder_with(&path, &self.config.dsd)?;
        let plan = self.plan_for(&path, &decoder);
        self.prepare_output_for(plan_rate(&decoder, plan.output_rate));
        self.track
            .load_decoder_at(&path, decoder, position, plan.engagement)?;
        if was_playing {
            self.track.play()?;
        }
        Ok(())
    }

    fn resampling_info(&self) -> Option<ResamplingInfo> {
        let output = self.output.as_ref()?;
        let file_rate = self.track.loaded_source_rate();
        let playback_rate = self.track.playback_rate.or(file_rate);
        let fallback = match (self.mode, self.fallback.as_ref(), playback_rate) {
            (ResamplingMode::WindowsBuiltin, Some(fallback), Some(rate))
                if fallback.rate == rate =>
            {
                Some(fallback.reason.clone())
            }
            _ => None,
        };
        Some(ResamplingInfo {
            mode: self.mode,
            source_rate: file_rate.map(SampleRate::get),
            output_rate: output.sample_rate.get(),
            device_rate: output.device_rate.get(),
            conversion: conversion_for(
                playback_rate,
                output.sample_rate,
                output.device_rate,
                self.track.in_app,
            ),
            fallback_reason: fallback,
            dsee_hx: self.track.dsee_state,
            dsee_notice: self.track.dsee_notice.clone(),
            exclusive: None,
            exclusive_fallback: None,
            dsee_output_rate: (self.track.dsee_state == DseeHxState::Active)
                .then(|| self.track.playback_rate.map(SampleRate::get))
                .flatten(),
        })
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), AudioError> {
        self.master_volume = volume;
        self.track.set_volume(volume);
        for mirror in &self.mirrors {
            mirror.control.set_gain(volume);
        }
        Ok(())
    }

    fn position(&self) -> Duration {
        self.track.position()
    }

    fn is_empty(&self) -> bool {
        self.track.is_empty()
    }

    fn output_status(&mut self) -> OutputStatus {
        // Exclusive mode was switched on: the worker rebuilds the backend as an exclusive one.
        if self.config.exclusive() && !self.as_fallback {
            return OutputStatus::DefaultDeviceChanged;
        }
        self.poll_mirrors();
        let Some(output) = self.output.as_ref() else {
            return OutputStatus::Lost("output stream is closed".to_owned());
        };
        if let Some(message) = output
            .signals
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return OutputStatus::Lost(message);
        }

        let now = Instant::now();
        let beats = output.signals.heartbeat.load(Ordering::Relaxed);
        if beats != self.last_heartbeat.0 {
            self.last_heartbeat = (beats, now);
        } else if now.duration_since(self.last_heartbeat.1) >= OUTPUT_STALL_TIMEOUT {
            return OutputStatus::Lost("output stream stopped requesting audio".to_owned());
        }

        let generation = crate::device_notify::device_change_generation();
        if generation != self.seen_device_generation
            || now.duration_since(self.last_device_check) >= DEFAULT_DEVICE_POLL_INTERVAL
        {
            self.seen_device_generation = generation;
            self.last_device_check = now;
            if let (Some(opened), Some(default), true) = (
                output.device_id.as_deref(),
                default_output_device_id(),
                self.config.primary_device().is_none(),
            ) {
                if opened != default {
                    return OutputStatus::DefaultDeviceChanged;
                }
            }
        }
        OutputStatus::Healthy
    }
}

pub(crate) fn default_output_device_id() -> Option<String> {
    rodio::cpal::default_host()
        .default_output_device()?
        .id()
        .ok()
        .map(|id| id.to_string())
}

/// Open the default endpoint on a helper thread, giving up after `timeout`.
/// `rate` requests a strict shared-mode rate; `None` uses the mix format.
fn open_output_with_timeout(
    rate: Option<SampleRate>,
    timeout: Duration,
    device: &Option<String>,
) -> Result<OpenOutput, AudioError> {
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let device = device.clone();
    thread::Builder::new()
        .name("moe-audio-output-open".to_owned())
        .spawn(move || {
            // If the worker gave up, the send fails and the late stream is
            // dropped here instead of on the worker.
            let _ = result_tx.send(open_output(rate, device.as_deref()));
        })
        .map_err(|error| AudioError::OutputDevice(error.to_string()))?;
    match result_rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(AudioError::OutputDevice(format!(
            "opening the output device did not finish within {} ms",
            timeout.as_millis()
        ))),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(AudioError::OutputDevice(
            "the output device thread stopped unexpectedly".to_owned(),
        )),
    }
}

/// About 50 ms of audio rounded to a power of two, like Rodio's own default
/// for the device's mix rate.
fn stream_buffer_frames(rate: SampleRate) -> u32 {
    let frames = (rate.get() / 20).max(1);
    let next = frames.next_power_of_two();
    let previous = next >> 1;
    if previous > 0 && frames - previous <= next - frames {
        previous
    } else {
        next
    }
}

/// Open the chosen device, or the system default when none is chosen or the chosen one is gone.
fn open_output(rate: Option<SampleRate>, chosen: Option<&str>) -> Result<OpenOutput, AudioError> {
    let chosen_device = chosen.and_then(find_output_device);
    if chosen.is_some() && chosen_device.is_none() {
        eprintln!("[audio] the chosen output device was not found; using the system default");
    }
    let device = match chosen_device {
        Some(device) => device,
        None => rodio::cpal::default_host()
            .default_output_device()
            .ok_or_else(|| AudioError::OutputDevice("no default output device".to_owned()))?,
    };
    let device_id = device.id().ok().map(|id| id.to_string());
    let device_rate = device
        .default_output_config()
        .ok()
        .and_then(|config| SampleRate::new(config.sample_rate()));
    let signals = Arc::new(OutputSignals::default());
    let builder = DeviceSinkBuilder::from_device(device)
        .map_err(|error| AudioError::OutputDevice(error.to_string()))?
        .with_error_callback(stream_error_callback(Arc::clone(&signals)));
    let mut sink = match rate {
        // CPAL's shared mode enables AUTOCONVERTPCM, so the Windows audio
        // engine converts this rate to the mix format.
        Some(rate) => builder
            .with_sample_rate(rate)
            .with_buffer_size(rodio::cpal::BufferSize::Fixed(stream_buffer_frames(rate)))
            .open_stream(),
        None => builder.open_sink_or_fallback(),
    }
    .map_err(|error| AudioError::OutputDevice(error.to_string()))?;
    let sample_rate = sink.config().sample_rate();
    if let Some(requested) = rate {
        if sample_rate != requested {
            return Err(AudioError::OutputDevice(format!(
                "the stream opened at {} Hz instead of {} Hz",
                sample_rate.get(),
                requested.get()
            )));
        }
    }
    sink.log_on_drop(false);
    sink.mixer().add(Heartbeat {
        channels: sink.config().channel_count(),
        sample_rate,
        signals: Arc::clone(&signals),
        pending: 0,
    });
    Ok(OpenOutput {
        sink,
        device_id,
        signals,
        sample_rate,
        device_rate: device_rate.unwrap_or(sample_rate),
        mix: rate.is_none(),
    })
}

/// Open a mirror device (on a helper thread) and start its source.
fn open_mirror(
    setting: OutputDeviceSetting,
    hub: Arc<MirrorHub>,
    gain: f32,
) -> Result<MirrorOutput, String> {
    let device = match setting.id.as_deref() {
        Some(id) => find_output_device(id).ok_or_else(|| "the device was not found".to_owned())?,
        None => rodio::cpal::default_host()
            .default_output_device()
            .ok_or_else(|| "no default output device".to_owned())?,
    };
    let signals = Arc::new(OutputSignals::default());
    let mut sink = DeviceSinkBuilder::from_device(device)
        .map_err(|error| error.to_string())?
        .with_error_callback(stream_error_callback(Arc::clone(&signals)))
        .open_sink_or_fallback()
        .map_err(|error| error.to_string())?;
    sink.log_on_drop(false);
    let channels = usize::from(sink.config().channel_count().get());
    let rate = sink.config().sample_rate().get();
    let ring = hub.add_ring();
    let control = MirrorControl::new(gain, setting.delay_ms);
    control.set_map(setting.channel_map.clone());
    sink.mixer().add(MirrorSource::new(
        Arc::clone(&ring),
        Arc::clone(&control),
        channels,
        rate,
        setting.channel_map.clone(),
    ));
    sink.mixer().add(Heartbeat {
        channels: sink.config().channel_count(),
        sample_rate: sink.config().sample_rate(),
        signals: Arc::clone(&signals),
        pending: 0,
    });
    Ok(MirrorOutput {
        setting,
        sink: Some(sink),
        ring,
        hub,
        control,
        signals,
        watch: StallWatch::new(Instant::now()),
        rate_hz: rate,
        channels: channels as u16,
    })
}

fn stream_error_callback(
    signals: Arc<OutputSignals>,
) -> impl FnMut(StreamError) + Clone + Send + 'static {
    move |error| {
        // Underruns are glitches, not a dead stream.
        if matches!(error, StreamError::BufferUnderrun) {
            return;
        }
        let mut failure = signals
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if failure.is_none() {
            *failure = Some(error.to_string());
        }
    }
}

/// Silent, endless mixer input whose only job is proving that CPAL still pulls
/// samples. It uses the sink's own format so the mixer does no conversion.
struct Heartbeat {
    channels: ChannelCount,
    sample_rate: SampleRate,
    signals: Arc<OutputSignals>,
    pending: u32,
}

impl Iterator for Heartbeat {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        self.pending += 1;
        if self.pending >= HEARTBEAT_BATCH {
            self.signals
                .heartbeat
                .fetch_add(u64::from(self.pending), Ordering::Relaxed);
            self.pending = 0;
        }
        Some(0.0)
    }
}

impl Source for Heartbeat {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Result of a seek request.
#[derive(Debug, PartialEq, Eq)]
struct SeekOutcome {
    position: Duration,
    /// `Player::try_seek` missed its deadline; the output should be rebuilt.
    timed_out: bool,
}

/// One local track on a mixer, driven only through non-waiting Rodio calls.
struct TrackPlayer {
    /// DSD settings shared with the backend (replaced by the backend's own after construction).
    dsd: Arc<DsdOptions>,
    mixer: Mixer,
    /// The mixer's rate; tracks at other rates are wrapped in [`Resampled`].
    output_rate: SampleRate,
    // Shared only with abandoned seek helpers, which never lock anything.
    player: Option<Arc<Player>>,
    path: Option<PathBuf>,
    /// Track position at which the current `Player` started.
    base: Duration,
    paused: bool,
    volume: f32,
    /// Decoder rate of the loaded file. Status display uses this.
    source_rate: Option<SampleRate>,
    /// Rate of the samples appended to the mixer, after DSEE HX when it is active.
    playback_rate: Option<SampleRate>,
    dsee_enabled: bool,
    /// What DSEE HX learned from the loaded track's header (local or remote), so a seek does not read it again.
    dsee_probe: crate::dsee::ProbeCache,
    dsee_state: DseeHxState,
    dsee_notice: Option<String>,
    /// The current source goes through the in-app high-quality resampler.
    in_app: bool,
    /// Copies of the track for the mirror devices, and the main output's own delay, channel map and volume.
    hub: Arc<MirrorHub>,
    delay_ms: Arc<AtomicU32>,
    map: Option<Vec<ChannelRoute>>,
    device_channels: usize,
    /// Samples of delay-line silence the current player has played, and samples per second of the chain.
    silence: Arc<AtomicU64>,
    silence_rate: f64,
}

impl TrackPlayer {
    fn new(mixer: Mixer, output_rate: SampleRate) -> Self {
        Self {
            hub: MirrorHub::new(),
            delay_ms: Arc::new(AtomicU32::new(0)),
            map: None,
            device_channels: 2,
            silence: Arc::new(AtomicU64::new(0)),
            silence_rate: 0.0,
            dsd: Arc::new(DsdOptions::default()),
            mixer,
            output_rate,
            player: None,
            path: None,
            base: Duration::ZERO,
            paused: true,
            volume: 1.0,
            source_rate: None,
            playback_rate: None,
            dsee_enabled: false,
            dsee_probe: crate::dsee::ProbeCache::default(),
            dsee_state: DseeHxState::Off,
            dsee_notice: None,
            in_app: false,
        }
    }

    #[cfg(test)]
    fn load(&mut self, path: &Path) -> Result<Option<Duration>, AudioError> {
        let decoder = open_decoder_with(path, &self.dsd)?;
        self.load_decoder(path, decoder, crate::dsee::Engagement::Off)
    }

    fn load_decoder(
        &mut self,
        path: &Path,
        decoder: AudioDecoder,
        engagement: crate::dsee::Engagement,
    ) -> Result<Option<Duration>, AudioError> {
        let duration = decoder.total_duration();
        self.paused = true;
        self.start(path, decoder, Duration::ZERO, engagement)?;
        self.path = Some(path.to_path_buf());
        Ok(duration)
    }

    #[cfg(test)]
    fn load_at(
        &mut self,
        path: &Path,
        position: Duration,
    ) -> Result<(Option<Duration>, Duration), AudioError> {
        let decoder = open_decoder_with(path, &self.dsd)?;
        self.load_decoder_at(path, decoder, position, crate::dsee::Engagement::Off)
    }

    /// Load paused at `position` by seeking the decoder before it is attached,
    /// so no render callback is involved. Used when rebuilding the output.
    fn load_decoder_at(
        &mut self,
        path: &Path,
        mut decoder: AudioDecoder,
        position: Duration,
        engagement: crate::dsee::Engagement,
    ) -> Result<(Option<Duration>, Duration), AudioError> {
        let duration = decoder.total_duration();
        let target = duration.map_or(position, |duration| position.min(duration));
        let actual = if target.is_zero() {
            Duration::ZERO
        } else if decoder.try_seek(target).is_ok() {
            target
        } else {
            // A failed seek may leave the decoder mid-stream; start over.
            decoder = open_decoder_with(path, &self.dsd)?;
            Duration::ZERO
        };
        self.paused = true;
        self.start(path, decoder, actual, engagement)?;
        self.path = Some(path.to_path_buf());
        Ok((duration, actual))
    }

    fn play(&mut self) -> Result<(), AudioError> {
        let Some(player) = self.player.as_ref() else {
            return Err(AudioError::NoTrackLoaded);
        };
        self.paused = false;
        player.play();
        Ok(())
    }

    fn pause(&mut self) {
        self.paused = true;
        if let Some(player) = self.player.as_ref() {
            player.pause();
        }
    }

    fn stop(&mut self) {
        if let Some(player) = self.player.take() {
            player.stop();
        }
        self.path = None;
        self.base = Duration::ZERO;
        self.paused = true;
        self.source_rate = None;
        self.playback_rate = None;
        self.dsee_state = DseeHxState::Off;
        self.dsee_notice = None;
        self.in_app = false;
    }

    /// Move to a new output mixer. The old player is returned for disposal;
    /// the caller attaches a fresh source afterwards.
    fn attach(&mut self, mixer: Mixer, output_rate: SampleRate) -> Option<Arc<Player>> {
        self.mixer = mixer;
        self.output_rate = output_rate;
        let previous = self.player.take();
        if let Some(previous) = previous.as_ref() {
            previous.stop();
        }
        previous
    }

    /// Track and position to resume from when the output must be swapped
    /// under a live player (a mode switch). `None` once the track finished.
    fn resume_point(&self) -> Option<(PathBuf, Duration)> {
        let path = self.path.clone()?;
        if self.is_empty() {
            return None;
        }
        Some((path, self.position()))
    }

    fn loaded_source_rate(&self) -> Option<SampleRate> {
        self.path.as_ref().and(self.source_rate)
    }

    fn seek(
        &mut self,
        position: Duration,
        output_alive: bool,
        timeout: Duration,
    ) -> Result<SeekOutcome, AudioError> {
        let Some(path) = self.path.clone() else {
            return Err(AudioError::NoTrackLoaded);
        };
        if self.dsee_state == DseeHxState::Active {
            let position = self.seek_by_reopening(&path, position)?;
            return Ok(SeekOutcome {
                position,
                timed_out: false,
            });
        }
        if output_alive {
            if let Some(player) = self.player.as_ref() {
                match try_seek_with_deadline(Arc::clone(player), position, timeout) {
                    Some(Ok(())) => {
                        // TrackPosition reports the absolute file position
                        // after a seek, so the start offset no longer applies.
                        self.base = Duration::ZERO;
                        return Ok(SeekOutcome {
                            position: player.get_pos(),
                            timed_out: false,
                        });
                    }
                    Some(Err(error)) => return Err(AudioError::Seek(error.to_string())),
                    None => {
                        let position = self.seek_by_reopening(&path, position)?;
                        return Ok(SeekOutcome {
                            position,
                            timed_out: true,
                        });
                    }
                }
            }
        }
        let position = self.seek_by_reopening(&path, position)?;
        Ok(SeekOutcome {
            position,
            timed_out: false,
        })
    }

    /// Fallback seek that never touches the render callback: reopen the file,
    /// seek the decoder here, and attach it to a fresh `Player` that keeps the
    /// current play/pause state and volume.
    fn seek_by_reopening(
        &mut self,
        path: &Path,
        position: Duration,
    ) -> Result<Duration, AudioError> {
        let mut decoder = open_decoder_with(path, &self.dsd)?;
        let plan = crate::dsee::plan(
            self.dsee_enabled,
            path,
            &track_signal(&decoder),
            &mut self.dsee_probe,
        );
        decoder
            .try_seek(position)
            .map_err(|error| AudioError::Seek(error.to_string()))?;
        self.start(path, decoder, position, plan.engagement)?;
        Ok(position)
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        if let Some(player) = self.player.as_ref() {
            player.set_volume(volume);
        }
    }

    fn position(&self) -> Duration {
        match self.player.as_ref() {
            Some(player) => {
                // The delay line's leading silence is not part of the track.
                let silence = if self.silence_rate > 0.0 {
                    Duration::from_secs_f64(
                        self.silence.load(Ordering::Relaxed) as f64 / self.silence_rate,
                    )
                } else {
                    Duration::ZERO
                };
                self.base + player.get_pos().saturating_sub(silence)
            }
            None => self.base,
        }
    }

    /// Hand a track's samples to `player` through the tee (copies for the mirrors), the delay line and, when the
    /// device has a channel map, the channel mixer.
    fn append_chain<S>(&mut self, player: &Player, source: S)
    where
        S: Source<Item = Sample> + Send + 'static,
    {
        let channels = usize::from(source.channels().get());
        self.silence = Arc::new(AtomicU64::new(0));
        self.silence_rate = f64::from(source.sample_rate().get()) * channels as f64;
        let tee = Tee::new(source, Arc::clone(&self.hub));
        let delayed = DelayLine::new(
            tee,
            Arc::clone(&self.delay_ms),
            Arc::clone(&self.hub),
            Arc::clone(&self.silence),
        );
        match self.map.as_deref() {
            Some(map) => {
                let mixer = ChannelMixer::new(Some(map), channels, self.device_channels);
                // The silence the mixer's output counts is per output channel.
                self.silence_rate = f64::from(delayed.sample_rate().get()) * channels as f64;
                player.append(MixSource::new(delayed, mixer));
            }
            None => player.append(delayed),
        }
    }

    fn is_empty(&self) -> bool {
        self.player.as_ref().is_none_or(|player| player.empty())
    }

    fn start(
        &mut self,
        path: &Path,
        decoder: AudioDecoder,
        base: Duration,
        engagement: crate::dsee::Engagement,
    ) -> Result<(), AudioError> {
        // The filter has to deliver its first samples before this player is
        // attached. Connecting an empty player first lets the live callback
        // sit in the mixer while DSEE HX is still starting, and the prefill
        // then times out.
        let file_rate = decoder.sample_rate();
        self.source_rate = Some(file_rate);
        if matches!(engagement, crate::dsee::Engagement::Run { .. }) {
            if let Some(previous) = self.player.take() {
                previous.stop();
            }
        }
        let playback = self.attach_dsee(path, decoder, base, engagement)?;
        let playback_rate = playback.sample_rate();
        self.playback_rate = Some(playback_rate);
        let player = Player::connect_new(&self.mixer);
        player.set_volume(self.volume);
        if self.paused {
            player.pause();
        }
        self.in_app = false;
        if needs_resampling(playback_rate, self.output_rate) {
            match Resampled::new(playback, self.output_rate) {
                Ok(resampled) => {
                    self.append_chain(&player, resampled);
                    self.in_app = true;
                }
                Err(rejected) => {
                    eprintln!(
                        "[audio] high-quality resampler unavailable for {} -> {} Hz ({}); \
                         using Rodio's converter",
                        playback_rate.get(),
                        self.output_rate.get(),
                        rejected.error
                    );
                    self.append_chain(&player, rejected.source);
                }
            }
        } else {
            self.append_chain(&player, playback);
        }
        if let Some(previous) = self.player.replace(Arc::new(player)) {
            previous.stop();
        }
        self.base = base;
        Ok(())
    }

    fn attach_dsee(
        &mut self,
        path: &Path,
        decoder: AudioDecoder,
        base: Duration,
        engagement: crate::dsee::Engagement,
    ) -> Result<PlaybackSource, AudioError> {
        match engagement {
            crate::dsee::Engagement::Run {
                bitrate_kbps,
                input_rate,
                input_bits,
            } => match crate::dsee::DseeSource::open(decoder, input_rate, bitrate_kbps, input_bits)
            {
                Ok(source) => {
                    self.dsee_state = DseeHxState::Active;
                    self.dsee_notice = None;
                    Ok(PlaybackSource::Dsee(source))
                }
                Err(crate::dsee::OpenError::Rejected(decoder, notice)) => {
                    self.dsee_state = DseeHxState::Unavailable;
                    self.dsee_notice = Some(notice);
                    Ok(PlaybackSource::Direct(decoder))
                }
                Err(crate::dsee::OpenError::Lost(notice)) => {
                    self.dsee_state = DseeHxState::Unavailable;
                    self.dsee_notice = Some(notice);
                    let mut decoder = open_decoder_with(path, &self.dsd)?;
                    if !base.is_zero() {
                        let _ = decoder.try_seek(base);
                    }
                    Ok(PlaybackSource::Direct(decoder))
                }
            },
            other => {
                let (state, notice) = crate::dsee::state_of(&other);
                self.dsee_state = state;
                self.dsee_notice = notice;
                Ok(PlaybackSource::Direct(decoder))
            }
        }
    }
}

type FileDecoder = AudioDecoder;

/// Whatever decodes the current file: rodio/Symphonia for most formats, the DSD→PCM converter for DSF and libopus for
/// Opus (`player_formats::decode_engine`).
pub(crate) enum AudioDecoder {
    Rodio(Decoder<BufReader<Box<dyn crate::remote_input::MediaRead>>>),
    /// Boxed: the converter state is several times larger than a rodio decoder.
    Dsd(Box<crate::dsd::DsdFileSource>),
    Opus(Box<crate::opus::OpusFileSource>),
    WavPack(Box<crate::wavpack::WavPackSource>),
}

impl Iterator for AudioDecoder {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        match self {
            Self::Rodio(source) => source.next(),
            Self::Dsd(source) => source.next(),
            Self::Opus(source) => source.next(),
            Self::WavPack(source) => source.next(),
        }
    }
}

impl Source for AudioDecoder {
    fn current_span_len(&self) -> Option<usize> {
        match self {
            Self::Rodio(source) => source.current_span_len(),
            Self::Dsd(source) => source.current_span_len(),
            Self::Opus(source) => source.current_span_len(),
            Self::WavPack(source) => source.current_span_len(),
        }
    }

    fn channels(&self) -> ChannelCount {
        match self {
            Self::Rodio(source) => source.channels(),
            Self::Dsd(source) => source.channels(),
            Self::Opus(source) => source.channels(),
            Self::WavPack(source) => source.channels(),
        }
    }

    fn sample_rate(&self) -> SampleRate {
        match self {
            Self::Rodio(source) => source.sample_rate(),
            Self::Dsd(source) => source.sample_rate(),
            Self::Opus(source) => source.sample_rate(),
            Self::WavPack(source) => source.sample_rate(),
        }
    }

    fn total_duration(&self) -> Option<Duration> {
        match self {
            Self::Rodio(source) => source.total_duration(),
            Self::Dsd(source) => source.total_duration(),
            Self::Opus(source) => source.total_duration(),
            Self::WavPack(source) => source.total_duration(),
        }
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        match self {
            Self::Rodio(source) => source.try_seek(position),
            Self::Dsd(source) => source.try_seek(position),
            Self::Opus(source) => source.try_seek(position),
            Self::WavPack(source) => source.try_seek(position),
        }
    }
}

enum PlaybackSource {
    Direct(FileDecoder),
    Dsee(crate::dsee::DseeSource<FileDecoder>),
}

impl Iterator for PlaybackSource {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        match self {
            Self::Direct(source) => source.next(),
            Self::Dsee(source) => source.next(),
        }
    }
}

impl Source for PlaybackSource {
    fn current_span_len(&self) -> Option<usize> {
        match self {
            Self::Direct(source) => source.current_span_len(),
            Self::Dsee(source) => source.current_span_len(),
        }
    }

    fn channels(&self) -> ChannelCount {
        match self {
            Self::Direct(source) => source.channels(),
            Self::Dsee(source) => source.channels(),
        }
    }

    fn sample_rate(&self) -> SampleRate {
        match self {
            Self::Direct(source) => source.sample_rate(),
            Self::Dsee(source) => source.sample_rate(),
        }
    }

    fn total_duration(&self) -> Option<Duration> {
        match self {
            Self::Direct(source) => source.total_duration(),
            Self::Dsee(source) => source.total_duration(),
        }
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        match self {
            Self::Direct(source) => source.try_seek(position),
            Self::Dsee(source) => source.try_seek(position),
        }
    }
}

fn track_signal(decoder: &FileDecoder) -> crate::dsee::TrackSignal {
    crate::dsee::TrackSignal {
        sample_rate: decoder.sample_rate().get(),
        channels: decoder.channels().get(),
        duration: decoder.total_duration(),
    }
}

fn plan_rate(decoder: &FileDecoder, output_rate: u32) -> SampleRate {
    SampleRate::new(output_rate).unwrap_or(decoder.sample_rate())
}

/// Run `Player::try_seek` on a helper thread. `None` means the deadline
/// passed; the helper and its `Arc<Player>` are abandoned and finish on their
/// own once the stream drains or is dropped (the seek feedback channel closes).
fn try_seek_with_deadline(
    player: Arc<Player>,
    position: Duration,
    timeout: Duration,
) -> Option<Result<(), rodio::source::SeekError>> {
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let spawned = thread::Builder::new()
        .name("moe-audio-seek".to_owned())
        .spawn(move || {
            let _ = result_tx.send(player.try_seek(position));
        });
    if spawned.is_err() {
        return None;
    }
    result_rx.recv_timeout(timeout).ok()
}

/// Open a file with the default DSD settings (tests and fixtures).
#[cfg(test)]
fn open_decoder(path: &Path) -> Result<AudioDecoder, AudioError> {
    open_decoder_with(path, &DsdOptions::default())
}

/// The check of a file before it is loaded: its decoder opens (headers read, format recognised), then it is dropped.
pub(crate) fn decoder_probe(dsd: Arc<DsdOptions>) -> crate::TrackProbe {
    Arc::new(move |path| open_decoder_with(path, &dsd).map(drop))
}

pub(crate) fn open_decoder_with(path: &Path, dsd: &DsdOptions) -> Result<AudioDecoder, AudioError> {
    match player_formats::decode_engine(path) {
        player_formats::DecodeEngine::Dsd => {
            return crate::dsd::open_dsf(path, dsd)
                .map(|source| AudioDecoder::Dsd(Box::new(source)));
        }
        player_formats::DecodeEngine::Opus => {
            return crate::opus::open_opus(path).map(|source| AudioDecoder::Opus(Box::new(source)));
        }
        player_formats::DecodeEngine::WavPack => {
            // DSD inside WavPack goes to the DSD converter like a DSF file (ADR 0025).
            if crate::dsd::is_dsd_path(path) {
                return crate::dsd::open_wavpack_dsd(path, dsd)
                    .map(|source| AudioDecoder::Dsd(Box::new(source)));
            }
            return crate::wavpack::open_wavpack(path)
                .map(|source| AudioDecoder::WavPack(Box::new(source)));
        }
        player_formats::DecodeEngine::Rodio => {}
    }
    // Ogg files may hold Vorbis (rodio) or Opus: look at the first packet.
    let ogg_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("ogg") || extension.eq_ignore_ascii_case("oga")
        });
    let is_opus = ogg_extension
        && if crate::remote_input::is_remote(path) {
            crate::remote_input::open(path).is_ok_and(|input| player_opus::sniff_opus(input.main))
        } else {
            player_opus::sniff_opus_file(path)
        };
    if is_opus {
        return crate::opus::open_opus(path).map(|source| AudioDecoder::Opus(Box::new(source)));
    }
    let (reader, len): (Box<dyn crate::remote_input::MediaRead>, u64) =
        if crate::remote_input::is_remote(path) {
            let input = crate::remote_input::open(path)?;
            (
                Box::new(crate::remote_input::SyncReader::new(input.main)),
                input.len,
            )
        } else {
            let file = File::open(path).map_err(|error| AudioError::FileOpen {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
            let len = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            (Box::new(file), len)
        };
    Decoder::builder()
        .with_data(BufReader::new(reader))
        .with_byte_len(len)
        .with_seekable(true)
        .build()
        .map(AudioDecoder::Rodio)
        .map_err(|error| match error {
            rodio::decoder::DecoderError::UnrecognizedFormat => AudioError::UnsupportedFormat {
                path: path.to_path_buf(),
            },
            _ => AudioError::Decode {
                path: path.to_path_buf(),
                message: error.to_string(),
            },
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU16;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    /// Symphonia's FLAC demuxer reports the end of the stream for a seek to the last sample (ADR 0036). Whatever a
    /// decoder refuses near the end must fall inside the tail window, so the worker ends the track instead of failing.
    #[test]
    fn decoders_that_refuse_a_seek_do_so_only_in_the_tail() {
        for name in [
            "seek-tone.flac",
            "seek-tone.mp3",
            "seek-tone.ogg",
            "seek-tone.wav",
            "aac-lc.m4a",
        ] {
            let total = open_decoder(&fixture(name))
                .unwrap()
                .total_duration()
                .expect("fixture duration");
            for back_ms in [0_u64, 1, 5, 20, 100, 500, 900] {
                let target = total.saturating_sub(Duration::from_millis(back_ms));
                let mut decoder = open_decoder(&fixture(name)).unwrap();
                if decoder.try_seek(target).is_err() {
                    assert!(
                        player_core::seek_failure_ends_track(target, Some(total)),
                        "{name}: a refused seek {back_ms} ms before the end"
                    );
                }
            }
            let mut decoder = open_decoder(&fixture(name)).unwrap();
            decoder
                .try_seek(total / 2)
                .unwrap_or_else(|error| panic!("{name}: mid-file seek: {error}"));
        }
    }

    /// Write a stereo Opus file (a tone per channel) to a temporary path; returns it and its length in frames.
    fn write_test_opus(name: &str, seconds: f64) -> (PathBuf, usize) {
        let frames = (48_000.0 * seconds) as usize;
        let path = std::env::temp_dir().join(format!("moe-{name}-{}.opus", std::process::id()));
        std::fs::write(
            &path,
            player_opus::testing::write_opus(frames, 2, 1_000.0, 0, 0),
        )
        .unwrap();
        (path, frames)
    }

    /// Write a stereo DSD64 file (1 kHz and 3 kHz tones) to a temporary path.
    fn write_test_dsf(name: &str, seconds: f64) -> (PathBuf, usize) {
        use player_dsd::testing::{build_dsf, modulate, tone};
        let rate = 2_822_400_u32;
        let samples = (f64::from(rate) * seconds) as usize / 16 * 16;
        let lsb = |bytes: Vec<u8>| bytes.into_iter().map(u8::reverse_bits).collect::<Vec<u8>>();
        let left = lsb(modulate(samples, tone(rate, 1_000.0, 0.25)));
        let right = lsb(modulate(samples, tone(rate, 3_000.0, 0.25)));
        let path = std::env::temp_dir().join(format!("moe-{name}-{}.dsf", std::process::id()));
        std::fs::write(
            &path,
            build_dsf(rate, samples as u64, 4096, true, &[left, right]),
        )
        .unwrap();
        (path, samples)
    }

    #[test]
    fn dsf_files_open_as_176_4_khz_pcm_and_seek() {
        let (path, samples) = write_test_dsf("open", 0.2);
        let mut decoder = open_decoder(&path).expect("DSF opens");
        assert!(matches!(decoder, AudioDecoder::Dsd(_)));
        assert_eq!(decoder.sample_rate().get(), 176_400);
        assert_eq!(decoder.channels().get(), 2);
        let frames = samples / 16;
        assert_eq!(
            decoder.total_duration().unwrap(),
            Duration::from_secs_f64(frames as f64 / 176_400.0)
        );
        assert_eq!(decoder.by_ref().count(), frames * 2);
        decoder.try_seek(Duration::from_millis(50)).expect("seek");
        assert_eq!(decoder.by_ref().count(), (frames - 8_820) * 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn dsd_inside_wavpack_plays_through_the_dsd_converter_at_176_4_khz() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../player-wavpack/tests/fixtures/dsd64.wv");
        let mut decoder = open_decoder(&path).expect("DSD WavPack opens");
        assert!(matches!(decoder, AudioDecoder::Dsd(_)));
        assert_eq!(decoder.sample_rate().get(), 176_400);
        assert_eq!(decoder.channels().get(), 2);
        // 0.2 s of DSD64: 2 822 400 / 5 bits per channel, 16 bits per output frame.
        let frames = 2_822_400 / 5 / 16;
        assert_eq!(
            decoder.total_duration().unwrap(),
            Duration::from_secs_f64(frames as f64 / 176_400.0)
        );
        let samples: Vec<f32> = decoder.by_ref().collect();
        assert_eq!(samples.len(), frames * 2);
        assert!(samples.iter().all(|sample| sample.is_finite()));
        assert!(
            samples.iter().any(|sample| sample.abs() > 0.1),
            "the tones are there"
        );
        decoder.try_seek(Duration::from_millis(50)).expect("seek");
        assert_eq!(decoder.by_ref().count(), (frames - 8_820) * 2);
    }

    #[test]
    fn wavpack_files_are_routed_to_the_wavpack_decoder() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../player-wavpack/tests/fixtures/s24.wv");
        let decoder = open_decoder(&path).expect("WavPack opens");
        assert!(matches!(decoder, AudioDecoder::WavPack(_)));
        assert_eq!(decoder.sample_rate().get(), 96_000);
        assert_eq!(decoder.channels().get(), 2);
    }

    #[test]
    fn opus_files_open_as_48_khz_pcm_and_seek() {
        let (path, frames) = write_test_opus("open", 2.0);
        let mut decoder = open_decoder(&path).expect("Opus opens");
        assert!(matches!(decoder, AudioDecoder::Opus(_)));
        assert_eq!(decoder.sample_rate().get(), 48_000);
        assert_eq!(decoder.channels().get(), 2);
        assert_eq!(
            decoder.total_duration().unwrap(),
            Duration::from_secs_f64(frames as f64 / 48_000.0)
        );
        assert_eq!(decoder.by_ref().count(), frames * 2);
        decoder.try_seek(Duration::from_millis(500)).expect("seek");
        assert_eq!(decoder.by_ref().count(), (frames - 24_000) * 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_opus_stream_in_an_ogg_file_is_sniffed_and_a_vorbis_ogg_still_goes_to_rodio() {
        let (opus_path, _) = write_test_opus("sniff", 0.5);
        let renamed = opus_path.with_extension("ogg");
        std::fs::rename(&opus_path, &renamed).unwrap();
        assert!(matches!(open_decoder(&renamed), Ok(AudioDecoder::Opus(_))));
        let _ = std::fs::remove_file(renamed);
        // The shipped Vorbis fixture must keep using rodio.
        let vorbis = fixture("seek-tone.ogg");
        if vorbis.exists() {
            assert!(matches!(open_decoder(&vorbis), Ok(AudioDecoder::Rodio(_))));
        }
    }

    #[test]
    fn a_corrupt_dsf_reports_a_decode_error_not_a_panic() {
        let path = std::env::temp_dir().join(format!("moe-bad-{}.dsf", std::process::id()));
        std::fs::write(&path, b"DSD but not really").unwrap();
        assert!(matches!(
            open_decoder(&path),
            Err(AudioError::Decode { .. })
        ));
        let _ = std::fs::remove_file(path);
    }

    /// Plays every DSF in the owner's library folder for a moment (`-- --ignored`; the folder must exist).
    #[test]
    #[ignore = "needs the DSD folder on drive D"]
    fn real_dsf_files_decode_and_seek() {
        let root = PathBuf::from(r"D:\Music\DSD");
        let mut checked = 0;
        let mut stack = vec![root];
        while let Some(directory) = stack.pop() {
            for entry in std::fs::read_dir(&directory).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|extension| extension == "dsf")
                    && checked < 8
                {
                    let mut decoder =
                        open_decoder(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
                    let duration = decoder.total_duration().expect("duration");
                    let head: Vec<f32> = decoder.by_ref().take(176_400 * 2).collect();
                    let rms = (head
                        .iter()
                        .map(|sample| f64::from(*sample).powi(2))
                        .sum::<f64>()
                        / head.len() as f64)
                        .sqrt();
                    let peak = head
                        .iter()
                        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
                    decoder.try_seek(duration / 2).expect("seek to the middle");
                    let middle = decoder.by_ref().take(4_000).count();
                    println!("{path:?}: {duration:?}, 1 s RMS {rms:.4}, peak {peak:.3}, middle frames {middle}");
                    assert!(head.iter().all(|sample| sample.is_finite()));
                    assert_eq!(middle, 4_000);
                    checked += 1;
                }
            }
        }
        assert!(checked > 0, "no DSF files found");
    }

    /// Decodes real Opus files from the owner's library (`-- --ignored`): the decoded length must match the stated
    /// duration, the signal must be finite and non-silent, and seeking to the middle must work.
    #[test]
    #[ignore = "needs Opus files in the library folder on drive D"]
    fn real_opus_files_decode_and_seek() {
        let mut files = Vec::new();
        let mut stack = vec![PathBuf::from(r"D:\Music\Loss")];
        while let Some(directory) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path
                    .extension()
                    .is_some_and(|extension| extension == "opus")
                {
                    files.push(path);
                }
            }
        }
        files.sort();
        assert!(!files.is_empty(), "no .opus files found");
        for path in files.iter().take(6) {
            let mut decoder =
                open_decoder(path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
            let duration = decoder.total_duration().expect("duration");
            let channels = usize::from(decoder.channels().get());
            let samples: Vec<f32> = decoder.by_ref().collect();
            let frames = samples.len() / channels;
            let expected = (duration.as_secs_f64() * 48_000.0).round() as usize;
            let peak = samples
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            println!(
                "{path:?}: {duration:?}, {frames} frames (expected {expected}), peak {peak:.3}"
            );
            assert_eq!(frames, expected);
            assert!(samples.iter().all(|sample| sample.is_finite()));
            assert!(peak > 0.01);
            decoder.try_seek(duration / 2).expect("seek to the middle");
            assert_eq!(decoder.by_ref().take(4_000).count(), 4_000);
        }
    }

    /// Whole-track peak and RMS of up to 24 DSF files spread over the library folder (`--release -- --ignored`),
    /// used to pick the default DSD gain.
    #[test]
    #[ignore = "needs the DSD folder on drive D; run with --release"]
    fn real_dsf_peak_levels() {
        let mut files = Vec::new();
        let mut stack = vec![PathBuf::from(r"D:\Music\DSD")];
        while let Some(directory) = stack.pop() {
            for entry in std::fs::read_dir(&directory).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|extension| extension == "dsf") {
                    files.push(path);
                }
            }
        }
        files.sort();
        let step = (files.len() / 24).max(1);
        let mut peaks = Vec::new();
        for path in files.iter().step_by(step) {
            let decoder = open_decoder(path).unwrap();
            let (mut peak, mut sum, mut count) = (0.0_f32, 0.0_f64, 0_u64);
            for sample in decoder {
                peak = peak.max(sample.abs());
                sum += f64::from(sample).powi(2);
                count += 1;
            }
            let rms = (sum / count as f64).sqrt();
            println!(
                "peak {:+.2} dBFS, RMS {:+.2} dBFS  {:?}",
                20.0 * f64::from(peak).log10(),
                20.0 * rms.log10(),
                path.file_name().unwrap()
            );
            peaks.push(peak);
        }
        peaks.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "median peak {:+.2} dBFS, max peak {:+.2} dBFS over {} files",
            20.0 * f64::from(peaks[peaks.len() / 2]).log10(),
            20.0 * f64::from(*peaks.last().unwrap()).log10(),
            peaks.len()
        );
    }

    #[test]
    fn matroska_flac_and_ogg_vorbis_decode_and_seek() {
        for name in ["metadata-smoke.mka", "metadata-smoke.ogg"] {
            let mut decoder = open_decoder(&fixture(name)).expect("audio decoder opens");
            assert_eq!(decoder.sample_rate().get(), 24_000, "{name}");
            let duration = decoder.total_duration().expect("container duration");
            assert!((Duration::from_millis(490)..=Duration::from_millis(510)).contains(&duration));
            assert!(
                decoder.next().is_some(),
                "decoder yields samples for {name}"
            );
            decoder
                .try_seek(Duration::from_millis(250))
                .expect("container seek");
            assert!(
                decoder.next().is_some(),
                "decoder resumes after seek for {name}"
            );
        }
    }

    /// A mixer whose output is never pulled behaves exactly like a WASAPI
    /// stream whose render thread exited after AUDCLNT_E_DEVICE_INVALIDATED.
    const MIXER_RATE: SampleRate = SampleRate::new(48_000).unwrap();

    fn rate(value: u32) -> SampleRate {
        SampleRate::new(value).unwrap()
    }

    fn dead_stream_mixer() -> (Mixer, rodio::mixer::MixerSource) {
        rodio::mixer::mixer(NonZeroU16::new(2).unwrap(), MIXER_RATE)
    }

    fn finishes_within<T: Send + 'static>(
        timeout: Duration,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(work());
        });
        rx.recv_timeout(timeout).ok()
    }

    fn backend(mode: ResamplingMode) -> RodioBackend {
        RodioBackend::new(Arc::new(ResamplingConfig::new(mode, false)))
            .expect("default output device")
    }

    /// Write a 16-bit stereo sine WAV for real-device tests.
    fn write_tone(path: &Path, sample_rate: u32, seconds: u32) {
        let frames = sample_rate * seconds;
        let data_len = frames * 4;
        let mut bytes = Vec::with_capacity(44 + data_len as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * 4).to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for frame in 0..frames {
            let phase =
                2.0 * std::f64::consts::PI * 440.0 * f64::from(frame) / f64::from(sample_rate);
            let sample = ((phase.sin() * 8_000.0) as i16).to_le_bytes();
            bytes.extend_from_slice(&sample);
            bytes.extend_from_slice(&sample);
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    #[ignore = "opens the real default output device (muted); run manually"]
    fn real_default_device_stays_healthy_and_seeks_in_place() {
        let mut backend = backend(ResamplingMode::HighQuality);
        backend.set_volume(0.0).unwrap();
        backend.load(&fixture("seek-tone.wav")).unwrap();
        backend.play().unwrap();
        thread::sleep(Duration::from_millis(400));
        assert_eq!(backend.output_status(), OutputStatus::Healthy);
        assert!(backend.output_alive());
        let started = Instant::now();
        let position = backend.seek(Duration::from_millis(1_000)).unwrap();
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(
            (Duration::from_millis(950)..=Duration::from_millis(1_100)).contains(&position),
            "{position:?}"
        );
        thread::sleep(Duration::from_millis(1_200));
        assert_eq!(backend.output_status(), OutputStatus::Healthy);
        let dropping = Instant::now();
        drop(backend);
        assert!(dropping.elapsed() < Duration::from_millis(200));
    }

    /// Plays WavPack files (`MOE_WV_PLAY`, `;`-separated) through the real playback engine on the default output device
    /// (muted): the position must advance in real time, seeking must land where asked, and the end of the file must
    /// be reached and reported as an empty player.
    #[test]
    #[ignore = "opens the real default output device (muted); needs MOE_WV_PLAY"]
    fn real_wavpack_files_play_seek_and_finish_on_the_real_device() {
        let files = std::env::var("MOE_WV_PLAY").expect("MOE_WV_PLAY");
        for file in files.split(';').filter(|file| !file.is_empty()) {
            let path = PathBuf::from(file);
            let mut backend = backend(ResamplingMode::HighQuality);
            backend.set_volume(0.0).unwrap();
            let duration = backend.load(&path).unwrap().expect("duration");
            backend.play().unwrap();
            let started = Instant::now();
            thread::sleep(Duration::from_millis(1_500));
            assert_eq!(backend.output_status(), OutputStatus::Healthy);
            let played = backend.position();
            let wall = started.elapsed();
            assert!(
                played + Duration::from_millis(300) > wall
                    && played < wall + Duration::from_millis(100),
                "{path:?}: position {played:?} after {wall:?}"
            );
            let info = backend.resampling_info().unwrap();
            let middle = duration / 2;
            let landed = backend.seek(middle).unwrap();
            assert!(
                landed >= middle.saturating_sub(Duration::from_millis(50))
                    && landed <= middle + Duration::from_millis(150),
                "{path:?}: seek to {middle:?} landed at {landed:?}"
            );
            thread::sleep(Duration::from_millis(500));
            assert!(backend.position() > middle + Duration::from_millis(300));
            let near_end = duration.saturating_sub(Duration::from_millis(1_500));
            backend.seek(near_end).unwrap();
            let waiting = Instant::now();
            while !backend.is_empty() && waiting.elapsed() < Duration::from_secs(6) {
                thread::sleep(Duration::from_millis(50));
            }
            assert!(backend.is_empty(), "{path:?}: playback did not finish");
            println!(
                "{path:?}: {duration:?}, source {:?} Hz → device {} Hz ({:?}), real-time position ok, seek ok, finished",
                info.source_rate, info.device_rate, info.conversion
            );
        }
    }

    /// The system default plus a second real device at once (muted): the mirror opens, consumes audio, the main
    /// output's delay does not shift the reported position, and removing the mirror closes it.
    #[test]
    #[ignore = "opens two real output devices (muted); run manually"]
    fn real_two_devices_play_together() {
        let devices = crate::output_devices::list_output_devices();
        assert!(devices.len() >= 2, "needs two output devices: {devices:?}");
        // Two chosen devices (the system default and chosen devices exclude each other).
        let first = &devices[0];
        let second = &devices[1];
        let mut backend = backend(ResamplingMode::HighQuality);
        backend.set_volume(0.0).unwrap();
        backend.load(&fixture("seek-tone.wav")).unwrap();
        let both = vec![
            OutputDeviceSetting {
                id: Some(first.id.clone()),
                enabled: true,
                delay_ms: 300,
                channel_map: None,
            },
            OutputDeviceSetting {
                id: Some(second.id.clone()),
                enabled: true,
                delay_ms: 0,
                channel_map: None,
            },
        ];
        backend.set_output_devices(both).unwrap();
        backend.play().unwrap();
        let started = Instant::now();
        while backend.mirrors.is_empty() && started.elapsed() < Duration::from_secs(5) {
            let _ = backend.output_status();
            thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            backend.mirrors.len(),
            1,
            "errors: {:?}",
            backend.reported_status
        );
        thread::sleep(Duration::from_millis(1_500));
        assert_eq!(backend.output_status(), OutputStatus::Healthy);
        let consumed = backend.mirrors[0].control.consumed.load(Ordering::Relaxed);
        assert!(consumed > 10_000, "the mirror consumed {consumed} frames");
        let position = backend.position();
        assert!(
            (Duration::from_millis(900)..=Duration::from_millis(1_800)).contains(&position),
            "{position:?}"
        );
        let after_seek = backend.seek(Duration::from_millis(1_000)).unwrap();
        assert!((Duration::from_millis(950)..=Duration::from_millis(1_100)).contains(&after_seek));
        let single = vec![OutputDeviceSetting {
            id: None,
            enabled: true,
            delay_ms: 0,
            channel_map: None,
        }];
        backend.set_output_devices(single).unwrap();
        assert!(backend.mirrors.is_empty());
        assert_eq!(backend.output_status(), OutputStatus::Healthy);
    }

    /// Measures how fast the named real devices really consume audio (muted): one silent counting source per device
    /// on its own mixer, frames pulled divided by wall-clock time. `MOE_CLOCK_DEVICES` is a comma-separated list of
    /// name fragments; `MOE_CLOCK_SECONDS` the length (default 120).
    #[test]
    #[ignore = "opens real output devices for minutes (silent); run manually"]
    fn measure_real_device_clocks() {
        struct Counter {
            channels: ChannelCount,
            rate: SampleRate,
            samples: Arc<AtomicU64>,
            pending: u64,
        }
        impl Iterator for Counter {
            type Item = Sample;
            fn next(&mut self) -> Option<Sample> {
                self.pending += 1;
                if self.pending >= 256 {
                    self.samples.fetch_add(self.pending, Ordering::Relaxed);
                    self.pending = 0;
                }
                Some(0.0)
            }
        }
        impl Source for Counter {
            fn current_span_len(&self) -> Option<usize> {
                None
            }
            fn channels(&self) -> ChannelCount {
                self.channels
            }
            fn sample_rate(&self) -> SampleRate {
                self.rate
            }
            fn total_duration(&self) -> Option<Duration> {
                None
            }
        }
        let wanted = std::env::var("MOE_CLOCK_DEVICES").unwrap_or_default();
        let seconds: u64 = std::env::var("MOE_CLOCK_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(120);
        let listed = crate::output_devices::list_output_devices();
        let mut sinks = Vec::new();
        for fragment in wanted
            .split(',')
            .map(str::trim)
            .filter(|fragment| !fragment.is_empty())
        {
            let device = listed
                .iter()
                .find(|device| device.name.contains(fragment))
                .unwrap_or_else(|| panic!("no device named like {fragment:?}: {listed:?}"));
            let found = find_output_device(&device.id).expect("device");
            let sink = DeviceSinkBuilder::from_device(found)
                .unwrap()
                .open_sink_or_fallback()
                .unwrap();
            let samples = Arc::new(AtomicU64::new(0));
            let channels = sink.config().channel_count();
            let rate = sink.config().sample_rate();
            sink.mixer().add(Counter {
                channels,
                rate,
                samples: Arc::clone(&samples),
                pending: 0,
            });
            println!(
                "{}: opened at {} Hz, {} ch",
                device.name,
                rate.get(),
                channels.get()
            );
            sinks.push((
                device.name.clone(),
                sink,
                samples,
                rate.get(),
                u64::from(channels.get()),
            ));
        }
        thread::sleep(Duration::from_secs(5));
        let started = Instant::now();
        let base: Vec<u64> = sinks
            .iter()
            .map(|sink| sink.2.load(Ordering::Relaxed))
            .collect();
        let mut elapsed_seconds = 0;
        let mut last_frames = vec![0_u64; sinks.len()];
        while elapsed_seconds < seconds {
            thread::sleep(Duration::from_secs(10));
            elapsed_seconds += 10;
            let elapsed = started.elapsed().as_secs_f64();
            let mut rates = Vec::new();
            for (index, (name, _, samples, nominal, channels)) in sinks.iter().enumerate() {
                let frames = (samples.load(Ordering::Relaxed) - base[index]) / channels;
                let rate = frames as f64 / elapsed;
                let ppm = (rate / f64::from(*nominal) - 1.0) * 1e6;
                let recent = (frames - last_frames[index]) as f64 / 10.0;
                last_frames[index] = frames;
                println!(
                    "t={elapsed:6.1}s  {name}: {rate:.3} Hz ({ppm:+.1} ppm vs {nominal} Hz), last 10 s: {recent:.0} Hz{}",
                    if recent < f64::from(*nominal) * 0.5 { "  <-- STALLED" } else { "" }
                );
                rates.push(rate / f64::from(*nominal));
            }
            if rates.len() == 2 {
                println!(
                    "           relative drift between the two: {:+.1} ppm",
                    (rates[0] / rates[1] - 1.0) * 1e6
                );
            }
        }
    }

    /// Plays silent 44.1 kHz and 96 kHz tones in both modes on the real
    /// default device and checks the stream rate and conversion path.
    #[test]
    #[ignore = "opens the real default output device (muted); run manually"]
    fn real_default_device_plays_44k_and_96k_in_both_modes() {
        let directory = std::env::temp_dir().join(format!("moe-resampling-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let tracks: Vec<(u32, PathBuf)> = [44_100, 96_000]
            .into_iter()
            .map(|rate| {
                let path = directory.join(format!("tone-{rate}.wav"));
                write_tone(&path, rate, 3);
                (rate, path)
            })
            .collect();

        for mode in [ResamplingMode::HighQuality, ResamplingMode::WindowsBuiltin] {
            let mut backend = backend(mode);
            backend.set_volume(0.0).unwrap();
            for (track_rate, path) in &tracks {
                backend.load(path).unwrap();
                backend.play().unwrap();
                thread::sleep(Duration::from_millis(500));
                assert_eq!(
                    backend.output_status(),
                    OutputStatus::Healthy,
                    "{mode:?} {track_rate}"
                );
                let info = backend.resampling_info().unwrap();
                eprintln!("{mode:?} {track_rate} Hz: {info:?}");
                assert_eq!(info.source_rate, Some(*track_rate));
                match mode {
                    ResamplingMode::HighQuality => {
                        assert_eq!(info.output_rate, info.device_rate);
                        let expected = if *track_rate == info.device_rate {
                            ResamplingConversion::None
                        } else {
                            ResamplingConversion::HighQuality
                        };
                        assert_eq!(info.conversion, expected);
                    }
                    ResamplingMode::WindowsBuiltin if info.fallback_reason.is_none() => {
                        assert_eq!(info.output_rate, *track_rate);
                        let expected = if *track_rate == info.device_rate {
                            ResamplingConversion::None
                        } else {
                            ResamplingConversion::Windows
                        };
                        assert_eq!(info.conversion, expected);
                    }
                    ResamplingMode::WindowsBuiltin => {
                        assert_eq!(info.conversion, ResamplingConversion::HighQuality);
                    }
                }
                let position = backend.seek(Duration::from_millis(1_500)).unwrap();
                assert!(
                    (Duration::from_millis(1_450)..=Duration::from_millis(1_600))
                        .contains(&position),
                    "{position:?}"
                );
                thread::sleep(Duration::from_millis(300));
                assert!(backend.position() > Duration::from_millis(1_550));
                assert_eq!(backend.output_status(), OutputStatus::Healthy);
            }
            // Live switch to the other mode keeps the position.
            let before = backend.position();
            let other = match mode {
                ResamplingMode::HighQuality => ResamplingMode::WindowsBuiltin,
                ResamplingMode::WindowsBuiltin => ResamplingMode::HighQuality,
            };
            backend.set_resampling_mode(other).unwrap();
            let after = backend.position();
            assert!(
                after >= before && after < before + Duration::from_millis(200),
                "{before:?} -> {after:?}"
            );
            thread::sleep(Duration::from_millis(300));
            assert_eq!(backend.output_status(), OutputStatus::Healthy);
            assert_eq!(backend.resampling_info().unwrap().mode, other);
            drop(backend);
        }
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn stream_buffer_is_about_50_ms_rounded_to_a_power_of_two() {
        assert_eq!(stream_buffer_frames(rate(44_100)), 2_048);
        assert_eq!(stream_buffer_frames(rate(48_000)), 2_048);
        assert_eq!(stream_buffer_frames(rate(96_000)), 4_096);
        assert_eq!(stream_buffer_frames(rate(192_000)), 8_192);
        assert_eq!(stream_buffer_frames(rate(8)), 1);
    }

    #[test]
    fn output_plan_follows_the_resampling_mode() {
        use OutputPlan::{Keep, OpenAt, OpenMix};
        use ResamplingMode::{HighQuality, WindowsBuiltin};
        let (k44, k48, k96) = (rate(44_100), rate(48_000), rate(96_000));
        // High quality always runs at the mix format, whatever the track.
        assert_eq!(plan_output(HighQuality, k44, k96, true, None), Keep);
        assert_eq!(plan_output(HighQuality, k96, k96, true, None), Keep);
        // ...and leaves a stream that Windows-builtin opened at a track rate.
        assert_eq!(plan_output(HighQuality, k44, k44, false, None), OpenMix);
        // Windows-builtin reopens only when the rate changes.
        assert_eq!(
            plan_output(WindowsBuiltin, k44, k96, true, None),
            OpenAt(k44)
        );
        assert_eq!(plan_output(WindowsBuiltin, k44, k44, false, None), Keep);
        assert_eq!(plan_output(WindowsBuiltin, k96, k96, true, None), Keep);
        assert_eq!(
            plan_output(WindowsBuiltin, k48, k44, false, None),
            OpenAt(k48)
        );
        // A rate the endpoint refused is not retried for every track.
        assert_eq!(plan_output(WindowsBuiltin, k44, k96, true, Some(k44)), Keep);
        assert_eq!(
            plan_output(WindowsBuiltin, k44, k48, false, Some(k44)),
            OpenMix
        );
        assert_eq!(
            plan_output(WindowsBuiltin, k48, k96, true, Some(k44)),
            OpenAt(k48)
        );
    }

    #[test]
    fn conversion_status_names_who_converts() {
        let (k44, k96) = (rate(44_100), rate(96_000));
        assert_eq!(
            conversion_for(None, k96, k96, false),
            ResamplingConversion::None
        );
        assert_eq!(
            conversion_for(Some(k96), k96, k96, false),
            ResamplingConversion::None
        );
        assert_eq!(
            conversion_for(Some(k44), k96, k96, true),
            ResamplingConversion::HighQuality
        );
        assert_eq!(
            conversion_for(Some(k44), k96, k96, false),
            ResamplingConversion::Basic
        );
        assert_eq!(
            conversion_for(Some(k44), k44, k96, false),
            ResamplingConversion::Windows
        );
    }

    #[test]
    fn tracks_at_other_rates_are_resampled_to_the_mixer_rate() {
        let (mixer, mut output) = dead_stream_mixer();
        let mut track = TrackPlayer::new(mixer, MIXER_RATE);
        track.load(&fixture("seek-tone.wav")).unwrap();
        assert_eq!(track.loaded_source_rate(), Some(rate(44_100)));
        assert!(track.in_app);
        track.play().unwrap();
        // 250 ms of 48 kHz stereo mixer output.
        let mut peak = 0.0f32;
        for _ in 0..(48_000 / 4 * 2) {
            peak = peak.max(output.next().unwrap().abs());
        }
        assert!(peak > 0.01, "the resampled tone should be audible");
        let position = track.position();
        assert!(
            (Duration::from_millis(230)..=Duration::from_millis(270)).contains(&position),
            "{position:?}"
        );

        // A track at the mixer's rate is attached untouched.
        let (mixer, _output) = rodio::mixer::mixer(NonZeroU16::new(2).unwrap(), rate(44_100));
        let mut native = TrackPlayer::new(mixer, rate(44_100));
        native.load(&fixture("seek-tone.wav")).unwrap();
        assert!(!native.in_app);
        native.stop();
        assert_eq!(native.loaded_source_rate(), None);
    }

    #[test]
    fn attaching_a_new_mixer_resumes_at_the_captured_position() {
        let (mixer, mut output) = dead_stream_mixer();
        let mut track = TrackPlayer::new(mixer, MIXER_RATE);
        track.load(&fixture("seek-tone.wav")).unwrap();
        track.set_volume(0.4);
        track.play().unwrap();
        for _ in 0..(48_000 / 2 * 2) {
            output.next();
        }
        let (path, position) = track.resume_point().unwrap();
        let (mixer, mut native_output) =
            rodio::mixer::mixer(NonZeroU16::new(2).unwrap(), rate(44_100));
        let old = track.attach(mixer, rate(44_100)).unwrap();
        assert!(track.player.is_none());
        track.seek_by_reopening(&path, position).unwrap();
        drop(old);
        assert!(!track.in_app, "the new mixer runs at the track's rate");
        let player = track.player.as_ref().unwrap();
        assert!(!player.is_paused());
        assert_eq!(player.volume(), 0.4);
        assert_eq!(track.position(), position);
        for _ in 0..(44_100 / 10 * 2) {
            native_output.next();
        }
        let advanced = track.position() - position;
        assert!(
            (Duration::from_millis(85)..=Duration::from_millis(115)).contains(&advanced),
            "{advanced:?}"
        );
    }

    #[test]
    fn legacy_stop_then_append_blocks_when_the_output_stream_is_dead() {
        // Characterizes the freeze: the previous backend called stop() then
        // append() on the same Player for every load. With no render callback
        // `append` waits in `sleep_until_end` forever.
        let finished = finishes_within(Duration::from_millis(300), || {
            let (mixer, _dead_output) = dead_stream_mixer();
            let player = Player::connect_new(&mixer);
            player.append(open_decoder(&fixture("seek-tone.wav")).unwrap());
            player.stop();
            player.append(open_decoder(&fixture("seek-tone.wav")).unwrap());
        });
        assert!(
            finished.is_none(),
            "stop()+append() is expected to wait for a render callback that never comes"
        );
    }

    #[test]
    fn track_player_never_waits_on_a_dead_output_stream() {
        let finished = finishes_within(Duration::from_secs(2), || {
            let (mixer, dead_output) = dead_stream_mixer();
            let mut track = TrackPlayer::new(mixer, MIXER_RATE);
            let duration = track.load(&fixture("seek-tone.wav")).unwrap();
            assert!(duration.is_some());
            track.set_volume(0.5);
            track.play().unwrap();
            // Output already judged dead: reopen-and-seek, no render callback.
            assert_eq!(
                track
                    .seek(Duration::from_millis(700), false, Duration::from_secs(1))
                    .unwrap(),
                SeekOutcome {
                    position: Duration::from_millis(700),
                    timed_out: false,
                }
            );
            assert_eq!(track.position(), Duration::from_millis(700));
            track.pause();
            track
                .seek(Duration::from_millis(1_200), false, Duration::from_secs(1))
                .unwrap();
            let (_, at) = track
                .load_at(&fixture("seek-tone.wav"), Duration::from_millis(900))
                .unwrap();
            assert_eq!(at, Duration::from_millis(900));
            assert_eq!(track.position(), Duration::from_millis(900));
            track.load(&fixture("seek-tone.flac")).unwrap();
            track.load(&fixture("seek-tone.mp3")).unwrap();
            track.stop();
            assert!(track.is_empty());
            assert_eq!(
                track.seek(Duration::from_millis(10), true, Duration::from_millis(50)),
                Err(AudioError::NoTrackLoaded)
            );
            track.load(&fixture("seek-tone.ogg")).unwrap();
            drop(dead_output);
            track.play().unwrap();
            track
                .seek(Duration::from_millis(300), false, Duration::from_secs(1))
                .unwrap();
        });
        assert!(
            finished.is_some(),
            "TrackPlayer must not block on a stream that stopped pulling audio"
        );
    }

    #[test]
    fn seek_timeout_falls_back_to_reopened_decoder_on_a_fresh_player() {
        let (mixer, _dead_output) = dead_stream_mixer();
        let mut track = TrackPlayer::new(mixer, MIXER_RATE);
        track.load(&fixture("seek-tone.wav")).unwrap();
        track.set_volume(0.3);
        track.play().unwrap();
        let stuck_player = Arc::clone(track.player.as_ref().unwrap());

        let started = Instant::now();
        // The stream looks alive, so try_seek is attempted and misses its
        // deadline because nothing pulls the mixer.
        let outcome = track
            .seek(Duration::from_millis(800), true, Duration::from_millis(100))
            .unwrap();
        assert!(started.elapsed() < Duration::from_millis(600));
        assert_eq!(
            outcome,
            SeekOutcome {
                position: Duration::from_millis(800),
                timed_out: true,
            }
        );
        let fresh = track.player.as_ref().unwrap();
        assert!(!Arc::ptr_eq(fresh, &stuck_player));
        assert!(!fresh.is_paused());
        assert_eq!(fresh.volume(), 0.3);
        assert_eq!(track.position(), Duration::from_millis(800));
    }

    #[test]
    fn healthy_stream_seeks_in_place_with_try_seek() {
        let (mixer, mut output) = dead_stream_mixer();
        let mut track = TrackPlayer::new(mixer, MIXER_RATE);
        track.load(&fixture("seek-tone.wav")).unwrap();
        track.play().unwrap();
        let original = Arc::clone(track.player.as_ref().unwrap());
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pump_stop = Arc::clone(&stop);
        let pump = thread::spawn(move || {
            while !pump_stop.load(Ordering::Relaxed) {
                for _ in 0..960 {
                    output.next();
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        let outcome = track
            .seek(Duration::from_millis(700), true, Duration::from_secs(1))
            .unwrap();
        stop.store(true, Ordering::Relaxed);
        pump.join().unwrap();
        assert!(!outcome.timed_out);
        assert!(Arc::ptr_eq(track.player.as_ref().unwrap(), &original));
        assert!(
            (Duration::from_millis(650)..=Duration::from_millis(900)).contains(&outcome.position),
            "in-place seek should land near 700 ms, got {:?}",
            outcome.position
        );
    }

    #[test]
    fn track_player_restarts_playback_state_and_volume_after_seek() {
        let (mixer, mut output) = dead_stream_mixer();
        let mut track = TrackPlayer::new(mixer, MIXER_RATE);
        track.load(&fixture("seek-tone.wav")).unwrap();
        track.set_volume(0.25);
        track.play().unwrap();
        track
            .seek(Duration::from_millis(500), false, Duration::from_secs(1))
            .unwrap();
        let player = track.player.as_ref().unwrap();
        assert!(!player.is_paused());
        assert_eq!(player.volume(), 0.25);
        // Pull 100 ms of output; position continues from the seek target.
        for _ in 0..(48_000 / 10 * 2) {
            output.next();
        }
        let position = track.position();
        assert!(
            (Duration::from_millis(550)..=Duration::from_millis(700)).contains(&position),
            "position should continue from the seek target, got {position:?}"
        );
        track.pause();
        track
            .seek(Duration::from_millis(900), false, Duration::from_secs(1))
            .unwrap();
        assert!(track.player.as_ref().unwrap().is_paused());
    }
}
