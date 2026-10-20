//! Tags, covers, editing and the spectrogram on whole songs made by the official `wavpack` 5.9.0 encoder (run by
//! hand; `MOE_WV_MANIFEST` is the manifest of `crates/player-wavpack/tests/official_encoder.rs`). The encoder wrote
//! Title/Artist/Album/Year and a `Cover Art (Front)` item into every file.
//!
//! `cargo test -p player-tags --release --test wavpack_official -- --ignored --nocapture`

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use player_tags::tags::read_cover;
use player_tags::{read_details, render_spectrogram, write_edit, SpectrogramOptions, TagEdit};
use player_wavpack::{ReadSeek, WavPackReader};

fn rows() -> Vec<(String, PathBuf, bool, bool)> {
    let manifest =
        std::fs::read_to_string(std::env::var("MOE_WV_MANIFEST").expect("MOE_WV_MANIFEST"))
            .unwrap();
    manifest
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            (
                cols[0].to_owned(),
                PathBuf::from(cols[1]),
                cols[5] == "1",
                cols[6] == "1",
            )
        })
        .collect()
}

fn decode(wv: &Path) -> Vec<f32> {
    let mut name = wv.as_os_str().to_owned();
    name.push("c");
    let correction = File::open(PathBuf::from(name))
        .ok()
        .map(|f| Box::new(BufReader::new(f)) as Box<dyn ReadSeek>);
    let mut reader =
        WavPackReader::open(BufReader::new(File::open(wv).unwrap()), correction).unwrap();
    let mut out = Vec::new();
    while reader.read_frames(&mut out).unwrap() > 0 {}
    out
}

#[test]
#[ignore = "needs MOE_WV_MANIFEST"]
fn official_files_have_readable_tags_and_covers() {
    for (name, wv, _, _) in rows() {
        let details = read_details(&wv).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(details.format, "WV", "{name}");
        assert_eq!(details.tag_format, "APEv2", "{name}");
        assert_eq!(details.fields.title, name, "{name}");
        assert_eq!(details.fields.artist, "MoeTest", "{name}");
        assert!(details.fields.album.starts_with("WavPack "), "{name}");
        assert!(
            details
                .fields
                .lyrics
                .starts_with("[00:01.00]第一行：官方編碼器"),
            "{name}: the Lyrics item the encoder wrote: {:?}",
            details.fields.lyrics
        );
        let cover = details
            .cover
            .as_ref()
            .unwrap_or_else(|| panic!("{name}: no cover"));
        assert!(cover.width > 100 && cover.height > 100, "{name}: {cover:?}");
        let (bytes, mime) = read_cover(&wv)
            .unwrap()
            .unwrap_or_else(|| panic!("{name}: no cover bytes"));
        assert_eq!(mime, "image/jpeg", "{name}");
        assert_eq!(bytes.len() as u64, cover.bytes, "{name}");
        println!(
            "{name}: tags ok ({} Hz, {:?} ch, bit depth {:?}), cover {}x{} {} KiB",
            details.properties.sample_rate.unwrap_or(0),
            details.properties.channels,
            details.properties.bit_depth,
            cover.width,
            cover.height,
            cover.bytes / 1024
        );
    }
}

#[test]
#[ignore = "needs MOE_WV_MANIFEST"]
fn editing_an_official_file_keeps_the_audio_the_cover_and_the_correction_file_pairing() {
    let work = std::env::temp_dir().join(format!("moe-wv-official-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    for (name, wv, _, two_file) in rows() {
        let copy = work.join(format!("{name}.wv"));
        std::fs::copy(&wv, &copy).unwrap();
        if two_file {
            let mut wvc = wv.as_os_str().to_owned();
            wvc.push("c");
            std::fs::copy(PathBuf::from(wvc), work.join(format!("{name}.wvc"))).unwrap();
        }
        let before = decode(&copy);
        let original_cover = read_cover(&copy).unwrap().unwrap().0;

        // 1. change fields only: the cover and the unknown items stay, the audio is untouched
        let details = read_details(&copy).unwrap();
        let mut edit = TagEdit {
            fields: details.fields.clone(),
            ..TagEdit::default()
        };
        edit.fields.title = format!("{name} 改過");
        edit.fields.lyrics = "[00:01.00]歌詞".to_owned();
        let details = write_edit(&copy, &edit).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(details.fields.title, format!("{name} 改過"), "{name}");
        assert_eq!(details.fields.lyrics, "[00:01.00]歌詞", "{name}");
        assert_eq!(
            read_cover(&copy).unwrap().unwrap().0,
            original_cover,
            "{name}: cover changed"
        );
        assert_eq!(decode(&copy), before, "{name}: audio changed by a tag edit");

        // 2. remove the cover, then put a new one in
        let mut edit = TagEdit {
            fields: details.fields.clone(),
            remove_cover: true,
            ..TagEdit::default()
        };
        edit.fields.album = "沒有封面".to_owned();
        let details = write_edit(&copy, &edit).unwrap();
        assert!(details.cover.is_none(), "{name}: cover not removed");
        assert_eq!(decode(&copy), before, "{name}: audio changed");
        let mut edit = TagEdit {
            fields: details.fields.clone(),
            replace_cover: Some(original_cover.clone()),
            ..TagEdit::default()
        };
        edit.fields.album = "新封面".to_owned();
        let details = write_edit(&copy, &edit).unwrap();
        assert!(details.cover.is_some(), "{name}: new cover missing");
        assert_eq!(decode(&copy), before, "{name}: audio changed");
        println!(
            "{name}: edit keeps the audio{} and handles the cover",
            if two_file {
                " and the .wvc pairing"
            } else {
                ""
            }
        );
    }
    let _ = std::fs::remove_dir_all(work);
}

#[test]
#[ignore = "needs MOE_WV_MANIFEST"]
fn official_files_render_spectrograms() {
    for (name, wv, _, _) in rows().into_iter().filter(|(n, ..)| {
        [
            "cd16_lossless",
            "hires24_hybrid512_wvc",
            "float32_lossless",
            "cd16_hybrid96_dns",
        ]
        .contains(&n.as_str())
    }) {
        let started = std::time::Instant::now();
        let spectrogram =
            render_spectrogram(&wv, SpectrogramOptions::default(), &AtomicBool::new(false))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            spectrogram.png.starts_with(&[0x89, b'P', b'N', b'G']),
            "{name}"
        );
        println!(
            "{name}: spectrogram {} Hz, {} bytes PNG in {:.1}s",
            spectrogram.sample_rate,
            spectrogram.png.len(),
            started.elapsed().as_secs_f64()
        );
    }
}
