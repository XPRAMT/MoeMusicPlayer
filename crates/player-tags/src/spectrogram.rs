//! STFT spectrogram rendering (ADR 0013). Only `columns` evenly spaced windows are
//! analysed, so cost is dominated by decoding rather than by track length.

use std::collections::VecDeque;
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use image::{ImageBuffer, Rgb};
use realfft::RealFftPlanner;
use serde::{Deserialize, Serialize};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum WindowFunction {
    #[default]
    Hann,
    Hamming,
    Blackman,
    BlackmanHarris,
    Rectangular,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Colormap {
    #[default]
    Spek,
    /// Black, deep blue, violet, crimson, orange, yellow, white: the palette the "Spek" choice used to be.
    Fire,
    Magma,
    Inferno,
    Viridis,
    Grayscale,
}

/// Which audio to analyse: a mono mix, one channel (0 = left, 1 = right…), or every
/// channel stacked top-to-bottom.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", tag = "mode", content = "index")]
pub enum ChannelSelect {
    #[default]
    Mix,
    Channel(u8),
    Separate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SpectrogramOptions {
    pub columns: u32,
    pub height: u32,
    pub fft_size: usize,
    pub window: WindowFunction,
    pub colormap: Colormap,
    pub channel: ChannelSelect,
    pub db_floor: f32,
    pub db_ceiling: f32,
    pub log_frequency: bool,
    /// Upper bound of the frequency axis; `None` = Nyquist.
    pub max_frequency_hz: Option<u32>,
    /// Lower bound used by the log axis (ignored for linear).
    pub min_log_frequency_hz: u32,
}

impl Default for SpectrogramOptions {
    fn default() -> Self {
        Self {
            columns: 1600,
            height: 512,
            fft_size: 2048,
            window: WindowFunction::Hann,
            colormap: Colormap::Spek,
            channel: ChannelSelect::Mix,
            db_floor: -120.0,
            db_ceiling: 0.0,
            log_frequency: false,
            max_frequency_hz: None,
            min_log_frequency_hz: 20,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Spectrogram {
    pub png: Vec<u8>,
    /// Row-major 0–255 intensity per pixel (same size as the PNG); dB = floor + v/255·range.
    pub levels: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u32,
    /// Display label per decoded channel (L, R, C, LFE, SL, SR…; `Ch n` when unknown).
    pub channel_labels: Vec<String>,
    pub duration_ms: u64,
    pub columns: u32,
    pub height: u32,
    /// Number of stacked panels (channels when `Separate`, else 1).
    pub panels: u32,
    /// Frequency shown at the top edge.
    pub max_frequency_hz: u32,
    /// Estimated frequency above which energy stays very low (lossy cutoff hint).
    pub cutoff_hz: Option<u32>,
}

type Stops = &'static [(f32, [f32; 3])];

/// The "Spek" palette, the spectrum colours of the Spek analyser (which describes them as a modified version of Dan Bruton's
/// wavelength-to-colour algorithm): from the floor violet, blue, cyan, green, yellow, orange, red, with the quietest tenth
/// fading to black. Written here as colour stops (positions are fractions of the ramp, which spans the lower 66.25 % of
/// the scale) plus that fade, not copied from Spek's code (Spek is GPL; this project cannot use GPL code).
const SPEK_RAMP_END: f32 = 0.6625;
/// Where the fade to black ends on the 0 to 1 scale: the quietest tenth of the ramp.
const SPEK_FADE_END: f32 = 0.1 / SPEK_RAMP_END;
const SPEK: Stops = &[
    (0.0, [170.0, 0.0, 255.0]),
    (0.15 / SPEK_RAMP_END, [0.0, 0.0, 255.0]),
    (0.275 / SPEK_RAMP_END, [0.0, 255.0, 255.0]),
    (0.325 / SPEK_RAMP_END, [0.0, 255.0, 0.0]),
    (0.5 / SPEK_RAMP_END, [255.0, 255.0, 0.0]),
    (1.0, [255.0, 0.0, 0.0]),
];

/// The colour for a level (0 = floor, 1 = ceiling).
fn spek_color(level: f32) -> [u8; 3] {
    let level = level.clamp(0.0, 1.0);
    let colour = interpolate(SPEK, level);
    let fade = if level < SPEK_FADE_END {
        level / SPEK_FADE_END
    } else {
        1.0
    };
    colour.map(|channel| (channel * fade).round().clamp(0.0, 255.0) as u8)
}

/// Piecewise-linear colour between stops (positions ascending from 0 to 1), channels 0 to 255.
fn interpolate(stops: Stops, t: f32) -> [f32; 3] {
    let pair = stops
        .windows(2)
        .find(|pair| t <= pair[1].0)
        .unwrap_or(&stops[stops.len() - 2..]);
    let ((a, ca), (b, cb)) = (pair[0], pair[1]);
    let k = ((t - a) / (b - a)).clamp(0.0, 1.0);
    std::array::from_fn(|c| ca[c] + (cb[c] - ca[c]) * k)
}
const FIRE: Stops = &[
    (0.0, [0.0, 0.0, 0.0]),
    (0.15, [0.0, 0.0, 120.0]),
    (0.3, [100.0, 0.0, 160.0]),
    (0.45, [200.0, 0.0, 100.0]),
    (0.6, [250.0, 60.0, 0.0]),
    (0.8, [255.0, 200.0, 0.0]),
    (1.0, [255.0, 255.0, 255.0]),
];
const MAGMA: Stops = &[
    (0.0, [0.0, 0.0, 4.0]),
    (0.2, [59.0, 15.0, 112.0]),
    (0.4, [140.0, 41.0, 129.0]),
    (0.6, [222.0, 73.0, 104.0]),
    (0.8, [254.0, 159.0, 109.0]),
    (1.0, [252.0, 253.0, 191.0]),
];
const INFERNO: Stops = &[
    (0.0, [0.0, 0.0, 4.0]),
    (0.2, [66.0, 10.0, 104.0]),
    (0.4, [147.0, 38.0, 103.0]),
    (0.6, [221.0, 81.0, 58.0]),
    (0.8, [252.0, 165.0, 10.0]),
    (1.0, [252.0, 255.0, 164.0]),
];
const VIRIDIS: Stops = &[
    (0.0, [68.0, 1.0, 84.0]),
    (0.25, [59.0, 82.0, 139.0]),
    (0.5, [33.0, 145.0, 140.0]),
    (0.75, [94.0, 201.0, 98.0]),
    (1.0, [253.0, 231.0, 37.0]),
];
const GRAY: Stops = &[(0.0, [0.0, 0.0, 0.0]), (1.0, [255.0, 255.0, 255.0])];

fn stops(map: Colormap) -> Stops {
    match map {
        // Spek is computed (see `spek_color`); the other maps interpolate between stops.
        Colormap::Spek => GRAY,
        Colormap::Fire => FIRE,
        Colormap::Magma => MAGMA,
        Colormap::Inferno => INFERNO,
        Colormap::Viridis => VIRIDIS,
        Colormap::Grayscale => GRAY,
    }
}

/// Labels for the decoded channel layout, in interleaved order.
pub fn channel_labels(
    channels: Option<symphonia::core::audio::Channels>,
    count: usize,
) -> Vec<String> {
    use symphonia::core::audio::Channels as C;
    let known: Vec<String> = channels
        .map(|layout| {
            layout
                .iter()
                .map(|channel| {
                    let label = match channel {
                        c if c == C::FRONT_LEFT => "L",
                        c if c == C::FRONT_RIGHT => "R",
                        c if c == C::FRONT_CENTRE => "C",
                        c if c == C::LFE1 => "LFE",
                        c if c == C::LFE2 => "LFE2",
                        c if c == C::REAR_LEFT => "RL",
                        c if c == C::REAR_RIGHT => "RR",
                        c if c == C::REAR_CENTRE => "RC",
                        c if c == C::FRONT_LEFT_CENTRE => "FLC",
                        c if c == C::FRONT_RIGHT_CENTRE => "FRC",
                        c if c == C::SIDE_LEFT => "SL",
                        c if c == C::SIDE_RIGHT => "SR",
                        c if c == C::TOP_CENTRE => "TC",
                        c if c == C::TOP_FRONT_LEFT => "TFL",
                        c if c == C::TOP_FRONT_CENTRE => "TFC",
                        c if c == C::TOP_FRONT_RIGHT => "TFR",
                        c if c == C::TOP_REAR_LEFT => "TRL",
                        c if c == C::TOP_REAR_CENTRE => "TRC",
                        c if c == C::TOP_REAR_RIGHT => "TRR",
                        _ => "",
                    };
                    label.to_owned()
                })
                .collect()
        })
        .unwrap_or_default();
    (0..count)
        .map(|i| match known.get(i) {
            Some(label) if !label.is_empty() => label.clone(),
            _ if count == 1 => "Mono".to_owned(),
            _ => format!("Ch {}", i + 1),
        })
        .collect()
}

/// 256-entry lookup table for a colormap.
pub fn colormap_lut(map: Colormap) -> Vec<[u8; 3]> {
    if map == Colormap::Spek {
        return (0..256).map(|i| spek_color(i as f32 / 255.0)).collect();
    }
    let stops = stops(map);
    (0..256)
        .map(|i| {
            let t = i as f32 / 255.0;
            interpolate(stops, t).map(|channel| channel.round() as u8)
        })
        .collect()
}

fn window_coefficients(kind: WindowFunction, n: usize) -> Vec<f32> {
    use std::f32::consts::PI;
    let m = n as f32;
    (0..n)
        .map(|i| {
            let x = 2.0 * PI * i as f32 / m;
            match kind {
                WindowFunction::Hann => 0.5 - 0.5 * x.cos(),
                WindowFunction::Hamming => 0.54 - 0.46 * x.cos(),
                WindowFunction::Blackman => 0.42 - 0.5 * x.cos() + 0.08 * (2.0 * x).cos(),
                WindowFunction::BlackmanHarris => {
                    0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos()
                        - 0.01168 * (3.0 * x).cos()
                }
                WindowFunction::Rectangular => 1.0,
            }
        })
        .collect()
}

struct Analyzer {
    fft: std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    output: Vec<realfft::num_complex::Complex<f32>>,
    norm: f32,
}

impl Analyzer {
    fn new(n: usize, kind: WindowFunction) -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(n);
        let window = window_coefficients(kind, n);
        // The level is 20 log10(|X| / N), as Spek reads it: only the window is applied, with no compensation for its gain, so
        // a full-scale sine reads -12 dB with the Hann window (|X| = N / 4) and the same noise reads as it does in Spek.
        let norm = n as f32;
        Self {
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            fft,
            window,
            norm,
        }
    }

    fn analyse(&mut self, samples: impl Iterator<Item = f32>) -> Vec<f32> {
        for ((slot, sample), w) in self.input.iter_mut().zip(samples).zip(&self.window) {
            *slot = sample * w;
        }
        let _ = self.fft.process(&mut self.input, &mut self.output);
        self.output
            .iter()
            .map(|c| 20.0 * (c.norm() / self.norm).max(1e-12).log10())
            .collect()
    }
}

/// Opus input (ADR 0017). libopus is built with cmake, which the Android build does not have, and Android plays Opus
/// with Media3, so there it has no Opus spectrogram.
#[cfg(not(target_os = "android"))]
mod opus_input {
    use std::fs::File;
    use std::io::BufReader;
    use std::path::Path;

    pub type Reader = player_opus::OpusReader<BufReader<File>>;

    pub fn is_opus(path: &Path) -> bool {
        player_formats::decode_engine(path) == player_formats::DecodeEngine::Opus
            || player_opus::sniff_opus_file(path)
    }

    /// The reader with its sample rate, channel count and exact length in frames.
    pub fn open(file: File) -> Result<(Reader, u32, usize, Option<u64>), String> {
        let reader = player_opus::OpusReader::open(BufReader::new(file))
            .map_err(|e| format!("無法開啟 Opus：{e}"))?;
        let info = reader.info();
        let channels = usize::from(info.channels).max(1);
        Ok((reader, player_opus::OPUS_RATE, channels, info.total_frames))
    }

    pub fn read(reader: &mut Reader, out: &mut Vec<f32>) -> Result<usize, String> {
        reader
            .read_frames(out)
            .map_err(|e| format!("解碼失敗：{e}"))
    }
}

#[cfg(target_os = "android")]
mod opus_input {
    use std::fs::File;
    use std::path::Path;

    pub struct Reader;

    pub fn is_opus(path: &Path) -> bool {
        player_formats::decode_engine(path) == player_formats::DecodeEngine::Opus
    }

    pub fn open(_file: File) -> Result<(Reader, u32, usize, Option<u64>), String> {
        Err("Android 不支援 Opus 頻譜圖。".to_owned())
    }

    pub fn read(_reader: &mut Reader, _out: &mut Vec<f32>) -> Result<usize, String> {
        Err("Android 不支援 Opus 頻譜圖。".to_owned())
    }
}

/// WavPack input (ADR 0025): libwavpack decodes it, Symphonia has no WavPack codec.
mod wavpack_input {
    use std::fs::File;
    use std::io::BufReader;
    use std::path::{Path, PathBuf};

    pub type Reader = player_wavpack::WavPackReader;

    pub fn is_wavpack(path: &Path) -> bool {
        player_formats::decode_engine(path) == player_formats::DecodeEngine::WavPack
    }

    /// The reader with its sample rate, channel count and exact length in frames.
    pub fn open(path: &Path, file: File) -> Result<(Reader, u32, usize, Option<u64>), String> {
        let mut correction_name = path.as_os_str().to_owned();
        correction_name.push("c");
        let correction = File::open(PathBuf::from(correction_name))
            .ok()
            .map(|file| Box::new(BufReader::new(file)) as Box<dyn player_wavpack::ReadSeek>);
        let reader = player_wavpack::WavPackReader::open(BufReader::new(file), correction)
            .map_err(|e| e.to_string())?;
        let info = reader.info();
        Ok((
            reader,
            info.sample_rate,
            usize::from(info.channels).max(1),
            info.total_frames,
        ))
    }

    pub fn read(reader: &mut Reader, out: &mut Vec<f32>) -> Result<usize, String> {
        reader.read_frames(out).map_err(|e| e.to_string())
    }
}

/// Render a spectrogram PNG for `path`.
pub fn render_spectrogram(
    path: &Path,
    options: SpectrogramOptions,
    cancel: &AtomicBool,
) -> Result<Spectrogram, String> {
    if !player_formats::spectrogram_supported(path) {
        return Err("此格式不支援頻譜圖。".to_owned());
    }
    let n = options.fft_size.clamp(256, 32768).next_power_of_two();
    let cols = options.columns.clamp(16, 8000) as usize;
    let height = options.height.clamp(64, 4096);

    let file = File::open(path).map_err(|e| format!("無法開啟檔案：{e}"))?;
    // Opus has its own decoder (ADR 0017); everything else goes through Symphonia.
    let use_opus = opus_input::is_opus(path);
    let use_wavpack = !use_opus && wavpack_input::is_wavpack(path);
    let mut opus_reader = None;
    let mut wavpack_reader = None;
    let mut symphonia_input = None;
    let (sample_rate, channel_count, known_frames, channel_layout) = if use_opus {
        let (reader, rate, channels, frames) = opus_input::open(file)?;
        opus_reader = Some(reader);
        (rate, channels, frames, None)
    } else if use_wavpack {
        let (reader, rate, channels, frames) = wavpack_input::open(path, file)?;
        wavpack_reader = Some(reader);
        (rate, channels, frames, None)
    } else {
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|e| format!("無法辨識音訊格式：{e}"))?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or("找不到音訊軌")?
            .clone();
        let sample_rate = track.codec_params.sample_rate.ok_or("未知取樣率")?;
        let channel_count = track
            .codec_params
            .channels
            .map(|c| c.count())
            .unwrap_or(2)
            .max(1);
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| format!("不支援的編碼：{e}"))?;
        let known_frames = track.codec_params.n_frames;
        let layout = track.codec_params.channels;
        symphonia_input = Some((format, decoder, track.id));
        (sample_rate, channel_count, known_frames, layout)
    };

    let streams = match options.channel {
        ChannelSelect::Separate => channel_count,
        _ => 1,
    };
    let select = |frame: &[f32], stream: usize| -> f32 {
        match options.channel {
            ChannelSelect::Mix => frame.iter().sum::<f32>() / frame.len() as f32,
            ChannelSelect::Channel(c) => frame
                .get(c as usize)
                .or(frame.first())
                .copied()
                .unwrap_or(0.0),
            ChannelSelect::Separate => frame.get(stream).copied().unwrap_or(0.0),
        }
    };

    let mut analyzer = Analyzer::new(n, options.window);
    let bins = n / 2 + 1;
    let mut rings: Vec<VecDeque<f32>> = (0..streams).map(|_| VecDeque::with_capacity(n)).collect();
    let mut collected: Vec<Vec<f32>> = vec![Vec::new(); streams];
    let mut spectra: Vec<Vec<Vec<f32>>> = vec![Vec::with_capacity(cols); streams];
    // Like Spek, a column is the average (of the dB values) of the FFTs that tile its time span, one after the other
    // without overlap; a span shorter than one FFT uses the last FFT-length of audio before its end.
    let interval_end = |c: usize, total: u64| -> u64 { (c as u64 + 1) * total / cols as u64 };
    let mut sums: Vec<Vec<f32>> = vec![vec![0.0; bins]; streams];
    let mut counted = 0usize;
    let mut interval_frames = 0u64;
    let mut next_col = 0usize;
    let mut index: u64 = 0;
    // Analyse the audio each stream last saw (zero-padded before the start) into the running sums.
    let accumulate = |analyzer: &mut Analyzer,
                      rings: &[VecDeque<f32>],
                      sums: &mut [Vec<f32>],
                      counted: &mut usize| {
        for (sum, ring) in sums.iter_mut().zip(rings) {
            let pad = n - ring.len();
            let column =
                analyzer.analyse(std::iter::repeat_n(0.0, pad).chain(ring.iter().copied()));
            for (total, level) in sum.iter_mut().zip(&column) {
                *total += level;
            }
        }
        *counted += 1;
    };
    // One decoded frame (all channels) from either decoder.
    let mut feed = |frame: &[f32]| {
        match known_frames {
            Some(total) => {
                for (s, ring) in rings.iter_mut().enumerate() {
                    if ring.len() == n {
                        ring.pop_front();
                    }
                    ring.push_back(select(frame, s));
                }
                interval_frames += 1;
                if interval_frames.is_multiple_of(n as u64) {
                    accumulate(&mut analyzer, &rings, &mut sums, &mut counted);
                }
                while next_col < cols && index + 1 >= interval_end(next_col, total) {
                    if counted == 0 {
                        accumulate(&mut analyzer, &rings, &mut sums, &mut counted);
                    }
                    for (s, sum) in sums.iter_mut().enumerate() {
                        spectra[s].push(sum.iter().map(|total| total / counted as f32).collect());
                        sum.fill(0.0);
                    }
                    counted = 0;
                    interval_frames = 0;
                    next_col += 1;
                }
            }
            None => {
                for (s, store) in collected.iter_mut().enumerate() {
                    store.push(select(frame, s));
                }
            }
        }
        index += 1;
    };

    if let Some(mut reader) = opus_reader {
        let mut buffer = Vec::new();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("已取消".into());
            }
            buffer.clear();
            let frames = opus_input::read(&mut reader, &mut buffer)?;
            if frames == 0 {
                break;
            }
            for frame in buffer.chunks(channel_count) {
                feed(frame);
            }
        }
    } else if let Some(mut reader) = wavpack_reader {
        let mut buffer = Vec::new();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("已取消".into());
            }
            buffer.clear();
            let frames = wavpack_input::read(&mut reader, &mut buffer)?;
            if frames == 0 {
                break;
            }
            for frame in buffer.chunks(channel_count) {
                feed(frame);
            }
        }
    } else if let Some((mut format, mut decoder, track_id)) = symphonia_input {
        let mut sample_buf: Option<SampleBuffer<f32>> = None;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("已取消".into());
            }
            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(SymError::ResetRequired) => break,
                Err(e) => return Err(format!("讀取封包失敗：{e}")),
            };
            if packet.track_id() != track_id {
                continue;
            }
            let decoded = match decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymError::DecodeError(_)) => continue,
                Err(e) => return Err(format!("解碼失敗：{e}")),
            };
            let spec = *decoded.spec();
            let channels = spec.channels.count().max(1);
            let needed = decoded.capacity() * channels;
            if sample_buf.as_ref().is_none_or(|b| b.capacity() < needed) {
                sample_buf = Some(SampleBuffer::new(decoded.capacity() as u64, spec));
            }
            let buf = sample_buf.as_mut().expect("buffer");
            buf.copy_interleaved_ref(decoded);
            for frame in buf.samples().chunks(channels) {
                feed(frame);
            }
        }
    }
    let total = known_frames.map(|t| t.min(index)).unwrap_or(index);
    if known_frames.is_none() {
        for (s, store) in collected.iter().enumerate() {
            let length = store.len() as u64;
            for c in 0..cols {
                let start = c as u64 * length / cols as u64;
                let end = interval_end(c, length).max(start);
                let mut ends: Vec<u64> = (1..)
                    .map(|k| start + k * n as u64)
                    .take_while(|block_end| *block_end <= end)
                    .collect();
                if ends.is_empty() {
                    ends.push(end);
                }
                let mut sum = vec![0.0_f32; bins];
                for block_end in &ends {
                    let block_end = *block_end as usize;
                    let first = block_end as i64 - n as i64;
                    let level = analyzer.analyse((first..block_end as i64).map(|i| {
                        usize::try_from(i)
                            .ok()
                            .and_then(|i| store.get(i))
                            .copied()
                            .unwrap_or(0.0)
                    }));
                    for (total, value) in sum.iter_mut().zip(&level) {
                        *total += value;
                    }
                }
                spectra[s].push(sum.iter().map(|total| total / ends.len() as f32).collect());
            }
        }
    }
    if spectra[0].is_empty() {
        return Err("沒有可分析的音訊".into());
    }

    let nyquist = sample_rate / 2;
    let max_hz = options
        .max_frequency_hz
        .map(|f| f.clamp(100, nyquist))
        .unwrap_or(nyquist);
    let min_log = (options.min_log_frequency_hz.max(1) as f32).min(max_hz as f32 / 2.0);
    let bins = n / 2 + 1;
    let hz_per_bin = sample_rate as f32 / n as f32;
    let panel_height = (height / streams as u32).max(16);
    // Map each pixel row (0 = top) to a [lo, hi) bin range.
    let row_bins: Vec<(usize, usize)> = (0..panel_height)
        .map(|y| {
            let frac_top = 1.0 - y as f32 / panel_height as f32;
            let frac_bottom = 1.0 - (y + 1) as f32 / panel_height as f32;
            let hz = |f: f32| {
                if options.log_frequency {
                    min_log * (max_hz as f32 / min_log).powf(f)
                } else {
                    f * max_hz as f32
                }
            };
            let lo = ((hz(frac_bottom) / hz_per_bin).floor() as usize).min(bins - 1);
            let hi = ((hz(frac_top) / hz_per_bin).ceil() as usize).clamp(lo + 1, bins);
            (lo, hi)
        })
        .collect();

    let lut = colormap_lut(options.colormap);
    let floor = options.db_floor.min(options.db_ceiling - 1.0);
    let range = options.db_ceiling - floor;
    let out_cols = spectra[0].len() as u32;
    let mut img = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(out_cols, panel_height * streams as u32);
    let mut levels = vec![0u8; (out_cols * panel_height * streams as u32) as usize];
    for (s, panel) in spectra.iter().enumerate() {
        for (x, column) in panel.iter().enumerate() {
            for (y, &(lo, hi)) in row_bins.iter().enumerate() {
                // A row that covers several bins shows their average (Spek draws one row per bin and scales the image).
                let db = column[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
                let t = ((db - floor) / range).clamp(0.0, 1.0);
                let level = (t * 255.0).round() as u8;
                let row = s as u32 * panel_height + y as u32;
                levels[(row * out_cols + x as u32) as usize] = level;
                img.put_pixel(x as u32, row, Rgb(lut[level as usize]));
            }
        }
    }
    let cutoff_hz = estimate_cutoff(&spectra[0], sample_rate, n);
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("PNG 編碼失敗：{e}"))?;
    Ok(Spectrogram {
        png,
        levels,
        sample_rate,
        channels: channel_count as u32,
        channel_labels: channel_labels(channel_layout, channel_count),
        duration_ms: total * 1000 / u64::from(sample_rate),
        columns: out_cols,
        height: img.height(),
        panels: streams as u32,
        max_frequency_hz: max_hz,
        cutoff_hz,
    })
}

const CUTOFF_FLOOR_DB: f32 = -112.0;

/// Highest bin whose median level across columns is above -112 dB (the old -100 dB, a full-scale sine reading 0 dB then,
/// -12 dB now), reported only when
/// clearly below Nyquist (a hint of lossy origin, not a verdict).
fn estimate_cutoff(spectra: &[Vec<f32>], sample_rate: u32, n: usize) -> Option<u32> {
    let bins = spectra.first()?.len();
    let mut top = 0usize;
    for b in 0..bins {
        let mut levels: Vec<f32> = spectra.iter().map(|c| c[b]).collect();
        levels.sort_by(|a, b| a.total_cmp(b));
        if levels[levels.len() / 2] > CUTOFF_FLOOR_DB {
            top = b;
        }
    }
    let hz = (top as u64 * u64::from(sample_rate) / n as u64) as u32;
    (hz > 0 && hz < sample_rate / 2 * 93 / 100).then_some(hz)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colormap_luts_have_expected_endpoints() {
        let gray = colormap_lut(Colormap::Grayscale);
        assert_eq!(gray[0], [0, 0, 0]);
        assert_eq!(gray[255], [255, 255, 255]);
        assert_eq!(colormap_lut(Colormap::Magma)[0], [0, 0, 4]);
        assert_eq!(colormap_lut(Colormap::Fire)[255], [255, 255, 255]);
    }

    #[test]
    fn the_spek_palette_runs_black_violet_blue_cyan_green_yellow_orange_red() {
        let lut = colormap_lut(Colormap::Spek);
        assert_eq!(lut[0], [0, 0, 0], "the floor is black");
        assert_eq!(lut[255], [255, 0, 0], "the ceiling is red");
        // The dB scale of the Spek colour bar is -120 dB (floor) to 0 dB (ceiling): level = (dB + 120) / 120.
        let at = |db: f32| lut[(((db + 120.0) / 120.0) * 255.0).round() as usize];
        let [r, g, b] = at(-110.0);
        assert!(
            b > r && g == 0 && b > 60,
            "violet at -110 dB: {:?}",
            at(-110.0)
        );
        let [r, g, b] = at(-90.0);
        assert!(
            b == 255 && g > 0 && g < 100 && r == 0,
            "blue at -90 dB: {:?}",
            at(-90.0)
        );
        let [r, g, b] = at(-70.0);
        assert!(
            r == 0 && g > 200 && b > 200,
            "cyan at -70 dB: {:?}",
            at(-70.0)
        );
        let [r, g, b] = at(-55.0);
        assert!(
            g == 255 && r < 140 && b == 0,
            "green at -55 dB: {:?}",
            at(-55.0)
        );
        let [r, g, b] = at(-30.0);
        assert!(
            r > 230 && g == 255 && b == 0,
            "yellow at -30 dB: {:?}",
            at(-30.0)
        );
        let [r, g, b] = at(-10.0);
        assert!(
            r == 255 && g > 40 && g < 160 && b == 0,
            "orange at -10 dB: {:?}",
            at(-10.0)
        );
        // Brightness never drops once past the quietest tenth, and the quiet end only gets darker towards the floor.
        assert!(lut.windows(2).take(25).all(|pair| pair[1][2] >= pair[0][2]));
    }

    #[test]
    fn rectangular_window_is_flat_and_hann_tapers() {
        assert!(window_coefficients(WindowFunction::Rectangular, 8)
            .iter()
            .all(|&w| w == 1.0));
        let hann = window_coefficients(WindowFunction::Hann, 8);
        assert!(hann[0] < 1e-6 && (hann[4] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_full_scale_sine_reads_minus_12_db_with_the_hann_window_as_in_spek() {
        // The level is 20 log10(|X| / N) with the window applied and not compensated: |X| = N / 4 for a full-scale sine.
        for n in [2048, 4096] {
            let mut analyzer = Analyzer::new(n, WindowFunction::Hann);
            let bin = 100.0;
            let column = analyzer.analyse(
                (0..n).map(|i| (2.0 * std::f32::consts::PI * bin * i as f32 / n as f32).sin()),
            );
            assert!((column[100] + 12.04).abs() < 0.1, "{n}: {}", column[100]);
        }
    }

    /// A 16-bit mono WAV file of `seconds` of a sine (`amplitude` of full scale) at `frequency`.
    fn write_sine_wav(
        name: &str,
        frequency: f32,
        amplitude: f32,
        seconds: u32,
    ) -> std::path::PathBuf {
        let rate = 44_100_u32;
        let frames = rate * seconds;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + frames * 2).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(frames * 2).to_le_bytes());
        for i in 0..frames {
            let value = (amplitude
                * (2.0 * std::f32::consts::PI * frequency * i as f32 / rate as f32).sin()
                * 32_767.0) as i16;
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let path =
            std::env::temp_dir().join(format!("moe-spectrogram-{}-{name}.wav", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_rendered_column_averages_the_ffts_of_its_span_and_reads_levels_like_spek() {
        // Half scale is 6 dB below full scale: -12 - 6 = -18 dB at the sine's frequency.
        let path = write_sine_wav("level", 5_512.5, 0.5, 4);
        let options = SpectrogramOptions {
            columns: 200,
            height: 1024,
            fft_size: 2048,
            ..SpectrogramOptions::default()
        };
        let rendered = render_spectrogram(&path, options, &AtomicBool::new(false)).expect("render");
        let _ = std::fs::remove_file(&path);
        assert_eq!(rendered.columns, 200);
        // The row of 5.5125 kHz (a quarter of Nyquist: a quarter of the way up from the bottom).
        let panel_height = rendered.height as usize;
        let row = panel_height - panel_height / 4;
        let columns = rendered.columns as usize;
        let level = |x: usize, y: usize| {
            f32::from(rendered.levels[y * columns + x]) / 255.0 * 120.0 - 120.0
        };
        let peak_of = |x: usize| {
            (row.saturating_sub(8)..row + 8)
                .map(|y| level(x, y))
                .fold(f32::MIN, f32::max)
        };
        let peak = peak_of(100);
        assert!((peak + 18.0).abs() < 2.5, "peak {peak} dB");
        // Far from the tone only the window's leakage remains, far below it.
        assert!(level(100, panel_height / 10) < -90.0);
        // Every column shows the same steady tone (each is an average over its own span).
        assert!(
            (1..columns - 1).all(|x| (peak_of(x) - peak).abs() < 3.0),
            "columns differ"
        );
    }

    #[test]
    fn surround_layout_gets_standard_labels() {
        use symphonia::core::audio::Channels as C;
        let layout = C::FRONT_LEFT
            | C::FRONT_RIGHT
            | C::FRONT_CENTRE
            | C::LFE1
            | C::SIDE_LEFT
            | C::SIDE_RIGHT;
        assert_eq!(
            channel_labels(Some(layout), 6),
            ["L", "R", "C", "LFE", "SL", "SR"]
        );
        assert_eq!(channel_labels(None, 3), ["Ch 1", "Ch 2", "Ch 3"]);
        assert_eq!(channel_labels(None, 1), ["Mono"]);
    }

    #[test]
    fn channel_select_serializes_with_mode_tag() {
        let json = serde_json::to_string(&ChannelSelect::Channel(1)).unwrap();
        assert_eq!(json, r#"{"mode":"channel","index":1}"#);
        let mix: ChannelSelect = serde_json::from_str(r#"{"mode":"mix"}"#).unwrap();
        assert_eq!(mix, ChannelSelect::Mix);
    }
}
