//! `ExclusiveBackend`: the `AudioBackend` for WASAPI exclusive mode. One track at a time goes from the decoder (rodio,
//! DSD, Opus) through an optional channel remix and rate conversion into a [`Session`]; at 100 % volume with a rate
//! and channel count the device takes, the samples reach it unchanged.
//!
//! Two things keep exclusive mode from getting in the way:
//! - A track the device cannot take (in use, exclusive control not allowed, a Bluetooth endpoint, no usable format)
//!   plays through a shared-mode `RodioBackend` instead, and the reason is reported next to the output status. The
//!   next track tries exclusive mode again.
//! - A stream that has been paused or has run out of tracks for [`IDLE_RELEASE`] is closed, so other programs can use
//!   the device. Playing again reopens it at the position it was left at.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rodio::{SampleRate, Source};

use super::dop::{DopOpenError, DopSource, DopTrackInfo};
use super::endpoint::Endpoint;
use super::format::Negotiated;
use super::session::{Route, SampleSource, Session};
use crate::mirror::MixSource;
use crate::output_devices::ChannelMixer;
use crate::resample::Resampled;
use crate::rodio_backend::{
    default_output_device_id, open_decoder_with, AudioDecoder, DEFAULT_DEVICE_POLL_INTERVAL,
};
use crate::{
    AudioBackend, AudioError, DsdSettings, DseeHxState, ExclusiveInfo, OutputStatus,
    ResamplingConfig, ResamplingConversion, ResamplingInfo, ResamplingMode,
};

/// Builds the endpoint on the render thread (WASAPI objects must stay on the thread that made them).
pub(crate) type EndpointFactory = Arc<dyn Fn() -> Result<Box<dyn Endpoint>, String> + Send + Sync>;
/// Builds the shared-mode backend that plays a track exclusive mode could not.
pub(crate) type FallbackFactory =
    Arc<dyn Fn() -> Result<Box<dyn AudioBackend>, AudioError> + Send + Sync>;

/// How long a paused (or finished) exclusive stream is kept before the device is let go.
pub(crate) const IDLE_RELEASE: Duration = Duration::from_secs(10);

struct Track {
    path: PathBuf,
    source_rate: u32,
    duration: Option<Duration>,
    /// Where in the track the current source started.
    base: Duration,
    /// Frames the device had played when the current source started.
    played_at_start: u64,
    /// The track plays as DoP (DSD bits straight to the device); `None` is the PCM route.
    dop: Option<DopTrackInfo>,
}

/// The stream was closed while idle; this is where the track was.
struct Released {
    position: Duration,
    /// The track had played to its end.
    finished: bool,
}

/// Why opening a track in exclusive mode failed.
enum LoadError {
    /// The device or its format negotiation (worth falling back to shared mode).
    Device(String),
    /// The file or the conversion (shared mode would fail the same way).
    Audio(AudioError),
}

pub(crate) struct ExclusiveBackend {
    config: Arc<ResamplingConfig>,
    factory: EndpointFactory,
    fallback_factory: FallbackFactory,
    /// The device the backend was built for; a different chosen device makes the worker rebuild it.
    device: Option<String>,
    session: Option<Session>,
    track: Option<Track>,
    released: Option<Released>,
    /// The shared-mode backend playing the current track, and why exclusive mode could not.
    fallback: Option<Box<dyn AudioBackend>>,
    fallback_reason: Option<String>,
    /// DoP is on but the current DSD track plays as converted PCM, and why.
    dop_fallback: Option<String>,
    idle_since: Option<Instant>,
    idle_release: Duration,
    /// Asks Windows which endpoint is the default right now (replaceable in tests).
    default_probe: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    /// The default endpoint when the stream was opened; only kept while the backend follows the system default.
    opened_default: Option<String>,
    last_default_check: Instant,
    seen_device_generation: u64,
    default_poll: Duration,
    volume: f32,
    paused: bool,
}

impl ExclusiveBackend {
    /// Nothing is opened until a track loads, so building the backend never takes the device from other programs.
    pub(crate) fn new(config: Arc<ResamplingConfig>) -> Self {
        let device = config.primary_device();
        let endpoint_id = device.clone();
        let factory: EndpointFactory = Arc::new(move || {
            super::endpoint::WasapiEndpoint::new(endpoint_id.as_deref())
                .map(|endpoint| Box::new(endpoint) as Box<dyn Endpoint>)
        });
        let fallback_config = Arc::clone(&config);
        let fallback_factory: FallbackFactory = Arc::new(move || {
            crate::rodio_backend::RodioBackend::new_fallback(Arc::clone(&fallback_config))
                .map(|backend| Box::new(backend) as Box<dyn AudioBackend>)
        });
        Self::with_factories(config, factory, fallback_factory, device)
    }

    pub(crate) fn with_factories(
        config: Arc<ResamplingConfig>,
        factory: EndpointFactory,
        fallback_factory: FallbackFactory,
        device: Option<String>,
    ) -> Self {
        Self {
            config,
            factory,
            fallback_factory,
            device,
            session: None,
            track: None,
            released: None,
            fallback: None,
            fallback_reason: None,
            dop_fallback: None,
            idle_since: None,
            idle_release: IDLE_RELEASE,
            default_probe: Arc::new(default_output_device_id),
            opened_default: None,
            last_default_check: Instant::now(),
            seen_device_generation: crate::device_notify::device_change_generation(),
            default_poll: DEFAULT_DEVICE_POLL_INTERVAL,
            volume: 1.0,
            paused: true,
        }
    }

    /// The track's samples as the stream wants them: remixed to the device's channel count, then converted to its rate.
    fn chain(decoder: AudioDecoder, negotiated: &Negotiated) -> Result<SampleSource, AudioError> {
        let remix = negotiated.remixed().then(|| {
            ChannelMixer::new(
                None,
                usize::from(negotiated.source_channels),
                usize::from(negotiated.format.channels),
            )
        });
        let target = SampleRate::new(negotiated.format.rate).unwrap_or(SampleRate::MIN);
        let convert = |error: &dyn std::fmt::Display| {
            AudioError::Backend(format!(
                "無法把取樣率轉換為 {} Hz：{error}",
                negotiated.format.rate
            ))
        };
        Ok(match (remix, negotiated.resampled()) {
            (None, false) => Box::new(decoder),
            (None, true) => Box::new(
                Resampled::new(decoder, target).map_err(|rejected| convert(&rejected.error))?,
            ),
            (Some(mixer), false) => Box::new(MixSource::new(decoder, mixer)),
            (Some(mixer), true) => Box::new(
                Resampled::new(MixSource::new(decoder, mixer), target)
                    .map_err(|rejected| convert(&rejected.error))?,
            ),
        })
    }

    fn close_session(&mut self) {
        if let Some(session) = self.session.take() {
            session.close();
        }
    }

    /// Try to open `path` as DoP at `position`, paused. `Ok(None)` means DoP cannot carry this track on this device
    /// (the reason is kept in `dop_fallback`) and the PCM route should be tried.
    fn open_dop(
        &mut self,
        path: &Path,
        position: Duration,
    ) -> Result<Option<Option<Duration>>, LoadError> {
        let (mut source, info) = match DopSource::open(path, 0) {
            Ok(opened) => opened,
            Err(DopOpenError::Unsupported(reason)) => {
                self.dop_fallback = Some(reason);
                return Ok(None);
            }
            Err(DopOpenError::Audio(error)) => return Err(LoadError::Audio(error)),
        };
        let frame = info.frame_at(position);
        if frame > 0 {
            source.seek_to_frame(frame).map_err(LoadError::Audio)?;
        }
        let reusable = self.session.as_ref().is_some_and(|session| {
            session.negotiated.dop
                && session.negotiated.format.rate == info.transport_rate
                && session.control.failure().is_none()
        });
        if !reusable {
            self.close_session();
            self.opened_default = if self.device.is_none() {
                (self.default_probe)()
            } else {
                None
            };
            let factory = Arc::clone(&self.factory);
            match Session::start_route(
                move || factory(),
                Route::Dop {
                    rate: info.transport_rate,
                },
                self.volume,
                true,
            ) {
                Ok(session) => self.session = Some(session),
                Err(reason) => {
                    self.dop_fallback = Some(reason);
                    return Ok(None);
                }
            }
        }
        let session = self.session.as_ref().expect("session was just opened");
        session.control.paused.store(true, Ordering::Release);
        let played_at_start = session.play_source(Box::new(source));
        self.track = Some(Track {
            path: path.to_path_buf(),
            source_rate: info.transport_rate,
            duration: Some(info.duration()),
            base: info.time_of(frame),
            played_at_start,
            dop: Some(info),
        });
        Ok(Some(Some(info.duration())))
    }

    /// Open `path` in exclusive mode at `position`, paused.
    fn open_exclusive(
        &mut self,
        path: &Path,
        position: Duration,
    ) -> Result<Option<Duration>, LoadError> {
        self.dop_fallback = None;
        if self.config.dsd.settings().dop && crate::dsd::is_dsd_path(path) {
            if let Some(duration) = self.open_dop(path, position)? {
                return Ok(duration);
            }
        }
        let mut decoder = open_decoder_with(path, &self.config.dsd).map_err(LoadError::Audio)?;
        let source_rate = decoder.sample_rate().get();
        let source_channels = decoder.channels().get();
        let duration = decoder.total_duration();
        let target = duration.map_or(position, |duration| position.min(duration));
        // The same format as the last track keeps the stream open (no re-open click); anything else reopens it.
        let reusable = self.session.as_ref().is_some_and(|session| {
            !session.negotiated.dop
                && session.negotiated.source_rate == source_rate
                && session.negotiated.source_channels == source_channels
                && session.control.failure().is_none()
        });
        if !reusable {
            self.close_session();
            // Following the system default: remember which endpoint it is now, to notice when Windows moves it.
            self.opened_default = if self.device.is_none() {
                (self.default_probe)()
            } else {
                None
            };
            let factory = Arc::clone(&self.factory);
            let session = Session::start(
                move || factory(),
                source_rate,
                source_channels,
                self.volume,
                true,
            )
            .map_err(LoadError::Device)?;
            self.session = Some(session);
        }
        let session = self.session.as_ref().expect("session was just opened");
        session.control.paused.store(true, Ordering::Release);
        if !target.is_zero() {
            decoder
                .try_seek(target)
                .map_err(|error| LoadError::Audio(AudioError::Seek(error.to_string())))?;
        }
        let chain = Self::chain(decoder, &session.negotiated).map_err(LoadError::Audio)?;
        let played_at_start = session.play_source(chain);
        self.track = Some(Track {
            path: path.to_path_buf(),
            source_rate,
            duration,
            base: target,
            played_at_start,
            dop: None,
        });
        Ok(duration)
    }

    /// Play the track through the shared-mode backend because exclusive mode could not.
    fn use_fallback(
        &mut self,
        path: &Path,
        position: Duration,
        reason: String,
    ) -> Result<Option<Duration>, AudioError> {
        self.close_session();
        // If the shared fallback fails too, the message carries both reasons.
        let both = |error: AudioError| {
            AudioError::Backend(format!(
                "獨佔模式無法使用：{reason}；改用共享模式也失敗：{error}"
            ))
        };
        let mut backend = (self.fallback_factory)().map_err(both)?;
        backend.set_volume(self.volume).map_err(both)?;
        let duration = if position.is_zero() {
            backend.load(path).map_err(both)?
        } else {
            backend.load_at(path, position).map_err(both)?.0
        };
        self.fallback = Some(backend);
        self.fallback_reason = Some(reason);
        Ok(duration)
    }

    /// Load (or reopen) a track: exclusive mode first, the shared fallback when the device cannot take it.
    fn open(&mut self, path: &Path, position: Duration) -> Result<Option<Duration>, AudioError> {
        match self.open_exclusive(path, position) {
            Ok(duration) => Ok(duration),
            Err(LoadError::Audio(error)) => Err(error),
            Err(LoadError::Device(reason)) => self.use_fallback(path, position, reason),
        }
    }

    /// Let the device go; the track stays where it is.
    fn release(&mut self) {
        let position = self.position();
        let finished = self
            .session
            .as_ref()
            .is_some_and(|session| session.ring.finished());
        self.close_session();
        self.released = Some(Released { position, finished });
        self.idle_since = None;
    }

    fn reset_for_load(&mut self) {
        self.fallback = None;
        self.fallback_reason = None;
        self.dop_fallback = None;
        self.released = None;
        self.idle_since = None;
        self.track = None;
        self.paused = true;
    }
}

impl AudioBackend for ExclusiveBackend {
    fn load(&mut self, path: &Path) -> Result<Option<Duration>, AudioError> {
        self.reset_for_load();
        self.open(path, Duration::ZERO)
    }

    fn load_at(
        &mut self,
        path: &Path,
        position: Duration,
    ) -> Result<(Option<Duration>, Duration), AudioError> {
        self.reset_for_load();
        let duration = self.open(path, position)?;
        let target = duration.map_or(position, |duration| position.min(duration));
        Ok((duration, target))
    }

    fn play(&mut self) -> Result<(), AudioError> {
        if let Some(backend) = self.fallback.as_mut() {
            return backend.play();
        }
        if let Some(released) = self.released.take() {
            let Some(path) = self.track.as_ref().map(|track| track.path.clone()) else {
                return Err(AudioError::NoTrackLoaded);
            };
            self.open(&path, released.position)?;
            if let Some(backend) = self.fallback.as_mut() {
                return backend.play();
            }
        }
        let (Some(session), Some(_)) = (self.session.as_ref(), self.track.as_ref()) else {
            return Err(AudioError::NoTrackLoaded);
        };
        self.paused = false;
        self.idle_since = None;
        session.control.paused.store(false, Ordering::Release);
        Ok(())
    }

    fn pause(&mut self) -> Result<(), AudioError> {
        if let Some(backend) = self.fallback.as_mut() {
            return backend.pause();
        }
        self.paused = true;
        if let Some(session) = self.session.as_ref() {
            session.control.paused.store(true, Ordering::Release);
        }
        // Pausing gives the device back at once; `play()` opens it again where the track was.
        if self.session.is_some() && self.track.is_some() {
            self.release();
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        if let Some(mut backend) = self.fallback.take() {
            let _ = backend.stop();
        }
        // Stopping gives the device back to other programs.
        self.fallback_reason = None;
        self.released = None;
        self.track = None;
        self.paused = true;
        self.close_session();
        Ok(())
    }

    fn seek(&mut self, position: Duration) -> Result<Duration, AudioError> {
        if let Some(backend) = self.fallback.as_mut() {
            return backend.seek(position);
        }
        let Some(track) = self.track.as_mut() else {
            return Err(AudioError::NoTrackLoaded);
        };
        let target = track
            .duration
            .map_or(position, |duration| position.min(duration));
        if let Some(released) = self.released.as_mut() {
            // The stream is closed: the new position is used when it opens again.
            released.position = target;
            released.finished = false;
            return Ok(target);
        }
        let Some(session) = self.session.as_ref() else {
            return Err(AudioError::NoTrackLoaded);
        };
        if let Some(info) = track.dop {
            // DSD has no filter state: the new source starts at exactly this frame.
            let frame = info.frame_at(target);
            let (source, _) = DopSource::open(&track.path, frame).map_err(|error| match error {
                DopOpenError::Audio(error) => error,
                DopOpenError::Unsupported(reason) => AudioError::Backend(reason),
            })?;
            track.played_at_start = session.play_source(Box::new(source));
            track.base = info.time_of(frame);
            return Ok(track.base);
        }
        let decoder = {
            let mut decoder = open_decoder_with(&track.path, &self.config.dsd)?;
            if !target.is_zero() {
                decoder
                    .try_seek(target)
                    .map_err(|error| AudioError::Seek(error.to_string()))?;
            }
            decoder
        };
        let chain = Self::chain(decoder, &session.negotiated)?;
        track.played_at_start = session.play_source(chain);
        track.base = target;
        Ok(target)
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), AudioError> {
        self.volume = volume;
        if let Some(session) = self.session.as_ref() {
            session
                .control
                .volume
                .store(volume.to_bits(), Ordering::Relaxed);
        }
        if let Some(backend) = self.fallback.as_mut() {
            backend.set_volume(volume)?;
        }
        Ok(())
    }

    fn set_resampling_mode(&mut self, mode: ResamplingMode) -> Result<(), AudioError> {
        match self.fallback.as_mut() {
            Some(backend) => backend.set_resampling_mode(mode),
            None => Ok(()),
        }
    }

    fn set_dsee_hx(&mut self, enabled: bool) -> Result<(), AudioError> {
        match self.fallback.as_mut() {
            Some(backend) => backend.set_dsee_hx(enabled),
            None => Ok(()),
        }
    }

    fn set_dsd_settings(&mut self, settings: DsdSettings) -> Result<(), AudioError> {
        let before = self.config.dsd.settings();
        self.config.dsd.set(settings);
        if let Some(backend) = self.fallback.as_mut() {
            return backend.set_dsd_settings(settings);
        }
        // The gain is followed live by the PCM sources; DoP on or off, or another PCM output rate, needs the current
        // DSD track opened again (where it is, in the state it is in).
        let after = self.config.dsd.settings();
        let route_changed =
            before.dop != after.dop || (!after.dop && before.output_rate != after.output_rate);
        if !route_changed || self.released.is_some() {
            return Ok(());
        }
        let Some(path) = self
            .track
            .as_ref()
            .map(|track| track.path.clone())
            .filter(|path| crate::dsd::is_dsd_path(path))
        else {
            return Ok(());
        };
        let position = self.position();
        let was_playing = !self.paused;
        self.open(&path, position)?;
        if was_playing {
            self.play()?;
        }
        Ok(())
    }

    fn position(&self) -> Duration {
        if let Some(backend) = self.fallback.as_ref() {
            return backend.position();
        }
        if let Some(released) = self.released.as_ref() {
            return released.position;
        }
        let (Some(session), Some(track)) = (self.session.as_ref(), self.track.as_ref()) else {
            return Duration::ZERO;
        };
        let rate = u64::from(session.negotiated.format.rate).max(1);
        let played = session
            .ring
            .played_frames()
            .saturating_sub(track.played_at_start);
        // The endpoint buffer holds about one period that has been taken from the ring but is not audible yet.
        let audible = played.saturating_sub(session.control.latency_frames.load(Ordering::Relaxed));
        let position = track.base + Duration::from_secs_f64(audible as f64 / rate as f64);
        track
            .duration
            .map_or(position, |duration| position.min(duration))
    }

    fn is_empty(&self) -> bool {
        if let Some(backend) = self.fallback.as_ref() {
            return backend.is_empty();
        }
        if let Some(released) = self.released.as_ref() {
            return released.finished;
        }
        match (self.session.as_ref(), self.track.as_ref()) {
            (Some(session), Some(_)) => session.ring.finished(),
            _ => true,
        }
    }

    fn resampling_info(&self) -> Option<ResamplingInfo> {
        if let Some(backend) = self.fallback.as_ref() {
            let mut info = backend.resampling_info()?;
            info.exclusive_fallback = self.fallback_reason.clone();
            return Some(info);
        }
        let session = self.session.as_ref()?;
        let negotiated = &session.negotiated;
        let format = negotiated.format;
        let dop = self.track.as_ref().and_then(|track| track.dop);
        // DoP is the file's own bits: the player's volume does not touch them.
        let volume_applied = dop.is_none() && self.volume < 0.9999;
        Some(ResamplingInfo {
            mode: ResamplingMode::HighQuality,
            source_rate: self.track.as_ref().map(|track| track.source_rate),
            output_rate: format.rate,
            device_rate: format.rate,
            conversion: if negotiated.resampled() {
                ResamplingConversion::HighQuality
            } else {
                ResamplingConversion::None
            },
            fallback_reason: None,
            dsee_hx: DseeHxState::Off,
            dsee_notice: None,
            dsee_output_rate: None,
            exclusive: Some(ExclusiveInfo {
                bits: format.container.valid_bits(),
                channels: format.channels,
                source_channels: negotiated.source_channels,
                bit_perfect: !negotiated.resampled() && !negotiated.remixed() && !volume_applied,
                volume_applied,
                underruns: session.control.underruns.load(Ordering::Relaxed),
                dop_dsd_rate: dop.map(|info| info.dsd_rate),
                dop_fallback: self.dop_fallback.clone(),
            }),
            exclusive_fallback: None,
        })
    }

    fn output_status(&mut self) -> OutputStatus {
        // Leaving exclusive mode, or choosing another device, rebuilds the backend (the worker restores the track).
        if !self.config.exclusive() || self.config.primary_device() != self.device {
            return OutputStatus::DefaultDeviceChanged;
        }
        if let Some(backend) = self.fallback.as_mut() {
            return backend.output_status();
        }
        if let Some(reason) = self
            .session
            .as_ref()
            .and_then(|session| session.control.failure())
        {
            return OutputStatus::Lost(reason);
        }
        // Following the system default and Windows moved it: the worker rebuilds the backend on the new default (the
        // shared-mode backend does the same; an exclusive stream is bound to the endpoint it opened).
        if self.device.is_none() && self.session.is_some() {
            let now = Instant::now();
            let generation = crate::device_notify::device_change_generation();
            if generation != self.seen_device_generation
                || now.duration_since(self.last_default_check) >= self.default_poll
            {
                self.seen_device_generation = generation;
                self.last_default_check = now;
                if let (Some(opened), Some(current)) =
                    (self.opened_default.as_ref(), (self.default_probe)())
                {
                    if *opened != current {
                        return OutputStatus::DefaultDeviceChanged;
                    }
                }
            }
        }
        // Paused, or out of audio at the end of the queue: after a while the device is let go.
        let idle = self.track.is_some()
            && self
                .session
                .as_ref()
                .is_some_and(|session| self.paused || session.ring.finished());
        if idle {
            let now = Instant::now();
            let since = *self.idle_since.get_or_insert(now);
            if now.duration_since(since) >= self.idle_release {
                self.release();
            }
        } else {
            self.idle_since = None;
        }
        OutputStatus::Healthy
    }
}

impl Drop for ExclusiveBackend {
    fn drop(&mut self) {
        self.close_session();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exclusive::endpoint::fake::{Capture, FakeEndpoint};
    use crate::exclusive::format::{Container, StreamFormat};
    use std::time::Instant;

    fn format(rate: u32, channels: u16, container: Container) -> StreamFormat {
        StreamFormat {
            rate,
            channels,
            container,
        }
    }

    /// What the fake shared-mode fallback was asked to do.
    #[derive(Default)]
    struct SharedLog {
        calls: std::sync::Mutex<Vec<String>>,
    }

    struct FakeShared {
        log: Arc<SharedLog>,
    }

    impl AudioBackend for FakeShared {
        fn load(&mut self, path: &Path) -> Result<Option<Duration>, AudioError> {
            self.log
                .calls
                .lock()
                .unwrap()
                .push(format!("load {}", path.display()));
            Ok(Some(Duration::from_secs(1)))
        }
        fn play(&mut self) -> Result<(), AudioError> {
            self.log.calls.lock().unwrap().push("play".to_owned());
            Ok(())
        }
        fn pause(&mut self) -> Result<(), AudioError> {
            self.log.calls.lock().unwrap().push("pause".to_owned());
            Ok(())
        }
        fn stop(&mut self) -> Result<(), AudioError> {
            self.log.calls.lock().unwrap().push("stop".to_owned());
            Ok(())
        }
        fn seek(&mut self, position: Duration) -> Result<Duration, AudioError> {
            self.log.calls.lock().unwrap().push("seek".to_owned());
            Ok(position)
        }
        fn set_volume(&mut self, _volume: f32) -> Result<(), AudioError> {
            Ok(())
        }
        fn position(&self) -> Duration {
            Duration::from_secs(5)
        }
        fn is_empty(&self) -> bool {
            false
        }
        fn resampling_info(&self) -> Option<ResamplingInfo> {
            Some(ResamplingInfo {
                mode: ResamplingMode::HighQuality,
                source_rate: Some(44_100),
                output_rate: 96_000,
                device_rate: 96_000,
                conversion: ResamplingConversion::HighQuality,
                fallback_reason: None,
                dsee_hx: DseeHxState::Off,
                dsee_notice: None,
                dsee_output_rate: None,
                exclusive: None,
                exclusive_fallback: None,
            })
        }
    }

    struct Rig {
        backend: ExclusiveBackend,
        capture: Arc<Capture>,
        /// The formats the fake device takes right now (change it to make the device refuse or accept).
        accepts: Arc<std::sync::Mutex<Vec<StreamFormat>>>,
        shared: Arc<SharedLog>,
    }

    fn rig(accepts: Vec<StreamFormat>) -> Rig {
        let capture = Arc::new(Capture::default());
        let accepts = Arc::new(std::sync::Mutex::new(accepts));
        let shared = Arc::new(SharedLog::default());
        let device_capture = Arc::clone(&capture);
        let device_accepts = Arc::clone(&accepts);
        let factory: EndpointFactory = Arc::new(move || {
            Ok(Box::new(FakeEndpoint {
                accepts: device_accepts.lock().unwrap().clone(),
                capture: Arc::clone(&device_capture),
                period_frames: 256,
                tick: Duration::from_micros(300),
            }) as Box<dyn Endpoint>)
        });
        let shared_log = Arc::clone(&shared);
        let fallback: FallbackFactory = Arc::new(move || {
            Ok(Box::new(FakeShared {
                log: Arc::clone(&shared_log),
            }) as Box<dyn AudioBackend>)
        });
        let config = Arc::new(ResamplingConfig::new(ResamplingMode::HighQuality, false));
        config.set_exclusive(true);
        let mut backend = ExclusiveBackend::with_factories(config, factory, fallback, None);
        // No real device lookups in unit tests (the tests run in parallel threads).
        backend.default_probe = Arc::new(|| None);
        Rig {
            backend,
            capture,
            accepts,
            shared,
        }
    }

    fn fake_backend(accepts: Vec<StreamFormat>) -> (ExclusiveBackend, Arc<Capture>) {
        let rig = rig(accepts);
        (rig.backend, rig.capture)
    }

    /// A 16-bit stereo WAV whose samples are known (left rises, right falls), so the bytes the device receives can be
    /// compared with the file's own.
    fn write_wav(name: &str, rate: u32, frames: usize) -> (PathBuf, Vec<i16>) {
        let samples: Vec<i16> = (0..frames)
            .flat_map(|index| {
                let value = ((index * 7 + 13) % 30_000) as i16 + 1;
                [value, -value]
            })
            .collect();
        let data_len = (samples.len() * 2) as u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 4).to_le_bytes());
        bytes.extend_from_slice(&4_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for sample in &samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let path =
            std::env::temp_dir().join(format!("moe-exclusive-{}-{name}.wav", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        (path, samples)
    }

    fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
        let started = Instant::now();
        while !condition() {
            assert!(
                started.elapsed() < Duration::from_secs(8),
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A stereo DSD64 DSF of `bytes` bytes per channel with a deterministic bit pattern; returns the file and the
    /// per-channel bytes (MSB-first, as the stream hands them out).
    fn write_dsf(name: &str, bytes: usize) -> (PathBuf, Vec<u8>, Vec<u8>) {
        let left: Vec<u8> = (0..bytes).map(|i| (i * 37 + 11) as u8).collect();
        let right: Vec<u8> = (0..bytes).map(|i| (i * 91 + 5) as u8).collect();
        let image = player_dsd::testing::build_dsf(
            2_822_400,
            bytes as u64 * 8,
            4096,
            false,
            &[left.clone(), right.clone()],
        );
        let path =
            std::env::temp_dir().join(format!("moe-exclusive-{}-{name}.dsf", std::process::id()));
        std::fs::write(&path, image).unwrap();
        (path, left, right)
    }

    /// The 24-in-32 words the fake device received, as (marker, left bits, right bits) per frame.
    fn dop_frames(bytes: &[u8]) -> Vec<(u8, u16, u16)> {
        let words: Vec<i32> = bytes
            .chunks(4)
            .filter(|chunk| chunk.len() == 4)
            .map(|chunk| i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) >> 8)
            .collect();
        words
            .chunks(2)
            .filter(|frame| frame.len() == 2)
            .map(|frame| ((frame[0] >> 16) as u8, frame[0] as u16, frame[1] as u16))
            .collect()
    }

    fn dop_on(rig: &Rig) {
        rig.backend.config.dsd.set(crate::DsdSettings {
            dop: true,
            ..Default::default()
        });
    }

    #[test]
    fn dop_reaches_the_device_as_the_dsd_bits_under_unbroken_alternating_markers() {
        let (path, left, right) = write_dsf("dop-bits", 8000);
        let mut rig = rig(vec![format(176_400, 2, Container::Int24In32)]);
        dop_on(&rig);
        rig.backend.load(&path).unwrap();
        rig.backend.play().unwrap();
        wait_until("the track to end", || rig.backend.is_empty());
        // Idle frames keep flowing after the last audio.
        std::thread::sleep(Duration::from_millis(40));
        let info = rig.backend.resampling_info().expect("info");
        let exclusive = info.exclusive.expect("exclusive info");
        assert_eq!(exclusive.dop_dsd_rate, Some(2_822_400));
        assert_eq!(exclusive.bits, 24);
        assert!(exclusive.bit_perfect && !exclusive.volume_applied);
        assert_eq!(exclusive.dop_fallback, None);
        assert_eq!(
            rig.capture.opened.lock().unwrap().as_slice(),
            [format(176_400, 2, Container::Int24In32)]
        );

        let frames = dop_frames(&rig.capture.bytes.lock().unwrap());
        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(
                frame.0,
                player_dsd::dop::DOP_MARKERS[index & 1],
                "the marker breaks at frame {index} (idle fill included)"
            );
        }
        let first = u16::from_be_bytes([left[0], left[1]]);
        let start = frames
            .iter()
            .position(|frame| frame.1 == first)
            .expect("the first audio frame");
        assert!(start > 0, "idle frames come before the audio");
        assert!(frames[..start]
            .iter()
            .all(|frame| frame.1 == 0x6969 && frame.2 == 0x6969));
        for index in 0..4000 {
            let expected_left = u16::from_be_bytes([left[index * 2], left[index * 2 + 1]]);
            let expected_right = u16::from_be_bytes([right[index * 2], right[index * 2 + 1]]);
            assert_eq!(
                (frames[start + index].1, frames[start + index].2),
                (expected_left, expected_right),
                "DSD bits at frame {index}"
            );
        }
        assert!(
            frames[start + 4000..]
                .iter()
                .rev()
                .take(20)
                .all(|frame| frame.1 == 0x6969),
            "idle pattern after the last audio"
        );
    }

    #[test]
    fn dop_ignores_the_player_volume() {
        let (path, left, _) = write_dsf("dop-volume", 4000);
        let mut rig = rig(vec![format(176_400, 2, Container::Int24In32)]);
        dop_on(&rig);
        rig.backend.set_volume(0.3).unwrap();
        rig.backend.load(&path).unwrap();
        rig.backend.play().unwrap();
        wait_until("the track to end", || rig.backend.is_empty());
        let frames = dop_frames(&rig.capture.bytes.lock().unwrap());
        let first = u16::from_be_bytes([left[0], left[1]]);
        assert!(
            frames.iter().any(|frame| frame.1 == first),
            "the bits are untouched at volume 30 %"
        );
        let exclusive = rig.backend.resampling_info().unwrap().exclusive.unwrap();
        assert!(exclusive.bit_perfect && !exclusive.volume_applied);
    }

    #[test]
    fn a_device_that_refuses_dop_plays_the_track_as_converted_pcm_and_says_why() {
        let (path, _, _) = write_dsf("dop-refused", 30_000);
        // Only 16-bit PCM at 176.4 kHz: DoP needs 24 bits, the converted PCM route takes 16.
        let mut rig = rig(vec![format(176_400, 2, Container::Int16)]);
        dop_on(&rig);
        rig.backend.load(&path).unwrap();
        let exclusive = rig.backend.resampling_info().unwrap().exclusive.unwrap();
        assert_eq!(exclusive.dop_dsd_rate, None);
        assert!(exclusive.dop_fallback.is_some(), "the reason is reported");
        assert_eq!(
            rig.capture.opened.lock().unwrap().as_slice(),
            [format(176_400, 2, Container::Int16)]
        );
    }

    #[test]
    fn dop_is_not_used_when_the_setting_is_off() {
        let (path, _, _) = write_dsf("dop-off", 30_000);
        let mut rig = rig(vec![
            format(176_400, 2, Container::Int24In32),
            format(176_400, 2, Container::Int16),
        ]);
        rig.backend.load(&path).unwrap();
        let exclusive = rig.backend.resampling_info().unwrap().exclusive.unwrap();
        assert_eq!(exclusive.dop_dsd_rate, None);
        assert_eq!(exclusive.dop_fallback, None);
    }

    #[test]
    fn seeking_a_dop_track_lands_on_a_frame_and_keeps_the_markers_unbroken() {
        let (path, left, _) = write_dsf("dop-seek", 40_000);
        let mut rig = rig(vec![format(176_400, 2, Container::Int24In32)]);
        dop_on(&rig);
        rig.backend.load(&path).unwrap();
        rig.backend.play().unwrap();
        let landed = rig.backend.seek(Duration::from_millis(50)).unwrap();
        let frame = (0.050_f64 * 176_400.0) as u64;
        assert_eq!(landed, Duration::from_secs_f64(frame as f64 / 176_400.0));
        wait_until("the track to end", || rig.backend.is_empty());
        let frames = dop_frames(&rig.capture.bytes.lock().unwrap());
        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(
                frame.0,
                player_dsd::dop::DOP_MARKERS[index & 1],
                "frame {index}"
            );
        }
        let at_seek = u16::from_be_bytes([left[frame as usize * 2], left[frame as usize * 2 + 1]]);
        assert!(
            frames.iter().any(|f| f.1 == at_seek),
            "the seek target's bits reach the device"
        );
    }

    #[test]
    fn a_wav_file_reaches_a_16_bit_device_sample_for_sample() {
        let (path, samples) = write_wav("bit-exact", 44_100, 30_000);
        let (mut backend, capture) = fake_backend(vec![format(44_100, 2, Container::Int16)]);
        backend.load(&path).unwrap();
        backend.play().unwrap();
        wait_until("the track to end", || backend.is_empty());
        std::thread::sleep(Duration::from_millis(30));
        let bytes = capture.bytes.lock().unwrap().clone();
        let received: Vec<i16> = bytes
            .chunks(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let start = received.iter().position(|&value| value != 0).unwrap();
        assert_eq!(
            &received[start..start + samples.len()],
            &samples[..],
            "the file's samples, unchanged"
        );
        let info = backend.resampling_info().unwrap();
        let exclusive = info.exclusive.unwrap();
        assert!(exclusive.bit_perfect);
        assert_eq!((exclusive.bits, exclusive.channels), (16, 2));
        assert_eq!(info.conversion, ResamplingConversion::None);
    }

    #[test]
    fn a_24_bit_container_holds_the_16_bit_samples_shifted_up_exactly() {
        let (path, samples) = write_wav("container", 48_000, 10_000);
        let (mut backend, capture) = fake_backend(vec![format(48_000, 2, Container::Int24In32)]);
        backend.load(&path).unwrap();
        backend.play().unwrap();
        wait_until("the track to end", || backend.is_empty());
        std::thread::sleep(Duration::from_millis(30));
        let bytes = capture.bytes.lock().unwrap().clone();
        let received: Vec<i32> = bytes
            .chunks(4)
            .map(|word| i32::from_le_bytes([word[0], word[1], word[2], word[3]]) >> 8)
            .collect();
        let start = received.iter().position(|&value| value != 0).unwrap();
        let expected: Vec<i32> = samples
            .iter()
            .map(|&value| i32::from(value) * 256)
            .collect();
        assert_eq!(&received[start..start + expected.len()], &expected[..]);
    }

    #[test]
    fn a_rate_the_device_refuses_is_converted_and_the_status_says_so() {
        let (path, _) = write_wav("convert", 44_100, 44_100);
        let (mut backend, capture) = fake_backend(vec![format(48_000, 2, Container::Int24In32)]);
        let duration = backend.load(&path).unwrap().unwrap();
        assert!((duration.as_secs_f64() - 1.0).abs() < 0.01);
        backend.play().unwrap();
        wait_until("the track to end", || backend.is_empty());
        std::thread::sleep(Duration::from_millis(30));
        let info = backend.resampling_info().unwrap();
        assert_eq!((info.source_rate, info.output_rate), (Some(44_100), 48_000));
        assert_eq!(info.conversion, ResamplingConversion::HighQuality);
        assert!(!info.exclusive.unwrap().bit_perfect);
        // About one second of 48 kHz stereo 24-in-32 reached the device (plus the silence around it).
        let frames = capture.bytes.lock().unwrap().len() / 8;
        assert!(frames >= 47_000, "{frames} frames");
    }

    #[test]
    fn volume_below_full_marks_the_stream_as_not_bit_perfect() {
        let (path, _) = write_wav("volume", 44_100, 2_000);
        let (mut backend, _capture) = fake_backend(vec![format(44_100, 2, Container::Int24In32)]);
        backend.load(&path).unwrap();
        assert!(
            backend
                .resampling_info()
                .unwrap()
                .exclusive
                .unwrap()
                .bit_perfect
        );
        backend.set_volume(0.5).unwrap();
        assert!(
            !backend
                .resampling_info()
                .unwrap()
                .exclusive
                .unwrap()
                .bit_perfect
        );
    }

    #[test]
    fn seek_continues_from_the_new_position_and_the_position_follows_playback() {
        let (path, samples) = write_wav("seek", 44_100, 88_200);
        let (mut backend, capture) = fake_backend(vec![format(44_100, 2, Container::Int16)]);
        let duration = backend.load(&path).unwrap().unwrap();
        assert!((duration.as_secs_f64() - 2.0).abs() < 0.01);
        backend.play().unwrap();
        let reached = backend.seek(Duration::from_millis(1_500)).unwrap();
        assert_eq!(reached, Duration::from_millis(1_500));
        wait_until("the track to end", || backend.is_empty());
        std::thread::sleep(Duration::from_millis(30));
        // After the seek the device received the tail of the file (0.5 s), not the head.
        let bytes = capture.bytes.lock().unwrap().clone();
        let received: Vec<i16> = bytes
            .chunks(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let tail_start = 66_150 * 2;
        let tail = &samples[tail_start..];
        let found = received
            .windows(64)
            .position(|window| window == &tail[..64])
            .expect("the tail of the file was played after the seek");
        assert_eq!(&received[found..found + tail.len()], tail);
        let position = backend.position();
        assert!(
            position >= Duration::from_millis(1_990) && position <= Duration::from_millis(2_000),
            "{position:?}"
        );
    }

    #[test]
    fn pause_holds_the_position_and_stop_gives_the_device_back() {
        let (path, _) = write_wav("pause", 44_100, 441_000);
        let (mut backend, capture) = fake_backend(vec![format(44_100, 2, Container::Int24In32)]);
        backend.load(&path).unwrap();
        backend.play().unwrap();
        wait_until("playback to start", || {
            backend.position() > Duration::from_millis(100)
        });
        backend.pause().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let held = backend.position();
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(backend.position(), held);
        backend.stop().unwrap();
        assert!(backend.session.is_none(), "the stream is closed");
        assert_eq!(capture.opened.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_next_track_in_the_same_format_keeps_the_stream_open() {
        let (first, _) = write_wav("same-a", 44_100, 2_000);
        let (second, _) = write_wav("same-b", 44_100, 2_000);
        let (other, _) = write_wav("other", 48_000, 2_000);
        let (mut backend, capture) = fake_backend(vec![
            format(44_100, 2, Container::Int24In32),
            format(48_000, 2, Container::Int24In32),
        ]);
        backend.load(&first).unwrap();
        backend.load(&second).unwrap();
        assert_eq!(
            capture.opened.lock().unwrap().len(),
            1,
            "same format: no re-open"
        );
        backend.load(&other).unwrap();
        assert_eq!(
            capture.opened.lock().unwrap().len(),
            2,
            "a new rate re-opens the stream"
        );
    }

    #[test]
    fn leaving_exclusive_mode_or_losing_the_device_asks_for_a_rebuild() {
        let (path, _) = write_wav("status", 44_100, 100_000);
        let (mut backend, capture) = fake_backend(vec![format(44_100, 2, Container::Int24In32)]);
        assert_eq!(backend.output_status(), OutputStatus::Healthy);
        backend.load(&path).unwrap();
        backend.play().unwrap();
        capture.fail.store(true, Ordering::Relaxed);
        wait_until("the failure", || {
            matches!(backend.output_status(), OutputStatus::Lost(_))
        });
        let (mut backend, _) = fake_backend(vec![format(44_100, 2, Container::Int24In32)]);
        backend.config.set_exclusive(false);
        assert_eq!(backend.output_status(), OutputStatus::DefaultDeviceChanged);
    }

    /// Opens a real endpoint in exclusive mode and plays a second of a silent (volume 0) track through it.
    /// `MOE_EXCLUSIVE_DEVICE` is a fragment of the device name (default "Realtek Digital").
    #[test]
    #[ignore = "takes a real output device exclusively for a few seconds (silent); run manually"]
    fn real_device_opens_exclusive_and_plays() {
        use crate::output_devices::{list_output_devices, OutputDeviceSetting};
        let fragment =
            std::env::var("MOE_EXCLUSIVE_DEVICE").unwrap_or_else(|_| "Realtek Digital".to_owned());
        // Rates to play (Hz, comma separated); each plays a one-second tone.
        let rates: Vec<u32> = std::env::var("MOE_EXCLUSIVE_RATES")
            .unwrap_or_else(|_| "44100,96000".to_owned())
            .split(',')
            .filter_map(|rate| rate.trim().parse().ok())
            .collect();
        let seconds: usize = std::env::var("MOE_EXCLUSIVE_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        let devices = list_output_devices();
        let device = devices
            .iter()
            .find(|device| device.name.contains(&fragment))
            .unwrap_or_else(|| panic!("no device named like {fragment:?}: {devices:?}"));
        let config = Arc::new(ResamplingConfig::new(ResamplingMode::HighQuality, false));
        config.set_exclusive(true);
        config.set_devices(vec![OutputDeviceSetting {
            id: Some(device.id.clone()),
            ..OutputDeviceSetting::default()
        }]);
        let mut backend = ExclusiveBackend::new(Arc::clone(&config));
        backend.set_volume(0.0).unwrap();
        println!("device: {}, {seconds} s per rate", device.name);
        for rate in rates {
            let (path, _) = write_wav(&format!("real-{rate}"), rate, rate as usize * seconds);
            let started = Instant::now();
            let duration = backend.load(&path).unwrap();
            if let Some(session) = backend.session.as_ref() {
                let frames = session.control.latency_frames.load(Ordering::Relaxed);
                println!(
                    "    buffer {frames} frames = {:.3} ms at {rate} Hz",
                    frames as f64 * 1000.0 / f64::from(rate)
                );
            }
            let opened = started.elapsed();
            backend.play().unwrap();
            let playing_since = Instant::now();
            let info = backend.resampling_info().unwrap();
            let Some(exclusive) = info.exclusive.clone() else {
                panic!(
                    "{rate} Hz fell back to shared mode: {:?}",
                    info.exclusive_fallback
                );
            };
            let mut next_report = 5;
            while !backend.is_empty() {
                assert!(
                    playing_since.elapsed() < Duration::from_secs(seconds as u64 + 20),
                    "timed out playing {rate} Hz"
                );
                std::thread::sleep(Duration::from_millis(50));
                if playing_since.elapsed().as_secs() >= next_report {
                    println!(
                        "    {:>2} s: position {:>7.3} s, underruns {}",
                        next_report,
                        backend.position().as_secs_f64(),
                        backend
                            .resampling_info()
                            .unwrap()
                            .exclusive
                            .unwrap()
                            .underruns
                    );
                    next_report += 5;
                }
            }
            let played_for = playing_since.elapsed();
            let after = backend.resampling_info().unwrap().exclusive.unwrap();
            println!(
                "{rate:>6} Hz -> opened {} bit {} ch at {} Hz ({}), open {:>4} ms, played {:>5} ms of {:?}, underruns {}",
                exclusive.bits,
                exclusive.channels,
                info.output_rate,
                if info.output_rate == rate { "no conversion" } else { "converted" },
                opened.as_millis(),
                played_for.as_millis(),
                duration,
                after.underruns
            );
            assert_eq!(backend.output_status(), OutputStatus::Healthy);
        }
        backend.stop().unwrap();
    }

    /// Follows the real system default endpoint: opens it exclusively, remembers which endpoint that was, and stays
    /// healthy while Windows keeps it as the default (silent; switch the default by hand during the wait to see
    /// `DefaultDeviceChanged`, `MOE_DEFAULT_WAIT_SECONDS` sets the wait).
    #[test]
    #[ignore = "takes the real default output device exclusively for a few seconds (silent); run manually"]
    fn real_default_device_is_followed() {
        let wait: u64 = std::env::var("MOE_DEFAULT_WAIT_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(3);
        let config = Arc::new(ResamplingConfig::new(ResamplingMode::HighQuality, false));
        config.set_exclusive(true);
        let mut backend = ExclusiveBackend::new(Arc::clone(&config));
        backend.set_volume(0.0).unwrap();
        let (path, _) = write_wav("real-default", 44_100, 44_100 * 3);
        backend.load(&path).unwrap();
        backend.play().unwrap();
        println!("opened default: {:?}", backend.opened_default);
        assert!(backend.opened_default.is_some());
        let started = Instant::now();
        let mut status = OutputStatus::Healthy;
        while started.elapsed() < Duration::from_secs(wait) && status == OutputStatus::Healthy {
            std::thread::sleep(Duration::from_millis(200));
            status = backend.output_status();
        }
        println!("after {:?}: {status:?}", started.elapsed());
    }

    /// One backend holds a real device exclusively; a second one asked for the same device must say the device is in
    /// use (through the fallback reason, or in the error when shared mode cannot open the device either).
    #[test]
    #[ignore = "takes a real output device exclusively for a few seconds (silent); run manually"]
    fn real_device_in_use_is_reported_with_its_reason() {
        use crate::output_devices::{list_output_devices, OutputDeviceSetting};
        let fragment =
            std::env::var("MOE_EXCLUSIVE_DEVICE").unwrap_or_else(|_| "Realtek Digital".to_owned());
        let devices = list_output_devices();
        let device = devices
            .iter()
            .find(|device| device.name.contains(&fragment))
            .unwrap_or_else(|| panic!("no device named like {fragment:?}: {devices:?}"));
        let make = || {
            let config = Arc::new(ResamplingConfig::new(ResamplingMode::HighQuality, false));
            config.set_exclusive(true);
            config.set_devices(vec![OutputDeviceSetting {
                id: Some(device.id.clone()),
                ..OutputDeviceSetting::default()
            }]);
            ExclusiveBackend::new(config)
        };
        let (path, _) = write_wav("in-use", 44_100, 44_100);
        let mut holder = make();
        holder.set_volume(0.0).unwrap();
        holder.load(&path).unwrap();
        assert!(holder.resampling_info().unwrap().exclusive.is_some());
        let mut second = make();
        second.set_volume(0.0).unwrap();
        match second.load(&path) {
            Ok(_) => {
                let reason = second.resampling_info().unwrap().exclusive_fallback;
                println!("fell back to shared mode: {reason:?}");
                assert!(reason.unwrap().contains("使用"));
            }
            Err(error) => {
                println!("error: {error}");
                assert!(error.to_string().contains("使用"), "{error}");
            }
        }
    }

    #[test]
    fn a_device_that_takes_no_format_plays_the_track_in_shared_mode_and_says_why() {
        let (path, _) = write_wav("refused", 44_100, 1_000);
        let mut rig = rig(Vec::new());
        let duration = rig.backend.load(&path).unwrap();
        assert_eq!(
            duration,
            Some(Duration::from_secs(1)),
            "the shared backend loaded it"
        );
        let info = rig.backend.resampling_info().unwrap();
        assert!(info.exclusive.is_none());
        assert!(
            info.exclusive_fallback.as_deref().unwrap().contains("格式"),
            "{:?}",
            info.exclusive_fallback
        );
        rig.backend.play().unwrap();
        rig.backend.pause().unwrap();
        assert_eq!(
            rig.backend.seek(Duration::from_secs(3)).unwrap(),
            Duration::from_secs(3)
        );
        assert_eq!(
            rig.backend.position(),
            Duration::from_secs(5),
            "the shared backend reports the position"
        );
        let calls = rig.shared.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 4);
        assert!(calls[0].starts_with("load"));
        assert_eq!(&calls[1..], ["play", "pause", "seek"]);
    }

    #[test]
    fn the_next_track_tries_exclusive_mode_again_and_drops_the_shared_fallback() {
        let (first, _) = write_wav("again-a", 44_100, 2_000);
        let (second, _) = write_wav("again-b", 44_100, 2_000);
        let mut rig = rig(Vec::new());
        rig.backend.load(&first).unwrap();
        assert!(rig
            .backend
            .resampling_info()
            .unwrap()
            .exclusive_fallback
            .is_some());
        // The device is free now (for example the other program let go of it).
        *rig.accepts.lock().unwrap() = vec![format(44_100, 2, Container::Int24In32)];
        rig.backend.load(&second).unwrap();
        let info = rig.backend.resampling_info().unwrap();
        assert!(info.exclusive.is_some() && info.exclusive_fallback.is_none());
        assert!(rig.backend.fallback.is_none());
        assert_eq!(rig.capture.opened.lock().unwrap().len(), 1);
    }

    #[test]
    fn when_the_shared_fallback_fails_too_the_error_names_both_reasons() {
        let (path, _) = write_wav("both", 44_100, 1_000);
        let capture = Arc::new(Capture::default());
        let factory: EndpointFactory = Arc::new(move || {
            Ok(Box::new(FakeEndpoint {
                accepts: Vec::new(),
                capture: Arc::clone(&capture),
                period_frames: 256,
                tick: Duration::from_micros(300),
            }) as Box<dyn Endpoint>)
        });
        let fallback: FallbackFactory = Arc::new(|| {
            Err(AudioError::OutputDevice(
                "shared mode is busy too".to_owned(),
            ))
        });
        let config = Arc::new(ResamplingConfig::new(ResamplingMode::HighQuality, false));
        config.set_exclusive(true);
        let mut backend = ExclusiveBackend::with_factories(config, factory, fallback, None);
        backend.default_probe = Arc::new(|| None);
        let message = backend.load(&path).unwrap_err().to_string();
        assert!(message.contains("獨佔模式無法使用"), "{message}");
        assert!(message.contains("格式"), "the exclusive reason: {message}");
        assert!(
            message.contains("shared mode is busy too"),
            "the shared reason: {message}"
        );
    }

    #[test]
    fn a_file_that_cannot_be_opened_is_an_error_not_a_reason_to_fall_back() {
        let mut rig = rig(Vec::new());
        let error = rig
            .backend
            .load(Path::new("Z:/no/such/file.flac"))
            .unwrap_err();
        assert!(rig.backend.fallback.is_none(), "{error}");
        assert!(rig.shared.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn the_device_is_released_after_the_idle_time_and_playing_reopens_it_where_it_was() {
        let (path, _) = write_wav("idle", 44_100, 441_000);
        let mut rig = rig(vec![format(44_100, 2, Container::Int24In32)]);
        rig.backend.idle_release = Duration::from_millis(200);
        rig.backend.load(&path).unwrap();
        rig.backend.play().unwrap();
        wait_until("playback to start", || {
            rig.backend.position() > Duration::from_millis(300)
        });
        // Playing is not idle, however long it takes.
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(rig.backend.output_status(), OutputStatus::Healthy);
        assert!(rig.backend.session.is_some());
        rig.backend.pause().unwrap();
        let paused_at = rig.backend.position();
        wait_until("the device to be released", || {
            rig.backend.output_status();
            rig.backend.session.is_none()
        });
        assert!(rig.backend.resampling_info().is_none());
        assert_eq!(rig.backend.position(), paused_at, "the position is kept");
        assert!(!rig.backend.is_empty(), "the track has not ended");
        // A seek while released only moves the position.
        let target = rig.backend.seek(Duration::from_millis(5_000)).unwrap();
        assert_eq!(target, Duration::from_millis(5_000));
        assert!(rig.backend.session.is_none());
        rig.backend.play().unwrap();
        assert_eq!(
            rig.capture.opened.lock().unwrap().len(),
            2,
            "the device was opened again"
        );
        assert!(rig.backend.position() >= Duration::from_millis(5_000));
        wait_until("playing to continue", || {
            rig.backend.position() > Duration::from_millis(5_300)
        });
    }

    #[test]
    fn following_the_system_default_a_change_of_the_default_asks_for_a_rebuild() {
        let (path, _) = write_wav("default-follow", 44_100, 441_000);
        let mut rig = rig(vec![format(44_100, 2, Container::Int24In32)]);
        let default = Arc::new(std::sync::Mutex::new(Some("endpoint-A".to_owned())));
        let probe_default = Arc::clone(&default);
        rig.backend.default_probe = Arc::new(move || probe_default.lock().unwrap().clone());
        rig.backend.default_poll = Duration::from_millis(50);
        rig.backend.load(&path).unwrap();
        rig.backend.play().unwrap();
        assert_eq!(rig.backend.opened_default.as_deref(), Some("endpoint-A"));
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(
            rig.backend.output_status(),
            OutputStatus::Healthy,
            "the default did not move"
        );
        // Windows makes another device the default.
        *default.lock().unwrap() = Some("endpoint-B".to_owned());
        wait_until("the change to be noticed", || {
            rig.backend.output_status() == OutputStatus::DefaultDeviceChanged
        });
    }

    #[test]
    fn a_chosen_device_does_not_follow_the_system_default() {
        let (path, _) = write_wav("default-chosen", 44_100, 441_000);
        let mut rig = rig(vec![format(44_100, 2, Container::Int24In32)]);
        rig.backend.device = Some("chosen-endpoint".to_owned());
        rig.backend
            .config
            .set_devices(vec![crate::output_devices::OutputDeviceSetting {
                id: Some("chosen-endpoint".to_owned()),
                ..Default::default()
            }]);
        let default = Arc::new(std::sync::Mutex::new(Some("endpoint-A".to_owned())));
        let probe_default = Arc::clone(&default);
        rig.backend.default_probe = Arc::new(move || probe_default.lock().unwrap().clone());
        rig.backend.default_poll = Duration::from_millis(20);
        rig.backend.load(&path).unwrap();
        *default.lock().unwrap() = Some("endpoint-B".to_owned());
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(rig.backend.output_status(), OutputStatus::Healthy);
    }

    #[test]
    fn a_track_that_ended_is_released_too_and_still_counts_as_ended() {
        let (path, _) = write_wav("idle-end", 44_100, 4_410);
        let mut rig = rig(vec![format(44_100, 2, Container::Int24In32)]);
        rig.backend.idle_release = Duration::from_millis(150);
        rig.backend.load(&path).unwrap();
        rig.backend.play().unwrap();
        wait_until("the track to end", || rig.backend.is_empty());
        wait_until("the device to be released", || {
            rig.backend.output_status();
            rig.backend.session.is_none()
        });
        assert!(rig.backend.is_empty());
    }

    #[test]
    fn a_track_that_is_loaded_but_never_played_is_released_after_the_idle_time() {
        let (path, _) = write_wav("idle-loaded", 44_100, 44_100);
        let mut rig = rig(vec![format(44_100, 2, Container::Int24In32)]);
        rig.backend.idle_release = Duration::from_millis(150);
        rig.backend.load(&path).unwrap();
        wait_until("the device to be released", || {
            rig.backend.output_status();
            rig.backend.session.is_none()
        });
        assert_eq!(rig.backend.position(), Duration::ZERO);
    }
}
