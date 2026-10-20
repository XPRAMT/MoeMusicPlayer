//! Whole real songs encoded by the official `wavpack` 5.9.0 command-line encoder (run by hand; see ADR 0025):
//! lossless 16/24/32-bit integer and 32-bit float, several compression modes, and hybrid lossy files with and
//! without their `.wvc` correction file.
//!
//! The manifest (`MOE_WV_MANIFEST`) has one tab-separated row per file:
//! `name  wv  source.wav  wvunpack-with-wvc.wav  wvunpack-wv-only.wav  lossless(0|1)  two-file(0|1)`.
//! Every file must decode sample for sample like the official `wvunpack` (with and without the correction file),
//! a lossless result must equal the original WAV, and seeks must land on the right samples.
//!
//! `cargo test -p player-wavpack --release --test official_encoder -- --ignored --nocapture`

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::Instant;

use player_wavpack::{ReadSeek, WavPackReader};

struct Wav {
    samples: Vec<f32>,
    rate: u32,
    channels: u16,
    bits: u16,
    float: bool,
}

fn read_wav(path: &Path) -> Wav {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    assert_eq!(&bytes[..4], b"RIFF", "{path:?}");
    let (mut format, mut channels, mut rate, mut bits) = (0u16, 0u16, 0u32, 0u16);
    let mut data = &bytes[0..0];
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        if id == b"fmt " {
            format = u16::from_le_bytes([body[0], body[1]]);
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            bits = u16::from_le_bytes([body[14], body[15]]);
            if format == 0xFFFE {
                format = u16::from_le_bytes([body[24], body[25]]);
            }
        } else if id == b"data" {
            data = body;
        }
        at += 8 + size + (size & 1);
    }
    let float = format == 3;
    let width = usize::from(bits / 8);
    let samples = data
        .chunks_exact(width)
        .map(|b| match (float, width) {
            (true, 4) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            (false, 2) => (f64::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0) as f32,
            (false, 3) => {
                (f64::from(i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) / 8_388_608.0) as f32
            }
            (false, 4) => {
                (f64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])) / 2_147_483_648.0) as f32
            }
            _ => panic!("{path:?}: unsupported WAV sample format {format}/{bits}"),
        })
        .collect();
    Wav {
        samples,
        rate,
        channels,
        bits,
        float,
    }
}

fn open(wv: &Path, with_correction: bool) -> WavPackReader {
    let correction = if with_correction {
        let mut name = wv.as_os_str().to_owned();
        name.push("c");
        File::open(PathBuf::from(name))
            .ok()
            .map(|f| Box::new(BufReader::new(f)) as Box<dyn ReadSeek>)
    } else {
        None
    };
    WavPackReader::open(BufReader::new(File::open(wv).unwrap()), correction).unwrap()
}

fn decode(reader: &mut WavPackReader) -> Vec<f32> {
    let mut out = Vec::new();
    while reader.read_frames(&mut out).unwrap() > 0 {}
    out
}

fn mismatches(a: &[f32], b: &[f32]) -> usize {
    a.iter()
        .zip(b)
        .filter(|(x, y)| x.to_bits() != y.to_bits())
        .count()
}

#[test]
#[ignore = "needs MOE_WV_MANIFEST (made by the official wavpack encoder)"]
fn official_encoder_files_decode_like_wvunpack() {
    let manifest =
        std::fs::read_to_string(std::env::var("MOE_WV_MANIFEST").expect("MOE_WV_MANIFEST"))
            .unwrap();
    let mut files = 0;
    for line in manifest.lines().filter(|l| !l.trim().is_empty()) {
        let cols: Vec<&str> = line.split('\t').collect();
        let (name, wv) = (cols[0], PathBuf::from(cols[1]));
        let source = read_wav(Path::new(cols[2]));
        let reference = read_wav(Path::new(cols[3]));
        let reference_alone = read_wav(Path::new(cols[4]));
        let (lossless, two_file) = (cols[5] == "1", cols[6] == "1");

        // 1. with the correction file (if any): equal to the official decode, and to the original when lossless
        let mut reader = open(&wv, true);
        let info = reader.info();
        let started = Instant::now();
        let ours = decode(&mut reader);
        let seconds = started.elapsed().as_secs_f64();
        assert_eq!(
            (info.sample_rate, info.channels),
            (reference.rate, reference.channels),
            "{name}"
        );
        assert_eq!(
            info.correction_file, two_file,
            "{name}: correction file in use"
        );
        assert_eq!(info.lossless, lossless, "{name}: lossless flag");
        assert_eq!(info.hybrid, !lossless || two_file, "{name}: hybrid flag");
        assert_eq!(info.float, source.float, "{name}: float flag");
        assert_eq!(
            ours.len(),
            reference.samples.len(),
            "{name}: length vs wvunpack"
        );
        assert_eq!(
            mismatches(&ours, &reference.samples),
            0,
            "{name}: differs from wvunpack"
        );
        if lossless {
            assert_eq!(
                ours.len(),
                source.samples.len(),
                "{name}: length vs original"
            );
            assert_eq!(
                mismatches(&ours, &source.samples),
                0,
                "{name}: not equal to the original WAV"
            );
        }
        let audio_seconds =
            ours.len() as f64 / f64::from(info.channels) / f64::from(info.sample_rate);

        // 2. the .wv on its own (the lossy half of a two-file set): equal to wvunpack of the lone file
        let mut alone = open(&wv, false);
        let lone = decode(&mut alone);
        assert_eq!(
            lone.len(),
            reference_alone.samples.len(),
            "{name}: lone length"
        );
        assert_eq!(
            mismatches(&lone, &reference_alone.samples),
            0,
            "{name}: lone .wv differs from wvunpack"
        );
        if !lossless {
            let err = lone
                .iter()
                .zip(&source.samples)
                .map(|(a, b)| f64::from(a - b).powi(2))
                .sum::<f64>()
                / lone.len() as f64;
            let signal = source
                .samples
                .iter()
                .map(|v| f64::from(*v).powi(2))
                .sum::<f64>()
                / lone.len() as f64;
            let snr = 10.0 * (signal / err.max(1e-30)).log10();
            assert!(snr > 15.0, "{name}: lossy SNR {snr:.1} dB is too low");
            println!("{name}: lossy SNR against the original {snr:.1} dB");
        }

        // 3. seeks land on the right samples
        let channels = usize::from(info.channels);
        let frames = ours.len() / channels;
        for fraction in [0.0, 0.0371, 0.25, 0.5, 0.618_033_9, 0.9, 0.9999] {
            let frame = ((frames as f64 * fraction) as usize).min(frames - 1);
            reader.seek_to_frame(frame as u64).unwrap();
            let mut chunk = Vec::new();
            reader.read_frames(&mut chunk).unwrap();
            let start = frame * channels;
            assert_eq!(
                mismatches(&chunk, &ours[start..start + chunk.len()]),
                0,
                "{name}: seek to {frame}"
            );
        }
        println!(
            "{name}: {} Hz {} ch {}-bit{} {}{}, {audio_seconds:.1} s, decode {:.0}x real time; matches wvunpack{}",
            info.sample_rate,
            info.channels,
            source.bits,
            if source.float { " float" } else { "" },
            if lossless { "lossless" } else { "hybrid lossy" },
            if two_file { " + wvc" } else { "" },
            audio_seconds / seconds,
            if lossless { ", equals the original, seeks exact" } else { ", seeks exact" },
        );
        files += 1;
    }
    assert!(files > 0, "empty manifest");
    println!("{files} official-encoder files verified");
}
