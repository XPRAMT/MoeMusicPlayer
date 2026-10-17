//! The rodio source for DSD→PCM playback (ADR 0016): pulls DSD from a [`DsdStream`], converts it block by block and
//! hands out interleaved `f32` samples at a fixed PCM rate, delay-compensated and trimmed to the exact frame count.

use std::sync::Arc;
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};

use player_dsd::convert::{ConversionPlan, DsdPcmConverter};
use player_dsd::stream::{DsdError, DsdInfo, DsdStream, DSD_BASE_RATE, DSD_IDLE_BYTE};

/// PCM rate every DSD track is converted to unless told otherwise (ADR 0016).
pub(crate) const DEFAULT_OUTPUT_RATE: u32 = DSD_BASE_RATE * 4;
/// Digital gain applied to the ±1 bit mapping. The mapping puts DSD's nominal 0 dB reference (50% modulation) near
/// −6 dBFS, so DSD would play several dB quieter than PCM. Measured over 25 of the owner's 99 DSF files (whole-track
/// peaks): median −4.45 dBFS, maximum −3.52 dBFS. +3 dB lifts the loudest of them to −0.5 dBFS, which still clips
/// nothing, and brings DSD closer to PCM loudness; samples stay `f32` until the device.
pub(crate) const DEFAULT_GAIN_DB: f32 = 3.0;

/// Bytes per channel read from the stream per refill (one DSF block).
const READ_BYTES: usize = 4096;
/// Idle-pattern bytes per channel fed after the end of the stream to flush the filters; far more than the filters'
/// delay, so the cap only protects against a logic error.
const MAX_FLUSH_BYTES: usize = 64 * READ_BYTES;
/// Minimum bytes replayed before a seek target so every filter has forgotten its (unknown) state.
const MIN_PREROLL_BYTES: usize = 2048;
/// Most channels the converter plays in this version.
const MAX_CHANNELS: u16 = 2;

pub(crate) struct DsdPcmSource<S: DsdStream> {
    stream: S,
    info: DsdInfo,
    converter: DsdPcmConverter,
    out_rate: u32,
    channels: usize,
    bytes_per_frame: u64,
    discard_frames: u64,
    preroll_bytes: u64,
    total_frames: u64,
    /// Raw converter outputs still to be dropped (delay compensation, or the pre-roll after a seek).
    discard: u64,
    /// Absolute index of the next frame to be produced into `pcm`.
    frames_emitted: u64,
    input_done: bool,
    flushed_bytes: usize,
    in_buf: Vec<Vec<u8>>,
    chan_out: Vec<Vec<f32>>,
    /// Interleaved output of the current refill.
    pcm: Vec<Sample>,
    pcm_pos: usize,
}

impl<S: DsdStream> DsdPcmSource<S> {
    pub(crate) fn new(stream: S, out_rate: u32, gain_db: f32) -> Result<Self, DsdError> {
        let info = stream.info();
        if info.channels == 0 || info.channels > MAX_CHANNELS {
            return Err(DsdError::Unsupported(format!(
                "目前只支援單聲道與雙聲道 DSD（此檔 {} 聲道）",
                info.channels
            )));
        }
        if info.rate_multiple().is_none() {
            return Err(DsdError::Unsupported(format!(
                "不是 44.1 kHz 系列的 DSD 取樣率（{} Hz）",
                info.sample_rate
            )));
        }
        let plan = ConversionPlan::new(info.sample_rate, out_rate)?;
        let channels = usize::from(info.channels);
        let bytes_per_frame = plan.bytes_per_frame() as u64;
        let discard_frames = plan.discard_frames() as u64;
        let preroll_bytes = (plan.settle_bytes() * 2).max(MIN_PREROLL_BYTES) as u64;
        let total_frames = info.samples_per_channel / plan.ratio as u64;
        let converter = DsdPcmConverter::new(plan, channels, gain_db);
        Ok(Self {
            stream,
            info,
            converter,
            out_rate,
            channels,
            bytes_per_frame,
            discard_frames,
            preroll_bytes,
            total_frames,
            discard: discard_frames,
            frames_emitted: 0,
            input_done: false,
            flushed_bytes: 0,
            in_buf: vec![Vec::new(); channels],
            chan_out: vec![Vec::new(); channels],
            pcm: Vec::new(),
            pcm_pos: 0,
        })
    }

    /// Convert more input until at least one frame is ready or the track is finished.
    fn refill(&mut self) {
        self.pcm.clear();
        self.pcm_pos = 0;
        while self.pcm.is_empty() && self.frames_emitted < self.total_frames {
            let mut count = 0;
            if !self.input_done {
                match self.stream.read(READ_BYTES, &mut self.in_buf) {
                    Ok(read) => count = read,
                    Err(error) => {
                        eprintln!("[audio] DSD read failed, ending the track: {error}");
                        self.input_done = true;
                    }
                }
                if count == 0 {
                    self.input_done = true;
                }
            }
            if count == 0 {
                // Past the end: feed idle pattern so the filters can deliver the last frames.
                if self.flushed_bytes >= MAX_FLUSH_BYTES {
                    self.total_frames = self.frames_emitted;
                    break;
                }
                for buffer in &mut self.in_buf {
                    buffer.clear();
                    buffer.resize(READ_BYTES, DSD_IDLE_BYTE);
                }
                self.flushed_bytes += READ_BYTES;
            }
            self.converter.process(&self.in_buf, &mut self.chan_out);
            let produced = self.chan_out[0].len() as u64;
            let skip = self.discard.min(produced);
            self.discard -= skip;
            let take = (produced - skip).min(self.total_frames - self.frames_emitted);
            for frame in skip..skip + take {
                for channel in &self.chan_out {
                    self.pcm.push(channel[frame as usize]);
                }
            }
            self.frames_emitted += take;
        }
    }

    /// Reposition so the next frame produced is `frame` (clamped to the end).
    pub(crate) fn seek_to_frame(&mut self, frame: u64) -> Result<(), DsdError> {
        let target = frame.min(self.total_frames);
        let raw_target = target + self.discard_frames;
        // Start on a frame boundary of the input so the decimation phase matches a sequential run.
        let start_byte = (raw_target * self.bytes_per_frame).saturating_sub(self.preroll_bytes)
            / self.bytes_per_frame
            * self.bytes_per_frame;
        self.stream.seek_to_byte(start_byte)?;
        self.converter.reset();
        self.discard = raw_target - start_byte / self.bytes_per_frame;
        self.frames_emitted = target;
        self.input_done = false;
        self.flushed_bytes = 0;
        self.pcm.clear();
        self.pcm_pos = 0;
        Ok(())
    }

    fn duration_of(&self, frames: u64) -> Duration {
        Duration::from_secs_f64(frames as f64 / f64::from(self.out_rate))
    }
}

impl<S: DsdStream> Iterator for DsdPcmSource<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.pcm_pos >= self.pcm.len() {
            self.refill();
            if self.pcm.is_empty() {
                return None;
            }
        }
        let sample = self.pcm[self.pcm_pos];
        self.pcm_pos += 1;
        Some(sample)
    }
}

impl<S: DsdStream> Source for DsdPcmSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.channels as u16).expect("DSD channel count is 1 or 2")
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.out_rate).expect("DSD output rate")
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(self.duration_of(
            self.info.samples_per_channel / (self.info.sample_rate / self.out_rate) as u64,
        ))
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        let frame = (position.as_secs_f64() * f64::from(self.out_rate)).round() as u64;
        self.seek_to_frame(frame)
            .map_err(|error| SeekError::Other(Arc::new(error)))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use player_dsd::dsf::DsfReader;
    use player_dsd::testing::{build_dsf, fit_sine, modulate, tone};

    /// A stereo DSD64 file with 1 kHz on the left and 3 kHz on the right, `seconds` long, in file (LSB-first) order.
    fn stereo_file(seconds: f64) -> (Vec<u8>, usize) {
        let rate = 2_822_400_u32;
        let samples = (f64::from(rate) * seconds) as usize / 16 * 16;
        let msb_left = modulate(samples, tone(rate, 1_000.0, 0.25));
        let msb_right = modulate(samples, tone(rate, 3_000.0, 0.25));
        let lsb = |bytes: Vec<u8>| bytes.into_iter().map(u8::reverse_bits).collect::<Vec<u8>>();
        (
            build_dsf(
                rate,
                samples as u64,
                4096,
                true,
                &[lsb(msb_left), lsb(msb_right)],
            ),
            samples,
        )
    }

    fn open(file: Vec<u8>) -> DsdPcmSource<DsfReader<Cursor<Vec<u8>>>> {
        DsdPcmSource::new(
            DsfReader::open(Cursor::new(file)).unwrap(),
            DEFAULT_OUTPUT_RATE,
            0.0,
        )
        .unwrap()
    }

    fn channel(samples: &[Sample], index: usize) -> Vec<f32> {
        samples.iter().skip(index).step_by(2).copied().collect()
    }

    #[test]
    fn plays_the_exact_duration_with_both_channels_at_the_right_level() {
        let (file, samples) = stereo_file(0.25);
        let mut source = open(file);
        assert_eq!(source.channels().get(), 2);
        assert_eq!(source.sample_rate().get(), 176_400);
        let expected_frames = samples / 16;
        assert_eq!(
            source.total_duration().unwrap(),
            Duration::from_secs_f64(expected_frames as f64 / 176_400.0)
        );
        let all: Vec<Sample> = source.by_ref().collect();
        assert_eq!(all.len(), expected_frames * 2);
        let (left, right) = (channel(&all, 0), channel(&all, 1));
        let start = left.len() / 10;
        let (left_level, _) = fit_sine(&left[start..left.len() - start], start, 176_400, 1_000.0);
        let (right_level, _) =
            fit_sine(&right[start..right.len() - start], start, 176_400, 3_000.0);
        assert!((left_level / 0.25 - 1.0).abs() < 0.002, "{left_level}");
        assert!((right_level / 0.25 - 1.0).abs() < 0.002, "{right_level}");
        // The tail is not silenced by the filter flush: the tone is still at full level at the very end.
        let tail = &left[left.len() - 2000..left.len() - 100];
        let (tail_level, _) = fit_sine(tail, left.len() - 2000, 176_400, 1_000.0);
        assert!((tail_level / 0.25 - 1.0).abs() < 0.01, "{tail_level}");
        assert!(source.next().is_none());
    }

    #[test]
    fn seeking_matches_sequential_playback() {
        let (file, _) = stereo_file(0.3);
        let reference: Vec<Sample> = open(file.clone()).collect();
        let mut source = open(file);
        for &frame in &[0_u64, 1, 17, 5000, 5001, 20_000, 31_000] {
            let position = Duration::from_secs_f64(frame as f64 / 176_400.0);
            source.try_seek(position).unwrap();
            let got: Vec<Sample> = source.by_ref().take(600).collect();
            let want =
                &reference[frame as usize * 2..(frame as usize * 2 + 600).min(reference.len())];
            assert_eq!(got.len(), want.len(), "frame {frame}");
            for (index, (a, b)) in got.iter().zip(want).enumerate() {
                assert!(
                    (a - b).abs() < 2e-6,
                    "frame {frame} sample {index}: {a} vs {b}"
                );
            }
        }
    }

    #[test]
    fn seeking_past_the_end_yields_nothing_and_seeking_back_works_again() {
        let (file, _) = stereo_file(0.1);
        let mut source = open(file);
        source.try_seek(Duration::from_secs(10)).unwrap();
        assert!(source.next().is_none());
        source.try_seek(Duration::ZERO).unwrap();
        assert!(source.next().is_some());
    }

    #[test]
    fn unsupported_streams_are_refused_with_a_reason() {
        // Three channels.
        let rate = 2_822_400_u32;
        let bytes = vec![0x69_u8; 1000];
        let file = build_dsf(
            rate,
            8000,
            4096,
            true,
            &[bytes.clone(), bytes.clone(), bytes.clone()],
        );
        let error = DsdPcmSource::new(DsfReader::open(Cursor::new(file)).unwrap(), 176_400, 0.0)
            .err()
            .unwrap();
        assert!(matches!(error, DsdError::Unsupported(_)), "{error}");
        // 48 kHz family.
        let file = build_dsf(3_072_000, 8000, 4096, true, &[bytes]);
        let error = DsdPcmSource::new(DsfReader::open(Cursor::new(file)).unwrap(), 176_400, 0.0)
            .err()
            .unwrap();
        assert!(matches!(error, DsdError::Unsupported(_)), "{error}");
    }

    #[test]
    fn mono_dsd_plays_as_one_channel() {
        let rate = 2_822_400_u32;
        let samples = rate as usize / 10 / 16 * 16;
        let bytes: Vec<u8> = modulate(samples, tone(rate, 2_000.0, 0.25))
            .into_iter()
            .map(u8::reverse_bits)
            .collect();
        let file = build_dsf(rate, samples as u64, 4096, true, &[bytes]);
        let mut source = open(file);
        assert_eq!(source.channels().get(), 1);
        let all: Vec<Sample> = source.by_ref().collect();
        assert_eq!(all.len(), samples / 16);
    }
}
