//! Real-music round trip (run by hand): FLAC → WavPack with ffmpeg at compression levels 0 and 3 (ffmpeg’s higher levels take many minutes per track), decode with
//! `player-wavpack`, compare sample for sample with ffmpeg's decode of the same FLAC, time the decode, and check that
//! random seeks land on the right samples.
//!
//! `MOE_WV_FILES` is a `;`-separated list of FLAC files; ffmpeg must be on the PATH.
//! `cargo test -p player-wavpack --release --test real_music -- --ignored --nocapture`

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use player_wavpack::WavPackReader;

fn ffmpeg(args: &[&str]) {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "ffmpeg {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn raw_samples(path: &Path) -> Vec<f32> {
    std::fs::read(path)
        .unwrap()
        .chunks_exact(4)
        .map(|b| (f64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])) / 2_147_483_648.0) as f32)
        .collect()
}

#[test]
#[ignore = "needs MOE_WV_FILES and ffmpeg"]
fn flac_to_wavpack_round_trip() {
    let files = std::env::var("MOE_WV_FILES").expect("MOE_WV_FILES");
    let work = std::env::temp_dir().join(format!("moe-wv-real-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    for (index, flac) in files.split(';').filter(|f| !f.is_empty()).enumerate() {
        let reference_path = work.join(format!("ref{index}.raw"));
        ffmpeg(&[
            "-i",
            flac,
            "-vn",
            "-f",
            "s32le",
            reference_path.to_str().unwrap(),
        ]);
        let expected = raw_samples(&reference_path);
        for level in [0, 3] {
            let wv = work.join(format!("t{index}-{level}.wv"));
            let encode = Instant::now();
            ffmpeg(&[
                "-i",
                flac,
                "-vn",
                "-c:a",
                "wavpack",
                "-compression_level",
                &level.to_string(),
                wv.to_str().unwrap(),
            ]);
            let encode_seconds = encode.elapsed().as_secs_f64();
            let mut reader =
                WavPackReader::open(BufReader::new(File::open(&wv).unwrap()), None).unwrap();
            let info = reader.info();
            let started = Instant::now();
            let mut out = Vec::new();
            while reader.read_frames(&mut out).unwrap() > 0 {}
            let seconds = started.elapsed().as_secs_f64();
            let audio_seconds =
                out.len() as f64 / f64::from(info.channels) / f64::from(info.sample_rate);
            assert_eq!(out.len(), expected.len(), "{flac} level {level}: length");
            let mismatches = out.iter().zip(&expected).filter(|(a, b)| a != b).count();
            assert_eq!(mismatches, 0, "{flac} level {level}: sample mismatches");
            println!(
                "{index} level {level}: {} Hz {} ch {}-bit container, {:.1} s audio, wv {:.1}% of flac size, decode {:.0}x real time, ffmpeg encode {:.1}s — bit exact",
                info.sample_rate,
                info.channels,
                info.bits_per_sample,
                audio_seconds,
                100.0 * std::fs::metadata(&wv).unwrap().len() as f64
                    / std::fs::metadata(flac).unwrap().len() as f64,
                audio_seconds / seconds,
                encode_seconds,
            );
            if level == 3 {
                let channels = usize::from(info.channels);
                let frames = out.len() / channels;
                for fraction in [0.0, 0.123, 0.5, 0.777, 0.99] {
                    let frame = (frames as f64 * fraction) as usize;
                    reader.seek_to_frame(frame as u64).unwrap();
                    let mut chunk = Vec::new();
                    reader.read_frames(&mut chunk).unwrap();
                    let start = frame * channels;
                    assert_eq!(
                        chunk[..],
                        expected[start..start + chunk.len()],
                        "seek to {frame}"
                    );
                }
                println!("{index}   seek: 5 positions exact");
            }
            let _ = std::fs::remove_file(&wv);
        }
        let _ = std::fs::remove_file(&reference_path);
    }
    let _ = std::fs::remove_dir(PathBuf::from(&work));
}
