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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectrogramOptions {
    pub columns: u32,
    pub height: u32,
    pub fft_size: usize,
}

impl Default for SpectrogramOptions {
    fn default() -> Self {
        Self {
            columns: 1600,
            height: 512,
            fft_size: 4096,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Spectrogram {
    pub png: Vec<u8>,
    pub sample_rate: u32,
    pub duration_ms: u64,
    pub columns: u32,
    pub height: u32,
    /// Estimated frequency above which energy stays below -90 dBFS-ish (lossy cutoff hint).
    pub cutoff_hz: Option<u32>,
}

const MAGMA: [(f32, [f32; 3]); 6] = [
    (0.0, [0.0, 0.0, 4.0]),
    (0.2, [59.0, 15.0, 112.0]),
    (0.4, [140.0, 41.0, 129.0]),
    (0.6, [222.0, 73.0, 104.0]),
    (0.8, [254.0, 159.0, 109.0]),
    (1.0, [252.0, 253.0, 191.0]),
];

fn colormap(t: f32) -> Rgb<u8> {
    let t = t.clamp(0.0, 1.0);
    for pair in MAGMA.windows(2) {
        let (a, ca) = pair[0];
        let (b, cb) = pair[1];
        if t <= b {
            let k = (t - a) / (b - a);
            return Rgb(std::array::from_fn(|i| (ca[i] + (cb[i] - ca[i]) * k) as u8));
        }
    }
    Rgb([252, 253, 191])
}

const DB_FLOOR: f32 = -120.0;

/// Decode `path` and render a magma-coloured STFT spectrogram PNG. Only `columns` windows
/// are analysed (evenly spaced), so cost is dominated by decoding, not by track length.
pub fn render_spectrogram(
    path: &Path,
    options: SpectrogramOptions,
    cancel: &AtomicBool,
) -> Result<Spectrogram, String> {
    let n = options.fft_size.max(256).next_power_of_two();
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
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("不支援的編碼：{e}"))?;
    let known_frames = track.codec_params.n_frames;

    // Column scheduling: with a known length we analyse on the fly from a ring buffer;
    // otherwise samples are collected first (rare: streams without a frame count).
    let mut collected: Vec<f32> = Vec::new();
    let mut ring = vec![0.0f32; n];
    let mut index: u64 = 0;
    let mut spectra: Vec<Vec<f32>> = Vec::with_capacity(cols);
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let window: Vec<f32> = (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos())
        .collect();
    let mut input = fft.make_input_vec();
    let mut output = fft.make_output_vec();
    let norm = (n as f32) / 4.0;

    let mut analyse = |samples: &mut dyn FnMut(usize) -> f32, spectra: &mut Vec<Vec<f32>>| {
        for (i, slot) in input.iter_mut().enumerate() {
            *slot = samples(i) * window[i];
        }
        if fft.process(&mut input, &mut output).is_ok() {
            spectra.push(
                output
                    .iter()
                    .map(|c| (20.0 * (c.norm() / norm).max(1e-9).log10()).max(DB_FLOOR))
                    .collect(),
            );
        }
    };

    let column_end = |c: usize, total: u64| -> u64 {
        let total = total.max(n as u64);
        let span = total - n as u64;
        (span * c as u64) / (cols as u64 - 1).max(1) + n as u64 - 1
    };
    let mut next_col = 0usize;
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
        let buf =
            sample_buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if buf.capacity() < decoded.capacity() * channels {
            *buf = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        buf.copy_interleaved_ref(decoded);
        for frame in buf.samples().chunks(channels) {
            let mono = frame.iter().sum::<f32>() / channels as f32;
            match known_frames {
                Some(total) => {
                    ring[(index % n as u64) as usize] = mono;
                    while next_col < cols && index == column_end(next_col, total) {
                        let start = index + 1 - n as u64;
                        let ring_ref = &ring;
                        analyse(
                            &mut |i| ring_ref[((start + i as u64) % n as u64) as usize],
                            &mut spectra,
                        );
                        next_col += 1;
                    }
                }
                None => collected.push(mono),
            }
            index += 1;
        }
    }
    let total = known_frames.map(|t| t.min(index)).unwrap_or(index);
    if known_frames.is_none() || spectra.len() < cols {
        if known_frames.is_some() {
            // Frame count overstated: fall back to whatever columns were produced.
        } else {
            let len = collected.len();
            for c in 0..cols {
                let end = column_end(c, len as u64) as usize;
                let start = end + 1 - n;
                analyse(
                    &mut |i| collected.get(start + i).copied().unwrap_or(0.0),
                    &mut spectra,
                );
            }
        }
    }
    if spectra.is_empty() {
        return Err("沒有可分析的音訊".into());
    }

    let bins = n / 2 + 1;
    let out_cols = spectra.len() as u32;
    let mut img = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(out_cols, height);
    for (x, column) in spectra.iter().enumerate() {
        for y in 0..height {
            // Row 0 = Nyquist, bottom row = 0 Hz. Max over the bins mapped to this row.
            let row = height - 1 - y;
            let lo = (row as usize * bins) / height as usize;
            let hi = (((row as usize + 1) * bins) / height as usize)
                .max(lo + 1)
                .min(bins);
            let db = column[lo..hi].iter().copied().fold(DB_FLOOR, f32::max);
            img.put_pixel(x as u32, y, colormap((db - DB_FLOOR) / -DB_FLOOR));
        }
    }
    let cutoff_hz = estimate_cutoff(&spectra, sample_rate, n);
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("PNG 編碼失敗：{e}"))?;
    Ok(Spectrogram {
        png,
        sample_rate,
        duration_ms: total * 1000 / u64::from(sample_rate),
        columns: out_cols,
        height,
        cutoff_hz,
    })
}

/// Highest bin whose 95th-percentile level across columns exceeds -90 dB. Reported only
/// when clearly below Nyquist (a hint of lossy origin, not a verdict).
fn estimate_cutoff(spectra: &[Vec<f32>], sample_rate: u32, n: usize) -> Option<u32> {
    let bins = spectra.first()?.len();
    let mut top = 0usize;
    for b in 0..bins {
        let mut levels: Vec<f32> = spectra.iter().map(|c| c[b]).collect();
        levels.sort_by(|a, b| a.total_cmp(b));
        if levels[(levels.len() * 95 / 100).min(levels.len() - 1)] > -90.0 {
            top = b;
        }
    }
    let hz = (top as u64 * u64::from(sample_rate) / n as u64) as u32;
    (hz > 0 && hz < sample_rate / 2 * 95 / 100).then_some(hz)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colormap_endpoints() {
        assert_eq!(colormap(0.0), Rgb([0, 0, 4]));
        assert_eq!(colormap(1.0), Rgb([252, 253, 191]));
    }
}
