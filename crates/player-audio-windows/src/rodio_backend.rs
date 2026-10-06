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

use crate::resample::{needs_resampling, Resampled};
use crate::{
    AudioBackend, AudioError, DseeHxState, OutputStatus, ResamplingConfig, ResamplingConversion,
    ResamplingInfo, ResamplingMode,
};

/// Upper bound for opening a WASAPI endpoint. A driver that hangs inside
/// Activate/Initialize must not take the worker with it.
const OUTPUT_OPEN_TIMEOUT: Duration = Duration::from_secs(3);
/// How often the worker compares the opened endpoint with the system default.
const DEFAULT_DEVICE_POLL_INTERVAL: Duration = Duration::from_secs(1);
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
    last_heartbeat: (u64, Instant),
    /// Shared with the backend factory so a rebuilt output starts in the
    /// current mode and, in Windows-builtin mode, at the current track's rate.
    config: Arc<ResamplingConfig>,
    mode: ResamplingMode,
    dsee_enabled: bool,
    /// Windows-builtin only: the track rate the endpoint refused, and why.
    fallback: Option<OutputFallback>,
}

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
        let output = match requested {
            Some(rate) => match open_output_with_timeout(Some(rate), OUTPUT_OPEN_TIMEOUT) {
                Ok(output) => output,
                Err(error) => {
                    let reason = error.to_string();
                    log_refused_rate(rate, &reason);
                    let output = open_output_with_timeout(None, OUTPUT_OPEN_TIMEOUT)?;
                    fallback = Some(OutputFallback { rate, reason });
                    output
                }
            },
            None => open_output_with_timeout(None, OUTPUT_OPEN_TIMEOUT)?,
        };
        let mut track = TrackPlayer::new(output.sink.mixer().clone(), output.sample_rate);
        track.dsee_enabled = config.dsee_hx();
        let now = Instant::now();
        Ok(Self {
            output: Some(output),
            track,
            last_device_check: now,
            last_heartbeat: (0, now),
            config: Arc::clone(&config),
            mode,
            dsee_enabled: config.dsee_hx(),
            fallback,
        })
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
                match open_output_with_timeout(Some(rate), OUTPUT_OPEN_TIMEOUT) {
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
        match open_output_with_timeout(None, OUTPUT_OPEN_TIMEOUT) {
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
        let old_player = self
            .track
            .attach(output.sink.mixer().clone(), output.sample_rate);
        let old_output = self.output.replace(output);
        dispose_off_worker(old_output, old_player);
        let now = Instant::now();
        self.last_heartbeat = (0, now);
        self.last_device_check = now;
    }

    fn plan_for(&self, path: &Path, decoder: &Decoder<BufReader<File>>) -> crate::dsee::DseePlan {
        crate::dsee::plan(self.dsee_enabled, path, &track_signal(decoder))
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

impl Drop for RodioBackend {
    fn drop(&mut self) {
        dispose_off_worker(self.output.take(), self.track.player.take());
    }
}

impl AudioBackend for RodioBackend {
    fn load(&mut self, path: &Path) -> Result<Option<Duration>, AudioError> {
        let decoder = open_decoder(path)?;
        let plan = self.plan_for(path, &decoder);
        self.prepare_output_for(plan_rate(&decoder, plan.output_rate));
        self.track.load_decoder(path, decoder, plan.engagement)
    }

    fn play(&mut self) -> Result<(), AudioError> {
        self.track.play()
    }

    fn pause(&mut self) -> Result<(), AudioError> {
        self.track.pause();
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        self.track.stop();
        Ok(())
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
        let decoder = open_decoder(path)?;
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
        let decoder = open_decoder(&path)?;
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
            dsee_output_rate: (self.track.dsee_state == DseeHxState::Active)
                .then(|| self.track.playback_rate.map(SampleRate::get))
                .flatten(),
        })
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), AudioError> {
        self.track.set_volume(volume);
        Ok(())
    }

    fn position(&self) -> Duration {
        self.track.position()
    }

    fn is_empty(&self) -> bool {
        self.track.is_empty()
    }

    fn output_status(&mut self) -> OutputStatus {
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

        if now.duration_since(self.last_device_check) >= DEFAULT_DEVICE_POLL_INTERVAL {
            self.last_device_check = now;
            if let (Some(opened), Some(default)) =
                (output.device_id.as_deref(), default_output_device_id())
            {
                if opened != default {
                    return OutputStatus::DefaultDeviceChanged;
                }
            }
        }
        OutputStatus::Healthy
    }
}

fn default_output_device_id() -> Option<String> {
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
) -> Result<OpenOutput, AudioError> {
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("moe-audio-output-open".to_owned())
        .spawn(move || {
            // If the worker gave up, the send fails and the late stream is
            // dropped here instead of on the worker.
            let _ = result_tx.send(open_output(rate));
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

fn open_output(rate: Option<SampleRate>) -> Result<OpenOutput, AudioError> {
    let device = rodio::cpal::default_host()
        .default_output_device()
        .ok_or_else(|| AudioError::OutputDevice("no default output device".to_owned()))?;
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
    dsee_state: DseeHxState,
    dsee_notice: Option<String>,
    /// The current source goes through the in-app high-quality resampler.
    in_app: bool,
}

impl TrackPlayer {
    fn new(mixer: Mixer, output_rate: SampleRate) -> Self {
        Self {
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
            dsee_state: DseeHxState::Off,
            dsee_notice: None,
            in_app: false,
        }
    }

    #[cfg(test)]
    fn load(&mut self, path: &Path) -> Result<Option<Duration>, AudioError> {
        let decoder = open_decoder(path)?;
        self.load_decoder(path, decoder, crate::dsee::Engagement::Off)
    }

    fn load_decoder(
        &mut self,
        path: &Path,
        decoder: Decoder<BufReader<File>>,
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
        let decoder = open_decoder(path)?;
        self.load_decoder_at(path, decoder, position, crate::dsee::Engagement::Off)
    }

    /// Load paused at `position` by seeking the decoder before it is attached,
    /// so no render callback is involved. Used when rebuilding the output.
    fn load_decoder_at(
        &mut self,
        path: &Path,
        mut decoder: Decoder<BufReader<File>>,
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
            decoder = open_decoder(path)?;
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
        let mut decoder = open_decoder(path)?;
        let plan = crate::dsee::plan(self.dsee_enabled, path, &track_signal(&decoder));
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
            Some(player) => self.base + player.get_pos(),
            None => self.base,
        }
    }

    fn is_empty(&self) -> bool {
        self.player.as_ref().is_none_or(|player| player.empty())
    }

    fn start(
        &mut self,
        path: &Path,
        decoder: Decoder<BufReader<File>>,
        base: Duration,
        engagement: crate::dsee::Engagement,
    ) -> Result<(), AudioError> {
        // A brand-new Player is never in the stopped state, so `append` does
        // not wait for the render callback to drain an older source.
        let player = Player::connect_new(&self.mixer);
        player.set_volume(self.volume);
        if self.paused {
            player.pause();
        }
        let file_rate = decoder.sample_rate();
        self.source_rate = Some(file_rate);
        let playback = self.attach_dsee(path, decoder, base, engagement)?;
        let playback_rate = playback.sample_rate();
        self.playback_rate = Some(playback_rate);
        self.in_app = false;
        if needs_resampling(playback_rate, self.output_rate) {
            match Resampled::new(playback, self.output_rate) {
                Ok(resampled) => {
                    player.append(resampled);
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
                    player.append(rejected.source);
                }
            }
        } else {
            player.append(playback);
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
        decoder: Decoder<BufReader<File>>,
        base: Duration,
        engagement: crate::dsee::Engagement,
    ) -> Result<PlaybackSource, AudioError> {
        match engagement {
            crate::dsee::Engagement::Run {
                bitrate_kbps,
                input_rate,
            } => match crate::dsee::DseeSource::open(decoder, input_rate, bitrate_kbps) {
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
                    let mut decoder = open_decoder(path)?;
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

type FileDecoder = Decoder<BufReader<File>>;

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

fn open_decoder(path: &Path) -> Result<Decoder<BufReader<File>>, AudioError> {
    let file = File::open(path).map_err(|error| AudioError::FileOpen {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    Decoder::try_from(file).map_err(|error| match error {
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
