pub mod artwork;
pub mod library;
pub mod lyrics;
pub mod playback_queue;
pub mod playback_statistics;
pub mod playlist;
pub mod sync;
#[cfg(windows)]
mod windows_locator;

pub use artwork::{ArtworkImage, MAX_ARTWORK_BYTES, MAX_ARTWORK_DIMENSION, MAX_ARTWORK_PIXELS};
pub use library::{
    FileFingerprint, LibraryRoot, ListTracksQuery, MediaIndex, MediaLocator, MediaScanProgress,
    MediaScanProgressUnit, MediaSourceError, MediaSourceKind, MediaTrackRecord, Page, SourceId,
    SourceScan, SourceScanState, SyncCancellation, TrackId, TrackIdentity, TrackMetadata,
    TrackMetadataError, TrackSummary, UserMetadataField, TRACK_METADATA_VERSION,
};
pub use lyrics::{
    auto_lyric_candidate, explain_lyric_candidate_match, merge_lrc_auxiliary, parse_lrc, parse_yrc,
    preserve_qrc, rank_lyric_candidates, with_raw_karaoke, LyricAuxiliaryKind, LyricCandidate,
    LyricFormat, LyricLine, LyricMatchScore, LyricProvider, LyricsTrackMetadata, ParsedLyrics,
    TrackLyrics, MAX_LYRIC_PAYLOAD_BYTES,
};
pub use playback_queue::{
    PlaybackQueue, PlaybackQueueContext, PlaybackQueueEntry, PlaybackQueueSnapshot,
    QueueRepeatMode, QueueTrackListeningStats,
};
pub use playback_statistics::{PlaybackCheckpoint, PlaybackStatistics};
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
