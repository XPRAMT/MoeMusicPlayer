use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use serde::{Deserialize, Serialize};

/// Increment when the metadata parser adds or changes persisted fields or extraction rules.
/// Existing tracks below this version are eligible for bounded background backfill.
pub const TRACK_METADATA_VERSION: u32 = 6;
use uuid::Uuid;

/// Normalize a recording date to `YYYY`, `YYYY-MM`, or `YYYY-MM-DD`.
/// Accepts one- or two-digit month/day components; rejects invalid dates and suffixes.
pub fn parse_recording_date(value: &str) -> Option<String> {
    let mut parts = value.trim().split('-');
    let year = parse_four_digit_year(parts.next()?.as_bytes())?;
    let Some(month_text) = parts.next() else {
        return Some(format!("{year:04}"));
    };
    let month = parse_one_or_two_digits(month_text.as_bytes())?;
    if !(1..=12).contains(&month) {
        return None;
    }
    let Some(day_text) = parts.next() else {
        return parts
            .next()
            .is_none()
            .then(|| format!("{year:04}-{month:02}"));
    };
    if parts.next().is_some() {
        return None;
    }
    let day = parse_one_or_two_digits(day_text.as_bytes())?;
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => return None,
    };
    (1..=days_in_month)
        .contains(&day)
        .then(|| format!("{year:04}-{month:02}-{day:02}"))
}

/// Calendar year from a normalized release date (`YYYY…`).
pub fn recording_date_year(release_date: &str) -> Option<u16> {
    parse_four_digit_year(release_date.as_bytes().get(..4)?)
}

fn parse_four_digit_year(bytes: &[u8]) -> Option<u16> {
    if bytes.len() != 4 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let year = std::str::from_utf8(bytes).ok()?.parse::<u16>().ok()?;
    (year != 0).then_some(year)
}

fn parse_one_or_two_digits(bytes: &[u8]) -> Option<u8> {
    if !(1..=2).contains(&bytes.len()) || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

/// Stable identity owned by the application, independent of any OS media index.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TrackId(Uuid);

impl TrackId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for TrackId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TrackId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Identity of one configured library source. Created by the app when a root is added.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(Uuid);

impl SourceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for SourceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaSourceKind {
    WindowsSystemIndex,
    WindowsFilesystem,
    PlaylistFile,
    /// Legacy Android MediaStore volume source; migrated to `AndroidFilesystem` (ADR 0014).
    AndroidMediaStore,
    /// Legacy Android SAF document-tree source; migrated to `AndroidFilesystem` (ADR 0014).
    AndroidSaf,
    /// Android folder scanned through its real path under All files access (ADR 0014).
    AndroidFilesystem,
    /// A folder on an SMB or WebDAV server (ADR 0026).
    Remote,
    Other,
}

/// Runtime locator. Filesystem paths stay native `PathBuf`s; adapters must not convert them
/// through lossy UTF-8. A persisted Windows path is encoded separately as UTF-16 bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MediaLocator {
    FileSystem(PathBuf),
    ContentUri(String),
    /// A file on a remote server (ADR 0026): the source it belongs to and its path relative to that source's root,
    /// `/`-separated and NFC-normalized. The server address and the credentials are not part of the locator.
    Remote {
        source_id: SourceId,
        path: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LibraryRoot {
    pub id: SourceId,
    pub kind: MediaSourceKind,
    pub display_name: String,
    pub locator: MediaLocator,
    pub enabled: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TrackIdentity {
    /// App-owned library root ID. It is stable across launches and supplied to the adapter.
    pub source_id: SourceId,
    /// Stable item ID within this adapter/root, such as a MediaStore row ID or document ID.
    pub source_item_id: String,
    /// Optional adapter-normalized key used only to correlate the same item across adapters.
    /// This is not a display path and is not a replacement for the native locator.
    pub locator_key: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileFingerprint {
    pub size_bytes: u64,
    /// UTC Unix milliseconds. `None` means this source did not provide a reliable timestamp.
    pub modified_at_utc_ms: Option<i64>,
    /// Filesystem creation time in UTC Unix milliseconds, when the source exposes it.
    /// It is presentation/sort metadata and does not affect whether audio tags need rereading.
    pub created_at_utc_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration_ms: Option<u64>,
    pub codec: Option<String>,
    pub bitrate_bps: Option<u32>,
    pub sample_rate_hz: Option<u32>,
    /// Normalized `YYYY`, `YYYY-MM`, or `YYYY-MM-DD` from `RecordingDate`.
    pub release_date: Option<String>,
    /// Calendar year derived from `release_date` (kept for older consumers).
    pub year: Option<u16>,
    /// Source bits per sample, populated only for codecs known to be lossless.
    pub bit_depth: Option<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaTrackRecord {
    pub identity: TrackIdentity,
    pub locator: MediaLocator,
    pub fingerprint: FileFingerprint,
    /// Adapter-provided metadata, when available without opening/parsing the media file.
    pub metadata: Option<TrackMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceScanState {
    /// The source traversal completed; only this state allows absent-item reconciliation.
    Complete,
    /// Some items may be missing from this result; observed items can be merged, no deletion.
    Incomplete { reason: String },
    /// The source cannot currently be queried; all previous mappings must be retained.
    Unavailable { reason: String },
    /// User permission is missing or revoked; all previous mappings must be retained.
    PermissionRevoked { reason: String },
}

impl SourceScanState {
    pub fn allows_reconciliation(&self) -> bool {
        matches!(self, Self::Complete)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaSourceError {
    pub source_item_id: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceScan {
    pub source_id: SourceId,
    pub state: SourceScanState,
    pub tracks: Vec<MediaTrackRecord>,
    pub errors: Vec<MediaSourceError>,
}

/// Cooperative cancellation shared by the sync engine and a platform scanner.
#[derive(Clone, Default)]
pub struct SyncCancellation(Arc<AtomicBool>);

impl SyncCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaScanProgressUnit {
    /// Number of filesystem directory entries visited. The final total is not known in advance.
    FilesystemEntries,
    /// Number of source rows returned by a media provider.
    Tracks,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaScanProgress {
    pub processed: u64,
    pub total: Option<u64>,
    pub unit: MediaScanProgressUnit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackMetadataError {
    pub message: String,
}

/// Platform boundary: enumerate stable source records, then parse metadata on demand.
pub trait MediaIndex {
    fn scan(&mut self, root: &LibraryRoot) -> SourceScan;

    /// Enumerate a source while reporting optional progress. Adapters with a blocking or
    /// incremental API should override this method and check cancellation between work units.
    fn scan_with_progress(
        &mut self,
        root: &LibraryRoot,
        progress: &mut dyn FnMut(MediaScanProgress),
        cancellation: &SyncCancellation,
    ) -> SourceScan {
        if cancellation.is_cancelled() {
            return SourceScan {
                source_id: root.id,
                state: SourceScanState::Incomplete {
                    reason: "scan cancelled before enumeration".to_owned(),
                },
                tracks: Vec::new(),
                errors: Vec::new(),
            };
        }

        let mut scan = self.scan(root);
        if cancellation.is_cancelled() && scan.state.allows_reconciliation() {
            scan.state = SourceScanState::Incomplete {
                reason: "scan cancelled before a complete result was available".to_owned(),
            };
        }
        if !matches!(
            scan.state,
            SourceScanState::Unavailable { .. } | SourceScanState::PermissionRevoked { .. }
        ) {
            let tracks = scan.tracks.len() as u64;
            progress(MediaScanProgress {
                processed: tracks,
                total: Some(tracks),
                unit: MediaScanProgressUnit::Tracks,
            });
        }
        scan
    }

    fn read_metadata(
        &mut self,
        track: &MediaTrackRecord,
    ) -> Result<TrackMetadata, TrackMetadataError>;

    /// A metadata reader that may run on several threads at once, for adapters whose file parsing keeps no exclusive
    /// state. The sync engine reads the tracks that need parsing with it on `METADATA_READ_THREADS` threads; adapters
    /// without one (None) are read one track at a time through `read_metadata`.
    fn parallel_metadata_reader(&self) -> Option<&dyn MetadataReader> {
        None
    }
}

/// Parses one track's metadata; shared between threads (see `MediaIndex::parallel_metadata_reader`).
pub trait MetadataReader: Sync {
    fn read(&self, track: &MediaTrackRecord) -> Result<TrackMetadata, TrackMetadataError>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackSortField {
    #[default]
    Title,
    Artist,
    Album,
    Year,
    CreatedAt,
    AudioFormat,
    Duration,
    PlayCount,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackSortDirection {
    #[default]
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackListSort {
    #[serde(default)]
    pub field: TrackSortField,
    #[serde(default)]
    pub direction: TrackSortDirection,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTracksQuery {
    pub offset: u64,
    pub limit: u32,
    pub query: Option<String>,
    #[serde(default)]
    pub field_filter: Option<TrackFieldFilter>,
    #[serde(default)]
    pub sort: TrackListSort,
}

/// A binary filter on one user-facing metadata field.
/// Title and album match the whole value. Artist matches that whole value or one
/// name inside it. Multiple names are separated by `|`, `\`, `/`, `;`, `,`, or a space.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackFieldFilter {
    pub field: TrackField,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackField {
    Title,
    Artist,
    Album,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub offset: u64,
    pub limit: u32,
    pub total_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackSummary {
    pub id: TrackId,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration_ms: Option<u64>,
    pub codec: Option<String>,
    pub bitrate_bps: Option<u32>,
    pub sample_rate_hz: Option<u32>,
    /// Normalized `YYYY`, `YYYY-MM`, or `YYYY-MM-DD` when known.
    #[serde(default)]
    pub release_date: Option<String>,
    pub year: Option<u16>,
    /// Creation time of the earliest enabled filesystem mapping, in UTC Unix milliseconds.
    pub created_at_utc_ms: Option<i64>,
    pub bit_depth: Option<u8>,
    /// Cumulative credited listening time from `track_playback_statistics`.
    pub played_ms: u64,
    /// File stem used when `title` is missing. Never a directory, full path, or URI query.
    #[serde(default)]
    pub file_name: Option<String>,
}

/// Title shown in lists and playback chrome. A blank tag is missing; artist and album are not used.
pub fn display_track_title(title: Option<&str>, file_name: Option<&str>) -> Option<String> {
    non_blank(title)
        .or_else(|| non_blank(file_name))
        .map(str::to_owned)
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

/// File stem for a library or playlist locator. Content URIs contribute only a decoded file name.
pub fn locator_display_file_stem(locator: &MediaLocator) -> Option<String> {
    match locator {
        MediaLocator::FileSystem(path) => file_stem_from_path(path),
        MediaLocator::ContentUri(uri) => file_stem_from_content_uri(uri),
        MediaLocator::Remote { path, .. } => {
            let name = path.rsplit('/').find(|part| !part.trim().is_empty())?;
            let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            non_blank(Some(stem)).map(str::to_owned)
        }
    }
}

/// The text form of a remote locator, used where a locator has to be a line of text (an exported playlist):
/// `moeremote://<source id>/<percent-encoded path>`. The server address and the credentials are never in it.
pub fn remote_locator_uri(source_id: SourceId, path: &str) -> String {
    let mut out = format!("{REMOTE_URI_SCHEME}{source_id}/");
    for (index, segment) in path.split('/').enumerate() {
        if index > 0 {
            out.push('/');
        }
        for byte in segment.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                out.push(char::from(byte));
            } else {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

/// The inverse of [`remote_locator_uri`]; `None` for anything that is not one.
pub fn parse_remote_locator_uri(value: &str) -> Option<MediaLocator> {
    let rest = value.strip_prefix(REMOTE_URI_SCHEME)?;
    let (id, encoded) = rest.split_once('/')?;
    let source_id = SourceId::parse(id).ok()?;
    let path = percent_decode(encoded)?;
    Some(MediaLocator::Remote { source_id, path })
}

const REMOTE_URI_SCHEME: &str = "moeremote://";

fn file_stem_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    non_blank(Some(stem.as_ref())).map(str::to_owned)
}

fn file_stem_from_content_uri(uri: &str) -> Option<String> {
    let without_fragment = uri.split_once('#').map(|(head, _)| head).unwrap_or(uri);
    let path = without_fragment
        .split_once('?')
        .map(|(head, _)| head)
        .unwrap_or(without_fragment);
    let segment = path.rsplit('/').find(|part| !part.is_empty())?;
    let decoded = percent_decode(segment)?;
    if decoded.contains("://") || decoded.contains('?') || decoded.contains('#') {
        return None;
    }
    let name = decoded
        .rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())?
        .trim();
    let name = match name.split_once(':') {
        Some((volume, rest))
            if !volume.is_empty()
                && !volume.contains('.')
                && volume
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
                && !rest.contains(':') =>
        {
            rest.trim()
        }
        _ => name,
    };
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', '?', '#', '&', '=', ':'])
        || name.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    file_stem_from_path(Path::new(name))
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[index + 1..index + 3]) {
                if let Ok(value) = u8::from_str_radix(hex, 16) {
                    decoded.push(value);
                    index += 3;
                    continue;
                }
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(decoded).ok()
}

/// Fields which the user is allowed to override independently of source metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserMetadataField {
    Title,
    Artist,
    Album,
    AlbumArtist,
}

#[cfg(test)]
mod tests {
    use super::{
        display_track_title, locator_display_file_stem, parse_recording_date, recording_date_year,
        MediaLocator,
    };

    #[test]
    fn missing_title_uses_file_stem_and_ignores_other_tags() {
        assert_eq!(
            display_track_title(Some("  晴天  "), Some("ignored")).as_deref(),
            Some("晴天")
        );
        assert_eq!(
            display_track_title(Some("   "), Some("曲 目")).as_deref(),
            Some("曲 目")
        );
        assert_eq!(display_track_title(None, Some("  ")), None);
        assert_eq!(display_track_title(Some(""), None), None);
    }

    #[test]
    fn locator_file_stem_keeps_only_the_file_name() {
        assert_eq!(
            locator_display_file_stem(&MediaLocator::ContentUri(
                "content://com.android.externalstorage.documents/document/primary%3AMusic%2F%E6%9B%B2%20%E7%9B%AE.flac?displayName=secret.mp3#frag".into()
            ))
            .as_deref(),
            Some("曲 目")
        );
        assert_eq!(
            locator_display_file_stem(&MediaLocator::ContentUri(
                "content://com.android.externalstorage.documents/document/primary%3Asong.flac"
                    .into()
            ))
            .as_deref(),
            Some("song")
        );
        assert_eq!(
            locator_display_file_stem(&MediaLocator::ContentUri(
                "content://media/external/audio/media/42?title=Nope".into()
            )),
            None
        );
    }

    #[test]
    fn recording_date_keeps_available_precision() {
        assert_eq!(parse_recording_date("2024").as_deref(), Some("2024"));
        assert_eq!(parse_recording_date("2024-03").as_deref(), Some("2024-03"));
        assert_eq!(parse_recording_date("2024-3").as_deref(), Some("2024-03"));
        assert_eq!(
            parse_recording_date("2024-02-29").as_deref(),
            Some("2024-02-29")
        );
        assert_eq!(
            parse_recording_date("2024-2-9").as_deref(),
            Some("2024-02-09")
        );
        assert_eq!(
            parse_recording_date("2024-02-9").as_deref(),
            Some("2024-02-09")
        );
        assert_eq!(parse_recording_date("2023-02-29"), None);
        assert_eq!(parse_recording_date("2024-13"), None);
        assert_eq!(parse_recording_date("2024-02-30"), None);
        assert_eq!(parse_recording_date("0000"), None);
        assert_eq!(parse_recording_date("2024-01-01T00:00:00"), None);
        assert_eq!(recording_date_year("2024-03-15"), Some(2024));
    }

    #[cfg(windows)]
    #[test]
    fn windows_extended_path_file_stem_drops_the_prefix_and_directory() {
        assert_eq!(
            locator_display_file_stem(&MediaLocator::FileSystem(std::path::PathBuf::from(
                r"D:\Music\資料夾\曲 目.live.flac"
            )))
            .as_deref(),
            Some("曲 目.live")
        );
        assert_eq!(
            locator_display_file_stem(&MediaLocator::FileSystem(std::path::PathBuf::from(
                r"\\?\D:\Music\資料夾\曲 目.flac"
            )))
            .as_deref(),
            Some("曲 目")
        );
        assert_eq!(
            locator_display_file_stem(&MediaLocator::FileSystem(std::path::PathBuf::from(
                r"\\?\UNC\server\share\音樂 東京\track.flac"
            )))
            .as_deref(),
            Some("track")
        );
    }
}
