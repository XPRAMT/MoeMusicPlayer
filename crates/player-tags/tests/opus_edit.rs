//! Opus tag editing (ADR 0017): lofty reads and writes Ogg Opus (Vorbis comments), so the details panel's existing
//! flow applies; the file must still decode with the same length afterwards.

use std::fs::File;
use std::path::PathBuf;

use player_opus::testing::write_opus;
use player_opus::OpusReader;
use player_tags::{read_details, write_edit, TagEdit};

fn temp_dir(label: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("moe-opus-tags-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn tiny_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::DynamicImage::new_rgb8(4, 4)
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

fn decoded_frames(path: &std::path::Path) -> usize {
    let mut reader = OpusReader::open(File::open(path).unwrap()).unwrap();
    let mut out = Vec::new();
    let channels = usize::from(reader.info().channels);
    while reader.read_frames(&mut out).unwrap() > 0 {}
    out.len() / channels
}

#[test]
fn opus_details_are_editable_and_the_audio_survives_an_edit() {
    let directory = temp_dir("edit");
    let path = directory.join("track.opus");
    std::fs::write(&path, write_opus(48_000, 2, 1_000.0, 0, 0)).unwrap();

    let details = read_details(&path).unwrap();
    assert!(!details.read_only);
    assert_eq!(details.format, "OPUS");
    assert_eq!(details.properties.channels, Some(2));
    assert_eq!(details.fields.title, "", "the synthetic file carries an empty TITLE");
    assert_eq!(decoded_frames(&path), 48_000);

    let mut edit = TagEdit {
        fields: details.fields.clone(),
        ..TagEdit::default()
    };
    edit.fields.title = "Opus 曲名".to_owned();
    edit.fields.artist = "Opus 演出者".to_owned();
    edit.fields.album = "Opus 專輯".to_owned();
    edit.fields.date = "2024-05-06".to_owned();
    edit.fields.lyrics = "[00:01.00]Opus 歌詞".to_owned();
    edit.replace_cover = Some(tiny_png());
    let details = write_edit(&path, &edit).unwrap();
    assert_eq!(details.fields.title, "Opus 曲名");
    assert_eq!(details.fields.artist, "Opus 演出者");
    assert_eq!(details.fields.album, "Opus 專輯");
    assert_eq!(details.fields.lyrics, "[00:01.00]Opus 歌詞");
    assert!(details.cover.is_some());
    assert_eq!(decoded_frames(&path), 48_000, "the audio decodes to the same length after the tag edit");

    // Grow the tag much larger (the Ogg pages are rewritten) and clear a field again.
    let mut edit = TagEdit {
        fields: details.fields.clone(),
        remove_cover: true,
        ..TagEdit::default()
    };
    edit.fields.lyrics = "歌詞行\n".repeat(20_000);
    edit.fields.album = String::new();
    let details = write_edit(&path, &edit).unwrap();
    assert!(details.cover.is_none());
    assert_eq!(details.fields.album, "");
    assert_eq!(details.fields.title, "Opus 曲名");
    assert_eq!(decoded_frames(&path), 48_000);
    assert!(!directory.join(".moetmp-track.opus").exists());
    let _ = std::fs::remove_dir_all(directory);
}
