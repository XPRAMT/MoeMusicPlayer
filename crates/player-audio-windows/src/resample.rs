//! In-app sample-rate conversion for the "高品質" resampling mode.
//!
//! [`Resampled`] wraps a decoded Rodio source and converts it to the output
//! stream's rate with rubato's synchronous FFT resampler, so Rodio's mixer
//! sees matching rates and never applies its own linear/drop converter.
//!
//! - The FFT resampler uses a 2048-frame fixed input chunk with one FFT
//!   sub-chunk and a Blackman-Harris² window (flat to ~21 kHz, aliasing below
//!   -130 dB in the throwaway benchmark on the user's machine).
//! - The resampler's algorithmic delay is trimmed after construction, after a
//!   seek and after a mid-stream rate change, so track positions and lyrics
//!   stay aligned with the file.
//! - At the end of a segment the last partial chunk is padded with silence
//!   (`partial_len`) and the resampler is drained until exactly the expected
//!   number of output frames has been produced.
//! - The output channel count is fixed for the lifetime of the wrapper; a
//!   mid-stream channel change is remapped (extra channels dropped, missing
//!   ones copied from the last channel). A mid-stream rate change flushes the
//!   old segment and rebuilds the resampler; that rebuild is the only
//!   allocation that can happen while the audio callback pulls samples.

use std::sync::Arc;
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler, ResamplerConstructionError, WindowFunction};

/// Fixed input chunk of the FFT resampler in frames.
const FFT_CHUNK_FRAMES: usize = 2048;
/// One FFT per chunk; more sub-chunks shorten the filter and roll off the top
/// octave (rubato's `Fft::new` default loses ~15 dB at 20 kHz for 192→48 kHz).
const FFT_SUB_CHUNKS: usize = 1;
/// Frames copied per step while a segment already matches the output rate.
const BYPASS_CHUNK_FRAMES: usize = 1024;
/// Frames wider than this are truncated (Rodio decoders never produce them).
const MAX_FRAME_CHANNELS: usize = 32;

/// True when a source at `source_rate` needs conversion for `output_rate`.
pub(crate) fn needs_resampling(source_rate: SampleRate, output_rate: SampleRate) -> bool {
    source_rate != output_rate
}

/// A source the resampler could not be built for, returned unchanged.
pub(crate) struct Rejected<S> {
    pub(crate) source: S,
    pub(crate) error: ResamplerConstructionError,
}

impl<S> std::fmt::Debug for Rejected<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Rejected")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

/// `None` when `source_rate` already equals `target_rate`.
fn build_resampler(
    source_rate: u32,
    target_rate: u32,
    channels: usize,
) -> Result<Option<Fft<f32>>, ResamplerConstructionError> {
    if source_rate == target_rate {
        return Ok(None);
    }
    Fft::<f32>::new_custom(
        source_rate as usize,
        target_rate as usize,
        FFT_CHUNK_FRAMES,
        FFT_SUB_CHUNKS,
        channels,
        WindowFunction::BlackmanHarris2,
        FixedSync::Input,
    )
    .map(Some)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Running,
    /// The input segment ended; `fed_partial` is set once the final partial
    /// chunk has been processed and only silence is fed to drain the delay.
    Flushing {
        fed_partial: bool,
    },
    Finished,
}

/// A Rodio source resampled to a fixed output rate. See the module docs.
pub(crate) struct Resampled<S> {
    inner: S,
    target_rate: u32,
    /// Output channel count; constant for the lifetime of the wrapper.
    channels: usize,
    segment_rate: u32,
    segment_channels: usize,
    /// Samples left in the inner source's current span, if it reports spans.
    span_left: Option<usize>,
    /// `None` while the current segment already matches `target_rate`.
    resampler: Option<Fft<f32>>,
    input: Vec<f32>,
    input_frames: usize,
    output: Vec<f32>,
    out_pos: usize,
    out_end: usize,
    /// Output frames still to discard for the resampler delay.
    skip_frames: usize,
    /// Input frames read from `inner` in the current segment.
    frames_in: u64,
    /// Output frames kept (after delay trimming) in the current segment.
    frames_out: u64,
    stage: Stage,
    pending_rate: Option<u32>,
}

impl<S: Source> Resampled<S> {
    /// Wrap `inner` so it plays at `target_rate`. Construction allocates the
    /// FFT plans and buffers; call it off the audio callback. On failure the
    /// untouched source is handed back so the caller can still play it.
    pub(crate) fn new(inner: S, target_rate: SampleRate) -> Result<Self, Box<Rejected<S>>> {
        let channels = usize::from(inner.channels().get()).clamp(1, MAX_FRAME_CHANNELS);
        let segment_rate = inner.sample_rate().get();
        let resampler = match build_resampler(segment_rate, target_rate.get(), channels) {
            Ok(resampler) => resampler,
            Err(error) => {
                return Err(Box::new(Rejected {
                    source: inner,
                    error,
                }))
            }
        };
        let span_left = inner.current_span_len();
        let mut resampled = Self {
            inner,
            target_rate: target_rate.get(),
            channels,
            segment_rate,
            segment_channels: channels,
            span_left,
            resampler: None,
            input: Vec::new(),
            input_frames: 0,
            output: Vec::new(),
            out_pos: 0,
            out_end: 0,
            skip_frames: 0,
            frames_in: 0,
            frames_out: 0,
            stage: Stage::Running,
            pending_rate: None,
        };
        resampled.install_segment(segment_rate, resampler);
        Ok(resampled)
    }

    /// Algorithmic delay of the current segment in output frames (trimmed).
    #[cfg(test)]
    pub(crate) fn output_delay_frames(&self) -> usize {
        self.resampler
            .as_ref()
            .map_or(0, |resampler| resampler.output_delay())
    }

    fn configure_segment(&mut self, rate: u32) -> Result<(), ResamplerConstructionError> {
        let resampler = build_resampler(rate, self.target_rate, self.channels)?;
        self.install_segment(rate, resampler);
        Ok(())
    }

    fn install_segment(&mut self, rate: u32, resampler: Option<Fft<f32>>) {
        self.segment_rate = rate;
        self.resampler = resampler;
        let (input_frames, output_frames) = match self.resampler.as_ref() {
            Some(resampler) => (resampler.input_frames_max(), resampler.output_frames_max()),
            None => (BYPASS_CHUNK_FRAMES, BYPASS_CHUNK_FRAMES),
        };
        self.input.clear();
        self.input.resize(input_frames * self.channels, 0.0);
        self.output.clear();
        self.output.resize(output_frames * self.channels, 0.0);
        self.reset_segment();
    }

    fn reset_segment(&mut self) {
        if let Some(resampler) = self.resampler.as_mut() {
            resampler.reset();
        }
        self.skip_frames = self
            .resampler
            .as_ref()
            .map_or(0, |resampler| resampler.output_delay());
        self.input_frames = 0;
        self.out_pos = 0;
        self.out_end = 0;
        self.frames_in = 0;
        self.frames_out = 0;
        self.stage = Stage::Running;
    }

    fn chunk_frames(&self) -> usize {
        self.resampler
            .as_ref()
            .map_or(BYPASS_CHUNK_FRAMES, |resampler| {
                resampler.input_frames_next()
            })
    }

    fn expected_output_frames(&self) -> u64 {
        if self.resampler.is_none() {
            return self.frames_in;
        }
        let target = u64::from(self.target_rate);
        let source = u64::from(self.segment_rate);
        (self.frames_in * target + source / 2) / source
    }

    /// Fill the input chunk. Returns `false` when the segment ended first
    /// (end of stream or a sample-rate change at a span boundary).
    fn read_input(&mut self, need: usize) -> bool {
        let channels = self.channels;
        let mut frame = [0.0 as Sample; MAX_FRAME_CHANNELS];
        while self.input_frames < need {
            if self.span_left == Some(0) {
                self.span_left = self.inner.current_span_len();
                self.segment_channels = usize::from(self.inner.channels().get()).max(1);
                let rate = self.inner.sample_rate().get();
                if rate != self.segment_rate {
                    self.pending_rate = Some(rate);
                    return false;
                }
            }
            let source_channels = self.segment_channels;
            for channel in 0..source_channels {
                // A trailing partial frame is dropped.
                let Some(sample) = self.inner.next() else {
                    return false;
                };
                if let Some(slot) = frame.get_mut(channel) {
                    *slot = sample;
                }
            }
            if let Some(left) = self.span_left.as_mut() {
                *left = left.saturating_sub(source_channels);
            }
            let last = source_channels.min(MAX_FRAME_CHANNELS) - 1;
            let base = self.input_frames * channels;
            for channel in 0..channels {
                self.input[base + channel] = frame[channel.min(last)];
            }
            self.input_frames += 1;
            self.frames_in += 1;
        }
        true
    }

    /// Convert the buffered chunk. `partial` is the number of valid frames
    /// when the chunk is the (silence-padded) end of a segment. Returns the
    /// raw number of frames the resampler produced.
    fn process(&mut self, partial: Option<usize>) -> usize {
        let channels = self.channels;
        let flushing = matches!(self.stage, Stage::Flushing { .. });
        let expected = self.expected_output_frames();
        let produced = match self.resampler.as_mut() {
            Some(resampler) => {
                let need = resampler.input_frames_next();
                if let Some(valid) = partial {
                    self.input[valid * channels..need * channels].fill(0.0);
                }
                let capacity = resampler.output_frames_max();
                let indexing = partial.map(|valid| Indexing {
                    input_offset: 0,
                    output_offset: 0,
                    partial_len: Some(valid),
                    active_channels_mask: None,
                });
                let input = InterleavedSlice::new(&self.input[..need * channels], channels, need);
                let output = InterleavedSlice::new_mut(
                    &mut self.output[..capacity * channels],
                    channels,
                    capacity,
                );
                match (input, output) {
                    (Ok(input), Ok(mut output)) => resampler
                        .process_into_buffer(&input, &mut output, indexing.as_ref())
                        .map_or(0, |(_, frames)| frames),
                    _ => 0,
                }
            }
            None => {
                let frames = partial.unwrap_or(self.input_frames);
                self.output[..frames * channels].copy_from_slice(&self.input[..frames * channels]);
                frames
            }
        };
        self.input_frames = 0;
        let skipped = produced.min(self.skip_frames);
        self.skip_frames -= skipped;
        let mut kept = produced - skipped;
        if flushing {
            let remaining =
                usize::try_from(expected.saturating_sub(self.frames_out)).unwrap_or(usize::MAX);
            kept = kept.min(remaining);
        }
        self.frames_out += kept as u64;
        self.out_pos = skipped * channels;
        self.out_end = (skipped + kept) * channels;
        produced
    }

    /// Produce the next block of output. Returns `false` at the end.
    fn refill(&mut self) -> bool {
        loop {
            match self.stage {
                Stage::Finished => return false,
                Stage::Running => {
                    let need = self.chunk_frames();
                    if self.read_input(need) {
                        self.process(None);
                    } else {
                        self.stage = Stage::Flushing { fed_partial: false };
                        continue;
                    }
                }
                Stage::Flushing { fed_partial } => {
                    if fed_partial && self.frames_out >= self.expected_output_frames() {
                        match self.pending_rate.take() {
                            Some(rate) => {
                                if self.configure_segment(rate).is_err() {
                                    self.stage = Stage::Finished;
                                    return false;
                                }
                                continue;
                            }
                            None => {
                                self.stage = Stage::Finished;
                                return false;
                            }
                        }
                    }
                    let partial = if fed_partial { 0 } else { self.input_frames };
                    self.stage = Stage::Flushing { fed_partial: true };
                    if self.process(Some(partial)) == 0 && fed_partial {
                        // The resampler made no progress; never spin forever.
                        self.frames_out = self.expected_output_frames();
                    }
                }
            }
            if self.out_pos < self.out_end {
                return true;
            }
        }
    }
}

impl<S: Source> Iterator for Resampled<S> {
    type Item = Sample;

    #[inline]
    fn next(&mut self) -> Option<Sample> {
        if self.out_pos >= self.out_end && !self.refill() {
            return None;
        }
        let sample = self.output[self.out_pos];
        self.out_pos += 1;
        Some(sample)
    }
}

impl<S: Source> Source for Resampled<S> {
    fn current_span_len(&self) -> Option<usize> {
        // Channels and rate never change on the output side.
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.channels as u16).expect("at least one channel")
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.target_rate).expect("non-zero output rate")
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        self.inner.try_seek(position)?;
        self.pending_rate = None;
        self.span_left = self.inner.current_span_len();
        self.segment_channels = usize::from(self.inner.channels().get()).max(1);
        let rate = self.inner.sample_rate().get();
        if rate == self.segment_rate {
            self.reset_segment();
            Ok(())
        } else {
            self.configure_segment(rate).map_err(|error| {
                self.stage = Stage::Finished;
                SeekError::Other(Arc::new(error))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;
    use std::f64::consts::PI;

    fn rate(value: u32) -> SampleRate {
        SampleRate::new(value).unwrap()
    }

    fn stereo_tone(sample_rate: u32, frequency: f64, seconds: f64, amplitude: f64) -> Vec<f32> {
        let frames = (f64::from(sample_rate) * seconds).round() as usize;
        (0..frames)
            .flat_map(|frame| {
                let value = (amplitude
                    * (2.0 * PI * frequency * frame as f64 / f64::from(sample_rate)).sin())
                    as f32;
                [value, value]
            })
            .collect()
    }

    fn buffer(sample_rate: u32, samples: Vec<f32>) -> SamplesBuffer {
        SamplesBuffer::new(ChannelCount::new(2).unwrap(), rate(sample_rate), samples)
    }

    fn left(samples: &[f32]) -> Vec<f64> {
        samples
            .iter()
            .step_by(2)
            .map(|sample| f64::from(*sample))
            .collect()
    }

    /// RMS of the middle half, which excludes start/end filter transients.
    fn middle_rms(signal: &[f64]) -> f64 {
        let middle = &signal[signal.len() / 4..signal.len() * 3 / 4];
        (middle.iter().map(|value| value * value).sum::<f64>() / middle.len() as f64).sqrt()
    }

    fn db(ratio: f64) -> f64 {
        20.0 * ratio.max(1e-20).log10()
    }

    #[test]
    fn output_length_matches_the_rate_ratio() {
        for (from, to) in [
            (44_100, 96_000),
            (48_000, 96_000),
            (192_000, 96_000),
            (44_100, 48_000),
            (96_000, 44_100),
        ] {
            let input = stereo_tone(from, 1_000.0, 1.25, 0.5);
            let input_frames = input.len() / 2;
            let resampled = Resampled::new(buffer(from, input), rate(to)).unwrap();
            assert_eq!(resampled.sample_rate().get(), to);
            assert_eq!(resampled.channels().get(), 2);
            let output: Vec<f32> = resampled.collect();
            let expected =
                (input_frames as u64 * u64::from(to) + u64::from(from) / 2) / u64::from(from);
            assert_eq!(output.len() % 2, 0);
            assert_eq!(output.len() as u64 / 2, expected, "{from} -> {to}");
        }
    }

    #[test]
    fn algorithmic_delay_is_trimmed() {
        let (from, to) = (44_100, 96_000);
        let mut input = vec![0.0f32; from as usize * 2];
        let impulse_frame = 22_050;
        input[impulse_frame * 2] = 1.0;
        input[impulse_frame * 2 + 1] = 1.0;
        let resampled = Resampled::new(buffer(from, input), rate(to)).unwrap();
        assert!(resampled.output_delay_frames() > 0);
        let output = left(&resampled.collect::<Vec<_>>());
        let peak = output
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
            .0;
        let expected = impulse_frame * to as usize / from as usize;
        assert!(
            peak.abs_diff(expected) <= 1,
            "peak at {peak}, expected {expected}"
        );
    }

    #[test]
    fn passband_is_flat_through_20_khz() {
        for (from, to) in [(44_100, 96_000), (44_100, 48_000), (192_000, 96_000)] {
            for frequency in [1_000.0, 10_000.0, 20_000.0] {
                let resampled = Resampled::new(
                    buffer(from, stereo_tone(from, frequency, 1.0, 0.5)),
                    rate(to),
                )
                .unwrap();
                let rms = middle_rms(&left(&resampled.collect::<Vec<_>>()));
                let gain = db(rms / (0.5 / 2f64.sqrt()));
                assert!(
                    gain.abs() < 0.05,
                    "{from}->{to} {frequency} Hz gain {gain:.3} dB"
                );
            }
        }
    }

    #[test]
    fn tones_above_the_new_nyquist_are_rejected() {
        // 30 kHz would alias to 18 kHz at 48 kHz; 90 kHz to 6 kHz at 96 kHz.
        for (from, to, frequency) in [
            (192_000, 48_000, 30_000.0),
            (192_000, 96_000, 90_000.0),
            (96_000, 44_100, 30_000.0),
        ] {
            let resampled = Resampled::new(
                buffer(from, stereo_tone(from, frequency, 1.0, 0.5)),
                rate(to),
            )
            .unwrap();
            let rms = middle_rms(&left(&resampled.collect::<Vec<_>>()));
            let rejection = db(rms / (0.5 / 2f64.sqrt()));
            assert!(
                rejection < -100.0,
                "{from}->{to} {frequency} Hz: {rejection:.1} dB"
            );
        }
    }

    #[test]
    fn equal_rates_are_bypassed_sample_for_sample() {
        assert!(!needs_resampling(rate(96_000), rate(96_000)));
        assert!(needs_resampling(rate(44_100), rate(96_000)));
        let input = stereo_tone(96_000, 1_000.0, 0.3, 0.5);
        let resampled = Resampled::new(buffer(96_000, input.clone()), rate(96_000)).unwrap();
        assert_eq!(resampled.output_delay_frames(), 0);
        assert_eq!(resampled.collect::<Vec<_>>(), input);
    }

    #[test]
    fn seek_resets_the_resampler_and_lands_on_the_target() {
        let (from, to) = (44_100, 96_000);
        let input = stereo_tone(from, 1_000.0, 1.0, 0.5);
        let mut resampled = Resampled::new(buffer(from, input), rate(to)).unwrap();
        // Consume part of the stream so the resampler holds history.
        for _ in 0..30_000 {
            resampled.next().unwrap();
        }
        resampled.try_seek(Duration::from_millis(500)).unwrap();
        let output = left(&resampled.collect::<Vec<_>>());
        assert!(
            output.len().abs_diff(48_000) <= 1,
            "remaining {} frames",
            output.len()
        );
        // After the filter has settled the phase continues from 0.5 s.
        for (index, value) in output.iter().enumerate().skip(4_096).take(2_000) {
            let expected = 0.5 * (2.0 * PI * 1_000.0 * (0.5 + index as f64 / f64::from(to))).sin();
            assert!(
                (value - expected).abs() < 1e-3,
                "frame {index}: {value} vs {expected}"
            );
        }
    }

    #[test]
    fn tail_is_flushed_to_the_last_frame() {
        let (from, to) = (48_000, 96_000);
        let frames = 10_000;
        let mut input = vec![0.0f32; frames * 2];
        // Impulse 20 frames before the end must survive the final partial chunk.
        input[(frames - 20) * 2] = 1.0;
        let output = left(
            &Resampled::new(buffer(from, input), rate(to))
                .unwrap()
                .collect::<Vec<_>>(),
        );
        assert_eq!(output.len(), frames * 2);
        let peak = output
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap();
        assert!(
            peak.0.abs_diff((frames - 20) * 2) <= 1,
            "peak at {}",
            peak.0
        );
        assert!(*peak.1 > 0.5);
    }

    /// A source made of spans with their own channel count and rate, like a
    /// chained stream.
    struct Spans {
        spans: Vec<(u16, u32, Vec<f32>)>,
        index: usize,
        position: usize,
    }

    impl Iterator for Spans {
        type Item = Sample;

        fn next(&mut self) -> Option<Sample> {
            let span = self.spans.get(self.index)?;
            let sample = span.2[self.position];
            self.position += 1;
            if self.position == span.2.len() {
                self.index += 1;
                self.position = 0;
            }
            Some(sample)
        }
    }

    impl Source for Spans {
        fn current_span_len(&self) -> Option<usize> {
            Some(
                self.spans
                    .get(self.index)
                    .map_or(0, |span| span.2.len() - self.position),
            )
        }
        fn channels(&self) -> ChannelCount {
            let span = self.spans.get(self.index).or(self.spans.last()).unwrap();
            ChannelCount::new(span.0).unwrap()
        }
        fn sample_rate(&self) -> SampleRate {
            let span = self.spans.get(self.index).or(self.spans.last()).unwrap();
            rate(span.1)
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    #[test]
    fn mid_stream_rate_and_channel_changes_rebuild_the_resampler() {
        let first = stereo_tone(44_100, 1_000.0, 0.5, 0.5);
        let mono: Vec<f32> = stereo_tone(48_000, 1_000.0, 0.5, 0.5)
            .into_iter()
            .step_by(2)
            .collect();
        let source = Spans {
            spans: vec![(2, 44_100, first), (1, 48_000, mono)],
            index: 0,
            position: 0,
        };
        let output = Resampled::new(source, rate(96_000))
            .unwrap()
            .collect::<Vec<_>>();
        assert!(
            (output.len() / 2).abs_diff(96_000) <= 2,
            "{} frames",
            output.len() / 2
        );
        let (head, tail) = output.split_at(48_000 * 2);
        let expected = 0.5 / 2f64.sqrt();
        assert!(db(middle_rms(&left(head)) / expected).abs() < 0.05);
        assert!(db(middle_rms(&left(tail)) / expected).abs() < 0.05);
        // The mono segment is copied to both output channels.
        assert!(tail.chunks(2).all(|frame| frame[0] == frame[1]));
    }
}
