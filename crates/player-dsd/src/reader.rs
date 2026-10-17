//! The seekable DSD→PCM reader (ADR 0016): pulls DSD from a [`DsdStream`], converts it block by block and produces
//! interleaved `f32` frames at a fixed PCM rate, delay-compensated and trimmed to the exact frame count. The Windows
//! playback source (rodio) and the Android data source both sit on top of it.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::convert::{ConversionPlan, DsdPcmConverter};
use crate::stream::{DsdError, DsdInfo, DsdStream, DSD_IDLE_BYTE};

/// Time constant of the gain smoothing: when the gain setting changes while a track plays, the gain eases to the new
/// value instead of jumping.
const GAIN_SMOOTHING_SECONDS: f64 = 0.3;
/// Bytes per channel read from the stream per refill (one DSF block).
const READ_BYTES: usize = 4096;
/// Idle-pattern bytes per channel fed after the end of the stream to flush the filters; far more than the filters'
/// delay, so the cap only protects against a logic error.
const MAX_FLUSH_BYTES: usize = 64 * READ_BYTES;
/// Minimum bytes replayed before a seek target so every filter has forgotten its (unknown) state.
const MIN_PREROLL_BYTES: usize = 2048;
/// Most channels the converter plays in this version.
pub const MAX_CHANNELS: u16 = 2;

pub struct DsdPcmReader<S: DsdStream> {
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
    /// Absolute index of the next frame to be produced.
    frames_emitted: u64,
    input_done: bool,
    flushed_bytes: usize,
    in_buf: Vec<Vec<u8>>,
    chan_out: Vec<Vec<f32>>,
    /// Target gain in dB (`f32` bits), shared so it can change while playing.
    gain_target: Arc<AtomicU32>,
    /// Linear gain applied now; eases toward the target.
    gain_now: f32,
    gain_smoothing: f32,
}

impl<S: DsdStream> DsdPcmReader<S> {
    /// A reader whose gain follows `gain_db_target` (dB as `f32` bits). It starts at the target's current value.
    pub fn new(stream: S, out_rate: u32, gain_db_target: Arc<AtomicU32>) -> Result<Self, DsdError> {
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
        let converter = DsdPcmConverter::new(plan, channels, 0.0);
        let gain_now = db_to_linear(f32::from_bits(gain_db_target.load(Ordering::Relaxed)));
        let gain_smoothing =
            (1.0 - (-1.0 / (GAIN_SMOOTHING_SECONDS * f64::from(out_rate))).exp()) as f32;
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
            gain_target: gain_db_target,
            gain_now,
            gain_smoothing,
        })
    }

    pub fn dsd_info(&self) -> DsdInfo {
        self.info
    }

    pub fn out_rate(&self) -> u32 {
        self.out_rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Frames in the whole track at the output rate.
    pub fn total_frames(&self) -> u64 {
        self.total_frames
    }

    /// Index of the next frame `read_frames` produces.
    pub fn position_frames(&self) -> u64 {
        self.frames_emitted
    }

    /// Convert until at least one frame is ready (or the track ends) and append the interleaved frames to `out`.
    /// Returns the number of frames appended; 0 is the end of the track.
    pub fn read_frames(&mut self, out: &mut Vec<f32>) -> Result<usize, DsdError> {
        let mut appended = 0_u64;
        while appended == 0 && self.frames_emitted < self.total_frames {
            let mut count = 0;
            if !self.input_done {
                count = self.stream.read(READ_BYTES, &mut self.in_buf)?;
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
            let target = db_to_linear(f32::from_bits(self.gain_target.load(Ordering::Relaxed)));
            for frame in skip..skip + take {
                // One-pole smoothing toward the target gain (a no-op once they match).
                if self.gain_now != target {
                    self.gain_now += (target - self.gain_now) * self.gain_smoothing;
                    if (target - self.gain_now).abs() < 1e-6 {
                        self.gain_now = target;
                    }
                }
                for channel in &self.chan_out {
                    out.push(channel[frame as usize] * self.gain_now);
                }
            }
            self.frames_emitted += take;
            appended += take;
        }
        Ok(appended as usize)
    }

    /// Reposition so the next frame produced is `frame` (clamped to the end).
    pub fn seek_to_frame(&mut self, frame: u64) -> Result<(), DsdError> {
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
        Ok(())
    }
}

fn db_to_linear(db: f32) -> f32 {
    10_f32.powf(db / 20.0)
}
