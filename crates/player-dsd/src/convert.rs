//! The DSD→PCM conversion core (ADR 0016), independent of rodio so it can be tested on plain buffers.
//!
//! Per channel: stage A turns every DSD byte into one sample at `R / 8` using 16 table lookups, then zero or more
//! halfband stages halve the rate until the target PCM rate is reached. All stages are linear phase, so the whole
//! chain has a constant, known delay.

use std::sync::Arc;

use crate::filters::{
    halfband_tap_count, halfband_taps, stage_a_tables, HalfbandTaps, STAGE_A_DELAY_SAMPLES,
    STAGE_A_GROUPS,
};
use crate::stream::{DsdError, DSD_IDLE_BYTE};

/// DSD samples (bits) per stage-A output.
const BITS_PER_BYTE: usize = 8;

/// Static description of a conversion: the rates and the stage layout.
#[derive(Clone, Debug)]
pub struct ConversionPlan {
    pub dsd_rate: u32,
    pub out_rate: u32,
    /// DSD samples per output frame (`dsd_rate / out_rate`).
    pub ratio: usize,
    /// Tap count of each halfband stage, first stage first.
    pub halfband_taps: Vec<usize>,
}

impl ConversionPlan {
    /// `out_rate` must divide `dsd_rate / 8` by a power of two (DSD64 → 176.4 kHz is ÷16, i.e. one halfband).
    pub fn new(dsd_rate: u32, out_rate: u32) -> Result<Self, DsdError> {
        if out_rate == 0 || !dsd_rate.is_multiple_of(out_rate) {
            return Err(DsdError::Unsupported(format!(
                "DSD 取樣率 {dsd_rate} Hz 無法整數轉換到 {out_rate} Hz"
            )));
        }
        let ratio = (dsd_rate / out_rate) as usize;
        if !ratio.is_multiple_of(BITS_PER_BYTE) || !(ratio / BITS_PER_BYTE).is_power_of_two() {
            return Err(DsdError::Unsupported(format!(
                "DSD 取樣率 {dsd_rate} Hz 無法以 8 的二次冪倍數轉換到 {out_rate} Hz"
            )));
        }
        let stages = (ratio / BITS_PER_BYTE).trailing_zeros() as usize;
        let halfband_taps = (0..stages)
            .map(|stage| {
                halfband_tap_count(
                    f64::from(dsd_rate) / BITS_PER_BYTE as f64 / f64::from(1_u32 << stage),
                )
            })
            .collect();
        Ok(Self {
            dsd_rate,
            out_rate,
            ratio,
            halfband_taps,
        })
    }

    /// Input bytes consumed per output frame.
    pub fn bytes_per_frame(&self) -> usize {
        self.ratio / BITS_PER_BYTE
    }

    /// Group delay of the whole chain, in DSD samples, counted from the newest input sample of the byte at which a
    /// raw output is produced.
    pub fn delay_dsd_samples(&self) -> f64 {
        let mut delay = STAGE_A_DELAY_SAMPLES;
        for (stage, taps) in self.halfband_taps.iter().enumerate() {
            // A stage-`k` input sample spans 8 · 2^k DSD samples.
            delay += ((taps - 1) / 2) as f64 * (BITS_PER_BYTE << stage) as f64;
        }
        delay
    }

    /// Raw outputs to drop at the start so that output frame `f` is centred within half a frame of DSD sample
    /// `f · ratio`.
    pub fn discard_frames(&self) -> usize {
        let late = self.delay_dsd_samples() - (self.ratio as f64 - 1.0);
        (late / self.ratio as f64).round().max(0.0) as usize
    }

    /// Input bytes the filters need to forget their (unknown) state: after this many bytes of real data every
    /// stage's output no longer depends on what came before.
    pub fn settle_bytes(&self) -> usize {
        let mut bytes = STAGE_A_GROUPS;
        for (stage, taps) in self.halfband_taps.iter().enumerate() {
            bytes += taps << stage;
        }
        bytes
    }
}

struct HalfbandStage {
    taps: Arc<HalfbandTaps>,
    /// The last `2 · half_span` input samples.
    history: Vec<f32>,
    /// Parity of the number of input samples seen so far; an output is produced on every second sample.
    phase: usize,
    scratch: Vec<f32>,
}

impl HalfbandStage {
    fn new(taps: Arc<HalfbandTaps>) -> Self {
        let history = vec![0.0; 2 * taps.half_span];
        Self {
            taps,
            history,
            phase: 0,
            scratch: Vec::new(),
        }
    }

    fn reset(&mut self) {
        self.history.iter_mut().for_each(|sample| *sample = 0.0);
        self.phase = 0;
    }

    /// Consume `input`, append the decimated output to `output`.
    fn process(&mut self, input: &[f32], output: &mut Vec<f32>) {
        let span = self.taps.half_span;
        let history_len = 2 * span;
        self.scratch.clear();
        self.scratch.extend_from_slice(&self.history);
        self.scratch.extend_from_slice(input);
        let buffer = &self.scratch;
        for position in 0..input.len() {
            if (self.phase + position) % 2 != 1 {
                continue;
            }
            // Filter window is buffer[position ..= position + 2 * span]; its centre is `position + span`.
            let center = position + span;
            let mut sum = self.taps.center * buffer[center];
            for (j, tap) in self.taps.odd.iter().enumerate() {
                let offset = 2 * j + 1;
                sum += tap * (buffer[center - offset] + buffer[center + offset]);
            }
            output.push(sum);
        }
        self.phase = (self.phase + input.len()) % 2;
        let keep_from = buffer.len() - history_len;
        self.history.copy_from_slice(&buffer[keep_from..]);
    }
}

/// Stage-A state: the last `STAGE_A_GROUPS` bytes (newest first) in a double-length ring.
struct ByteWindow {
    ring: [u8; 2 * STAGE_A_GROUPS],
    position: usize,
}

impl ByteWindow {
    fn primed() -> Self {
        Self {
            ring: [DSD_IDLE_BYTE; 2 * STAGE_A_GROUPS],
            position: 0,
        }
    }

    #[inline]
    fn push(&mut self, byte: u8) -> &[u8] {
        self.position = (self.position + STAGE_A_GROUPS - 1) % STAGE_A_GROUPS;
        self.ring[self.position] = byte;
        self.ring[self.position + STAGE_A_GROUPS] = byte;
        &self.ring[self.position..self.position + STAGE_A_GROUPS]
    }
}

struct ChannelState {
    window: ByteWindow,
    stages: Vec<HalfbandStage>,
    /// Ping-pong buffers between stages.
    ping: Vec<f32>,
    pong: Vec<f32>,
}

pub struct DsdPcmConverter {
    plan: ConversionPlan,
    tables: Arc<Vec<[f32; 256]>>,
    channels: Vec<ChannelState>,
    gain: f32,
}

impl DsdPcmConverter {
    pub fn new(plan: ConversionPlan, channels: usize, gain_db: f32) -> Self {
        let tables = Arc::new(stage_a_tables());
        let halfbands: Vec<Arc<HalfbandTaps>> = plan
            .halfband_taps
            .iter()
            .map(|count| Arc::new(halfband_taps(*count)))
            .collect();
        let channels = (0..channels)
            .map(|_| ChannelState {
                window: ByteWindow::primed(),
                stages: halfbands
                    .iter()
                    .map(|taps| HalfbandStage::new(Arc::clone(taps)))
                    .collect(),
                ping: Vec::new(),
                pong: Vec::new(),
            })
            .collect();
        Self {
            plan,
            tables,
            channels,
            gain: 10_f32.powf(gain_db / 20.0),
        }
    }

    pub fn plan(&self) -> &ConversionPlan {
        &self.plan
    }

    /// Forget all history: as if the stream began after silence.
    pub fn reset(&mut self) {
        for channel in &mut self.channels {
            channel.window = ByteWindow::primed();
            channel.stages.iter_mut().for_each(HalfbandStage::reset);
        }
    }

    /// Convert one chunk. `input[c]` are the DSD bytes of channel `c` (MSB-first); `output[c]` is cleared and filled
    /// with the PCM frames the chunk completes (not yet delay-compensated).
    pub fn process(&mut self, input: &[Vec<u8>], output: &mut [Vec<f32>]) {
        debug_assert_eq!(input.len(), self.channels.len());
        debug_assert_eq!(output.len(), self.channels.len());
        let tables = &self.tables;
        let gain = self.gain;
        for ((state, bytes), target) in self.channels.iter_mut().zip(input).zip(output.iter_mut()) {
            state.ping.clear();
            for &byte in bytes {
                let window = state.window.push(byte);
                let mut sum = 0.0_f32;
                for (group, history_byte) in window.iter().enumerate() {
                    sum += tables[group][usize::from(*history_byte)];
                }
                state.ping.push(sum);
            }
            // Run the halfband stages, alternating between the two scratch buffers.
            for stage in &mut state.stages {
                state.pong.clear();
                stage.process(&state.ping, &mut state.pong);
                std::mem::swap(&mut state.ping, &mut state.pong);
            }
            target.clear();
            target.extend(state.ping.iter().map(|sample| sample * gain));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;
    use crate::testing::{convert_all, fit_sine, modulate, tone};

    fn db(ratio: f64) -> f64 {
        20.0 * ratio.log10()
    }

    #[test]
    fn plan_accepts_the_dsd_rate_family_and_rejects_the_rest() {
        let dsd64 = ConversionPlan::new(2_822_400, 176_400).unwrap();
        assert_eq!(
            (
                dsd64.ratio,
                dsd64.bytes_per_frame(),
                dsd64.halfband_taps.len()
            ),
            (16, 2, 1)
        );
        assert_eq!(
            ConversionPlan::new(5_644_800, 176_400)
                .unwrap()
                .halfband_taps
                .len(),
            2
        );
        assert_eq!(
            ConversionPlan::new(11_289_600, 176_400)
                .unwrap()
                .halfband_taps
                .len(),
            3
        );
        assert_eq!(
            ConversionPlan::new(22_579_200, 176_400)
                .unwrap()
                .halfband_taps
                .len(),
            4
        );
        assert_eq!(
            ConversionPlan::new(2_822_400, 352_800)
                .unwrap()
                .halfband_taps
                .len(),
            0
        );
        assert!(ConversionPlan::new(2_822_400, 192_000).is_err());
        assert!(ConversionPlan::new(3_072_000, 176_400).is_err());
        assert!(ConversionPlan::new(2_822_400, 0).is_err());
    }

    #[test]
    fn a_constant_bit_pattern_converts_to_its_average() {
        // 0xFF is +1 for every sample; the converter must settle at unity gain (DC gain of the chain is 1).
        let plan = ConversionPlan::new(2_822_400, 176_400).unwrap();
        let mut converter = DsdPcmConverter::new(plan, 1, 0.0);
        let mut output = vec![Vec::new()];
        converter.process(&[vec![0xFF; 4096]], &mut output);
        let tail = &output[0][output[0].len() - 8..];
        for sample in tail {
            assert!((sample - 1.0).abs() < 1e-4, "{sample}");
        }
        // The idle pattern averages to zero.
        converter.reset();
        converter.process(&[vec![DSD_IDLE_BYTE; 4096]], &mut output);
        for sample in &output[0][output[0].len() - 8..] {
            assert!(sample.abs() < 1e-4, "{sample}");
        }
    }

    #[test]
    fn halfband_stage_rejects_the_alias_of_a_tone_near_its_nyquist() {
        // Input rate 352.8 kHz; a tone at 171.8 kHz would fold to 5 kHz after ÷2 if the filter were weak.
        let taps = Arc::new(halfband_taps(halfband_tap_count(352_800.0)));
        let mut stage = HalfbandStage::new(taps);
        let rate = 352_800.0_f64;
        let input: Vec<f32> = (0..200_000)
            .map(|index| (0.5 * (2.0 * PI * 171_800.0 * index as f64 / rate).sin()) as f32)
            .collect();
        let mut output = Vec::new();
        for chunk in input.chunks(4097) {
            stage.process(chunk, &mut output);
        }
        let settled = &output[2000..];
        let (alias, _) = fit_sine(settled, 2000, 176_400, 5_000.0);
        assert!(db(alias / 0.5) < -100.0, "alias at {} dBc", db(alias / 0.5));
        // And a passband tone passes at unity gain.
        let mut stage = HalfbandStage::new(Arc::new(halfband_taps(halfband_tap_count(352_800.0))));
        let input: Vec<f32> = (0..200_000)
            .map(|index| (0.5 * (2.0 * PI * 20_000.0 * index as f64 / rate).sin()) as f32)
            .collect();
        let mut output = Vec::new();
        stage.process(&input, &mut output);
        let (amplitude, _) = fit_sine(&output[2000..], 2000, 176_400, 20_000.0);
        assert!(
            db(amplitude / 0.5).abs() < 0.001,
            "{} dB",
            db(amplitude / 0.5)
        );
    }

    #[test]
    fn halfband_output_does_not_depend_on_how_the_input_is_chunked() {
        let taps = Arc::new(halfband_taps(halfband_tap_count(352_800.0)));
        let input: Vec<f32> = (0..9_999)
            .map(|index| ((index * 7919) % 1000) as f32 / 1000.0 - 0.5)
            .collect();
        let mut whole = Vec::new();
        HalfbandStage::new(Arc::clone(&taps)).process(&input, &mut whole);
        for chunk_size in [1, 2, 3, 100, 4096] {
            let mut stage = HalfbandStage::new(Arc::clone(&taps));
            let mut pieces = Vec::new();
            for chunk in input.chunks(chunk_size) {
                stage.process(chunk, &mut pieces);
            }
            assert_eq!(pieces, whole, "chunk size {chunk_size}");
        }
    }

    #[test]
    fn tones_across_the_audio_band_keep_their_level_for_every_rate() {
        for (dsd_rate, seconds) in [
            (2_822_400_u32, 0.2_f64),
            (5_644_800, 0.1),
            (11_289_600, 0.1),
        ] {
            let plan = ConversionPlan::new(dsd_rate, 176_400).unwrap();
            for frequency in [100.0, 1_000.0, 10_000.0, 20_000.0] {
                let samples = (f64::from(dsd_rate) * seconds) as usize;
                let samples = samples / plan.ratio * plan.ratio;
                let bytes = modulate(samples, tone(dsd_rate, frequency, 0.25));
                let frames = convert_all(&plan, &bytes, samples, 4096);
                let start = frames.len() / 10;
                let window = &frames[start..frames.len() - start];
                let (amplitude, _) = fit_sine(window, start, 176_400, frequency);
                let error = db(amplitude / 0.25);
                assert!(
                    error.abs() < 0.02,
                    "DSD {dsd_rate} {frequency} Hz: {error} dB"
                );
            }
        }
    }

    #[test]
    fn ultrasonic_noise_does_not_fold_into_the_audio_band() {
        let plan = ConversionPlan::new(2_822_400, 176_400).unwrap();
        let samples = 2_822_400 * 3 / 10 / 16 * 16;
        // R/8 − 5 kHz folds to 5 kHz in a ÷8 stage without proper filtering.
        let ultrasonic = 352_800.0 - 5_000.0;
        let bytes = modulate(samples, tone(2_822_400, ultrasonic, 0.25));
        let frames = convert_all(&plan, &bytes, samples, 4096);
        let start = frames.len() / 10;
        let window = &frames[start..frames.len() - start];
        let (alias, _) = fit_sine(window, start, 176_400, 5_000.0);
        assert!(
            db(alias / 0.25) < -95.0,
            "alias at {} dBc",
            db(alias / 0.25)
        );
    }

    #[test]
    fn output_is_aligned_with_the_input_within_a_frame() {
        for dsd_rate in [2_822_400_u32, 5_644_800, 11_289_600] {
            let plan = ConversionPlan::new(dsd_rate, 176_400).unwrap();
            let frequency = 1_000.0;
            let samples = dsd_rate as usize / 5 / plan.ratio * plan.ratio;
            let bytes = modulate(samples, tone(dsd_rate, frequency, 0.25));
            let frames = convert_all(&plan, &bytes, samples, 4096);
            let start = frames.len() / 10;
            let window = &frames[start..frames.len() - start];
            let (_, delay_frames) = fit_sine(window, start, 176_400, frequency);
            // Design target: within half a frame; the modulator itself lags its input by one DSD sample.
            assert!(
                delay_frames.abs() < 0.6,
                "DSD {dsd_rate}: delay {delay_frames} frames"
            );
        }
    }

    /// Prints measured conversion quality (run with `-- --ignored --nocapture`).
    #[test]
    #[ignore = "prints measurements"]
    fn report_conversion_quality() {
        for dsd_rate in [2_822_400_u32, 5_644_800, 11_289_600, 22_579_200] {
            let plan = ConversionPlan::new(dsd_rate, 176_400).unwrap();
            let samples = (dsd_rate as usize / 5) / plan.ratio * plan.ratio;
            println!(
                "DSD {dsd_rate}: ratio {}, halfband taps {:?}, delay {:.1} DSD samples = {:.2} frames, discard {}, settle {} bytes",
                plan.ratio,
                plan.halfband_taps,
                plan.delay_dsd_samples(),
                plan.delay_dsd_samples() / plan.ratio as f64,
                plan.discard_frames(),
                plan.settle_bytes()
            );
            for frequency in [100.0, 1_000.0, 10_000.0, 20_000.0] {
                let bytes = modulate(samples, tone(dsd_rate, frequency, 0.25));
                let frames = convert_all(&plan, &bytes, samples, 4096);
                let start = frames.len() / 10;
                let (amplitude, delay) = fit_sine(
                    &frames[start..frames.len() - start],
                    start,
                    176_400,
                    frequency,
                );
                println!(
                    "  {frequency:>7} Hz: level {:+.4} dB, delay {delay:+.3} frames",
                    db(amplitude / 0.25)
                );
            }
            let ultrasonic = f64::from(dsd_rate) / 8.0 - 5_000.0;
            let bytes = modulate(samples, tone(dsd_rate, ultrasonic, 0.25));
            let frames = convert_all(&plan, &bytes, samples, 4096);
            let start = frames.len() / 10;
            let (alias, _) = fit_sine(
                &frames[start..frames.len() - start],
                start,
                176_400,
                5_000.0,
            );
            println!(
                "  alias of {ultrasonic} Hz at 5 kHz: {:.1} dBc",
                db(alias / 0.25)
            );
        }
    }

    #[test]
    fn chunking_the_input_does_not_change_the_output() {
        let plan = ConversionPlan::new(2_822_400, 176_400).unwrap();
        let samples = 2_822_400 / 20 / 16 * 16;
        let bytes = modulate(samples, tone(2_822_400, 3_000.0, 0.3));
        let reference = convert_all(&plan, &bytes, samples, 4096);
        for chunk in [1, 7, 333, 1000] {
            assert_eq!(
                convert_all(&plan, &bytes, samples, chunk),
                reference,
                "chunk {chunk}"
            );
        }
    }

    #[test]
    fn restarting_after_preroll_matches_continuous_conversion() {
        let plan = ConversionPlan::new(2_822_400, 176_400).unwrap();
        let samples = 2_822_400 / 5 / 16 * 16;
        let bytes = modulate(samples, tone(2_822_400, 1_234.0, 0.3));
        let continuous = convert_all(&plan, &bytes, samples, 4096);
        // Restart in the middle at a pair-aligned byte, preroll, then compare.
        let bytes_per_frame = plan.bytes_per_frame();
        let target_frame = 20_000;
        let raw_target = target_frame + plan.discard_frames();
        let preroll = (plan.settle_bytes() * 2).max(2048);
        let start = ((raw_target * bytes_per_frame).saturating_sub(preroll) / bytes_per_frame)
            * bytes_per_frame;
        let mut converter = DsdPcmConverter::new(plan.clone(), 1, 0.0);
        let mut output = vec![Vec::new()];
        let mut raw_index = start / bytes_per_frame;
        let mut compared = 0;
        let mut position = start;
        while compared < 2_000 {
            let end = (position + 1024).min(bytes.len());
            converter.process(&[bytes[position..end].to_vec()], &mut output);
            position = end;
            for &sample in &output[0] {
                if raw_index >= raw_target {
                    let frame = raw_index - plan.discard_frames();
                    assert!(
                        (sample - continuous[frame]).abs() < 2e-6,
                        "frame {frame}: {sample} vs {}",
                        continuous[frame]
                    );
                    compared += 1;
                }
                raw_index += 1;
            }
        }
    }
}
