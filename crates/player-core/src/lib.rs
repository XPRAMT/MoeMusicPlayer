pub mod artwork;
mod filesystem_key;
pub mod library;
pub mod lyrics;
pub mod playback_queue;
pub mod playback_statistics;
pub mod playlist;
pub mod sync;
#[cfg(windows)]
mod windows_locator;
mod zh_t2s;

pub use artwork::{ArtworkImage, MAX_ARTWORK_BYTES, MAX_ARTWORK_DIMENSION, MAX_ARTWORK_PIXELS};
pub use filesystem_key::unix_path_from_key;
#[cfg(unix)]
pub use filesystem_key::{file_uri_for_path, unix_path_key};
pub mod remote;
pub use library::{
    read_lyric_sidecar, LyricSidecar, LyricSidecarRead, LyricSidecarState, LyricSidecarWrite,
    display_track_title, locator_display_file_stem, parse_recording_date, parse_remote_locator_uri,
    recording_date_year, remote_locator_uri, FileFingerprint, LibraryRoot, ListTracksQuery,
    MediaIndex, MediaLocator, MediaScanProgress, MediaScanProgressUnit, MediaSourceError,
    MediaSourceKind, MediaTrackRecord, MetadataReader, Page, SourceId, SourceScan, SourceScanState,
    SyncCancellation, TrackField, TrackFieldFilter, TrackId, TrackIdentity, TrackListSort,
    TrackMetadata, TrackMetadataError, TrackSortDirection, TrackSortField, TrackSummary,
    UserMetadataField, TRACK_METADATA_VERSION,
};
pub use lyrics::{
    auto_lyric_candidate, explain_lyric_candidate_match, merge_lrc_auxiliary,
    merge_same_timestamp_lines, parse_lrc, parse_yrc, preserve_qrc, rank_lyric_candidates,
    with_raw_karaoke, LyricAuxiliaryKind, LyricCandidate, LyricFormat, LyricLine, LyricMatchScore,
    LyricProvider, LyricsTrackMetadata, ParsedLyrics, TrackLyrics, AUTO_MATCH_MAX_DURATION_DIFF_MS,
    AUTO_MATCH_TITLE_MIN_SIMILARITY, MAX_LYRIC_PAYLOAD_BYTES,
};
pub use playback_queue::{
    PlaybackQueue, PlaybackQueueContext, PlaybackQueueEntry, PlaybackQueueSnapshot,
    QueueRepeatMode, QueueTrackListeningStats,
};
pub use playback_statistics::{PlaybackCheckpoint, PlaybackStatistics};
pub use playlist::{
    parse_m3u, parse_m3u8, write_m3u, write_m3u8, M3uExportOptions, Playlist, PlaylistEntry,
    PlaylistEntrySummary, PlaylistError, PlaylistId, PlaylistListSort, PlaylistPage,
    PlaylistSortDirection, PlaylistSortField, PlaylistSummary,
};
pub use sync::{
    LibraryRepository, SyncApplyOutcome, SyncApplyRequest, SyncApplyStats, SyncEngine, SyncError,
    SyncProgress, SyncProgressOutcome, SyncProgressStage, SyncProgressUnit, SyncReport,
    TrackSyncState,
};
#[cfg(windows)]
pub use windows_locator::windows_locator_key;

pub use remote::{
    read_remote_file_capped, remote_content, remote_sibling, remote_with_extension,
    set_remote_content, RemoteContent, RemoteFileHandle, RemoteStamp, RemoteStream,
};
