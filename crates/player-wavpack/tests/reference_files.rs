//! Files made by the reference `wavpack` 5.9.0 encoder (taken from the MIT/Apache-2.0 `wavicle` crate's fixtures, see
//! `tests/reference/README.md`): the decoder must reproduce the samples the reference `wvunpack` wrote, bit for bit,
//! for every sample format WavPack has, and decode the hybrid lossy file.

use std::fs::File;
use std::path::PathBuf;

use player_wavpack::WavPackReader;

fn reference(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/reference")
        .join(name)
}

fn decode(name: &str) -> (WavPackReader, Vec<f32>) {
    let mut reader =
        WavPackReader::open(File::open(reference(&format!("{name}.wv"))).unwrap(), None).unwrap();
    let mut out = Vec::new();
    while reader.read_frames(&mut out).unwrap() > 0 {}
    (reader, out)
}

/// The expected samples from the reference decoder's raw output; `bits` is the source sample width.
fn expected(name: &str, bits: u32, float: bool) -> Vec<f32> {
    let bytes = std::fs::read(reference(&format!("{name}.decoded.raw"))).unwrap();
    let width = (bits / 8) as usize;
    bytes
        .chunks_exact(width)
        .map(|b| {
            if float {
                f32::from_le_bytes([b[0], b[1], b[2], b[3]])
            } else {
                let value = match width {
                    2 => i32::from(i16::from_le_bytes([b[0], b[1]])),
                    3 => i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8,
                    _ => i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                };
                (f64::from(value) / 2f64.powi(bits as i32 - 1)) as f32
            }
        })
        .collect()
}

fn check(name: &str, bits: u32, float: bool, rate: u32, channels: u16) {
    let (reader, samples) = decode(name);
    let info = reader.info();
    assert_eq!(
        (info.sample_rate, info.channels),
        (rate, channels),
        "{name}"
    );
    assert_eq!(info.float, float, "{name}");
    assert!(info.lossless, "{name}");
    let want = expected(name, bits, float);
    assert_eq!(samples.len(), want.len(), "{name}: length");
    let wrong = samples
        .iter()
        .zip(&want)
        .filter(|(a, b)| a.to_bits() != b.to_bits())
        .count();
    assert_eq!(
        wrong, 0,
        "{name}: {wrong} samples differ from the reference decode"
    );
}

#[test]
fn integer_formats_match_the_reference_decoder() {
    check("int16_mono", 16, false, 44_100, 1);
    check("int16_stereo", 16, false, 48_000, 2);
    check("int24_stereo", 24, false, 48_000, 2);
    check("int32_stereo", 32, false, 48_000, 2);
    check("int32_shifted_mono", 32, false, 48_000, 1);
    check("multiblock_int16_mono", 16, false, 44_100, 1);
    check("false_stereo", 16, false, 48_000, 2);
    check("silence_stereo", 16, false, 48_000, 2);
}

#[test]
fn float_formats_match_the_reference_decoder_including_special_values() {
    check("f32_mono", 32, true, 48_000, 1);
    check("f32_stereo", 32, true, 48_000, 2);
    check("f32_specials_mono", 32, true, 48_000, 1);
}

#[test]
fn a_24_bit_file_reports_24_bits() {
    let (reader, _) = decode("int24_stereo");
    assert_eq!(reader.info().bits_per_sample, 24);
}

#[test]
fn hybrid_lossy_file_decodes_to_the_right_length() {
    let mut reader = WavPackReader::open(
        File::open(reference("hybrid_lossy_int16_stereo.wv")).unwrap(),
        None,
    )
    .unwrap();
    let info = reader.info();
    assert!(info.hybrid && !info.lossless);
    assert_eq!((info.sample_rate, info.channels), (48_000, 2));
    let mut out = Vec::new();
    while reader.read_frames(&mut out).unwrap() > 0 {}
    assert_eq!(Some((out.len() / 2) as u64), info.total_frames);
    assert!(out.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    assert!(out.iter().any(|v| v.abs() > 0.05), "not silence");
}

#[test]
fn every_reference_file_seeks_to_exact_samples() {
    for name in [
        "int16_stereo",
        "int24_stereo",
        "multiblock_int16_mono",
        "f32_stereo",
    ] {
        let (mut reader, all) = decode(name);
        let channels = usize::from(reader.info().channels);
        let frames = all.len() / channels;
        for frame in [0, frames / 3, frames / 2, frames - 1] {
            reader.seek_to_frame(frame as u64).unwrap();
            let mut chunk = Vec::new();
            reader.read_frames(&mut chunk).unwrap();
            let start = frame * channels;
            assert_eq!(
                chunk[..],
                all[start..start + chunk.len()],
                "{name} seek {frame}"
            );
        }
    }
}
