//! `ExclusiveBackend`: the `AudioBackend` for WASAPI exclusive mode. One track at a time goes from the decoder (rodio,
//! DSD, Opus) through an optional channel remix and rate conversion into a [`Session`]; at 100 % volume with a rate
//! and channel count the device takes, the samples reach it unchanged.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use rodio::{SampleRate, Source};

use super::endpoint::Endpoint;
use super::format::Negotiated;
use super::session::{SampleSource, Session};
use crate::mirror::MixSource;
use crate::output_devices::ChannelMixer;
use crate::resample::Resampled;
use crate::rodio_backend::{open_decoder_with, AudioDecoder};
use crate::{
    AudioBackend, AudioError, DseeHxState, ExclusiveInfo, OutputStatus, ResamplingConfig,
    ResamplingConversion, ResamplingInfo, ResamplingMode,
};

/// Builds the endpoint on the render thread (WASAPI objects must stay on the thread that made them).
pub(crate) type EndpointFactory = Arc<dyn Fn() -> Result<Box<dyn Endpoint>, String> + Send + Sync>;

struct Track {
    path: PathBuf,
    source_rate: u32,
    duration: Option<Duration>,
    /// Where in the track the current source started.
    base: Duration,
    /// Frames the device had played when the current source started.
    played_at_start: u64,
}

pub(crate) struct ExclusiveBackend {
    config: Arc<ResamplingConfig>,
    factory: EndpointFactory,
    /// The device the backend was built for; a different chosen device makes the worker rebuild it.
    device: Option<String>,
    session: Option<Session>,
    track: Option<Track>,
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
        Self::with_factory(config, factory, device)
    }

    pub(crate) fn with_factory(
        config: Arc<ResamplingConfig>,
        factory: EndpointFactory,
        device: Option<String>,
    ) -> Self {
        Self {
            config,
            factory,
            device,
            session: None,
            track: None,
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
}

impl AudioBackend for ExclusiveBackend {
    fn load(&mut self, path: &Path) -> Result<Option<Duration>, AudioError> {
        let decoder = open_decoder_with(path, &self.config.dsd)?;
        let source_rate = decoder.sample_rate().get();
        let source_channels = decoder.channels().get();
        let duration = decoder.total_duration();
        self.paused = true;
        self.track = None;
        // The same format as the last track keeps the stream open (no re-open click); anything else reopens it.
        let reusable = self.session.as_ref().is_some_and(|session| {
            session.negotiated.source_rate == source_rate
                && session.negotiated.source_channels == source_channels
                && session.control.failure().is_none()
        });
        if !reusable {
            self.close_session();
            let factory = Arc::clone(&self.factory);
            let session = Session::start(
                move || factory(),
                source_rate,
                source_channels,
                self.volume,
                true,
            )
            .map_err(AudioError::Backend)?;
            self.session = Some(session);
        }
        let session = self.session.as_ref().expect("session was just opened");
        session.control.paused.store(true, Ordering::Release);
        let chain = Self::chain(decoder, &session.negotiated)?;
        let played_at_start = session.play_source(chain);
        self.track = Some(Track {
            path: path.to_path_buf(),
            source_rate,
            duration,
            base: Duration::ZERO,
            played_at_start,
        });
        Ok(duration)
    }

    fn play(&mut self) -> Result<(), AudioError> {
        let (Some(session), Some(_)) = (self.session.as_ref(), self.track.as_ref()) else {
            return Err(AudioError::NoTrackLoaded);
        };
        self.paused = false;
        session.control.paused.store(false, Ordering::Release);
        Ok(())
    }

    fn pause(&mut self) -> Result<(), AudioError> {
        self.paused = true;
        if let Some(session) = self.session.as_ref() {
            session.control.paused.store(true, Ordering::Release);
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        // Stopping gives the device back to other programs.
        self.track = None;
        self.paused = true;
        self.close_session();
        Ok(())
    }

    fn seek(&mut self, position: Duration) -> Result<Duration, AudioError> {
        let (Some(session), Some(track)) = (self.session.as_ref(), self.track.as_mut()) else {
            return Err(AudioError::NoTrackLoaded);
        };
        let target = track
            .duration
            .map_or(position, |duration| position.min(duration));
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

    fn load_at(
        &mut self,
        path: &Path,
        position: Duration,
    ) -> Result<(Option<Duration>, Duration), AudioError> {
        let duration = self.load(path)?;
        let target = duration.map_or(position, |duration| position.min(duration));
        if target.is_zero() {
            return Ok((duration, Duration::ZERO));
        }
        let actual = self.seek(target)?;
        Ok((duration, actual))
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), AudioError> {
        self.volume = volume;
        if let Some(session) = self.session.as_ref() {
            session
                .control
                .volume
                .store(volume.to_bits(), Ordering::Relaxed);
        }
        Ok(())
    }

    fn position(&self) -> Duration {
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
        match (self.session.as_ref(), self.track.as_ref()) {
            (Some(session), Some(_)) => session.ring.finished(),
            _ => true,
        }
    }

    fn resampling_info(&self) -> Option<ResamplingInfo> {
        let session = self.session.as_ref()?;
        let negotiated = &session.negotiated;
        let format = negotiated.format;
        let volume_applied = self.volume < 0.9999;
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
            }),
        })
    }

    fn output_status(&mut self) -> OutputStatus {
        // Leaving exclusive mode, or choosing another device, rebuilds the backend (the worker restores the track).
        if !self.config.exclusive() || self.config.primary_device() != self.device {
            return OutputStatus::DefaultDeviceChanged;
        }
        if let Some(reason) = self
            .session
            .as_ref()
            .and_then(|session| session.control.failure())
        {
            return OutputStatus::Lost(reason);
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

    fn fake_backend(accepts: Vec<StreamFormat>) -> (ExclusiveBackend, Arc<Capture>) {
        let capture = Arc::new(Capture::default());
        let device_capture = Arc::clone(&capture);
        let factory: EndpointFactory = Arc::new(move || {
            Ok(Box::new(FakeEndpoint {
                accepts: accepts.clone(),
                capture: Arc::clone(&device_capture),
                period_frames: 256,
                tick: Duration::from_micros(300),
            }) as Box<dyn Endpoint>)
        });
        let config = Arc::new(ResamplingConfig::new(ResamplingMode::HighQuality, false));
        config.set_exclusive(true);
        (
            ExclusiveBackend::with_factory(config, factory, None),
            capture,
        )
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
        for (name, rate) in [("real-44k", 44_100_u32), ("real-96k", 96_000)] {
            let (path, _) = write_wav(name, rate, rate as usize);
            let duration = backend.load(&path).unwrap();
            backend.play().unwrap();
            let info = backend.resampling_info().unwrap();
            println!("{name}: {info:?} duration {duration:?}");
            wait_until("the second of audio to play", || backend.is_empty());
            let exclusive = info.exclusive.unwrap();
            println!(
                "  opened {} bit {} ch at {} Hz (track {} Hz), underruns {}",
                exclusive.bits,
                exclusive.channels,
                info.output_rate,
                rate,
                backend
                    .resampling_info()
                    .unwrap()
                    .exclusive
                    .unwrap()
                    .underruns
            );
            assert_eq!(backend.output_status(), OutputStatus::Healthy);
        }
        backend.stop().unwrap();
    }

    #[test]
    fn a_device_that_takes_no_format_reports_why_the_track_cannot_play() {
        let (path, _) = write_wav("refused", 44_100, 1_000);
        let (mut backend, _) = fake_backend(Vec::new());
        let error = backend.load(&path).unwrap_err();
        assert!(error.to_string().contains("格式"), "{error}");
    }
}
