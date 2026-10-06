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

use crate::{AudioBackend, AudioError, OutputStatus};

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
}

struct OpenOutput {
    // The device sink owns CPAL's WASAPI stream and must outlive the player.
    sink: MixerDeviceSink,
    device_id: Option<String>,
    signals: Arc<OutputSignals>,
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
    /// thread and is abandoned after `OUTPUT_OPEN_TIMEOUT`.
    pub(super) fn new() -> Result<Self, AudioError> {
        let output = open_default_output_with_timeout(OUTPUT_OPEN_TIMEOUT)?;
        let track = TrackPlayer::new(output.sink.mixer().clone());
        let now = Instant::now();
        Ok(Self {
            output: Some(output),
            track,
            last_device_check: now,
            last_heartbeat: (0, now),
        })
    }
}

impl Drop for RodioBackend {
    fn drop(&mut self) {
        // Dropping a CPAL stream joins its render thread. If a driver call is
        // stuck there, the join would block; hand teardown to a disposable
        // thread so the audio worker always stays responsive.
        let output = self.output.take();
        let player = self.track.player.take();
        let _ = thread::Builder::new()
            .name("moe-audio-output-dispose".to_owned())
            .spawn(move || {
                drop(player);
                drop(output);
            });
    }
}

impl AudioBackend for RodioBackend {
    fn load(&mut self, path: &Path) -> Result<Option<Duration>, AudioError> {
        self.track.load(path)
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
        self.track.load_at(path, position)
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

fn open_default_output_with_timeout(timeout: Duration) -> Result<OpenOutput, AudioError> {
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("moe-audio-output-open".to_owned())
        .spawn(move || {
            // If the worker gave up, the send fails and the late stream is
            // dropped here instead of on the worker.
            let _ = result_tx.send(open_default_output());
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

fn open_default_output() -> Result<OpenOutput, AudioError> {
    let device = rodio::cpal::default_host()
        .default_output_device()
        .ok_or_else(|| AudioError::OutputDevice("no default output device".to_owned()))?;
    let device_id = device.id().ok().map(|id| id.to_string());
    let signals = Arc::new(OutputSignals::default());
    let mut sink = DeviceSinkBuilder::from_device(device)
        .map_err(|error| AudioError::OutputDevice(error.to_string()))?
        .with_error_callback(stream_error_callback(Arc::clone(&signals)))
        .open_sink_or_fallback()
        .map_err(|error| AudioError::OutputDevice(error.to_string()))?;
    sink.log_on_drop(false);
    sink.mixer().add(Heartbeat {
        channels: sink.config().channel_count(),
        sample_rate: sink.config().sample_rate(),
        signals: Arc::clone(&signals),
        pending: 0,
    });
    Ok(OpenOutput {
        sink,
        device_id,
        signals,
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
    // Shared only with abandoned seek helpers, which never lock anything.
    player: Option<Arc<Player>>,
    path: Option<PathBuf>,
    /// Track position at which the current `Player` started.
    base: Duration,
    paused: bool,
    volume: f32,
}

impl TrackPlayer {
    fn new(mixer: Mixer) -> Self {
        Self {
            mixer,
            player: None,
            path: None,
            base: Duration::ZERO,
            paused: true,
            volume: 1.0,
        }
    }

    fn load(&mut self, path: &Path) -> Result<Option<Duration>, AudioError> {
        let decoder = open_decoder(path)?;
        let duration = decoder.total_duration();
        self.paused = true;
        self.start(decoder, Duration::ZERO);
        self.path = Some(path.to_path_buf());
        Ok(duration)
    }

    /// Load paused at `position` by seeking the decoder before it is attached,
    /// so no render callback is involved. Used when rebuilding the output.
    fn load_at(
        &mut self,
        path: &Path,
        position: Duration,
    ) -> Result<(Option<Duration>, Duration), AudioError> {
        let mut decoder = open_decoder(path)?;
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
        self.start(decoder, actual);
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
        decoder
            .try_seek(position)
            .map_err(|error| AudioError::Seek(error.to_string()))?;
        self.start(decoder, position);
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

    fn start(&mut self, decoder: Decoder<BufReader<File>>, base: Duration) {
        // A brand-new Player is never in the stopped state, so `append` does
        // not wait for the render callback to drain an older source.
        let player = Player::connect_new(&self.mixer);
        player.set_volume(self.volume);
        if self.paused {
            player.pause();
        }
        player.append(decoder);
        if let Some(previous) = self.player.replace(Arc::new(player)) {
            // Only flags the old source; the mixer drops it on its next pull.
            // A seek helper may still hold it; it is never joined.
            previous.stop();
        }
        self.base = base;
    }
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
    use std::num::{NonZeroU16, NonZeroU32};

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    /// A mixer whose output is never pulled behaves exactly like a WASAPI
    /// stream whose render thread exited after AUDCLNT_E_DEVICE_INVALIDATED.
    fn dead_stream_mixer() -> (Mixer, rodio::mixer::MixerSource) {
        rodio::mixer::mixer(
            NonZeroU16::new(2).unwrap(),
            NonZeroU32::new(48_000).unwrap(),
        )
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

    #[test]
    #[ignore = "opens the real default output device (muted); run manually"]
    fn real_default_device_stays_healthy_and_seeks_in_place() {
        let mut backend = RodioBackend::new().expect("default output device");
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
            let mut track = TrackPlayer::new(mixer);
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
        let mut track = TrackPlayer::new(mixer);
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
        let mut track = TrackPlayer::new(mixer);
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
        let mut track = TrackPlayer::new(mixer);
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
