//! On-demand local lyric lookup for enabled Windows filesystem mappings.

use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::Path,
};

use lofty::{config::ParseOptions, prelude::TaggedFileExt, probe::Probe, tag::ItemKey};
use player_core::{parse_any_lyrics, LyricProvider, LyricsContent, MediaLocator, ParsedLyrics, MAX_LYRIC_PAYLOAD_BYTES};

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

/// Read one local source for a track (ADR 0041): the lyrics embedded in its tags first, then the
/// sidecar beside it (`.lrc`, `.ass`, `.srt`). An enabled content URI is ignored by this
/// Windows-only filesystem adapter.
pub fn find_lyrics(locators: &[MediaLocator]) -> LyricsLookup {
    let mut oversized = false;
    if let LyricsLookup::Found(found) = find_embedded_lyrics_flag(locators, &mut oversized) {
        return LyricsLookup::Found(found);
    }
    let paths = filesystem_paths(locators);
    for path in &paths {
        if let Some(lyrics) = sidecar_lyrics(path, &mut oversized) {
            return LyricsLookup::Found(FoundLyrics {
                provider: LyricProvider::Sidecar,
                lyrics,
            });
        }
    }
    for (source_id, path) in &remote_tracks(locators) {
        if let Some(lyrics) = remote_sidecar_lyrics(*source_id, path, &mut oversized) {
            return LyricsLookup::Found(FoundLyrics {
                provider: LyricProvider::Sidecar,
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

fn filesystem_paths(locators: &[MediaLocator]) -> Vec<&std::path::PathBuf> {
    locators
        .iter()
        .filter_map(|locator| match locator {
            MediaLocator::FileSystem(path) => Some(path),
            // Remote tracks are read separately, through the remote content provider.
            MediaLocator::ContentUri(_) | MediaLocator::Remote { .. } => None,
        })
        .collect()
}

fn find_embedded_lyrics_flag(locators: &[MediaLocator], oversized: &mut bool) -> LyricsLookup {
    for path in filesystem_paths(locators) {
        if let Some(lyrics) = embedded_lyrics(path, oversized) {
            return LyricsLookup::Found(FoundLyrics {
                provider: LyricProvider::Embedded,
                lyrics,
            });
        }
    }
    for (source_id, path) in &remote_tracks(locators) {
        if let Some(lyrics) = remote_embedded_lyrics(*source_id, path, oversized) {
            return LyricsLookup::Found(FoundLyrics {
                provider: LyricProvider::Embedded,
                lyrics,
            });
        }
    }
    LyricsLookup::Missing
}
/// The lyrics embedded in the tags of a track (local or remote), without looking for a `.lrc` beside it: for tracks whose
/// sidecars the library sync already recorded.
pub fn find_embedded_lyrics(locators: &[MediaLocator]) -> LyricsLookup {
    let mut oversized = false;
    match find_embedded_lyrics_flag(locators, &mut oversized) {
        LyricsLookup::Found(found) => LyricsLookup::Found(found),
        _ if oversized => LyricsLookup::Oversized,
        _ => LyricsLookup::Missing,
    }
}
/// Embedded and sidecar lyrics for the same track, when both exist, in the playback order
/// (embedded first, as [`find_lyrics`] uses it).
pub fn find_all_lyrics(locators: &[MediaLocator]) -> Vec<FoundLyrics> {
    let mut found = Vec::new();
    let mut oversized = false;
    if let LyricsLookup::Found(embedded) = find_embedded_lyrics_flag(locators, &mut oversized) {
        found.push(embedded);
    }
    let sidecar = filesystem_paths(locators)
        .into_iter()
        .find_map(|path| sidecar_lyrics(path, &mut oversized))
        .or_else(|| {
            remote_tracks(locators)
                .into_iter()
                .find_map(|(source_id, path)| remote_sidecar_lyrics(source_id, path, &mut oversized))
        });
    if let Some(lyrics) = sidecar {
        found.push(FoundLyrics {
            provider: LyricProvider::Sidecar,
            lyrics,
        });
    }
    found
}
/// The remote tracks among `locators`: the source and the path under its root.
fn remote_tracks(locators: &[MediaLocator]) -> Vec<(player_core::SourceId, &str)> {
    locators
        .iter()
        .filter_map(|locator| match locator {
            MediaLocator::Remote { source_id, path } => Some((*source_id, path.as_str())),
            _ => None,
        })
        .collect()
}

/// The sidecar beside a remote track: the first of `.lrc`, `.ass`, `.srt` that holds lyrics.
fn remote_sidecar_lyrics(
    source_id: player_core::SourceId,
    path: &str,
    oversized: &mut bool,
) -> Option<ParsedLyrics> {
    player_core::LYRIC_SIDECAR_EXTENSIONS.iter().find_map(|extension| {
        let sidecar = player_core::remote_with_extension(path, extension);
        let bytes =
            player_core::read_remote_file_capped(source_id, &sidecar, MAX_LYRIC_PAYLOAD_BYTES as u64);
        let Some(bytes) = bytes else {
            // A present but oversized sidecar is reported like a local one.
            if let Some(handle) =
                player_core::remote_content().and_then(|c| c.open(source_id, &sidecar).ok())
            {
                if handle.len > MAX_LYRIC_PAYLOAD_BYTES as u64 {
                    *oversized = true;
                }
            }
            return None;
        };
        parse_lyric_text(&bytes)
    })
}

/// A lyric text read by its content (ADR 0041); a UTF-8 BOM is dropped, broken subtitles are none.
fn parse_lyric_text(bytes: &[u8]) -> Option<ParsedLyrics> {
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)).ok()?;
    parse_any_lyrics(text).ok().and_then(LyricsContent::usable)
}
/// The lyrics embedded in a remote track's tags.
fn remote_embedded_lyrics(
    source_id: player_core::SourceId,
    path: &str,
    oversized: &mut bool,
) -> Option<ParsedLyrics> {
    let handle = player_core::remote_content()?.open(source_id, path).ok()?;
    let tagged_file = Probe::new(std::io::BufReader::new(handle.reader))
        .options(ParseOptions::new().read_properties(false))
        .guess_file_type()
        .ok()?
        .read()
        .ok()?;
    lyrics_in_tags(tagged_file.tags(), oversized)
}

fn sidecar_lyrics(audio_path: &Path, oversized: &mut bool) -> Option<ParsedLyrics> {
    player_core::LYRIC_SIDECAR_EXTENSIONS
        .iter()
        .find_map(|extension| sidecar_lyrics_with(audio_path, extension, oversized))
}

fn sidecar_lyrics_with(
    audio_path: &Path,
    extension: &str,
    oversized: &mut bool,
) -> Option<ParsedLyrics> {
    let audio_parent = audio_path.parent()?;
    let native_parent = to_extended_path(audio_parent).ok()?;
    let canonical_parent = fs::canonicalize(&native_parent).ok()?;
    let candidate = audio_path.with_extension(extension);
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
    parse_lyric_text(&bytes)
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
    lyrics_in_tags(tagged_file.tags(), oversized)
}

fn lyrics_in_tags(tags: &[lofty::tag::Tag], oversized: &mut bool) -> Option<ParsedLyrics> {
    for key in [ItemKey::Lyrics, ItemKey::UnsyncLyrics] {
        for raw in tags.iter().filter_map(|tag| tag.get_string(key)) {
            if raw.len() > MAX_LYRIC_PAYLOAD_BYTES {
                *oversized = true;
                continue;
            }
            if let Some(lyrics) = parse_lyric_text(raw.as_bytes()) {
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
        assert_eq!(found[0].provider, LyricProvider::Embedded);
        assert_eq!(found[0].lyrics.lines[0].text, "嵌入式歌詞");
        assert_eq!(found[1].provider, LyricProvider::Sidecar);
        assert_eq!(found[1].lyrics.lines[0].text, "同名歌詞");

        match find_lyrics(&only_path(audio)) {
            LyricsLookup::Found(found) => assert_eq!(found.provider, LyricProvider::Embedded),
            other => panic!("playback lookup puts embedded lyrics first, got {other:?}"),
        }
    }

    #[test]
    fn sidecars_are_tried_lrc_then_ass_then_srt_and_broken_ones_are_skipped() {
        let dir = TestDirectory::new();
        let audio = dir.path().join("song.mp3");
        fs::write(&audio, b"not audio").expect("placeholder");
        fs::write(dir.path().join("song.srt"), "1\n00:00:01,000 --> 00:00:02,000\nfrom srt\n").unwrap();
        let lookup = |audio: &Path| match find_lyrics(&only_path(audio.to_path_buf())) {
            LyricsLookup::Found(found) => {
                assert_eq!(found.provider, LyricProvider::Sidecar);
                found.lyrics.lines[0].text.clone()
            }
            other => panic!("{other:?}"),
        };
        assert_eq!(lookup(&audio), "from srt");
        fs::write(
            dir.path().join("song.ass"),
            "[Events]\nFormat: Start, End, Style, Text\nDialogue: 0:00:01.00,0:00:02.00,D,from ass\n",
        )
        .unwrap();
        assert_eq!(lookup(&audio), "from ass");
        // A sidecar that looks like a subtitle but does not parse is passed over.
        fs::write(dir.path().join("song.lrc"), "1\n00:00:xx --> broken\n").unwrap();
        assert_eq!(lookup(&audio), "from ass");
        fs::write(dir.path().join("song.lrc"), "[00:01.00]from lrc").unwrap();
        assert_eq!(lookup(&audio), "from lrc");
    }

    #[test]
    fn embedded_subtitle_text_that_does_not_parse_falls_through_to_the_sidecar() {
        let dir = TestDirectory::new();
        let audio = dir.path().join("broken.mp3");
        write_id3v2_unsynced_lyrics(&audio, "[Events]\nDialogue: garbage");
        fs::write(dir.path().join("broken.lrc"), "[00:01.00]sidecar").unwrap();
        match find_lyrics(&only_path(audio)) {
            LyricsLookup::Found(found) => assert_eq!(found.provider, LyricProvider::Sidecar),
            other => panic!("{other:?}"),
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

    fn wavpack_fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../player-wavpack/tests/fixtures")
            .join(name)
    }

    /// A WavPack file whose APEv2 `Lyrics` item was written by the official `wvtag` (ADR 0025).
    #[test]
    fn embedded_apev2_lyrics_of_a_wavpack_file_are_found_and_win_over_the_sidecar() {
        let dir = TestDirectory::new();
        let audio = dir.path().join("曲目.wv");
        fs::copy(wavpack_fixture("s16_lyrics.wv"), &audio).expect("copy the WavPack fixture");
        match find_lyrics(&only_path(audio.clone())) {
            LyricsLookup::Found(found) => {
                assert_eq!(found.provider, LyricProvider::Embedded);
                assert!(found.lyrics.synced);
                let texts: Vec<&str> = found
                    .lyrics
                    .lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect();
                assert_eq!(texts, ["第一行歌詞", "Second line", "第三行 ♪"]);
            }
            other => panic!("expected embedded lyrics, got {other:?}"),
        }
        fs::write(dir.path().join("曲目.lrc"), "[00:01.00]同名歌詞").expect("write LRC sidecar");
        let found = find_all_lyrics(&only_path(audio.clone()));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].provider, LyricProvider::Embedded);
        assert_eq!(found[1].provider, LyricProvider::Sidecar);
        match find_lyrics(&only_path(audio)) {
            LyricsLookup::Found(found) => assert_eq!(found.provider, LyricProvider::Embedded),
            other => panic!("playback lookup puts embedded lyrics first, got {other:?}"),
        }
    }

    /// What the song-details panel writes into a `.wv` is what the lyrics lookup reads back.
    #[test]
    fn lyrics_written_by_the_details_panel_into_a_wavpack_file_are_read_back() {
        let dir = TestDirectory::new();
        let audio = dir.path().join("edited.wv");
        fs::copy(wavpack_fixture("s16.wv"), &audio).expect("copy the WavPack fixture");
        assert_eq!(
            find_lyrics(&only_path(audio.clone())),
            LyricsLookup::Missing
        );

        let details = player_tags::read_details(&audio).expect("details");
        let mut edit = player_tags::TagEdit {
            fields: details.fields.clone(),
            ..player_tags::TagEdit::default()
        };
        edit.fields.lyrics = "[00:02.00]編輯後的歌詞\n[00:05.00]Another line".to_owned();
        player_tags::write_edit(&audio, &edit).expect("write the tag");
        match find_lyrics(&only_path(audio.clone())) {
            LyricsLookup::Found(found) => {
                assert_eq!(found.provider, LyricProvider::Embedded);
                assert_eq!(found.lyrics.lines[0].text, "編輯後的歌詞");
                assert_eq!(found.lyrics.lines.len(), 2);
            }
            other => panic!("expected the lyrics written by the panel, got {other:?}"),
        }

        // Clearing the field removes the lyrics again.
        let details = player_tags::read_details(&audio).expect("details");
        let mut edit = player_tags::TagEdit {
            fields: details.fields.clone(),
            ..player_tags::TagEdit::default()
        };
        edit.fields.lyrics.clear();
        player_tags::write_edit(&audio, &edit).expect("clear the lyrics");
        assert_eq!(find_lyrics(&only_path(audio)), LyricsLookup::Missing);
    }

    /// Whole songs from the official encoder, each with an LRC in its `Lyrics` item (`MOE_WV_MANIFEST`; run by hand).
    #[test]
    #[ignore = "needs MOE_WV_MANIFEST"]
    fn official_encoder_files_carry_readable_embedded_lyrics() {
        let manifest = fs::read_to_string(std::env::var("MOE_WV_MANIFEST").unwrap()).unwrap();
        let mut checked = 0;
        for line in manifest.lines().filter(|line| !line.trim().is_empty()) {
            let wv = PathBuf::from(line.split('\t').nth(1).unwrap().replace('/', "\\"));
            match find_lyrics(&only_path(wv.clone())) {
                LyricsLookup::Found(found) => {
                    assert_eq!(found.provider, LyricProvider::Embedded, "{wv:?}");
                    assert!(found.lyrics.synced, "{wv:?}");
                    assert_eq!(found.lyrics.lines.len(), 4, "{wv:?}");
                    assert_eq!(found.lyrics.lines[0].text, "第一行：官方編碼器", "{wv:?}");
                    assert_eq!(found.lyrics.lines[3].text, "Fourth line ♪", "{wv:?}");
                }
                other => panic!("{wv:?}: expected embedded lyrics, got {other:?}"),
            }
            checked += 1;
        }
        println!("{checked} official-encoder files with readable embedded lyrics");
        assert!(checked > 0);
    }
}
