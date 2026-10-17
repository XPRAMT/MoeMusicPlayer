//! DSF tag editing (ADR 0013 D10, revised): the audio is untouched, the trailing ID3v2.4 tag is rewritten.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use lofty::config::WriteOptions;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt, TagType};
use player_dsd::dsf::DsfReader;
use player_dsd::stream::DsdStream;
use player_dsd::testing::build_dsf;
use player_tags::{read_details, write_edit, TagEdit};

fn temp_dir(label: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("moe-tags-{label}-{}", std::process::id()));
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

/// A stereo DSD64 file with a trailing ID3v2 tag (title, artist, album).
fn write_tagged_dsf(path: &Path) {
    let payload: Vec<u8> = (0..10_000_u32)
        .map(|value| (value * 17 % 253) as u8)
        .collect();
    let mut file = build_dsf(2_822_400, 80_000, 4096, true, &[payload.clone(), payload]);
    let offset = file.len() as u64;
    file[20..28].copy_from_slice(&offset.to_le_bytes());
    let mut tag = Tag::new(TagType::Id3v2);
    tag.set_title("原曲名".to_owned());
    tag.set_artist("原演出者".to_owned());
    tag.set_album("原專輯".to_owned());
    let mut bytes = Vec::new();
    tag.dump_to(&mut bytes, WriteOptions::default()).unwrap();
    file.extend_from_slice(&bytes);
    std::fs::write(path, file).unwrap();
}

fn audio_bytes(path: &Path) -> Vec<Vec<u8>> {
    let mut reader = DsfReader::open(BufReader::new(File::open(path).unwrap())).unwrap();
    let mut all = vec![Vec::new(); usize::from(reader.info().channels)];
    let mut buffers = vec![Vec::new(); all.len()];
    while reader.read(4096, &mut buffers).unwrap() > 0 {
        for (target, buffer) in all.iter_mut().zip(&buffers) {
            target.extend_from_slice(buffer);
        }
    }
    all
}

#[test]
fn dsf_details_are_editable_and_report_dsd_properties() {
    let directory = temp_dir("details");
    let path = directory.join("track.dsf");
    write_tagged_dsf(&path);
    let details = read_details(&path).unwrap();
    assert!(!details.read_only, "DSF must be editable");
    assert_eq!(details.format, "DSD64");
    assert_eq!(details.tag_format, "ID3v2");
    assert_eq!(details.fields.title, "原曲名");
    assert_eq!(details.fields.artist, "原演出者");
    assert_eq!(details.properties.sample_rate, Some(2_822_400));
    assert_eq!(details.properties.bit_depth, Some(1));
    assert_eq!(details.properties.channels, Some(2));
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn editing_fields_and_cover_keeps_the_audio_and_the_file_playable() {
    let directory = temp_dir("edit");
    let path = directory.join("track.dsf");
    write_tagged_dsf(&path);
    let audio_before = audio_bytes(&path);

    let mut edit = TagEdit {
        fields: read_details(&path).unwrap().fields,
        ..TagEdit::default()
    };
    edit.fields.title = "新曲名".to_owned();
    edit.fields.album_artist = "新專輯演出者".to_owned();
    edit.fields.date = "2024".to_owned();
    edit.fields.lyrics = "[00:01.00]歌詞".to_owned();
    edit.replace_cover = Some(tiny_png());
    let details = write_edit(&path, &edit).unwrap();
    assert_eq!(details.fields.title, "新曲名");
    assert_eq!(details.fields.album_artist, "新專輯演出者");
    assert_eq!(details.fields.artist, "原演出者", "untouched fields stay");
    assert_eq!(details.fields.lyrics, "[00:01.00]歌詞");
    assert!(details.cover.is_some());
    assert_eq!(audio_bytes(&path), audio_before, "audio data is untouched");
    assert!(
        !directory.join(".moetmp-track.dsf").exists(),
        "temp file is cleaned up"
    );

    // Remove the cover and clear the album.
    let mut edit = TagEdit {
        fields: details.fields.clone(),
        remove_cover: true,
        ..TagEdit::default()
    };
    edit.fields.album = String::new();
    let details = write_edit(&path, &edit).unwrap();
    assert!(details.cover.is_none());
    assert_eq!(details.fields.album, "");
    assert_eq!(details.fields.title, "新曲名");
    assert_eq!(audio_bytes(&path), audio_before);

    // The file still opens with every field readable through the generic tag reader.
    let (_, tag) = player_dsd::id3::read_dsf(&path).unwrap();
    assert_eq!(tag.unwrap().get_string(ItemKey::TrackTitle), Some("新曲名"));
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn an_untagged_dsf_gets_a_tag_when_edited() {
    let directory = temp_dir("untagged");
    let path = directory.join("plain.dsf");
    std::fs::write(
        &path,
        build_dsf(2_822_400, 8_000, 4096, true, &[vec![0x69; 1000]]),
    )
    .unwrap();
    let mut edit = TagEdit::default();
    edit.fields.title = "有標籤了".to_owned();
    let details = write_edit(&path, &edit).unwrap();
    assert_eq!(details.fields.title, "有標籤了");
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn dff_stays_read_only() {
    let directory = temp_dir("dff");
    let path = directory.join("track.dff");
    std::fs::write(&path, b"FRM8").unwrap();
    let error = write_edit(&path, &TagEdit::default()).unwrap_err();
    assert!(error.contains("DFF"), "{error}");
    let details = read_details(&path).unwrap();
    assert!(details.read_only);
    let _ = std::fs::remove_dir_all(directory);
}
