//! Embedded lyrics are read by one function for every container and for local and remote files alike. A DSF keeps them
//! in its trailing ID3v2 tag (lofty does not know the container), WavPack in APEv2 items.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use lofty::config::WriteOptions;
use lofty::tag::{ItemKey, Tag, TagExt, TagType};
use player_dsd::testing::build_dsf;
use player_tags::embedded::{read_embedded_lyric_texts, read_embedded_tags_of_path};

fn temp_dir(label: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "moe-embedded-lyrics-{label}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn local_and_remote(path: &Path) -> (Vec<String>, Vec<String>) {
    let local = read_embedded_tags_of_path(path)
        .map(|tags| tags.lyric_texts())
        .unwrap_or_default();
    let remote = read_embedded_lyric_texts(Cursor::new(std::fs::read(path).unwrap()), path);
    (local, remote)
}

#[test]
fn lyrics_written_into_a_dsf_are_read_back_locally_and_remotely() {
    let dir = temp_dir("dsf");
    let path = dir.join("track.dsf");
    let payload: Vec<u8> = (0..10_000_u32).map(|v| (v * 17 % 253) as u8).collect();
    let mut file = build_dsf(2_822_400, 80_000, 4096, true, &[payload.clone(), payload]);
    let offset = file.len() as u64;
    file[20..28].copy_from_slice(&offset.to_le_bytes());
    let mut tag = Tag::new(TagType::Id3v2);
    // The details panel writes lyrics as USLT for an ID3v2 tag.
    tag.insert_text(ItemKey::UnsyncLyrics, "[00:01.00]DSD 歌詞".to_owned());
    let mut bytes = Vec::new();
    tag.dump_to(&mut bytes, WriteOptions::default()).unwrap();
    file.extend_from_slice(&bytes);
    std::fs::write(&path, file).unwrap();

    let (local, remote) = local_and_remote(&path);
    assert_eq!(local, ["[00:01.00]DSD 歌詞"]);
    assert_eq!(local, remote);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn wavpack_apev2_lyrics_are_read_locally_and_remotely() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../player-wavpack/tests/fixtures/s16_lyrics.wv");
    let (local, remote) = local_and_remote(&fixture);
    assert!(!local.is_empty(), "the Lyrics item of the APEv2 tag");
    assert_eq!(local, remote);
}
