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
            fft_size: 4096,
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

const SPEK: Stops = &[
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
        Colormap::Spek => SPEK,
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
    let stops = stops(map);
    (0..256)
        .map(|i| {
            let t = i as f32 / 255.0;
            let pair = stops
                .windows(2)
                .find(|pair| t <= pair[1].0)
                .unwrap_or(&stops[stops.len() - 2..]);
            let ((a, ca), (b, cb)) = (pair[0], pair[1]);
            let k = ((t - a) / (b - a)).clamp(0.0, 1.0);
            std::array::from_fn(|c| (ca[c] + (cb[c] - ca[c]) * k).round() as u8)
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
        // Coherent gain so a full-scale sine reads ~0 dBFS regardless of window.
        let norm = window.iter().sum::<f32>() / 2.0;
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

/// Render a spectrogram PNG for `path`.
pub fn render_spectrogram(
    path: &Path,
    options: SpectrogramOptions,
    cancel: &AtomicBool,
) -> Result<Spectrogram, String> {
    let n = options.fft_size.clamp(256, 32768).next_power_of_two();
    let cols = options.columns.clamp(16, 8000) as usize;
    let height = options.height.clamp(64, 4096);

    let file = File::open(path).map_err(|e| format!("無法開啟檔案：{e}"))?;
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
    let mut format = probed.format;
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
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("不支援的編碼：{e}"))?;
    let known_frames = track.codec_params.n_frames;

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
    let mut rings: Vec<VecDeque<f32>> = (0..streams).map(|_| VecDeque::with_capacity(n)).collect();
    let mut collected: Vec<Vec<f32>> = vec![Vec::new(); streams];
    let mut spectra: Vec<Vec<Vec<f32>>> = vec![Vec::with_capacity(cols); streams];
    let column_end = |c: usize, total: u64| -> u64 {
        let total = total.max(n as u64);
        (total - n as u64) * c as u64 / (cols as u64 - 1).max(1) + n as u64 - 1
    };
    let mut next_col = 0usize;
    let mut index: u64 = 0;
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
        if packet.track_id() != track.id {
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
            match known_frames {
                Some(total) => {
                    for (s, ring) in rings.iter_mut().enumerate() {
                        if ring.len() == n {
                            ring.pop_front();
                        }
                        ring.push_back(select(frame, s));
                    }
                    while next_col < cols && index == column_end(next_col, total) {
                        for (s, ring) in rings.iter().enumerate() {
                            let column = analyzer.analyse(ring.iter().copied());
                            spectra[s].push(column);
                        }
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
        }
    }
    let total = known_frames.map(|t| t.min(index)).unwrap_or(index);
    if known_frames.is_none() {
        for (s, store) in collected.iter().enumerate() {
            for c in 0..cols {
                let end = column_end(c, store.len() as u64) as usize;
                let start = end + 1 - n;
                let column =
                    analyzer.analyse((start..=end).map(|i| store.get(i).copied().unwrap_or(0.0)));
                spectra[s].push(column);
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
    for (s, panel) in spectra.iter().enumerate() {
        for (x, column) in panel.iter().enumerate() {
            for (y, &(lo, hi)) in row_bins.iter().enumerate() {
                let db = column[lo..hi].iter().copied().fold(f32::MIN, f32::max);
                let t = ((db - floor) / range).clamp(0.0, 1.0);
                let rgb = lut[(t * 255.0).round() as usize];
                img.put_pixel(x as u32, s as u32 * panel_height + y as u32, Rgb(rgb));
            }
        }
    }
    let cutoff_hz = estimate_cutoff(&spectra[0], sample_rate, n);
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("PNG 編碼失敗：{e}"))?;
    Ok(Spectrogram {
        png,
        sample_rate,
        channels: channel_count as u32,
        channel_labels: channel_labels(track.codec_params.channels, channel_count),
        duration_ms: total * 1000 / u64::from(sample_rate),
        columns: out_cols,
        height: img.height(),
        panels: streams as u32,
        max_frequency_hz: max_hz,
        cutoff_hz,
    })
}

/// Highest bin whose median level across columns is above -100 dB, reported only when
/// clearly below Nyquist (a hint of lossy origin, not a verdict).
fn estimate_cutoff(spectra: &[Vec<f32>], sample_rate: u32, n: usize) -> Option<u32> {
    let bins = spectra.first()?.len();
    let mut top = 0usize;
    for b in 0..bins {
        let mut levels: Vec<f32> = spectra.iter().map(|c| c[b]).collect();
        levels.sort_by(|a, b| a.total_cmp(b));
        if levels[levels.len() / 2] > -100.0 {
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
        assert_eq!(colormap_lut(Colormap::Spek)[255], [255, 255, 255]);
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
    fn full_scale_sine_reads_near_zero_db() {
        let n = 4096;
        let mut analyzer = Analyzer::new(n, WindowFunction::Hann);
        let bin = 100.0;
        let column = analyzer.analyse(
            (0..n).map(|i| (2.0 * std::f32::consts::PI * bin * i as f32 / n as f32).sin()),
        );
        assert!(column[100] > -1.0 && column[100] < 1.0, "{}", column[100]);
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
