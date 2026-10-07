//! On-demand local lyric lookup for enabled Windows filesystem mappings.

use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::Path,
};

use lofty::{config::ParseOptions, prelude::TaggedFileExt, probe::Probe, tag::ItemKey};
use player_core::{parse_lrc, LyricProvider, MediaLocator, ParsedLyrics, MAX_LYRIC_PAYLOAD_BYTES};

use crate::windows::to_extended_path;

const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoundLyrics {
    pub provider: LyricProvider,
    pub lyrics: ParsedLyrics,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LyricsLookup {
    Found(FoundLyrics),
    Missing,
    Oversized,
}

/// Read one local source for a track. LRC sidecars take precedence over embedded tags, and an
/// enabled content URI is ignored by this Windows-only filesystem adapter.
pub fn find_lyrics(locators: &[MediaLocator]) -> LyricsLookup {
    let paths = locators
        .iter()
        .filter_map(|locator| match locator {
            MediaLocator::FileSystem(path) => Some(path),
            MediaLocator::ContentUri(_) => None,
        })
        .collect::<Vec<_>>();

    let mut oversized = false;
    for path in &paths {
        if let Some(lyrics) = sidecar_lyrics(path, &mut oversized) {
            return LyricsLookup::Found(FoundLyrics {
                provider: LyricProvider::Sidecar,
                lyrics,
            });
        }
    }
    for path in &paths {
        if let Some(lyrics) = embedded_lyrics(path, &mut oversized) {
            return LyricsLookup::Found(FoundLyrics {
                provider: LyricProvider::Embedded,
                lyrics,
            });
        }
    }
    if oversized {
        LyricsLookup::Oversized
    } else {
        LyricsLookup::Missing
    }
}

/// Sidecar and embedded lyrics for the same track, when both exist. Playback lookup still uses
/// [`find_lyrics`], which returns only the sidecar when one is present.
pub fn find_all_lyrics(locators: &[MediaLocator]) -> Vec<FoundLyrics> {
    let paths = locators
        .iter()
        .filter_map(|locator| match locator {
            MediaLocator::FileSystem(path) => Some(path),
            MediaLocator::ContentUri(_) => None,
        })
        .collect::<Vec<_>>();

    let mut found = Vec::new();
    let mut oversized = false;
    let mut saw_sidecar = false;
    let mut saw_embedded = false;
    for path in &paths {
        if !saw_sidecar {
            if let Some(lyrics) = sidecar_lyrics(path, &mut oversized) {
                found.push(FoundLyrics {
                    provider: LyricProvider::Sidecar,
                    lyrics,
                });
                saw_sidecar = true;
            }
        }
        if !saw_embedded {
            if let Some(lyrics) = embedded_lyrics(path, &mut oversized) {
                found.push(FoundLyrics {
                    provider: LyricProvider::Embedded,
                    lyrics,
                });
                saw_embedded = true;
            }
        }
        if saw_sidecar && saw_embedded {
            break;
        }
    }
    found
}

fn sidecar_lyrics(audio_path: &Path, oversized: &mut bool) -> Option<ParsedLyrics> {
    let audio_parent = audio_path.parent()?;
    let native_parent = to_extended_path(audio_parent).ok()?;
    let canonical_parent = fs::canonicalize(&native_parent).ok()?;
    let candidate = audio_path.with_extension("lrc");
    let native_candidate = to_extended_path(&candidate).ok()?;
    let mut file = open_regular_file_without_following_reparse_points(&native_candidate)?;

    // Sidecars must stay beside their audio file. Reject symlinks and reparse points above, then
    // recheck the canonical parent to catch unexpected redirection between the two operations.
    if fs::canonicalize(&native_candidate).ok()?.parent() != Some(canonical_parent.as_path()) {
        return None;
    }
    let metadata = file.metadata().ok()?;
    if metadata.len() == 0 {
        return None;
    }
    if metadata.len() > MAX_LYRIC_PAYLOAD_BYTES as u64 {
        *oversized = true;
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref()
        .take(MAX_LYRIC_PAYLOAD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_LYRIC_PAYLOAD_BYTES {
        *oversized = true;
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?;
    parse_lrc(text)
        .ok()
        .filter(|lyrics| !lyrics.lines.is_empty())
}

fn embedded_lyrics(path: &Path, oversized: &mut bool) -> Option<ParsedLyrics> {
    let native_path = to_extended_path(path).ok()?;
    let file = open_regular_file_without_following_reparse_points(&native_path)?;
    let tagged_file = Probe::new(std::io::BufReader::new(file))
        .options(ParseOptions::new().read_properties(false))
        .guess_file_type()
        .ok()?
        .read()
        .ok()?;
    let tags = tagged_file.tags();

    for key in [ItemKey::Lyrics, ItemKey::UnsyncLyrics] {
        for raw in tags.iter().filter_map(|tag| tag.get_string(key)) {
            if raw.len() > MAX_LYRIC_PAYLOAD_BYTES {
                *oversized = true;
                continue;
            }
            if let Some(lyrics) = parse_lrc(raw)
                .ok()
                .filter(|lyrics| !lyrics.lines.is_empty())
            {
                return Some(lyrics);
            }
        }
    }
    None
}

fn open_regular_file_without_following_reparse_points(path: &Path) -> Option<File> {
    let initial_metadata = fs::symlink_metadata(path).ok()?;
    if !initial_metadata.is_file()
        || initial_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return None;
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let opened_metadata = file.metadata().ok()?;
    if !opened_metadata.is_file()
        || opened_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return None;
    }
    Some(file)
}

trait WindowsFileMetadataExt {
    fn file_attributes(&self) -> u32;
}

impl WindowsFileMetadataExt for std::fs::Metadata {
    fn file_attributes(&self) -> u32 {
        use std::os::windows::fs::MetadataExt;
        MetadataExt::file_attributes(self)
    }
}

trait WindowsOpenOptionsExt {
    fn custom_flags(&mut self, flags: u32) -> &mut Self;
}

impl WindowsOpenOptionsExt for OpenOptions {
    fn custom_flags(&mut self, flags: u32) -> &mut Self {
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptionsExt::custom_flags(self, flags)
    }
}

#[cfg(test)]
mod tests {
    use super::{find_all_lyrics, find_lyrics, LyricsLookup};
    use player_core::{LyricProvider, MediaLocator};
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("moe-lyrics-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn only_path(path: PathBuf) -> Vec<MediaLocator> {
        vec![MediaLocator::FileSystem(path)]
    }

    fn write_id3v2_unsynced_lyrics(path: &Path, lyrics: &str) {
        let mut content = vec![3, b'e', b'n', b'g', 0];
        content.extend(lyrics.as_bytes());
        content.push(0);
        let frame_size = u32::try_from(content.len()).expect("small fixture");
        let mut frame = b"USLT".to_vec();
        frame.extend([
            ((frame_size >> 21) & 0x7f) as u8,
            ((frame_size >> 14) & 0x7f) as u8,
            ((frame_size >> 7) & 0x7f) as u8,
            (frame_size & 0x7f) as u8,
            0,
            0,
        ]);
        frame.extend(content);
        let tag_size = u32::try_from(frame.len()).expect("small fixture");
        let mut tag = b"ID3\x04\0\0".to_vec();
        tag.extend([
            ((tag_size >> 21) & 0x7f) as u8,
            ((tag_size >> 14) & 0x7f) as u8,
            ((tag_size >> 7) & 0x7f) as u8,
            (tag_size & 0x7f) as u8,
        ]);
        tag.extend(frame);
        for _ in 0..3 {
            tag.extend([0xff, 0xfb, 0x90, 0x64]);
            tag.resize(tag.len() + 413, 0);
        }
        fs::write(path, tag).expect("write synthetic ID3 USLT frame");
    }

    #[test]
    fn local_lrc_sidecar_is_returned_without_reading_audio_tags() {
        let dir = TestDirectory::new();
        let audio = dir.path().join("曲目.mp3");
        fs::write(&audio, b"not audio, sidecar is enough").expect("write placeholder track");
        fs::write(
            dir.path().join("曲目.lrc"),
            "[00:01.25]第一行\n[00:02.50]第二行",
        )
        .expect("write LRC sidecar");

        match find_lyrics(&only_path(audio)) {
            LyricsLookup::Found(found) => {
                assert_eq!(found.provider, LyricProvider::Sidecar);
                assert!(found.lyrics.synced);
                assert_eq!(found.lyrics.lines[0].text, "第一行");
            }
            other => panic!("expected sidecar lyrics, got {other:?}"),
        }
    }

    #[test]
    fn sidecar_and_embedded_lyrics_are_both_listed() {
        let dir = TestDirectory::new();
        let audio = dir.path().join("both.mp3");
        write_id3v2_unsynced_lyrics(&audio, "[00:03.00]嵌入式歌詞");
        fs::write(dir.path().join("both.lrc"), "[00:01.00]同名歌詞").expect("write LRC sidecar");

        let found = find_all_lyrics(&only_path(audio.clone()));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].provider, LyricProvider::Sidecar);
        assert_eq!(found[0].lyrics.lines[0].text, "同名歌詞");
        assert_eq!(found[1].provider, LyricProvider::Embedded);
        assert_eq!(found[1].lyrics.lines[0].text, "嵌入式歌詞");

        match find_lyrics(&only_path(audio)) {
            LyricsLookup::Found(found) => assert_eq!(found.provider, LyricProvider::Sidecar),
            other => panic!("playback lookup should keep sidecar precedence, got {other:?}"),
        }
    }

    #[test]
    fn embedded_id3_unsynced_lyrics_are_read_only_on_demand() {
        let dir = TestDirectory::new();
        let audio = dir.path().join("embedded.mp3");
        write_id3v2_unsynced_lyrics(&audio, "[00:03.00]嵌入式歌詞");
        match find_lyrics(&only_path(audio)) {
            LyricsLookup::Found(found) => {
                assert_eq!(found.provider, LyricProvider::Embedded);
                assert!(found.lyrics.synced);
                assert_eq!(found.lyrics.lines[0].text, "嵌入式歌詞");
            }
            other => panic!("expected embedded lyrics, got {other:?}"),
        }
    }

    #[test]
    fn oversized_local_sidecar_is_reported_and_never_read_into_a_large_buffer() {
        let dir = TestDirectory::new();
        let audio = dir.path().join("track.mp3");
        fs::write(&audio, b"unused").expect("write placeholder track");
        let sidecar = dir.path().join("track.lrc");
        let file = fs::File::create(&sidecar).expect("create oversized LRC");
        file.set_len((player_core::MAX_LYRIC_PAYLOAD_BYTES + 1) as u64)
            .expect("sparsely grow LRC");

        assert_eq!(find_lyrics(&only_path(audio)), LyricsLookup::Oversized);
    }

    #[test]
    fn content_uri_is_not_treated_as_a_windows_file_path() {
        assert_eq!(
            find_lyrics(&[MediaLocator::ContentUri("content://media/1".into())]),
            LyricsLookup::Missing
        );
    }
}
