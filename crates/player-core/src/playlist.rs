use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{MediaLocator, TrackId};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlaylistId(Uuid);

impl PlaylistId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for PlaylistId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PlaylistId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// App-owned playlist. Imported file references stay attached to entries so an unindexed or
/// temporarily unavailable song can still be exported without changing its original reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    pub entries: Vec<PlaylistEntry>,
}

impl Playlist {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: PlaylistId::new(),
            name: name.into(),
            entries: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaylistEntry {
    /// An internal ID when this entry has been matched to the library. The native locator remains
    /// alongside it for lossless M3U export and for resolution after a later library sync.
    pub track_id: Option<TrackId>,
    pub locator: MediaLocator,
    /// Optional per-playlist title from `#EXTINF`; when present it takes display precedence.
    pub title: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct M3uExportOptions {
    /// When set, the playlist file must be saved directly in this directory and every local
    /// entry must be beneath it. The file will then contain paths relative to this common root.
    pub relative_root: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistSummary {
    pub id: PlaylistId,
    pub name: String,
    pub entry_count: u64,
}

/// IPC-safe playlist entry view. It intentionally has no native locator field.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntrySummary {
    pub position: u64,
    pub track_id: Option<TrackId>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    /// True when the track currently has at least one enabled source mapping. This does not
    /// probe file existence or platform permission; playback remains the final availability test.
    pub has_enabled_mapping: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistPage {
    pub items: Vec<PlaylistEntrySummary>,
    pub offset: u64,
    pub limit: u32,
    pub total_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlaylistError {
    InvalidUtf8 { byte_offset: usize },
    InvalidUtf16,
    UnsupportedM3u8Encoding,
    InvalidExtInf { line: usize },
    InvalidExtInfDuration { line: usize },
    InvalidPathEntry { line: usize },
    RelativeRootMismatch,
    EntryOutsideRelativeRoot { entry_index: usize },
    UnrepresentablePath { entry_index: usize },
    LineBreakInEntry { entry_index: usize },
    LineBreakInMetadata { entry_index: usize },
    LineBreakInPlaylistName,
}

impl fmt::Display for PlaylistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8 { byte_offset } => {
                write!(f, "playlist is not valid UTF-8 at byte {byte_offset}")
            }
            Self::InvalidUtf16 => f.write_str("playlist contains invalid BOM-marked UTF-16"),
            Self::UnsupportedM3u8Encoding => {
                f.write_str("M3U8 must be encoded as UTF-8, optionally with a UTF-8 BOM")
            }
            Self::InvalidExtInf { line } => {
                write!(f, "invalid #EXTINF record on line {line}")
            }
            Self::InvalidExtInfDuration { line } => {
                write!(f, "invalid #EXTINF duration on line {line}")
            }
            Self::InvalidPathEntry { line } => {
                write!(f, "invalid or empty path entry on line {line}")
            }
            Self::RelativeRootMismatch => f.write_str(
                "relative export requires the playlist file to be directly inside the selected common root",
            ),
            Self::EntryOutsideRelativeRoot { entry_index } => write!(
                f,
                "playlist entry {} is outside the selected relative export root",
                entry_index + 1
            ),
            Self::UnrepresentablePath { entry_index } => write!(
                f,
                "playlist entry {} contains a native path that cannot be represented as UTF-8",
                entry_index + 1
            ),
            Self::LineBreakInEntry { entry_index } => write!(
                f,
                "playlist entry {} contains a line break that M3U cannot represent",
                entry_index + 1
            ),
            Self::LineBreakInMetadata { entry_index } => write!(
                f,
                "playlist metadata for entry {} contains a line break",
                entry_index + 1
            ),
            Self::LineBreakInPlaylistName => {
                f.write_str("playlist name contains a line break that M3U cannot represent")
            }
        }
    }
}

impl Error for PlaylistError {}

/// Parse M3U bytes. UTF-8 is preferred; BOM-marked UTF-16 LE/BE is accepted for legacy M3U.
/// Relative paths are resolved against the directory containing `playlist_file`.
pub fn parse_m3u(bytes: &[u8], playlist_file: &Path) -> Result<Playlist, PlaylistError> {
    let text = decode_m3u(bytes)?;
    parse_m3u_text(&text, playlist_file)
}

/// Parse M3U8 bytes, requiring valid UTF-8 (a UTF-8 BOM is optional).
pub fn parse_m3u8(bytes: &[u8], playlist_file: &Path) -> Result<Playlist, PlaylistError> {
    let text = decode_m3u8(bytes)?;
    parse_m3u_text(&text, playlist_file)
}

/// Write standard extended M3U as UTF-8 without a BOM.
pub fn write_m3u8(
    playlist: &Playlist,
    playlist_file: &Path,
    options: &M3uExportOptions,
) -> Result<Vec<u8>, PlaylistError> {
    if contains_line_break(&playlist.name) {
        return Err(PlaylistError::LineBreakInPlaylistName);
    }

    if let Some(root) = &options.relative_root {
        if playlist_file.parent() != Some(root.as_path()) {
            return Err(PlaylistError::RelativeRootMismatch);
        }
    }

    let mut output = String::from("#EXTM3U\n");
    if !playlist.name.is_empty() {
        output.push_str("#PLAYLIST:");
        output.push_str(&playlist.name);
        output.push('\n');
    }

    for (entry_index, entry) in playlist.entries.iter().enumerate() {
        if entry.title.as_deref().is_some_and(contains_line_break) {
            return Err(PlaylistError::LineBreakInMetadata { entry_index });
        }

        let reference = match &entry.locator {
            MediaLocator::FileSystem(path) => {
                let path = if let Some(root) = &options.relative_root {
                    path.strip_prefix(root)
                        .map_err(|_| PlaylistError::EntryOutsideRelativeRoot { entry_index })?
                } else {
                    path.as_path()
                };
                path.to_str()
                    .ok_or(PlaylistError::UnrepresentablePath { entry_index })?
            }
            MediaLocator::ContentUri(uri) => uri.as_str(),
        };
        if reference.is_empty() || contains_line_break(reference) || reference.contains('\0') {
            return Err(PlaylistError::LineBreakInEntry { entry_index });
        }

        if entry.title.is_some() || entry.duration_ms.is_some() {
            output.push_str("#EXTINF:");
            output.push_str(
                &entry
                    .duration_ms
                    .map(format_duration)
                    .unwrap_or_else(|| "-1".to_owned()),
            );
            output.push(',');
            if let Some(title) = &entry.title {
                output.push_str(title);
            }
            output.push('\n');
        }

        // A relative filename beginning with '#' would otherwise be interpreted as a comment.
        if reference.starts_with('#') {
            output.push_str("./");
        }
        output.push_str(reference);
        output.push('\n');
    }

    Ok(output.into_bytes())
}

fn decode_m3u(bytes: &[u8]) -> Result<String, PlaylistError> {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16(&bytes[2..], true);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16(&bytes[2..], false);
    }
    decode_utf8(bytes)
}

fn decode_m3u8(bytes: &[u8]) -> Result<String, PlaylistError> {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(PlaylistError::UnsupportedM3u8Encoding);
    }
    decode_utf8(bytes)
}

fn decode_utf8(bytes: &[u8]) -> Result<String, PlaylistError> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8(bytes.to_vec()).map_err(|error| PlaylistError::InvalidUtf8 {
        byte_offset: error.utf8_error().valid_up_to(),
    })
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String, PlaylistError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(PlaylistError::InvalidUtf16);
    }
    let units = bytes
        .chunks_exact(2)
        .map(|pair| {
            let pair = [pair[0], pair[1]];
            if little_endian {
                u16::from_le_bytes(pair)
            } else {
                u16::from_be_bytes(pair)
            }
        })
        .collect::<Vec<_>>();
    String::from_utf16(&units).map_err(|_| PlaylistError::InvalidUtf16)
}

fn parse_m3u_text(text: &str, playlist_file: &Path) -> Result<Playlist, PlaylistError> {
    let fallback_name = playlist_file
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Imported playlist")
        .to_owned();
    let base_dir = playlist_file.parent().unwrap_or_else(|| Path::new("."));
    let mut playlist = Playlist::new(fallback_name);
    let mut pending_extinf = None;

    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix("#PLAYLIST:") {
            playlist.name = name.to_owned();
        } else if let Some(value) = line.strip_prefix("#EXTINF:") {
            let (duration, title) = value
                .split_once(',')
                .ok_or(PlaylistError::InvalidExtInf { line: line_number })?;
            let duration_ms = parse_duration(duration)
                .ok_or(PlaylistError::InvalidExtInfDuration { line: line_number })?;
            pending_extinf = Some((duration_ms, (!title.is_empty()).then(|| title.to_owned())));
        } else if line.starts_with('#') {
            continue;
        } else {
            if line.contains('\0') {
                return Err(PlaylistError::InvalidPathEntry { line: line_number });
            }
            let locator = parse_locator(line, base_dir);
            let (duration_ms, title) = pending_extinf.take().unwrap_or((None, None));
            playlist.entries.push(PlaylistEntry {
                track_id: None,
                locator,
                title,
                duration_ms,
            });
        }
    }

    Ok(playlist)
}

fn parse_locator(value: &str, base_dir: &Path) -> MediaLocator {
    if is_uri_reference(value) {
        return MediaLocator::ContentUri(value.to_owned());
    }

    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    };
    MediaLocator::FileSystem(path)
}

fn is_uri_reference(value: &str) -> bool {
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };
    if scheme.len() < 2
        || !scheme.as_bytes()[0].is_ascii_alphabetic()
        || !scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
    {
        return false;
    }
    true
}

fn parse_duration(value: &str) -> Option<Option<u64>> {
    if value == "-1" {
        return Some(None);
    }
    if value.is_empty() || value.starts_with('-') {
        return None;
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 3
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let seconds = whole.parse::<u64>().ok()?;
    let fraction_ms = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<u64>().ok()? * 100,
        2 => fraction.parse::<u64>().ok()? * 10,
        _ => fraction.parse::<u64>().ok()?,
    };
    let duration_ms = seconds.checked_mul(1000)?.checked_add(fraction_ms)?;
    Some(Some(duration_ms))
}

fn format_duration(duration_ms: u64) -> String {
    let seconds = duration_ms / 1000;
    let millis = duration_ms % 1000;
    if millis == 0 {
        return seconds.to_string();
    }
    let fraction = format!("{millis:03}");
    format!("{seconds}.{}", fraction.trim_end_matches('0'))
}

fn contains_line_break(value: &str) -> bool {
    value.contains('\n') || value.contains('\r')
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        parse_m3u, parse_m3u8, write_m3u8, M3uExportOptions, Playlist, PlaylistEntry, PlaylistError,
    };
    use crate::MediaLocator;

    #[test]
    fn m3u8_resolves_unicode_relative_path_and_preserves_extended_metadata() {
        let input = "#EXTM3U\r\n#PLAYLIST:精選 🎵\r\n#EXTINF:123.5,演奏者 - 曲目 01\r\n專輯 甲/曲目 🎧.flac\r\n";
        let file = PathBuf::from(r"C:\音樂\播放清單.m3u8");
        let playlist = parse_m3u8(input.as_bytes(), &file).expect("parse UTF-8 M3U8");

        assert_eq!(playlist.name, "精選 🎵");
        assert_eq!(playlist.entries.len(), 1);
        assert_eq!(
            playlist.entries[0].locator,
            MediaLocator::FileSystem(PathBuf::from(r"C:\音樂\專輯 甲\曲目 🎧.flac"))
        );
        assert_eq!(
            playlist.entries[0].title.as_deref(),
            Some("演奏者 - 曲目 01")
        );
        assert_eq!(playlist.entries[0].duration_ms, Some(123_500));
    }

    #[test]
    fn m3u_preserves_absolute_paths_and_uri_references() {
        let input = concat!(
            "#EXTM3U\n",
            "D:\\音樂\\絕對 路徑.flac\n",
            "content://media/external/audio/media/42\n",
        );
        let file = PathBuf::from(r"C:\lists\favorites.m3u");
        let playlist = parse_m3u(input.as_bytes(), &file).expect("parse M3U");

        assert_eq!(
            playlist.entries[0].locator,
            MediaLocator::FileSystem(PathBuf::from(r"D:\音樂\絕對 路徑.flac"))
        );
        assert_eq!(
            playlist.entries[1].locator,
            MediaLocator::ContentUri("content://media/external/audio/media/42".to_owned())
        );
    }

    #[test]
    fn m3u_accepts_bom_marked_utf16_but_m3u8_requires_utf8() {
        let source = "#EXTM3U\n相簿 🎵/曲目.flac\n";
        let mut utf16le = vec![0xFF, 0xFE];
        for unit in source.encode_utf16() {
            utf16le.extend_from_slice(&unit.to_le_bytes());
        }
        let parsed =
            parse_m3u(&utf16le, Path::new(r"C:\lists\x.m3u")).expect("parse BOM-marked UTF-16 M3U");
        assert_eq!(parsed.entries.len(), 1);

        assert!(matches!(
            parse_m3u8(&utf16le, Path::new(r"C:\lists\x.m3u8")),
            Err(PlaylistError::UnsupportedM3u8Encoding)
        ));
        assert!(matches!(
            parse_m3u8(&[0xFF, 0xFE], Path::new(r"C:\lists\x.m3u8")),
            Err(PlaylistError::UnsupportedM3u8Encoding)
        ));
        assert!(matches!(
            parse_m3u(&[0xFF, 0xFE, 0x00], Path::new(r"C:\lists\x.m3u")),
            Err(PlaylistError::InvalidUtf16)
        ));
    }

    #[test]
    fn writer_emits_utf8_and_relative_unicode_long_paths_that_round_trip() {
        let root = PathBuf::from(r"C:\音樂庫\專輯");
        let playlist_file = root.join("清單.m3u8");
        let long_name = format!("曲目 {}.flac", "長路徑🎧".repeat(24));
        let source = Playlist {
            id: crate::PlaylistId::new(),
            name: "我的清單 🎶".to_owned(),
            entries: vec![PlaylistEntry {
                track_id: None,
                locator: MediaLocator::FileSystem(root.join("日文 專輯").join(long_name)),
                title: Some("曲名, 使用者標題".to_owned()),
                duration_ms: Some(2012),
            }],
        };
        let bytes = write_m3u8(
            &source,
            &playlist_file,
            &M3uExportOptions {
                relative_root: Some(root.clone()),
            },
        )
        .expect("write M3U8");
        assert!(!bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        let text = std::str::from_utf8(&bytes).expect("UTF-8 output");
        assert!(text.contains("#PLAYLIST:我的清單 🎶"));
        assert!(text.contains("#EXTINF:2.012,曲名, 使用者標題"));
        assert!(text.contains("日文 專輯\\曲目 "));

        let parsed = parse_m3u8(&bytes, &playlist_file).expect("parse generated M3U8");
        assert_eq!(parsed.name, source.name);
        assert_eq!(parsed.entries[0].title, source.entries[0].title);
        assert_eq!(parsed.entries[0].duration_ms, source.entries[0].duration_ms);
        assert_eq!(parsed.entries[0].locator, source.entries[0].locator);
    }

    #[test]
    fn writer_rejects_wrong_root_outside_entry_and_metadata_injection() {
        let root = PathBuf::from(r"C:\Music");
        let file = root.join("list.m3u8");
        let mut playlist = Playlist::new("ok");
        playlist.entries.push(PlaylistEntry {
            track_id: None,
            locator: MediaLocator::FileSystem(PathBuf::from(r"D:\Elsewhere\song.flac")),
            title: Some("title".to_owned()),
            duration_ms: None,
        });
        let options = M3uExportOptions {
            relative_root: Some(root.clone()),
        };
        assert!(matches!(
            write_m3u8(
                &playlist,
                &PathBuf::from(r"C:\Music\sub\list.m3u8"),
                &options
            ),
            Err(PlaylistError::RelativeRootMismatch)
        ));
        assert!(matches!(
            write_m3u8(&playlist, &file, &options),
            Err(PlaylistError::EntryOutsideRelativeRoot { entry_index: 0 })
        ));

        playlist.entries[0].locator = MediaLocator::FileSystem(root.join("song.flac"));
        playlist.entries[0].title = Some("bad\ntitle".to_owned());
        assert!(matches!(
            write_m3u8(&playlist, &file, &options),
            Err(PlaylistError::LineBreakInMetadata { entry_index: 0 })
        ));
    }

    #[test]
    fn invalid_utf8_and_extinf_values_report_errors_instead_of_replacing_data() {
        assert!(matches!(
            parse_m3u8(&[0x23, 0xFF], Path::new("bad.m3u8")),
            Err(PlaylistError::InvalidUtf8 { byte_offset: 1 })
        ));
        assert!(matches!(
            parse_m3u8(b"#EXTINF:1.1234,title\nsong.flac\n", Path::new("bad.m3u8")),
            Err(PlaylistError::InvalidExtInfDuration { line: 1 })
        ));
        assert!(matches!(
            parse_m3u8(b"#EXTINF:123\nsong.flac\n", Path::new("bad.m3u8")),
            Err(PlaylistError::InvalidExtInf { line: 1 })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn writer_rejects_a_native_path_that_is_not_valid_unicode() {
        use std::os::windows::ffi::OsStringExt;

        let path = PathBuf::from(std::ffi::OsString::from_wide(&[
            b'C' as u16,
            b':' as u16,
            b'\\' as u16,
            0xD800,
        ]));
        let mut playlist = Playlist::new("test");
        playlist.entries.push(PlaylistEntry {
            track_id: None,
            locator: MediaLocator::FileSystem(path),
            title: None,
            duration_ms: None,
        });
        assert!(matches!(
            write_m3u8(
                &playlist,
                Path::new(r"C:\test\list.m3u8"),
                &M3uExportOptions::default()
            ),
            Err(PlaylistError::UnrepresentablePath { entry_index: 0 })
        ));
    }
}
