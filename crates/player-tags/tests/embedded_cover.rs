//! The embedded cover of a track is read by one function for every container and for local and remote files alike
//! (a local DSF showed no cover while the same file on a server did).

use std::io::Cursor;
use std::path::{Path, PathBuf};

use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{Accessor, Tag, TagExt, TagType};
use player_dsd::testing::build_dsf;
use player_tags::embedded::{read_embedded_tags, read_embedded_tags_of_path};
use player_tags::{write_edit, TagEdit};

fn temp_dir(label: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("moe-embedded-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

/// A noisy PNG of roughly `side * side * 3` bytes, so the tag is as large as a real album cover's.
fn noisy_png(side: u32) -> Vec<u8> {
    let mut state = 0x1234_5678_u32;
    let image = image::RgbImage::from_fn(side, side, |_, _| {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        image::Rgb([(state >> 8) as u8, (state >> 16) as u8, (state >> 24) as u8])
    });
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

fn write_dsf(path: &Path, cover: Option<&[u8]>) {
    let payload: Vec<u8> = (0..10_000_u32).map(|v| (v * 17 % 253) as u8).collect();
    let mut file = build_dsf(2_822_400, 80_000, 4096, true, &[payload.clone(), payload]);
    let offset = file.len() as u64;
    file[20..28].copy_from_slice(&offset.to_le_bytes());
    let mut tag = Tag::new(TagType::Id3v2);
    tag.set_title("カラフル".to_owned());
    tag.set_artist("ClariS".to_owned());
    tag.set_album("カラフル".to_owned());
    if let Some(cover) = cover {
        tag.push_picture(
            Picture::unchecked(cover.to_vec())
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Png)
                .build(),
        );
    }
    let mut bytes = Vec::new();
    tag.dump_to(&mut bytes, WriteOptions::default()).unwrap();
    file.extend_from_slice(&bytes);
    std::fs::write(path, file).unwrap();
}

/// What the local lookups read (the file) and what the remote ones read (any stream of the same bytes).
fn local_and_remote(path: &Path) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let local = read_embedded_tags_of_path(path)
        .and_then(|tags| tags.front_cover().map(|picture| picture.data().to_vec()));
    let stream = Cursor::new(std::fs::read(path).unwrap());
    let remote = read_embedded_tags(stream, path)
        .and_then(|tags| tags.front_cover().map(|picture| picture.data().to_vec()));
    (local, remote)
}

#[test]
fn a_local_dsf_cover_in_a_large_trailing_id3_tag_is_found_like_a_remote_one() {
    let dir = temp_dir("dsf");
    let path = dir.join("track.dsf");
    let cover = noisy_png(500);
    assert!(cover.len() > 600_000, "as large as a real cover tag");
    write_dsf(&path, Some(&cover));
    let (local, remote) = local_and_remote(&path);
    assert_eq!(local.as_deref(), Some(cover.as_slice()));
    assert_eq!(local, remote);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_dsf_without_a_cover_has_none_for_local_and_remote() {
    let dir = temp_dir("dsf-none");
    let path = dir.join("track.dsf");
    write_dsf(&path, None);
    assert_eq!(local_and_remote(&path), (None, None));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_wavpack_ape_cover_is_found_like_a_remote_one() {
    let dir = temp_dir("wv");
    let path = dir.join("track.wv");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../player-wavpack/tests/fixtures/s16.wv");
    std::fs::copy(fixture, &path).unwrap();
    assert_eq!(local_and_remote(&path), (None, None), "no cover yet");
    let cover = noisy_png(16);
    let mut edit = TagEdit::default();
    edit.fields.title = "wv".to_owned();
    edit.replace_cover = Some(cover);
    write_edit(&path, &edit).unwrap();
    let (local, remote) = local_and_remote(&path);
    assert!(local.is_some(), "the APEv2 cover");
    assert_eq!(local, remote);
    let _ = std::fs::remove_dir_all(dir);
}

/// The ID3v2 tag the user's real DSF carries (kept out of the repository): set `MOE_TEST_DSF_ID3` to its extracted bytes to run it.
#[test]
fn the_real_dsf_tag_has_a_cover_when_the_extracted_bytes_are_given() {
    let Ok(id3) = std::env::var("MOE_TEST_DSF_ID3") else {
        return;
    };
    let dir = temp_dir("real");
    let path = dir.join("real.dsf");
    let mut file = build_dsf(
        2_822_400,
        80_000,
        4096,
        true,
        &[vec![0x69; 10_000], vec![0x69; 10_000]],
    );
    let offset = file.len() as u64;
    file[20..28].copy_from_slice(&offset.to_le_bytes());
    file.extend_from_slice(&std::fs::read(id3).unwrap());
    std::fs::write(&path, file).unwrap();
    let (local, remote) = local_and_remote(&path);
    assert!(local.as_ref().is_some_and(|bytes| bytes.len() > 1000));
    assert_eq!(local.is_some(), remote.is_some());
    let _ = std::fs::remove_dir_all(dir);
}
