//! WavPack tag editing and spectrogram (ADR 0025): lofty reads and writes the APEv2 tag of a `.wv`; the audio must
//! decode bit-for-bit the same afterwards.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use player_tags::{read_details, render_spectrogram, write_edit, SpectrogramOptions, TagEdit};
use player_wavpack::WavPackReader;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../player-wavpack/tests/fixtures")
        .join(name)
}

fn temp_dir(label: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("moe-wv-tags-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn tiny_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::DynamicImage::new_rgb8(4, 4)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

fn decoded(path: &Path) -> Vec<f32> {
    let mut reader = WavPackReader::open(File::open(path).unwrap(), None).unwrap();
    let mut out = Vec::new();
    while reader.read_frames(&mut out).unwrap() > 0 {}
    out
}

#[test]
fn wavpack_details_are_editable_and_the_audio_survives_an_edit() {
    let directory = temp_dir("edit");
    let path = directory.join("track.wv");
    std::fs::copy(fixture("s16.wv"), &path).unwrap();
    let original = decoded(&path);

    let details = read_details(&path).unwrap();
    assert!(!details.read_only);
    assert_eq!(details.format, "WV");
    assert_eq!(details.properties.channels, Some(2));
    assert_eq!(details.properties.sample_rate, Some(44_100));

    let mut edit = TagEdit {
        fields: details.fields.clone(),
        ..TagEdit::default()
    };
    edit.fields.title = "WavPack 曲名".to_owned();
    edit.fields.artist = "WavPack 演出者".to_owned();
    edit.fields.album = "WavPack 專輯".to_owned();
    edit.fields.date = "2024-05-06".to_owned();
    edit.fields.lyrics = "[00:01.00]WavPack 歌詞".to_owned();
    edit.replace_cover = Some(tiny_png());
    let details = write_edit(&path, &edit).unwrap();
    assert_eq!(details.fields.title, "WavPack 曲名");
    assert_eq!(details.fields.artist, "WavPack 演出者");
    assert_eq!(details.fields.album, "WavPack 專輯");
    assert_eq!(details.fields.lyrics, "[00:01.00]WavPack 歌詞");
    assert!(details.cover.is_some());
    assert_eq!(
        decoded(&path),
        original,
        "the audio is unchanged by the tag edit"
    );

    // A later edit that leaves the cover alone keeps it.
    let mut edit = TagEdit {
        fields: details.fields.clone(),
        ..TagEdit::default()
    };
    edit.fields.album = "改過的專輯".to_owned();
    let details = write_edit(&path, &edit).unwrap();
    assert_eq!(details.fields.album, "改過的專輯");
    assert!(
        details.cover.is_some(),
        "the cover survives an edit of another field"
    );
    assert_eq!(decoded(&path), original);

    let mut edit = TagEdit {
        fields: details.fields.clone(),
        remove_cover: true,
        ..TagEdit::default()
    };
    edit.fields.title.clear();
    let details = write_edit(&path, &edit).unwrap();
    assert_eq!(details.fields.title, "");
    assert!(details.cover.is_none());
    assert_eq!(decoded(&path), original);
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn wavpack_spectrogram_shows_the_tone() {
    let options = SpectrogramOptions::default();
    let spectrogram =
        render_spectrogram(&fixture("s16.wv"), options, &AtomicBool::new(false)).unwrap();
    assert_eq!(spectrogram.sample_rate, 44_100);
    assert!(spectrogram.png.starts_with(&[0x89, b'P', b'N', b'G']));
}
