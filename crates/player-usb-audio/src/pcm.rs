//! The PCM the exclusive test sends (ADR 0046, P0: 16-bit stereo FLAC only, no volume, no resampling, no dither).
//!
//! `player-flac` decodes to `f32` by dividing Symphonia's left-aligned `i32` samples by 2^31, which is exact for 16-bit
//! samples (16 significant bits, a power-of-two divisor). Multiplying by 32 768 gives the original sample back; every
//! value is checked to be such a sample, so the bytes sent are the file's samples bit for bit.

use std::path::Path;

use player_flac::{FlacInfo, FlacReader};

/// Interleaved little-endian 16-bit PCM, ready to be cut into packets.
#[derive(Clone, Debug)]
pub struct Pcm16 {
    pub rate: u32,
    pub channels: u16,
    pub bytes: Vec<u8>,
}

impl Pcm16 {
    pub fn bytes_per_frame(&self) -> usize {
        2 * usize::from(self.channels)
    }

    pub fn frames(&self) -> u64 {
        (self.bytes.len() / self.bytes_per_frame().max(1)) as u64
    }
}

/// How many seconds from the start the first peak covers.
pub const LEVEL_FIRST_SECONDS: u32 = 3;
/// How many interleaved samples from the start the report lists.
pub const LEVEL_FIRST_SAMPLES: usize = 16;

/// How loud the PCM sent is, to rule out a silent buffer when nothing is heard (ADR 0046).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PcmLevels {
    pub first_seconds: u32,
    /// Peak absolute sample (0..=32768) over the first [`LEVEL_FIRST_SECONDS`].
    pub peak_first: u32,
    /// The same in dBFS (0 = full scale); `None` when every sample is 0.
    pub peak_first_dbfs: Option<f64>,
    /// Peak absolute sample over the whole buffer.
    pub peak: u32,
    pub peak_dbfs: Option<f64>,
    /// The first interleaved samples.
    pub first_samples: Vec<i16>,
}

/// dBFS of a peak absolute 16-bit sample (32768 = 0 dBFS); `None` for silence.
pub fn peak_dbfs(peak: u32) -> Option<f64> {
    (peak > 0).then(|| 20.0 * (f64::from(peak) / 32_768.0).log10())
}

/// Peaks and the first samples of `pcm`.
pub fn levels(pcm: &Pcm16) -> PcmLevels {
    let first_bytes =
        (u64::from(pcm.rate) * u64::from(LEVEL_FIRST_SECONDS)) as usize * pcm.bytes_per_frame();
    let mut peak_first = 0_u32;
    let mut peak = 0_u32;
    let mut first_samples = Vec::with_capacity(LEVEL_FIRST_SAMPLES);
    for (index, pair) in pcm.bytes.chunks_exact(2).enumerate() {
        let sample = i16::from_le_bytes([pair[0], pair[1]]);
        let magnitude = u32::from(sample.unsigned_abs());
        peak = peak.max(magnitude);
        if index * 2 < first_bytes {
            peak_first = peak_first.max(magnitude);
        }
        if first_samples.len() < LEVEL_FIRST_SAMPLES {
            first_samples.push(sample);
        }
    }
    PcmLevels {
        first_seconds: LEVEL_FIRST_SECONDS,
        peak_first,
        peak_first_dbfs: peak_dbfs(peak_first),
        peak,
        peak_dbfs: peak_dbfs(peak),
        first_samples,
    }
}

/// Why a file cannot be played by the P0 test.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PcmError {
    Open(String),
    NotSixteenBitStereo {
        bits: Option<u32>,
        channels: u16,
    },
    Decode(String),
    /// A decoded value is not a 16-bit sample (should never happen for a 16-bit file).
    NotExact,
}

impl std::fmt::Display for PcmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open(message) | Self::Decode(message) => f.write_str(message),
            Self::NotSixteenBitStereo { bits, channels } => write!(
                f,
                "只支援 16 bit 雙聲道 FLAC（此檔為 {} bit、{channels} 聲道）",
                bits.map_or_else(|| "?".to_owned(), |bits| bits.to_string())
            ),
            Self::NotExact => f.write_str("解碼結果不是 16 bit 取樣"),
        }
    }
}

impl std::error::Error for PcmError {}

/// Whether the stream is one the P0 test plays.
pub fn check_format(info: &FlacInfo) -> Result<(), PcmError> {
    if info.bits_per_sample == Some(16) && info.channels == 2 {
        Ok(())
    } else {
        Err(PcmError::NotSixteenBitStereo {
            bits: info.bits_per_sample,
            channels: info.channels,
        })
    }
}

/// Append `samples` (exact 16-bit values in ±1.0) as little-endian 16-bit integers.
pub fn pack_s16le(samples: &[f32], out: &mut Vec<u8>) -> Result<(), PcmError> {
    out.reserve(samples.len() * 2);
    for &sample in samples {
        let scaled = sample * 32_768.0;
        if scaled.fract() != 0.0 || !(-32_768.0..=32_767.0).contains(&scaled) {
            return Err(PcmError::NotExact);
        }
        out.extend_from_slice(&(scaled as i16).to_le_bytes());
    }
    Ok(())
}

/// The first `max_seconds` of a 16-bit stereo FLAC file, decoded whole before the stream starts.
pub fn decode_flac_s16(path: &Path, max_seconds: u32) -> Result<Pcm16, PcmError> {
    let mut reader =
        FlacReader::open_path(path).map_err(|error| PcmError::Open(error.to_string()))?;
    let info = *reader.info();
    check_format(&info)?;
    let limit = u64::from(info.sample_rate) * u64::from(max_seconds);
    let mut bytes =
        Vec::with_capacity((limit.min(info.total_frames.unwrap_or(limit)) * 4) as usize);
    let mut samples = Vec::new();
    let mut frames = 0_u64;
    while frames < limit {
        samples.clear();
        let read = reader
            .read_frames(&mut samples)
            .map_err(|error| PcmError::Decode(error.to_string()))?;
        if read == 0 {
            break;
        }
        let keep = (read as u64).min(limit - frames) as usize;
        pack_s16le(&samples[..keep * 2], &mut bytes)?;
        frames += keep as u64;
    }
    Ok(Pcm16 {
        rate: info.sample_rate,
        channels: info.channels,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use md5::{Digest, Md5};
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../player-flac/tests/fixtures")
            .join(name)
    }

    /// The MD5 signature in STREAMINFO (the last 16 of its 34 bytes, right after "fLaC" and the block header).
    fn streaminfo_md5(path: &Path) -> [u8; 16] {
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(&bytes[..4], b"fLaC");
        bytes[26..42].try_into().unwrap()
    }

    #[test]
    fn a_16_bit_stereo_flac_decodes_to_exactly_the_samples_its_md5_signs() {
        let path = fixture("s16_stereo_44100.flac");
        let pcm = decode_flac_s16(&path, 30).unwrap();
        assert_eq!((pcm.rate, pcm.channels, pcm.frames()), (44_100, 2, 44_100));
        let digest: [u8; 16] = Md5::digest(&pcm.bytes).into();
        assert_eq!(digest, streaminfo_md5(&path));
    }

    #[test]
    fn levels_give_the_peaks_in_dbfs_and_the_first_samples() {
        assert_eq!(peak_dbfs(0), None);
        assert_eq!(peak_dbfs(32_768), Some(0.0));
        assert!((peak_dbfs(16_384).unwrap() + 6.0206).abs() < 1e-3);
        assert!((peak_dbfs(1).unwrap() + 90.309).abs() < 1e-3);

        // 4 s at a tiny rate so the first-seconds window is visible: 3 s quiet, then one loud sample, and -32768.
        let rate = 4;
        let mut samples = vec![100_i16; (rate * 3 * 2) as usize];
        samples[1] = -200;
        samples.extend([16_384, -32_768, 0, 0, 0, 0, 0, 0]);
        let bytes = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let pcm = Pcm16 {
            rate,
            channels: 2,
            bytes,
        };
        let levels = levels(&pcm);
        assert_eq!((levels.peak_first, levels.peak), (200, 32_768));
        assert_eq!(levels.peak_dbfs, Some(0.0));
        assert!(
            (levels.peak_first_dbfs.unwrap() - 20.0 * (200.0_f64 / 32_768.0).log10()).abs() < 1e-9
        );
        assert_eq!(levels.first_samples.len(), LEVEL_FIRST_SAMPLES);
        assert_eq!(&levels.first_samples[..3], &[100, -200, 100]);

        let silent = levels_of_silence();
        assert_eq!(
            (silent.peak, silent.peak_dbfs, silent.peak_first_dbfs),
            (0, None, None)
        );
        assert_eq!(silent.first_samples, vec![0, 0]);
    }

    fn levels_of_silence() -> PcmLevels {
        levels(&Pcm16 {
            rate: 44_100,
            channels: 2,
            bytes: vec![0; 4],
        })
    }

    #[test]
    fn the_fixture_is_not_silent() {
        let levels = levels(&decode_flac_s16(&fixture("s16_stereo_44100.flac"), 30).unwrap());
        assert!(levels.peak > 0 && levels.peak_first > 0, "{levels:?}");
    }

    #[test]
    fn the_length_is_cut_at_the_limit() {
        let pcm = decode_flac_s16(&fixture("s16_stereo_44100.flac"), 0).unwrap();
        assert_eq!(pcm.frames(), 0);
    }

    #[test]
    fn other_bit_depths_and_channel_counts_are_refused() {
        assert_eq!(
            decode_flac_s16(&fixture("s24_stereo_96000.flac"), 30).unwrap_err(),
            PcmError::NotSixteenBitStereo {
                bits: Some(24),
                channels: 2
            }
        );
        assert!(matches!(
            decode_flac_s16(&fixture("s16_6ch_48000.flac"), 30),
            Err(PcmError::NotSixteenBitStereo { channels: 6, .. })
        ));
        assert!(matches!(
            decode_flac_s16(&fixture("missing.flac"), 30),
            Err(PcmError::Open(_))
        ));
    }

    #[test]
    fn packing_is_an_exact_bit_copy_and_refuses_anything_else() {
        let mut out = Vec::new();
        pack_s16le(&[-1.0, 0.0, 32_767.0 / 32_768.0, 1.0 / 32_768.0], &mut out).unwrap();
        assert_eq!(out, vec![0x00, 0x80, 0, 0, 0xFF, 0x7F, 0x01, 0x00]);
        assert_eq!(
            pack_s16le(&[0.5 / 32_768.0], &mut Vec::new()),
            Err(PcmError::NotExact)
        );
        assert_eq!(pack_s16le(&[1.0], &mut Vec::new()), Err(PcmError::NotExact));
    }
}
