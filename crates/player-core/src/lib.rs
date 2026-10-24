pub mod artwork;
mod filesystem_key;
pub mod library;
pub mod lyric_formats;
pub mod lyric_roles;
pub mod lyrics;
pub mod playback_queue;
pub mod playback_statistics;
pub mod playlist;
pub mod profile;
pub mod sync;
pub mod track_start;
#[cfg(windows)]
mod windows_locator;

pub use artwork::{ArtworkImage, MAX_ARTWORK_BYTES, MAX_ARTWORK_DIMENSION, MAX_ARTWORK_PIXELS};
pub use filesystem_key::unix_path_from_key;
#[cfg(unix)]
pub use filesystem_key::{file_uri_for_path, unix_path_key};
pub mod remote;
pub use library::{
    display_track_title, locator_display_file_stem, parse_recording_date, parse_remote_locator_uri,
    read_lyric_sidecar, recording_date_year, remote_locator_uri, volume_root, FileFingerprint,
    LibraryRoot, ListTracksQuery, LyricSidecar, LyricSidecarRead, LyricSidecarState,
    LyricSidecarWrite, MediaIndex, MediaLocator, MediaScanProgress, MediaScanProgressUnit,
    MediaSourceError, MediaSourceErrorKind, MediaSourceKind, MediaTrackRecord, MetadataReader,
    Page, SourceId, SourceScan, SourceScanState, SyncCancellation, TrackField, TrackFieldFilter,
    TrackId, TrackIdentity, TrackListSort, TrackMetadata, TrackMetadataError, TrackSortDirection,
    TrackSortField, TrackSummary, UserMetadataField, TRACK_METADATA_VERSION,
};
pub use lyric_formats::{
    detect_lyric_format, lyric_sidecar_rank, parse_any_lyrics, parse_ass, parse_qrc, parse_srt,
    DetectedLyricFormat, LyricsContent, LYRIC_SIDECAR_EXTENSIONS,
};
pub use lyrics::{
    auto_lyric_candidate, merge_lrc_auxiliary, merge_lrc_auxiliary_nearest,
    merge_same_timestamp_lines, parse_lrc, parse_yrc, preserve_qrc, rank_lyric_candidates,
    restore_words_from_raw_karaoke, traditional_word_texts, with_raw_karaoke, yrc_matches_lrc,
    LyricAuxiliaryKind, LyricCandidate, LyricFormat, LyricLine, LyricMatchScore, LyricProvider,
    LyricWord, LyricsTrackMetadata, ParsedLyrics, TrackLyrics, AUTO_MATCH_MAX_DURATION_DIFF_MS,
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
    begin_load, begin_load_abandonable, end_load, is_remote_link_lost, is_remote_network_error, is_remote_unreachable,
    load_cancel, read_remote_file_capped, read_remote_file_capped_checked, remote_content,
    remote_sibling, remote_with_extension, running_load_superseded, set_remote_content,
    LoadCancel, LoadCounter, RemoteContent, RemoteFileHandle, RemoteStamp, RemoteStream,
    LOAD_SUPERSEDED, REMOTE_LINK_LOST, REMOTE_UNREACHABLE,
};

pub use track_start::{
    is_track_unplayable, natural_walk_cursors, NaturalWalk, StartFailure, TRACK_UNPLAYABLE,
};
