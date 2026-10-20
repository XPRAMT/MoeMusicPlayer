use super::*;
use std::fs::File;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn decode_all(reader: &mut WavPackReader) -> Vec<f32> {
    let mut out = Vec::new();
    while reader.read_frames(&mut out).unwrap() > 0 {}
    out
}

/// Expected samples from the raw file the encoder was fed (ffmpeg wrote it from the same source).
fn expected_s16(name: &str) -> Vec<f32> {
    std::fs::read(fixture(name))
        .unwrap()
        .chunks_exact(2)
        .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
        .collect()
}

fn open(name: &str) -> WavPackReader {
    WavPackReader::open(File::open(fixture(name)).unwrap(), None).unwrap()
}

#[test]
fn lossless_16_bit_stereo_is_bit_exact() {
    let mut reader = open("s16.wv");
    let info = reader.info();
    assert_eq!(
        (info.sample_rate, info.channels, info.bits_per_sample),
        (44_100, 2, 16)
    );
    assert!(info.lossless && !info.float && !info.hybrid);
    assert_eq!(info.total_frames, Some(13_230));
    assert_eq!(decode_all(&mut reader), expected_s16("s16.raw"));
}

#[test]
fn lossless_24_bit_96k_is_bit_exact() {
    let mut reader = open("s24.wv");
    let info = reader.info();
    // ffmpeg's encoder stores 24-bit input in a 32-bit container (the low byte is zero).
    assert_eq!(
        (info.sample_rate, info.channels, info.bits_per_sample),
        (96_000, 2, 32)
    );
    let expected: Vec<f32> = std::fs::read(fixture("s24.raw"))
        .unwrap()
        .chunks_exact(4)
        .map(|b| (f64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])) / 2_147_483_648.0) as f32)
        .collect();
    assert_eq!(decode_all(&mut reader), expected);
}

#[test]
fn float_file_is_bit_exact() {
    let mut reader = open("f32.wv");
    let info = reader.info();
    assert_eq!((info.sample_rate, info.channels), (48_000, 1));
    assert!(info.float);
    let expected: Vec<f32> = std::fs::read(fixture("f32.raw"))
        .unwrap()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    assert_eq!(decode_all(&mut reader), expected);
}

#[test]
fn six_channels_decode_interleaved() {
    let mut reader = open("s16_51.wv");
    assert_eq!(reader.info().channels, 6);
    assert_eq!(decode_all(&mut reader), expected_s16("s16_51.raw"));
}

#[test]
fn seek_lands_on_the_exact_frame_and_matches_continuous_decoding() {
    let expected = expected_s16("s16.raw");
    let mut reader = open("s16.wv");
    for frame in [0u64, 1, 4_095, 4_096, 7_777, 13_229] {
        reader.seek_to_frame(frame).unwrap();
        assert_eq!(reader.position_frames(), frame);
        let mut out = Vec::new();
        reader.read_frames(&mut out).unwrap();
        let start = frame as usize * 2;
        assert_eq!(
            out[..],
            expected[start..start + out.len()],
            "seek to {frame}"
        );
    }
    // Back to the start after reaching the end.
    reader.seek_to_frame(13_230).unwrap();
    assert_eq!(reader.read_frames(&mut Vec::new()).unwrap(), 0);
    reader.seek_to_frame(0).unwrap();
    assert_eq!(decode_all(&mut reader), expected);
}

#[test]
fn garbage_is_rejected_with_a_message() {
    let error = WavPackReader::open(std::io::Cursor::new(vec![0u8; 4096]), None)
        .err()
        .expect("not a WavPack file");
    assert!(matches!(error, WavPackError::Open(_)), "{error}");
}

#[test]
fn header_check_recognizes_wvpk() {
    assert!(is_wavpack_header(
        &std::fs::read(fixture("s16.wv")).unwrap()
    ));
    assert!(!is_wavpack_header(b"RIFF"));
}

#[test]
fn dsd_in_wavpack_is_detected_and_decimated_like_wvunpack() {
    let mut reader = open("dsd64.wv");
    let info = reader.info();
    assert!(info.dsd && info.lossless && !info.float);
    assert_eq!(info.native_sample_rate, 2_822_400, "the 1-bit rate");
    assert_eq!(
        (info.sample_rate, info.channels, info.bits_per_sample),
        (352_800, 2, 24)
    );
    // `wvunpack --raw-pcm` writes the same 24-bit decimation (little-endian, packed).
    let expected: Vec<f32> = std::fs::read(fixture("dsd64.raw24"))
        .unwrap()
        .chunks_exact(3)
        .map(|b| (f64::from(i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) / 8_388_608.0) as f32)
        .collect();
    assert_eq!(decode_all(&mut reader), expected);
}

#[test]
fn summaries_label_lossless_lossy_and_dsd_files() {
    use crate::{summarize, WavPackSummary};
    let summary = |name: &str| summarize(&fixture(name));
    assert_eq!(
        summary("s16.wv"),
        Some(WavPackSummary {
            codec: "WVLL",
            sample_rate_hz: 44_100,
            bit_depth: Some(16)
        })
    );
    assert_eq!(
        summary("f32.wv"),
        Some(WavPackSummary {
            codec: "WVLL",
            sample_rate_hz: 48_000,
            bit_depth: Some(32)
        })
    );
    assert_eq!(
        summary("dsd64.wv"),
        Some(WavPackSummary {
            codec: "WVDSD",
            sample_rate_hz: 2_822_400,
            bit_depth: Some(1)
        })
    );
    let lossy = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/reference/hybrid_lossy_int16_stereo.wv");
    assert_eq!(
        summarize(&lossy),
        Some(WavPackSummary {
            codec: "WVL",
            sample_rate_hz: 48_000,
            bit_depth: None
        })
    );
    assert_eq!(summary("missing.wv"), None);
}
