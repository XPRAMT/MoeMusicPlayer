use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
    AndroidMediaStore,
    AndroidSaf,
    Other,
}

/// Runtime locator. Filesystem paths stay native `PathBuf`s; adapters must not convert them
/// through lossy UTF-8. A persisted Windows path is encoded separately as UTF-16 bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MediaLocator {
    FileSystem(PathBuf),
    ContentUri(String),
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
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTracksQuery {
    pub offset: u64,
    pub limit: u32,
    pub query: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub offset: u64,
    pub limit: u32,
    pub total_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
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
}

/// Fields which the user is allowed to override independently of source metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserMetadataField {
    Title,
    Artist,
    Album,
    AlbumArtist,
}
