pub mod artwork;
pub mod library;
pub mod playback_queue;
pub mod playlist;
pub mod sync;
#[cfg(windows)]
mod windows_locator;

pub use artwork::{ArtworkImage, MAX_ARTWORK_BYTES, MAX_ARTWORK_DIMENSION, MAX_ARTWORK_PIXELS};
pub use library::{
    FileFingerprint, LibraryRoot, ListTracksQuery, MediaIndex, MediaLocator, MediaScanProgress,
    MediaScanProgressUnit, MediaSourceError, MediaSourceKind, MediaTrackRecord, Page, SourceId,
    SourceScan, SourceScanState, SyncCancellation, TrackId, TrackIdentity, TrackMetadata,
    TrackMetadataError, TrackSummary, UserMetadataField,
};
pub use playback_queue::{PlaybackQueue, QueueRepeatMode};
pub use playlist::{
    parse_m3u, parse_m3u8, write_m3u, write_m3u8, M3uExportOptions, Playlist, PlaylistEntry,
    PlaylistEntrySummary, PlaylistError, PlaylistId, PlaylistPage, PlaylistSummary,
};
pub use sync::{
    LibraryRepository, SyncApplyOutcome, SyncApplyRequest, SyncApplyStats, SyncEngine, SyncError,
    SyncProgress, SyncProgressOutcome, SyncProgressStage, SyncProgressUnit, SyncReport,
    TrackSyncState,
};
#[cfg(windows)]
pub use windows_locator::windows_locator_key;
