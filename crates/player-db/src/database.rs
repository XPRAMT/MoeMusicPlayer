use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ferrous_opencc::{config::BuiltinConfig, OpenCC};
use icu_collator::{
    options::CollatorOptions,
    preferences::{CollationNumericOrdering, CollationType},
    Collator, CollatorBorrowed, CollatorPreferences,
};
use icu_locale::Locale;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use player_core::{
    FileFingerprint, LibraryRepository, LibraryRoot, ListTracksQuery, LyricCandidate,
    LyricProvider, MediaLocator, MediaSourceError, MediaSourceKind, MediaTrackRecord, Page,
    PlaybackCheckpoint, PlaybackQueueContext, PlaybackQueueEntry, PlaybackQueueSnapshot,
    PlaybackStatistics, Playlist, PlaylistEntry, PlaylistEntrySummary, PlaylistId,
    PlaylistListSort, PlaylistPage, PlaylistSortDirection, PlaylistSortField, PlaylistSummary,
    QueueRepeatMode, SourceId, SourceScanState, SyncApplyOutcome, SyncApplyRequest, SyncApplyStats,
    SyncCancellation, TrackField, TrackFieldFilter, TrackId, TrackIdentity, TrackListSort,
    TrackLyrics, TrackMetadata, TrackSortDirection, TrackSortField, TrackSummary, TrackSyncState,
    UserMetadataField, TRACK_METADATA_VERSION,
};
use rusqlite::{
    functions::FunctionFlags, params, Connection, OpenFlags, OptionalExtension, Transaction,
    TransactionBehavior,
};
use uuid::Uuid;

use crate::locator;

const SCHEMA_VERSION: i64 = 15;
const MUSIC_COLLATION_NAME: &str = "MUSIC_LOCALE_NATURAL";
const MUSIC_QUICK_INDEX_FUNCTION: &str = "music_quick_index_label";
const MUSIC_SPLIT_ARTISTS_FUNCTION: &str = "music_split_artists_json";
const MUSIC_TRIM_FUNCTION: &str = "music_trim_whitespace";
const MUSIC_IDENTITY_KEY_FUNCTION: &str = "music_identity_key";
const MUSIC_IDENTITY_TRADITIONAL_FUNCTION: &str = "music_identity_traditional";
const ICU4X_COLLATION_DATA_VERSION: &str = "icu_collator-2.3.1+icu_collator_data-2.3.0";
const MAX_PAGE_SIZE: u32 = 1000;
const MAX_TRACK_LYRICS_JSON_BYTES: usize = 6 * 1024 * 1024;
const MAX_AUTOMATIC_LYRICS_CACHE_BYTES: i64 = 64 * 1024 * 1024;
const MAX_LYRIC_CANDIDATE_JSON_BYTES: usize = 4 * 1024 * 1024;
const MAX_LYRIC_CANDIDATE_CACHE_BYTES: i64 = 16 * 1024 * 1024;

fn repair_provider_lrc_cache(provider: LyricProvider, lyrics: &mut player_core::ParsedLyrics) {
    if !matches!(provider, LyricProvider::NetEase | LyricProvider::Qq)
        || !matches!(
            lyrics.format,
            player_core::LyricFormat::Lrc | player_core::LyricFormat::Plain
        )
    {
        return;
    }
    if let Ok(mut reparsed) = player_core::parse_lrc(&lyrics.raw_text) {
        if reparsed.synced && reparsed.format == player_core::LyricFormat::Lrc {
            let mut auxiliary_by_cue = HashMap::with_capacity(lyrics.lines.len());
            for previous in &lyrics.lines {
                let cue = previous
                    .start_ms
                    .map(|start_ms| (start_ms, previous.text.clone()))
                    .or_else(|| {
                        let reparsed_line = player_core::parse_lrc(&previous.text).ok()?;
                        if !reparsed_line.synced || reparsed_line.lines.len() != 1 {
                            return None;
                        }
                        let line = &reparsed_line.lines[0];
                        Some((line.start_ms?, line.text.clone()))
                    });
                if let Some(cue) = cue {
                    auxiliary_by_cue.insert(
                        cue,
                        (previous.translation.clone(), previous.romanization.clone()),
                    );
                }
            }
            for line in &mut reparsed.lines {
                if let Some((translation, romanization)) = line
                    .start_ms
                    .and_then(|start_ms| auxiliary_by_cue.remove(&(start_ms, line.text.clone())))
                {
                    line.translation = translation;
                    line.romanization = romanization;
                }
            }
            reparsed.raw_karaoke = lyrics.raw_karaoke.clone();
            *lyrics = reparsed;
        }
    }
}

const COUNT_LIBRARY_SQL: &str = "SELECT COUNT(DISTINCT m.track_id) FROM source_mappings m
    JOIN library_roots r ON r.source_id=m.source_id AND r.enabled=1";
const COUNT_SEARCH_SQL: &str = "SELECT COUNT(DISTINCT m.track_id) FROM source_mappings m
    JOIN library_roots r ON r.source_id=m.source_id AND r.enabled=1
    JOIN tracks t ON t.track_id=m.track_id
    WHERE EXISTS (SELECT 1 FROM json_each(?1) term
       WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) LIKE '%' || term.value || '%' COLLATE NOCASE
          OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist) LIKE '%' || term.value || '%' COLLATE NOCASE
          OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) LIKE '%' || term.value || '%' COLLATE NOCASE
          OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist) LIKE '%' || term.value || '%' COLLATE NOCASE)";
const COUNT_FIELD_FILTER_SQL: &str = "SELECT COUNT(DISTINCT t.track_id) FROM tracks t
    WHERE EXISTS (SELECT 1 FROM source_mappings m JOIN library_roots r
                  ON r.source_id=m.source_id AND r.enabled=1 WHERE m.track_id=t.track_id)
      AND (?1 IS NULL OR EXISTS (SELECT 1 FROM json_each(?1) term
           WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) LIKE '%' || term.value || '%' COLLATE NOCASE
              OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist) LIKE '%' || term.value || '%' COLLATE NOCASE
              OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) LIKE '%' || term.value || '%' COLLATE NOCASE
              OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist) LIKE '%' || term.value || '%' COLLATE NOCASE))
      AND (?2 IS NULL OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) = ?2 COLLATE BINARY)
      AND (?3 IS NULL OR EXISTS (SELECT 1 FROM json_each(?3) requested_artist
          WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, '') = requested_artist.value COLLATE BINARY
             OR instr('|' || replace(replace(replace(replace(replace(replace(replace(replace(COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, ''), char(92), '|'), '/', '|'), ';', '|'), ',', '|'), '&', '|'), ' | ', '|'), '| ', '|'), ' |', '|') || '|', '|' || requested_artist.value || '|') > 0))
      AND (?4 IS NULL OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) = ?4 COLLATE BINARY)";
const TRACKS_PAGE_SELECT_SQL: &str = "SELECT t.track_id,
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist),
    t.track_number, t.disc_number, t.duration_ms, t.codec, t.bitrate_bps, t.sample_rate_hz,
    t.year, t.release_date, t.bit_depth,
    COALESCE((SELECT s.played_ms FROM track_playback_statistics s WHERE s.track_id=t.track_id), 0),
    (SELECT MIN(m.created_at_utc_ms) FROM source_mappings m JOIN library_roots r ON r.source_id=m.source_id WHERE m.track_id=t.track_id AND r.enabled=1)
 FROM tracks t
 WHERE EXISTS (SELECT 1 FROM source_mappings m JOIN library_roots r
               ON r.source_id=m.source_id AND r.enabled=1 WHERE m.track_id=t.track_id)
   AND (?1 IS NULL OR EXISTS (SELECT 1 FROM json_each(?1) term
        WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) LIKE '%' || term.value || '%' COLLATE NOCASE
           OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist) LIKE '%' || term.value || '%' COLLATE NOCASE
           OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) LIKE '%' || term.value || '%' COLLATE NOCASE
           OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist) LIKE '%' || term.value || '%' COLLATE NOCASE))
   AND (?4 IS NULL OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) = ?4 COLLATE BINARY)
   AND (?5 IS NULL OR EXISTS (SELECT 1 FROM json_each(?5) requested_artist
       WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, '') = requested_artist.value COLLATE BINARY
          OR instr('|' || replace(replace(replace(replace(replace(replace(replace(replace(COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, ''), char(92), '|'), '/', '|'), ';', '|'), ',', '|'), '&', '|'), ' | ', '|'), '| ', '|'), ' |', '|') || '|', '|' || requested_artist.value || '|') > 0))
   AND (?6 IS NULL OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) = ?6 COLLATE BINARY)";

/// Default title order retained for EXPLAIN QUERY PLAN coverage.
#[cfg_attr(not(test), allow(dead_code))]
const TRACKS_PAGE_SQL: &str = "SELECT t.track_id,
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist),
    t.track_number, t.disc_number, t.duration_ms, t.codec, t.bitrate_bps, t.sample_rate_hz,
    t.year, t.release_date, t.bit_depth,
    COALESCE((SELECT s.played_ms FROM track_playback_statistics s WHERE s.track_id=t.track_id), 0),
    (SELECT MIN(m.created_at_utc_ms) FROM source_mappings m JOIN library_roots r ON r.source_id=m.source_id WHERE m.track_id=t.track_id AND r.enabled=1)
 FROM tracks t
 WHERE EXISTS (SELECT 1 FROM source_mappings m JOIN library_roots r
               ON r.source_id=m.source_id AND r.enabled=1 WHERE m.track_id=t.track_id)
   AND (?1 IS NULL OR EXISTS (SELECT 1 FROM json_each(?1) term
        WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) LIKE '%' || term.value || '%' COLLATE NOCASE
           OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist) LIKE '%' || term.value || '%' COLLATE NOCASE
           OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) LIKE '%' || term.value || '%' COLLATE NOCASE
           OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist) LIKE '%' || term.value || '%' COLLATE NOCASE))
   AND (?4 IS NULL OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) = ?4 COLLATE BINARY)
   AND (CASE WHEN ?5 IS NULL THEN 1 WHEN COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, '') = ?5 COLLATE BINARY THEN 1 WHEN instr('|' || replace(replace(replace(replace(replace(replace(replace(COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, ''), char(92), '|'), '/', '|'), ';', '|'), ',', '|'), ' | ', '|'), '| ', '|'), ' |', '|') || '|', '|' || ?5 || '|') > 0 THEN 1 ELSE 0 END) = 1
   AND (?6 IS NULL OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) = ?6 COLLATE BINARY)
 ORDER BY t.sort_title COLLATE MUSIC_LOCALE_NATURAL, t.track_id
 LIMIT ?2 OFFSET ?3";
const TRACK_SUMMARY_SQL: &str = "SELECT t.track_id,
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist),
    t.track_number, t.disc_number, t.duration_ms, t.codec, t.bitrate_bps, t.sample_rate_hz,
    t.year, t.release_date, t.bit_depth,
    COALESCE((SELECT s.played_ms FROM track_playback_statistics s WHERE s.track_id=t.track_id), 0),
    (SELECT MIN(m.created_at_utc_ms) FROM source_mappings m JOIN library_roots r ON r.source_id=m.source_id WHERE m.track_id=t.track_id AND r.enabled=1)
 FROM tracks t WHERE t.track_id=?1";
const ARTIST_TRACKS_PREFIX_SQL: &str = "SELECT t.track_id,
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist),
    t.track_number, t.disc_number, t.duration_ms, t.codec, t.bitrate_bps, t.sample_rate_hz,
    t.year, t.release_date, t.bit_depth,
    COALESCE((SELECT s.played_ms FROM track_playback_statistics s WHERE s.track_id=t.track_id), 0),
    (SELECT MIN(m.created_at_utc_ms) FROM source_mappings m JOIN library_roots r ON r.source_id=m.source_id WHERE m.track_id=t.track_id AND r.enabled=1)
 FROM tracks t
 WHERE EXISTS (SELECT 1 FROM source_mappings m JOIN library_roots r
               ON r.source_id=m.source_id AND r.enabled=1 WHERE m.track_id=t.track_id)
   AND (?1 IS NULL OR EXISTS (SELECT 1 FROM json_each(?1) term
        WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) LIKE '%' || term.value || '%' COLLATE NOCASE
           OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist) LIKE '%' || term.value || '%' COLLATE NOCASE
           OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) LIKE '%' || term.value || '%' COLLATE NOCASE
           OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist) LIKE '%' || term.value || '%' COLLATE NOCASE))
   AND (
       EXISTS (SELECT 1 FROM json_each(?2) requested_artist
               WHERE trim(requested_artist.value)=''
                 AND NOT EXISTS (SELECT 1 FROM json_each(music_split_artists_json(
                     COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist)
                 )) parsed_artist WHERE trim(parsed_artist.value)<>''))
       OR EXISTS (SELECT 1 FROM json_each(?2) requested_artist
                  JOIN json_each(music_split_artists_json(
                      COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist)
                  )) artist_name
                    ON lower(trim(artist_name.value))=lower(trim(requested_artist.value))
                  WHERE trim(requested_artist.value)<>'')
   )";
const ALBUM_TRACKS_PAGE_SQL: &str = "SELECT t.track_id,
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist),
    t.track_number, t.disc_number, t.duration_ms, t.codec, t.bitrate_bps, t.sample_rate_hz,
    t.year, t.release_date, t.bit_depth,
    COALESCE((SELECT s.played_ms FROM track_playback_statistics s WHERE s.track_id=t.track_id), 0),
    (SELECT MIN(m.created_at_utc_ms) FROM source_mappings m JOIN library_roots r ON r.source_id=m.source_id WHERE m.track_id=t.track_id AND r.enabled=1)
 FROM tracks t
 WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, '') COLLATE BINARY
       IN (SELECT value FROM json_each(?1))
   AND EXISTS (SELECT 1 FROM source_mappings m JOIN library_roots r
               ON r.source_id=m.source_id AND r.enabled=1 WHERE m.track_id=t.track_id)";
const ALBUM_ARTWORK_CANDIDATES_SQL: &str = "SELECT t.track_id
 FROM tracks t
 WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, '') COLLATE BINARY
       IN (SELECT value FROM json_each(?1))
   AND EXISTS (SELECT 1 FROM source_mappings m JOIN library_roots r
               ON r.source_id=m.source_id AND r.enabled=1 WHERE m.track_id=t.track_id)
 ORDER BY COALESCE(t.disc_number, 2147483647), COALESCE(t.track_number, 2147483647),
          COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title, '') COLLATE MUSIC_LOCALE_NATURAL,
          t.track_id
 LIMIT ?2 OFFSET ?3";
const PLAYLIST_PAGE_SQL: &str = r#"
WITH resolved_entries AS (
    SELECT pe.position,
           CASE WHEN pe.track_id IS NOT NULL THEN pe.track_id
                ELSE (
                    SELECT CASE WHEN COUNT(DISTINCT sm.track_id)=1 THEN MIN(sm.track_id) END
                    FROM source_mappings sm
                    WHERE sm.locator_kind=pe.locator_kind
                      AND sm.locator_encoding=pe.locator_encoding
                      AND sm.locator_data=pe.locator_data
                )
           END AS track_id,
           pe.title AS entry_title,
           pe.duration_ms AS entry_duration_ms,
           pe.locator_kind,
           pe.locator_encoding,
           pe.locator_data
    FROM playlist_entries pe
    WHERE pe.playlist_id=?1
)
SELECT e.position, e.track_id,
       COALESCE(e.entry_title,
           (SELECT value FROM track_overrides WHERE track_id=e.track_id AND field='title'),
           t.title) AS title,
       COALESCE((SELECT value FROM track_overrides WHERE track_id=e.track_id AND field='artist'), t.artist) AS artist,
       COALESCE((SELECT value FROM track_overrides WHERE track_id=e.track_id AND field='album'), t.album) AS album,
       COALESCE(e.entry_duration_ms, t.duration_ms) AS duration_ms,
       t.codec, t.bitrate_bps, t.sample_rate_hz, t.year, t.release_date, t.bit_depth,
       EXISTS (
           SELECT 1 FROM source_mappings m
           JOIN library_roots r ON r.source_id=m.source_id
           WHERE m.track_id=e.track_id AND r.enabled=1
       ),
       COALESCE((SELECT s.played_ms FROM track_playback_statistics s WHERE s.track_id=e.track_id), 0),
       (SELECT MIN(m.created_at_utc_ms) FROM source_mappings m JOIN library_roots r ON r.source_id=m.source_id WHERE m.track_id=e.track_id AND r.enabled=1),
       e.locator_kind, e.locator_encoding, e.locator_data
FROM resolved_entries e
LEFT JOIN tracks t ON t.track_id=e.track_id
"#;

fn playlist_order_by(sort: PlaylistListSort, direction: &str) -> String {
    if sort.field == PlaylistSortField::AudioFormat {
        // Rank by the requested codec sequence, then source sample rate, then the
        // codec-appropriate source quality metric. Lossy bit depth is never consulted.
        return audio_format_order_by(direction, "e.position");
    }

    let expression = playlist_sort_expression(sort.field);
    if matches!(
        sort.field,
        PlaylistSortField::Title | PlaylistSortField::Artist | PlaylistSortField::Album
    ) {
        // Keep blank values in the same directional order as the ICU text groups:
        // last in ascending order and first when descending reverses the groups.
        format!("(length(trim({expression}, char(9)||char(10)||char(11)||char(12)||char(13)||' ')) = 0) {direction}, {expression} {direction}, e.position ASC")
    } else {
        format!("{expression} {direction}, e.position ASC")
    }
}

fn audio_format_order_by(direction: &str, tie_breaker: &str) -> String {
    // Keep one quality ordering for library and playlist views. Normalize older codec labels.
    format!(
        "CASE TRIM(UPPER(COALESCE(t.codec, ''))) \
         WHEN 'WAV' THEN 6 WHEN 'WAVE' THEN 6 WHEN 'FLAC' THEN 5 WHEN 'VORBIS' THEN 4 WHEN 'OGG VORBIS' THEN 4 \
         WHEN 'OPUS' THEN 3 WHEN 'MP4' THEN 2 WHEN 'M4A' THEN 2 WHEN 'AAC' THEN 2 WHEN 'MP4A' THEN 2 WHEN 'MP3' THEN 1 \
         ELSE CASE WHEN TRIM(UPPER(COALESCE(t.codec, ''))) LIKE 'DSD%' THEN 7 ELSE 0 END END {direction}, \
         COALESCE(t.sample_rate_hz, 0) {direction}, \
         CASE WHEN TRIM(UPPER(COALESCE(t.codec, ''))) IN ('WAV','WAVE','FLAC','AIFF','AIF','ALAC','APE','WV','WAVPACK','PCM','TAK','TTA') \
              THEN COALESCE(t.bit_depth, 0) ELSE COALESCE(t.bitrate_bps, 0) END {direction}, \
         {tie_breaker} ASC"
    )
}

fn playlist_sort_expression(field: PlaylistSortField) -> &'static str {
    match field {
        PlaylistSortField::Position => "e.position",
        PlaylistSortField::Title => "COALESCE(e.entry_title, (SELECT value FROM track_overrides WHERE track_id=e.track_id AND field='title'), t.title, '') COLLATE MUSIC_LOCALE_NATURAL",
        PlaylistSortField::Artist => "COALESCE((SELECT value FROM track_overrides WHERE track_id=e.track_id AND field='artist'), t.artist, '') COLLATE MUSIC_LOCALE_NATURAL",
        PlaylistSortField::Album => "COALESCE((SELECT value FROM track_overrides WHERE track_id=e.track_id AND field='album'), t.album, '') COLLATE MUSIC_LOCALE_NATURAL",
        PlaylistSortField::Year => "COALESCE(t.release_date, printf('%04d', t.year), '')",
        PlaylistSortField::CreatedAt => "(SELECT MIN(m.created_at_utc_ms) FROM source_mappings m JOIN library_roots r ON r.source_id=m.source_id WHERE m.track_id=e.track_id AND r.enabled=1)",
        PlaylistSortField::AudioFormat => unreachable!("audio format has a compound sort key"),
        PlaylistSortField::Duration => "COALESCE(e.entry_duration_ms, t.duration_ms, -1)",
        PlaylistSortField::PlayCount => "CASE WHEN COALESCE(e.entry_duration_ms, t.duration_ms, 0)>0 THEN COALESCE((SELECT s.played_ms FROM track_playback_statistics s WHERE s.track_id=e.track_id), 0)*1.0/COALESCE(e.entry_duration_ms, t.duration_ms) ELSE -1 END",
    }
}

const SCHEMA_V1: &str = r#"
CREATE TABLE library_roots (
    source_id TEXT PRIMARY KEY NOT NULL,
    source_kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    locator_kind TEXT NOT NULL,
    locator_encoding TEXT NOT NULL,
    locator_data BLOB NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))
);

CREATE TABLE tracks (
    track_id TEXT PRIMARY KEY NOT NULL,
    metadata_loaded INTEGER NOT NULL DEFAULT 0 CHECK (metadata_loaded IN (0, 1)),
    sort_title TEXT NOT NULL DEFAULT '',
    title TEXT,
    artist TEXT,
    album TEXT,
    album_artist TEXT,
    track_number INTEGER,
    disc_number INTEGER,
    duration_ms INTEGER,
    codec TEXT,
    bitrate_bps INTEGER,
    sample_rate_hz INTEGER
);
CREATE INDEX tracks_sort_title ON tracks(sort_title, track_id);

CREATE TABLE source_mappings (
    source_id TEXT NOT NULL REFERENCES library_roots(source_id),
    source_item_id TEXT NOT NULL,
    locator_key TEXT,
    locator_kind TEXT NOT NULL,
    locator_encoding TEXT NOT NULL,
    locator_data BLOB NOT NULL,
    size_bytes INTEGER NOT NULL,
    modified_at_utc_ms INTEGER,
    track_id TEXT NOT NULL REFERENCES tracks(track_id),
    PRIMARY KEY (source_id, source_item_id)
);
CREATE INDEX source_mappings_locator_key ON source_mappings(locator_key)
    WHERE locator_key IS NOT NULL;
CREATE INDEX source_mappings_track_id ON source_mappings(track_id);

CREATE TABLE track_overrides (
    track_id TEXT NOT NULL REFERENCES tracks(track_id),
    field TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (track_id, field)
);

CREATE TABLE library_sync_state (
    source_id TEXT PRIMARY KEY NOT NULL REFERENCES library_roots(source_id),
    last_attempt_utc_ms INTEGER,
    last_success_utc_ms INTEGER,
    last_state TEXT NOT NULL DEFAULT 'never',
    error_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT
);

CREATE TEMP TABLE IF NOT EXISTS sync_seen_items (
    source_id TEXT NOT NULL,
    source_item_id TEXT NOT NULL,
    PRIMARY KEY (source_id, source_item_id)
);
"#;

const SCHEMA_V2: &str = r#"
CREATE INDEX source_mappings_locator_exact
    ON source_mappings(locator_kind, locator_encoding, locator_data);

CREATE TABLE playlists (
    playlist_id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL
);

CREATE TABLE playlist_entries (
    playlist_id TEXT NOT NULL REFERENCES playlists(playlist_id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    track_id TEXT REFERENCES tracks(track_id),
    locator_kind TEXT NOT NULL,
    locator_encoding TEXT NOT NULL,
    locator_data BLOB NOT NULL,
    title TEXT,
    duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms >= 0),
    PRIMARY KEY (playlist_id, position)
);
CREATE INDEX playlist_entries_track_id ON playlist_entries(track_id);
"#;

const SCHEMA_V3: &str = r#"
CREATE TABLE theme_preferences (
    singleton_id INTEGER PRIMARY KEY NOT NULL CHECK (singleton_id = 1),
    background_hex TEXT NOT NULL CHECK (length(background_hex) = 7),
    accent_hex TEXT NOT NULL CHECK (length(accent_hex) = 7)
);
INSERT INTO theme_preferences (singleton_id, background_hex, accent_hex)
VALUES (1, '#000000', '#55D9FF');
"#;

const SCHEMA_V4: &str = r#"
CREATE INDEX playlist_entries_unmatched
    ON playlist_entries(playlist_id, position)
    WHERE track_id IS NULL;
"#;

const SCHEMA_V5: &str = r#"
CREATE TABLE playlist_file_sync_state (
    source_id TEXT PRIMARY KEY NOT NULL,
    playlist_id TEXT NOT NULL,
    locator_kind TEXT NOT NULL,
    locator_encoding TEXT NOT NULL,
    locator_data BLOB NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    modified_at_utc_ms INTEGER,
    content_sha256 BLOB NOT NULL CHECK (length(content_sha256) = 32)
);
"#;

const SCHEMA_V6: &str = r#"
ALTER TABLE tracks ADD COLUMN year INTEGER CHECK (year IS NULL OR (year >= 1 AND year <= 9999));
ALTER TABLE tracks ADD COLUMN bit_depth INTEGER CHECK (bit_depth IS NULL OR (bit_depth >= 1 AND bit_depth <= 64));
ALTER TABLE tracks ADD COLUMN metadata_version INTEGER NOT NULL DEFAULT 0 CHECK (metadata_version >= 0);
"#;

const SCHEMA_V7: &str = r#"
CREATE TABLE track_lyrics (
    track_id TEXT PRIMARY KEY NOT NULL REFERENCES tracks(track_id) ON DELETE CASCADE,
    lyrics_json TEXT NOT NULL,
    manually_selected INTEGER NOT NULL CHECK (manually_selected IN (0, 1)),
    updated_at_utc_ms INTEGER NOT NULL,
    payload_bytes INTEGER NOT NULL CHECK (payload_bytes >= 0)
);
CREATE INDEX track_lyrics_auto_cache_lru
    ON track_lyrics(manually_selected, updated_at_utc_ms);

CREATE TABLE lyric_candidates (
    track_id TEXT NOT NULL REFERENCES tracks(track_id) ON DELETE CASCADE,
    candidate_id TEXT NOT NULL,
    candidate_json TEXT NOT NULL,
    fetched_at_utc_ms INTEGER NOT NULL,
    payload_bytes INTEGER NOT NULL CHECK (payload_bytes >= 0),
    PRIMARY KEY (track_id, candidate_id)
);
CREATE INDEX lyric_candidates_cache_lru
    ON lyric_candidates(fetched_at_utc_ms, track_id, candidate_id);
"#;

const SCHEMA_V8: &str = r#"
CREATE TABLE playback_session (
    singleton_id INTEGER PRIMARY KEY NOT NULL CHECK (singleton_id = 1),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('library', 'playlist')),
    source_playlist_id TEXT,
    source_query TEXT,
    cursor INTEGER NOT NULL CHECK (cursor >= 0),
    position_ms INTEGER NOT NULL CHECK (position_ms >= 0),
    shuffle INTEGER NOT NULL CHECK (shuffle IN (0, 1)),
    repeat_mode TEXT NOT NULL CHECK (repeat_mode IN ('off', 'one', 'all')),
    random_state TEXT NOT NULL,
    entry_count INTEGER NOT NULL CHECK (entry_count > 0)
);
CREATE TABLE playback_session_entries (
    source_index INTEGER PRIMARY KEY NOT NULL CHECK (source_index >= 0),
    track_id TEXT NOT NULL,
    source_position INTEGER CHECK (source_position IS NULL OR source_position >= 0),
    traversal_order INTEGER NOT NULL UNIQUE CHECK (traversal_order >= 0)
);
"#;

const SCHEMA_V9: &str = r#"
CREATE TABLE track_playback_statistics (
    track_id TEXT PRIMARY KEY NOT NULL,
    played_ms INTEGER NOT NULL DEFAULT 0 CHECK (played_ms >= 0),
    observed_duration_ms INTEGER CHECK (observed_duration_ms IS NULL OR observed_duration_ms > 0)
);

CREATE TABLE playback_statistics_runtimes (
    runtime_id TEXT PRIMARY KEY NOT NULL,
    owner_pid INTEGER CHECK (owner_pid IS NULL OR owner_pid > 0),
    owner_process_started_utc_ms INTEGER CHECK (
        owner_process_started_utc_ms IS NULL OR owner_process_started_utc_ms >= 0
    ),
    registered_at_utc_ms INTEGER NOT NULL CHECK (registered_at_utc_ms >= 0)
);

CREATE TABLE playback_statistics_checkpoints (
    runtime_id TEXT NOT NULL REFERENCES playback_statistics_runtimes(runtime_id) ON DELETE CASCADE,
    track_id TEXT NOT NULL,
    played_ms INTEGER NOT NULL CHECK (played_ms >= 0),
    PRIMARY KEY (runtime_id, track_id)
);
"#;

const SCHEMA_V10: &str = "ALTER TABLE playback_session ADD COLUMN source_field_filter_json TEXT;";

const SCHEMA_V11: &str = r#"
ALTER TABLE tracks ADD COLUMN release_date TEXT;
UPDATE tracks
   SET release_date = printf('%04d', year)
 WHERE year IS NOT NULL
   AND (release_date IS NULL OR length(trim(release_date)) = 0);
"#;

const SCHEMA_V12: &str = r#"
DROP INDEX tracks_sort_title;
CREATE INDEX tracks_sort_title
    ON tracks(sort_title COLLATE NATURAL_NOCASE, track_id);
"#;

const SCHEMA_V13: &str = r#"
ALTER TABLE source_mappings ADD COLUMN created_at_utc_ms INTEGER;
"#;

const SCHEMA_V14: &str = r#"
ALTER TABLE playback_session ADD COLUMN queue_sort_json TEXT;
ALTER TABLE playback_session_entries ADD COLUMN shuffle_restore_order INTEGER
    CHECK (shuffle_restore_order IS NULL OR shuffle_restore_order >= 0);
"#;

const SCHEMA_V15: &str = r#"
DROP INDEX IF EXISTS tracks_sort_title;
CREATE TABLE music_collation_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    signature TEXT NOT NULL
);
"#;

const MUSIC_COLLATION_COMPARATOR_VERSION: u8 = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
struct MusicCollationSpec {
    locale: String,
    collation_type: Option<CollationType>,
}

impl MusicCollationSpec {
    fn system_default() -> Self {
        static SPEC: OnceLock<MusicCollationSpec> = OnceLock::new();
        SPEC.get_or_init(|| {
            let (locale, collation_type) = system_locale_and_collation();
            Self::from_locale_parts(&locale, collation_type)
        })
        .clone()
    }

    fn from_locale_parts(locale: &str, collation_type: Option<CollationType>) -> Self {
        let locale = Locale::try_from_str(locale)
            .unwrap_or(Locale::UNKNOWN)
            .to_string();
        Self {
            locale,
            collation_type,
        }
    }

    fn signature(&self) -> String {
        format!(
            "{ICU4X_COLLATION_DATA_VERSION}|{}|{}|numeric=true|strength=locale-default|comparator={MUSIC_COLLATION_COMPARATOR_VERSION}",
            self.locale,
            self.collation_type
                .map_or("default", |value| value.as_str()),
        )
    }

    fn from_signature(signature: &str) -> Option<Self> {
        let mut parts = signature.split('|');
        let _data_version = parts.next()?;
        let locale = parts.next()?;
        let collation_type = match parts.next()? {
            "default" => None,
            "zhuyin" => Some(CollationType::Zhuyin),
            "stroke" => Some(CollationType::Stroke),
            "pinyin" => Some(CollationType::Pinyin),
            "phonebk" => Some(CollationType::Phonebk),
            "phonetic" => Some(CollationType::Phonetic),
            "trad" => Some(CollationType::Trad),
            "eor" => Some(CollationType::Eor),
            _ => return None,
        };
        Some(Self::from_locale_parts(locale, collation_type))
    }

    fn build_collator(
        locale: &str,
        collation_type: Option<CollationType>,
    ) -> Arc<CollatorBorrowed<'static>> {
        let locale = Locale::try_from_str(locale).unwrap_or(Locale::UNKNOWN);
        let mut preferences = CollatorPreferences::from(&locale);
        preferences.numeric_ordering = Some(CollationNumericOrdering::True);
        preferences.collation_type = collation_type;
        Arc::new(
            Collator::try_new(preferences, CollatorOptions::default())
                .expect("compiled ICU4X collation data must be available"),
        )
    }

    fn uses_zhuyin(&self) -> bool {
        self.locale.split('-').take(2).eq(["zh", "TW"])
            || self.collation_type == Some(CollationType::Zhuyin)
    }

    fn uses_japanese_order(&self) -> bool {
        self.locale
            .split('-')
            .next()
            .is_some_and(|language| language.eq_ignore_ascii_case("ja"))
    }

    fn build_collators(&self) -> MusicCollators {
        let default = Self::build_collator(&self.locale, self.collation_type);
        let han = Self::build_collator(
            &self.locale,
            Some(if self.uses_zhuyin() {
                CollationType::Zhuyin
            } else {
                CollationType::Pinyin
            }),
        );
        let kana = Self::build_collator("ja", None);
        let mut boundaries = if self.uses_zhuyin() {
            zhuyin_group_boundaries()
        } else {
            pinyin_group_boundaries()
        };
        boundaries.sort_by(|left, right| han.compare(left.exemplar, right.exemplar));
        MusicCollators {
            default,
            han,
            kana,
            han_boundaries: boundaries,
            japanese_order: self.uses_japanese_order(),
        }
    }
}

#[derive(Clone, Copy)]
struct HanGroupBoundary {
    label: &'static str,
    exemplar: &'static str,
}

#[derive(Clone)]
struct MusicCollators {
    default: Arc<CollatorBorrowed<'static>>,
    han: Arc<CollatorBorrowed<'static>>,
    kana: Arc<CollatorBorrowed<'static>>,
    han_boundaries: Vec<HanGroupBoundary>,
    japanese_order: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LeadingKind {
    Symbol,
    Number,
    Latin,
    Han,
    Kana,
    Other,
    Unknown,
}

fn pinyin_group_boundaries() -> Vec<HanGroupBoundary> {
    [
        ("A", "阿"),
        ("B", "八"),
        ("C", "擦"),
        ("D", "大"),
        ("E", "饿"),
        ("F", "发"),
        ("G", "嘎"),
        ("H", "哈"),
        ("J", "家"),
        ("K", "卡"),
        ("L", "拉"),
        ("M", "妈"),
        ("N", "拿"),
        ("O", "哦"),
        ("P", "趴"),
        ("Q", "七"),
        ("R", "日"),
        ("S", "撒"),
        ("T", "他"),
        ("W", "挖"),
        ("X", "西"),
        ("Y", "呀"),
        ("Z", "咋"),
    ]
    .into_iter()
    .map(|(label, exemplar)| HanGroupBoundary { label, exemplar })
    .collect()
}

fn zhuyin_group_boundaries() -> Vec<HanGroupBoundary> {
    [
        ("ㄅ", "八"),
        ("ㄆ", "趴"),
        ("ㄇ", "媽"),
        ("ㄈ", "發"),
        ("ㄉ", "大"),
        ("ㄊ", "他"),
        ("ㄋ", "拿"),
        ("ㄌ", "拉"),
        ("ㄍ", "嘎"),
        ("ㄎ", "卡"),
        ("ㄏ", "哈"),
        ("ㄐ", "家"),
        ("ㄑ", "七"),
        ("ㄒ", "西"),
        ("ㄓ", "知"),
        ("ㄔ", "吃"),
        ("ㄕ", "詩"),
        ("ㄖ", "日"),
        ("ㄗ", "咋"),
        ("ㄘ", "擦"),
        ("ㄙ", "撒"),
        ("ㄚ", "阿"),
        ("ㄛ", "哦"),
        ("ㄜ", "餓"),
        ("ㄞ", "哀"),
        ("ㄟ", "欸"),
        ("ㄠ", "熬"),
        ("ㄡ", "歐"),
        ("ㄢ", "安"),
        ("ㄣ", "恩"),
        ("ㄤ", "昂"),
        ("ㄥ", "鞥"),
        ("ㄦ", "兒"),
        ("ㄧ", "一"),
        ("ㄨ", "烏"),
        ("ㄩ", "迂"),
    ]
    .into_iter()
    .map(|(label, exemplar)| HanGroupBoundary { label, exemplar })
    .collect()
}

#[cfg(target_os = "windows")]
fn system_locale_and_collation() -> (String, Option<CollationType>) {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;

    let mut buffer = [0_u16; 85];
    // GetUserDefaultLocaleName returns a null-terminated locale name or zero on failure.
    let length = unsafe { GetUserDefaultLocaleName(&mut buffer) };
    if length <= 1 {
        return ("und".to_owned(), None);
    }
    let locale = String::from_utf16_lossy(&buffer[..(length - 1) as usize]);
    let (locale, collation_type) = split_windows_sort_name(&locale);
    (locale, collation_type)
}

#[cfg(target_os = "windows")]
fn split_windows_sort_name(locale: &str) -> (String, Option<CollationType>) {
    let Some((base, sort_name)) = locale.rsplit_once('_') else {
        return (locale.replace('_', "-"), None);
    };
    let collation_type = match sort_name.to_ascii_lowercase().as_str() {
        "pronun" => Some(CollationType::Zhuyin),
        "stroke" => Some(CollationType::Stroke),
        "phoneb" | "phonebk" => Some(CollationType::Phonebk),
        "pinyin" => Some(CollationType::Pinyin),
        "phonetic" => Some(CollationType::Phonetic),
        "trad" => Some(CollationType::Trad),
        "eor" => Some(CollationType::Eor),
        _ => return (locale.replace('_', "-"), None),
    };
    (base.replace('_', "-"), collation_type)
}

#[cfg(target_os = "android")]
fn system_locale_and_collation() -> (String, Option<CollationType>) {
    use jni::{objects::JString, JavaVM};

    let detected = (|| -> Option<(String, String)> {
        let context = ndk_context::android_context();
        // ndk-context exposes the process-owned JavaVM handle initialized by the Tauri runtime.
        let vm = unsafe { JavaVM::from_raw(context.vm().cast()) }.ok()?;
        let mut env = vm.attach_current_thread().ok()?;
        let locale_class = env.find_class("java/util/Locale").ok()?;
        let locale = env
            .call_static_method(locale_class, "getDefault", "()Ljava/util/Locale;", &[])
            .ok()?
            .l()
            .ok()?;
        let tag_object = env
            .call_method(&locale, "toLanguageTag", "()Ljava/lang/String;", &[])
            .ok()?
            .l()
            .ok()?;
        let country_object = env
            .call_method(&locale, "getCountry", "()Ljava/lang/String;", &[])
            .ok()?
            .l()
            .ok()?;
        let tag: String = env.get_string(&JString::from(tag_object)).ok()?.into();
        let country: String = env.get_string(&JString::from(country_object)).ok()?.into();
        Some((tag, country))
    })();

    match detected {
        Some((locale, country)) if country.eq_ignore_ascii_case("TW") => {
            (locale, Some(CollationType::Zhuyin))
        }
        Some((locale, _)) => (locale, None),
        None => ("und".to_owned(), None),
    }
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
fn system_locale_and_collation() -> (String, Option<CollationType>) {
    let locale = ["LC_COLLATE", "LC_ALL", "LANG"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "und".to_owned());
    let locale = locale.split('.').next().unwrap_or("und").replace('_', "-");
    (locale, None)
}

fn leading_char(value: &str) -> Option<char> {
    value.trim().chars().next()
}

fn is_han(value: char) -> bool {
    matches!(value as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF |
        0x20000..=0x2A6DF | 0x2A700..=0x2B73F | 0x2B740..=0x2B81F |
        0x2B820..=0x2CEAF | 0x2CEB0..=0x2EBEF | 0x30000..=0x3134F |
        0x31350..=0x323AF)
}

fn is_kana(value: char) -> bool {
    matches!(value as u32, 0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9D)
}

fn classify_leading(value: Option<char>) -> LeadingKind {
    match value {
        None => LeadingKind::Unknown,
        Some(character) if character.is_numeric() => LeadingKind::Number,
        Some(character) if character.is_ascii_alphabetic() => LeadingKind::Latin,
        Some(character) if is_han(character) => LeadingKind::Han,
        Some(character) if is_kana(character) => LeadingKind::Kana,
        Some(character) if character.is_alphabetic() => LeadingKind::Other,
        Some(_) => LeadingKind::Symbol,
    }
}

fn kana_group_char(character: char) -> char {
    match character as u32 {
        0x30A1..=0x30F6 => char::from_u32(character as u32 - 0x60).unwrap_or(character),
        _ => character,
    }
}

fn han_group_index(collators: &MusicCollators, character: char) -> usize {
    // Japanese first-letter navigation groups all Han characters under one entry.
    // Their actual order within that group still comes from the ja-JP collator.
    if collators.japanese_order {
        return 0;
    }
    let value = character.to_string();
    let index = collators.han_boundaries.partition_point(|boundary| {
        collators.han.compare(&value, boundary.exemplar) != Ordering::Less
    });
    index.saturating_sub(1)
}

fn localized_rank(kind: LeadingKind, collators: &MusicCollators) -> u8 {
    match kind {
        LeadingKind::Symbol => 0,
        LeadingKind::Number => 1,
        LeadingKind::Latin => 2,
        LeadingKind::Kana if collators.japanese_order => 3,
        LeadingKind::Han if collators.japanese_order => 4,
        LeadingKind::Han => 3,
        LeadingKind::Kana => 4,
        LeadingKind::Other => 5,
        LeadingKind::Unknown => 6,
    }
}

fn quick_index_label(collators: &MusicCollators, value: &str) -> String {
    let Some(character) = leading_char(value) else {
        return "未知".to_owned();
    };
    match classify_leading(Some(character)) {
        LeadingKind::Symbol => "&".to_owned(),
        LeadingKind::Number => "#".to_owned(),
        LeadingKind::Latin => character.to_ascii_uppercase().to_string(),
        LeadingKind::Han if collators.japanese_order => "漢字".to_owned(),
        LeadingKind::Han => collators.han_boundaries[han_group_index(collators, character)]
            .label
            .to_owned(),
        LeadingKind::Kana => kana_group_char(character).to_string(),
        LeadingKind::Other => character.to_string(),
        LeadingKind::Unknown => "未知".to_owned(),
    }
}

fn compare_music_names(collators: &MusicCollators, left: &str, right: &str) -> Ordering {
    let left_char = leading_char(left);
    let right_char = leading_char(right);
    let left_kind = classify_leading(left_char);
    let right_kind = classify_leading(right_char);
    let category = localized_rank(left_kind, collators).cmp(&localized_rank(right_kind, collators));
    if category != Ordering::Equal {
        return category;
    }

    let leading_group = match (left_kind, left_char, right_kind, right_char) {
        (LeadingKind::Latin, Some(left), LeadingKind::Latin, Some(right)) => {
            left.to_ascii_uppercase().cmp(&right.to_ascii_uppercase())
        }
        (LeadingKind::Han, Some(left), LeadingKind::Han, Some(right)) => {
            han_group_index(collators, left).cmp(&han_group_index(collators, right))
        }
        (LeadingKind::Kana, Some(left), LeadingKind::Kana, Some(right)) => {
            let left = kana_group_char(left).to_string();
            let right = kana_group_char(right).to_string();
            collators.kana.compare(&left, &right)
        }
        (LeadingKind::Other, Some(left), LeadingKind::Other, Some(right)) => collators
            .default
            .compare(&left.to_string(), &right.to_string()),
        _ => Ordering::Equal,
    };
    if leading_group != Ordering::Equal {
        return leading_group;
    }

    let collator = match left_kind {
        LeadingKind::Han if collators.japanese_order => &collators.default,
        LeadingKind::Han => &collators.han,
        LeadingKind::Kana => &collators.kana,
        _ => &collators.default,
    };
    collator
        .compare(left.trim(), right.trim())
        // ICU numeric comparison handles arbitrary-length runs; this stable tie-break keeps
        // equivalent whitespace and numeric spellings deterministic.
        .then_with(|| natural_title_cmp(left, right))
}

pub fn natural_title_cmp(left: &str, right: &str) -> Ordering {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let (mut left_index, mut right_index) = (0, 0);

    while left_index < left.len() && right_index < right.len() {
        let left_is_digit = left[left_index].is_ascii_digit();
        let right_is_digit = right[right_index].is_ascii_digit();
        if left_is_digit && right_is_digit {
            let left_start = left_index;
            let right_start = right_index;
            while left_index < left.len() && left[left_index].is_ascii_digit() {
                left_index += 1;
            }
            while right_index < right.len() && right[right_index].is_ascii_digit() {
                right_index += 1;
            }

            let left_digits = &left[left_start..left_index];
            let right_digits = &right[right_start..right_index];
            let left_significant = left_digits
                .iter()
                .position(|digit| *digit != b'0')
                .map_or(&left_digits[left_digits.len()..], |offset| {
                    &left_digits[offset..]
                });
            let right_significant = right_digits
                .iter()
                .position(|digit| *digit != b'0')
                .map_or(&right_digits[right_digits.len()..], |offset| {
                    &right_digits[offset..]
                });
            let number_order = left_significant
                .len()
                .cmp(&right_significant.len())
                .then_with(|| left_significant.cmp(right_significant));
            if number_order != Ordering::Equal {
                return number_order;
            }
            // Numerically equal runs are stable: fewer leading zeroes sort first.
            let width_order = left_digits.len().cmp(&right_digits.len());
            if width_order != Ordering::Equal {
                return width_order;
            }
            continue;
        }

        let left_byte = left[left_index].to_ascii_lowercase();
        let right_byte = right[right_index].to_ascii_lowercase();
        match left_byte.cmp(&right_byte) {
            Ordering::Equal => {
                left_index += 1;
                right_index += 1;
            }
            order => return order,
        }
    }

    left.len().cmp(&right.len())
}

fn register_natural_title_collation(connection: &Connection) -> rusqlite::Result<()> {
    connection.create_collation("NATURAL_NOCASE", natural_title_cmp)
}

fn register_music_collation(
    connection: &Connection,
    collation_spec: &MusicCollationSpec,
) -> rusqlite::Result<()> {
    let collators = collation_spec.build_collators();
    let opencc = Arc::new(
        OpenCC::from_config(BuiltinConfig::S2t)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
    );
    connection.create_collation(MUSIC_COLLATION_NAME, {
        let collators = collators.clone();
        move |left, right| compare_music_names(&collators, left, right)
    })?;
    connection.create_scalar_function(
        MUSIC_QUICK_INDEX_FUNCTION,
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        move |context| {
            let value = context.get::<String>(0)?;
            Ok(quick_index_label(&collators, &value))
        },
    )?;
    connection.create_scalar_function(
        MUSIC_SPLIT_ARTISTS_FUNCTION,
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |context| {
            let value = context.get::<Option<String>>(0)?;
            serde_json::to_string(&split_artist_names(value.as_deref()))
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
        },
    )?;
    connection.create_scalar_function(
        MUSIC_IDENTITY_KEY_FUNCTION,
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        {
            let opencc = Arc::clone(&opencc);
            move |context| {
                let value = context.get::<Option<String>>(0)?.unwrap_or_default();
                Ok(music_identity_key(&value, &opencc))
            }
        },
    )?;
    connection.create_scalar_function(
        MUSIC_IDENTITY_TRADITIONAL_FUNCTION,
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        move |context| {
            let value = context.get::<Option<String>>(0)?.unwrap_or_default();
            Ok(music_identity_traditional(&value, &opencc))
        },
    )?;
    connection.create_scalar_function(
        MUSIC_TRIM_FUNCTION,
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |context| {
            let value = context.get::<Option<String>>(0)?;
            Ok(value.unwrap_or_default().trim().to_owned())
        },
    )
}

fn music_identity_traditional(value: &str, opencc: &OpenCC) -> String {
    let value = value.trim();
    if !value.chars().any(is_han_character) || value.chars().any(is_japanese_kana) {
        value.to_owned()
    } else {
        opencc.convert(value).trim().to_owned()
    }
}

fn music_identity_key(value: &str, opencc: &OpenCC) -> String {
    music_identity_traditional(value, opencc)
        .nfkc()
        .collect::<String>()
        .to_lowercase()
}

fn is_han_character(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2FA1F
            | 0x30000..=0x323AF
    )
}

fn is_japanese_kana(character: char) -> bool {
    matches!(
        character as u32,
        0x3040..=0x309F
            | 0x30A0..=0x30FF
            | 0x31F0..=0x31FF
            | 0x32D0..=0x32FE
            | 0x3300..=0x3357
            | 0xFF66..=0xFF9F
            | 0x1B000..=0x1B16F
            | 0x1AFF0..=0x1AFFF
    )
}

fn ensure_music_collation_index(
    connection: &Connection,
    collation_spec: &MusicCollationSpec,
) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS music_collation_metadata (
             singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
             signature TEXT NOT NULL
         );",
    )?;
    let stored_signature = connection
        .query_row(
            "SELECT signature FROM music_collation_metadata WHERE singleton=1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let index_exists = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name='tracks_sort_title')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    let signature = collation_spec.signature();
    if stored_signature.as_deref() == Some(signature.as_str()) && index_exists {
        return Ok(());
    }

    let tx = connection.unchecked_transaction()?;
    tx.execute_batch(
        "DROP INDEX IF EXISTS tracks_sort_title;
         CREATE INDEX tracks_sort_title
             ON tracks(sort_title COLLATE MUSIC_LOCALE_NATURAL, track_id);",
    )?;
    tx.execute(
        "INSERT INTO music_collation_metadata(singleton, signature) VALUES (1, ?1)
         ON CONFLICT(singleton) DO UPDATE SET signature=excluded.signature",
        [signature],
    )?;
    tx.commit()
}

fn repeat_mode_name(mode: QueueRepeatMode) -> &'static str {
    match mode {
        QueueRepeatMode::Off => "off",
        QueueRepeatMode::One => "one",
        QueueRepeatMode::All => "all",
    }
}

fn parse_repeat_mode(value: &str) -> Option<QueueRepeatMode> {
    match value {
        "off" => Some(QueueRepeatMode::Off),
        "one" => Some(QueueRepeatMode::One),
        "all" => Some(QueueRepeatMode::All),
        _ => None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaylistFileSyncState {
    pub playlist_id: PlaylistId,
    pub locator: MediaLocator,
    pub fingerprint: FileFingerprint,
    pub content_sha256: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaybackSessionCheckpoint {
    pub queue: PlaybackQueueSnapshot,
    pub position_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaybackStatisticsRuntime {
    pub runtime_id: Uuid,
    pub owner_pid: Option<u32>,
    pub owner_process_started_utc_ms: Option<i64>,
    pub registered_at_utc_ms: i64,
}

pub struct Database {
    connection: Mutex<Connection>,
    collation_spec: MusicCollationSpec,
    collators: MusicCollators,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumDiscCount {
    pub disc_number: u32,
    pub count: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumDetailFacets {
    pub disc_counts: Vec<AlbumDiscCount>,
    pub track_count: u64,
    pub unknown_disc_track_count: u64,
    pub quick_index_ascending: QuickIndexSnapshot,
    pub quick_index_descending: QuickIndexSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistNameCount {
    pub name: String,
    pub count: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistIndexEntry {
    pub key: String,
    pub names: Vec<ArtistNameCount>,
    pub artist_names: Vec<String>,
    pub display_name: String,
    pub track_count: u64,
    pub album_count: u64,
    pub played_ms: u64,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumIndexEntry {
    pub key: String,
    pub display_name: String,
    pub aliases: Vec<String>,
    pub track_count: u64,
    pub duration_ms: u64,
    pub played_ms: u64,
    pub unmarked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexSnapshotInfo {
    pub snapshot_id: String,
    pub quick_index: QuickIndexSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickIndexGroup {
    pub label: String,
    pub before_index: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickIndexSnapshot {
    pub labels: Vec<String>,
    pub groups: Vec<QuickIndexGroup>,
    pub total_count: u64,
}

impl QuickIndexSnapshot {
    fn push_group(&mut self, group: QuickIndexGroup) {
        if !self.labels.iter().any(|label| label == &group.label) {
            self.labels.push(group.label.clone());
        }
        self.groups.push(group);
    }
}

#[derive(Clone, Debug)]
pub struct QueueSortCandidate {
    pub entry_index: usize,
    pub summary: Option<TrackSummary>,
}

impl fmt::Debug for Database {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Database").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum DatabaseError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    Poisoned,
    UnsupportedSchemaVersion(i64),
    UnsupportedLocatorEncoding(String),
    CorruptData(String),
    CheckpointBusy {
        log_frames: i64,
        checkpointed_frames: i64,
    },
    InvalidNumber(&'static str),
    SourceMismatch,
    TrackNotFound(TrackId),
    NoEnabledTrackMapping(TrackId),
    NonFilesystemTrackLocator(TrackId),
    NonContentUriTrackLocator(TrackId),
    TrackFileUnavailable(TrackId),
    InvalidThemeColor(&'static str),
    UnknownPlaybackStatisticsRuntime(Uuid),
}

impl fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "SQLite error: {error}"),
            Self::Io(error) => write!(f, "database path error: {error}"),
            Self::Poisoned => f.write_str("database connection mutex was poisoned"),
            Self::UnsupportedSchemaVersion(version) => {
                write!(
                    f,
                    "database schema version {version} is newer than this app"
                )
            }
            Self::UnsupportedLocatorEncoding(encoding) => {
                write!(f, "cannot open a filesystem locator encoded as {encoding}")
            }
            Self::CorruptData(reason) => write!(f, "invalid database data: {reason}"),
            Self::CheckpointBusy {
                log_frames,
                checkpointed_frames,
            } => write!(
                f,
                "SQLite WAL checkpoint remained busy ({checkpointed_frames}/{log_frames} frames checkpointed)"
            ),
            Self::InvalidNumber(field) => write!(f, "{field} does not fit SQLite INTEGER"),
            Self::SourceMismatch => {
                f.write_str("a source scan contains an identity from a different library root")
            }
            Self::TrackNotFound(track_id) => write!(f, "track {track_id} was not found"),
            Self::NoEnabledTrackMapping(track_id) => {
                write!(f, "track {track_id} has no enabled source mapping")
            }
            Self::NonFilesystemTrackLocator(track_id) => write!(
                f,
                "track {track_id} has no enabled filesystem locator; its available locator is not a local path"
            ),
            Self::NonContentUriTrackLocator(track_id) => write!(
                f,
                "track {track_id} has no enabled content URI locator; its available locator is not an Android content URI"
            ),
            Self::TrackFileUnavailable(track_id) => write!(
                f,
                "track {track_id} has no currently accessible filesystem file; its source may be offline or the file may have been removed"
            ),
            Self::InvalidThemeColor(field) => write!(
                f,
                "theme preference {field} must be a six-digit hexadecimal color such as #55D9FF"
            ),
            Self::UnknownPlaybackStatisticsRuntime(runtime_id) => write!(
                f,
                "playback statistics runtime {runtime_id} is not registered or has already finished"
            ),
        }
    }
}

impl Error for DatabaseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for DatabaseError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

impl From<std::io::Error> for DatabaseError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LibrarySyncState {
    pub last_attempt_utc_ms: Option<i64>,
    pub last_success_utc_ms: Option<i64>,
    pub state: String,
    pub error_count: u64,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThemePreferences {
    pub background_hex: String,
    pub accent_hex: String,
}

impl Default for ThemePreferences {
    fn default() -> Self {
        Self {
            background_hex: "#000000".to_owned(),
            accent_hex: "#55D9FF".to_owned(),
        }
    }
}

impl ThemePreferences {
    fn validate(&self) -> Result<(), DatabaseError> {
        validate_theme_color(&self.background_hex, "backgroundHex")?;
        validate_theme_color(&self.accent_hex, "accentHex")
    }
}

impl Database {
    /// Open an existing database only after a structural check on a genuinely read-only handle.
    /// This method never rebuilds or replaces the database and checks before writable migrations.
    pub fn open_checked(path: impl AsRef<Path>) -> Result<Self, DatabaseError> {
        let collation_spec = MusicCollationSpec::system_default();
        let path = path.as_ref();
        if path == Path::new(":memory:") || !path.exists() {
            return Self::open_with_collation(path, collation_spec);
        }
        let metadata = std::fs::metadata(path)?;
        if metadata.len() == 0 {
            return Err(DatabaseError::CorruptData(
                "existing SQLite database file is empty; automatic initialization was refused"
                    .to_owned(),
            ));
        }

        // Validate an existing DB through a genuinely read-only handle. Setting
        // query_only on a read-write handle is not sufficient: SQLite may still
        // recover/checkpoint a WAL while opening or closing that handle.
        let validation_connection =
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        register_natural_title_collation(&validation_connection)?;
        let stored_version: i64 =
            validation_connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let has_collation_metadata: bool = validation_connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='music_collation_metadata')",
            [],
            |row| row.get(0),
        )?;
        if stored_version >= 15 && has_collation_metadata {
            let stored_signature = validation_connection
                .query_row(
                    "SELECT signature FROM music_collation_metadata WHERE singleton=1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            let validation_spec = stored_signature
                .as_deref()
                .and_then(MusicCollationSpec::from_signature)
                .unwrap_or_else(|| collation_spec.clone());
            register_music_collation(&validation_connection, &validation_spec)?;
        }
        validation_connection.busy_timeout(Duration::from_secs(5))?;
        let check: String = validation_connection
            .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
            .map_err(|error| {
                DatabaseError::CorruptData(format!("startup quick_check failed: {error}"))
            })?;
        if check != "ok" {
            return Err(DatabaseError::CorruptData(format!(
                "startup quick_check failed: {check}"
            )));
        }

        // Keep the read-only handle open while acquiring the writer handle so
        // the path cannot be replaced between validation and normal open on
        // platforms that enforce SQLite's file-sharing locks.
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        drop(validation_connection);
        Self::from_connection_with_collation(connection, collation_spec)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, DatabaseError> {
        Self::open_with_collation(path, MusicCollationSpec::system_default())
    }

    fn open_with_collation(
        path: impl AsRef<Path>,
        collation_spec: MusicCollationSpec,
    ) -> Result<Self, DatabaseError> {
        let path = path.as_ref();
        if path != Path::new(":memory:") {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)?;
            }
        }
        let connection = Connection::open(path)?;
        Self::from_connection_with_collation(connection, collation_spec)
    }

    pub fn open_in_memory() -> Result<Self, DatabaseError> {
        Self::from_connection_with_collation(
            Connection::open_in_memory()?,
            MusicCollationSpec::system_default(),
        )
    }

    #[cfg(test)]
    fn from_connection(connection: Connection) -> Result<Self, DatabaseError> {
        Self::from_connection_with_collation(connection, MusicCollationSpec::system_default())
    }

    /// Checkpoint pending WAL pages after callers have stopped and joined their writers.
    pub fn checkpoint_wal(&self) -> Result<(), DatabaseError> {
        let connection = self.lock()?;
        let (busy, log_frames, checkpointed_frames): (i64, i64, i64) =
            connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        if busy != 0 {
            return Err(DatabaseError::CheckpointBusy {
                log_frames,
                checkpointed_frames,
            });
        }
        Ok(())
    }

    fn from_connection_with_collation(
        connection: Connection,
        collation_spec: MusicCollationSpec,
    ) -> Result<Self, DatabaseError> {
        register_natural_title_collation(&connection)?;
        register_music_collation(&connection, &collation_spec)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        let database = Self {
            connection: Mutex::new(connection),
            collators: collation_spec.build_collators(),
            collation_spec,
        };
        database.migrate()?;
        database.backfill_missing_title_sort_keys()?;
        Ok(database)
    }

    fn backfill_missing_title_sort_keys(&self) -> Result<(), DatabaseError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let candidates = {
            let mut statement = tx.prepare(
                "SELECT t.track_id,
                        (SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'),
                        t.title
                 FROM tracks t
                 WHERE t.sort_title=''",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        if candidates.is_empty() {
            return Ok(());
        }
        let track_ids = candidates
            .iter()
            .map(|(id, _, _)| parse_track_id(id))
            .collect::<Result<Vec<_>, _>>()?;
        let file_names = file_names_for_tracks(&tx, &track_ids)?;
        for (id, override_title, metadata_title) in candidates {
            let track_id = parse_track_id(&id)?;
            let file_name = file_names.get(&track_id).map(String::as_str);
            let sort_title = effective_sort_title(
                override_title.as_deref(),
                metadata_title.as_deref(),
                file_name,
            );
            if !sort_title.is_empty() {
                tx.execute(
                    "UPDATE tracks SET sort_title=?2 WHERE track_id=?1 AND sort_title=''",
                    params![id, sort_title],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn migrate(&self) -> Result<(), DatabaseError> {
        let mut connection = self.lock()?;
        let mut version: i64 =
            connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(DatabaseError::UnsupportedSchemaVersion(version));
        }
        if version == 0 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V1)?;
            tx.pragma_update(None, "user_version", 1)?;
            tx.commit()?;
            version = 1;
        }
        if version == 1 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V2)?;
            tx.pragma_update(None, "user_version", 2)?;
            tx.commit()?;
            version = 2;
        }
        if version == 2 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V3)?;
            tx.pragma_update(None, "user_version", 3)?;
            tx.commit()?;
            version = 3;
        }
        if version == 3 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V4)?;
            tx.pragma_update(None, "user_version", 4)?;
            tx.commit()?;
            version = 4;
        }
        if version == 4 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V5)?;
            tx.pragma_update(None, "user_version", 5)?;
            tx.commit()?;
            version = 5;
        }
        if version == 5 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V6)?;
            tx.pragma_update(None, "user_version", 6)?;
            tx.commit()?;
            version = 6;
        }
        if version == 6 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V7)?;
            tx.pragma_update(None, "user_version", 7)?;
            tx.commit()?;
            version = 7;
        }
        if version == 7 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V8)?;
            tx.pragma_update(None, "user_version", 8)?;
            tx.commit()?;
            version = 8;
        }
        if version == 8 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V9)?;
            tx.pragma_update(None, "user_version", 9)?;
            tx.commit()?;
            version = 9;
        }
        if version == 9 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V10)?;
            tx.pragma_update(None, "user_version", 10)?;
            tx.commit()?;
            version = 10;
        }
        if version == 10 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V11)?;
            tx.pragma_update(None, "user_version", 11)?;
            tx.commit()?;
            version = 11;
        }
        if version == 11 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V12)?;
            tx.pragma_update(None, "user_version", 12)?;
            tx.commit()?;
            version = 12;
        }
        if version == 12 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V13)?;
            tx.pragma_update(None, "user_version", 13)?;
            tx.commit()?;
            version = 13;
        }
        if version == 13 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V14)?;
            tx.pragma_update(None, "user_version", 14)?;
            tx.commit()?;
            version = 14;
        }
        if version == 14 {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V15)?;
            tx.pragma_update(None, "user_version", 15)?;
            tx.commit()?;
        }
        connection.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS sync_seen_items (
                source_id TEXT NOT NULL, source_item_id TEXT NOT NULL,
                PRIMARY KEY (source_id, source_item_id)
            );
            CREATE TEMP TABLE IF NOT EXISTS index_snapshot_input (
                snapshot_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                identity_key TEXT NOT NULL,
                display_name TEXT NOT NULL,
                metric_primary INTEGER NOT NULL,
                metric_secondary INTEGER NOT NULL,
                payload_json TEXT NOT NULL
            );
            CREATE TEMP TABLE IF NOT EXISTS index_snapshot_rows (
                snapshot_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                position INTEGER NOT NULL,
                group_label TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                PRIMARY KEY (snapshot_id, position)
            );
            CREATE TEMP TABLE IF NOT EXISTS index_identity_map (
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                identity_key TEXT NOT NULL,
                traditional TEXT NOT NULL,
                PRIMARY KEY (kind, name)
            );
            CREATE TEMP TABLE IF NOT EXISTS index_ordered_values (
                position INTEGER PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TEMP TABLE IF NOT EXISTS queue_sort_values (
                entry_index INTEGER NOT NULL,
                stable_order INTEGER NOT NULL,
                known INTEGER NOT NULL,
                track_id TEXT NOT NULL,
                sort_title TEXT NOT NULL,
                artist TEXT,
                album TEXT,
                release_date TEXT,
                year INTEGER,
                created_at_utc_ms INTEGER,
                codec TEXT,
                bitrate_bps INTEGER,
                sample_rate_hz INTEGER,
                bit_depth INTEGER,
                duration_ms INTEGER,
                played_ms INTEGER NOT NULL,
                PRIMARY KEY(entry_index)
            );
            CREATE INDEX IF NOT EXISTS temp.index_snapshot_rows_by_id_position
                ON index_snapshot_rows(snapshot_id, position);",
        )?;
        ensure_music_collation_index(&connection, &self.collation_spec)?;
        Ok(())
    }

    /// Sort display labels with the same SQLite ICU4X collation and natural-number rules used by
    /// track and playlist ordering. Returned indices refer to the original input slice.
    pub fn sort_text_indices(&self, values: &[String]) -> Result<Vec<usize>, DatabaseError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction()?;
        tx.execute("DELETE FROM index_ordered_values", [])?;
        {
            let mut insert =
                tx.prepare("INSERT INTO index_ordered_values(position, value) VALUES (?1, ?2)")?;
            for (position, value) in values.iter().enumerate() {
                insert.execute(params![i64::try_from(position).unwrap_or(i64::MAX), value])?;
            }
        }
        let mut statement = tx.prepare(
            "SELECT position FROM index_ordered_values
             ORDER BY value COLLATE MUSIC_LOCALE_NATURAL, position",
        )?;
        let indices = statement
            .query_map([], |row| row.get::<_, i64>(0))?
            .map(|row| {
                let index = row?;
                usize::try_from(index).map_err(|_| {
                    DatabaseError::CorruptData("text sort returned a negative row index".into())
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        tx.commit()?;
        Ok(indices)
    }

    /// Classify first-character index groups using the same ICU4X data and ordering as library SQL.
    pub fn quick_index_groupings(&self, values: &[String]) -> (Vec<String>, Vec<String>) {
        let groups = values
            .iter()
            .map(|value| quick_index_label(&self.collators, value))
            .collect::<Vec<_>>();
        let mut labels = Vec::new();
        let mut seen = HashSet::new();
        for label in &groups {
            if seen.insert(label.as_str()) {
                labels.push(label.clone());
            }
        }
        (labels, groups)
    }

    /// Build group boundaries for an already sorted sequence of labels. This is used for merged
    /// Windows library results, where part of the sequence comes from the in-memory SQLite catalog.
    pub fn quick_index_for_ordered_values(
        &self,
        values: &[String],
    ) -> Result<QuickIndexSnapshot, DatabaseError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction()?;
        tx.execute("DELETE FROM index_ordered_values", [])?;
        {
            let mut insert =
                tx.prepare("INSERT INTO index_ordered_values(position, value) VALUES (?1, ?2)")?;
            for (position, value) in values.iter().enumerate() {
                insert.execute(params![i64::try_from(position).unwrap_or(i64::MAX), value])?;
            }
        }
        let mut statement = tx.prepare(
            "WITH labels AS (
                SELECT position, music_quick_index_label(value) AS group_label,
                       COUNT(*) OVER () AS total_count
                FROM index_ordered_values
             ), transitions AS (
                SELECT position, group_label, total_count,
                       LAG(group_label) OVER (ORDER BY position) AS previous_label
                FROM labels
             )
             SELECT position, group_label, total_count FROM transitions
             WHERE previous_label IS NULL OR previous_label <> group_label
             ORDER BY position",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?.max(0) as u64,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?.max(0) as u64,
            ))
        })?;
        let mut snapshot = QuickIndexSnapshot::default();
        for row in rows {
            let (before_index, label, total_count) = row?;
            snapshot.total_count = total_count;
            snapshot.push_group(QuickIndexGroup {
                label,
                before_index,
            });
        }
        drop(statement);
        tx.commit()?;
        Ok(snapshot)
    }

    /// Sort an in-memory playback traversal through the database's ICU4X SQLite connection.
    /// Queue entries stay distinct even when their TrackId is repeated.
    pub fn sort_queue_candidates(
        &self,
        candidates: &[QueueSortCandidate],
        sort: TrackListSort,
    ) -> Result<Vec<usize>, DatabaseError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction()?;
        tx.execute("DELETE FROM queue_sort_values", [])?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO queue_sort_values (
                    entry_index, stable_order, known, track_id, sort_title, artist, album,
                    release_date, year, created_at_utc_ms, codec, bitrate_bps, sample_rate_hz,
                    bit_depth, duration_ms, played_ms
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
                 )",
            )?;
            for (stable_order, candidate) in candidates.iter().enumerate() {
                let summary = candidate.summary.as_ref();
                let sort_title = summary
                    .and_then(|track| {
                        player_core::display_track_title(
                            track.title.as_deref(),
                            track.file_name.as_deref(),
                        )
                    })
                    .unwrap_or_default();
                insert.execute(params![
                    i64::try_from(candidate.entry_index).unwrap_or(i64::MAX),
                    i64::try_from(stable_order).unwrap_or(i64::MAX),
                    i64::from(summary.is_some()),
                    summary.map_or_else(String::new, |track| track.id.to_string()),
                    sort_title,
                    summary.and_then(|track| track.artist.clone()),
                    summary.and_then(|track| track.album.clone()),
                    summary.and_then(|track| track.release_date.clone()),
                    summary.and_then(|track| track.year.map(i64::from)),
                    summary.and_then(|track| track.created_at_utc_ms),
                    summary.and_then(|track| track.codec.clone()),
                    summary.and_then(|track| track.bitrate_bps.map(i64::from)),
                    summary.and_then(|track| track.sample_rate_hz.map(i64::from)),
                    summary.and_then(|track| track.bit_depth.map(i64::from)),
                    summary.and_then(|track| track
                        .duration_ms
                        .map(|value| i64::try_from(value).unwrap_or(i64::MAX))),
                    summary.map_or(0, |track| i64::try_from(track.played_ms)
                        .unwrap_or(i64::MAX)),
                ])?;
            }
        }
        let order_by = queue_sort_order_by(sort);
        let query = format!(
            "SELECT entry_index FROM queue_sort_values t
             ORDER BY known DESC, {order_by}, stable_order ASC"
        );
        let mut statement = tx.prepare(&query)?;
        let indices = statement
            .query_map([], |row| row.get::<_, i64>(0))?
            .map(|row| {
                usize::try_from(row?).map_err(|_| {
                    DatabaseError::CorruptData("negative queue entry index".to_owned())
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        tx.commit()?;
        Ok(indices)
    }

    fn materialize_index_snapshot(
        tx: &Transaction<'_>,
        kind: &str,
        sort_field: &str,
        direction: &str,
        reverse_ties: bool,
        grouping_enabled: bool,
        snapshot_id: &str,
    ) -> Result<IndexSnapshotInfo, DatabaseError> {
        let direction_sql = match direction {
            "asc" => "ASC",
            "desc" => "DESC",
            _ => {
                return Err(DatabaseError::CorruptData(
                    "invalid index sort direction".to_owned(),
                ))
            }
        };
        let tie_direction_sql = if reverse_ties { direction_sql } else { "ASC" };
        let mut order_by = match (kind, sort_field) {
            ("artist", "title") | ("album", "title") | ("artist-navigation", "title") | ("album-navigation", "title") => format!(
                "display_name COLLATE {MUSIC_COLLATION_NAME} {direction_sql}, identity_key {tie_direction_sql}"
            ),
            ("artist", "trackCount") => format!(
                "metric_primary {direction_sql}, display_name COLLATE {MUSIC_COLLATION_NAME} {tie_direction_sql}, identity_key {tie_direction_sql}"
            ),
            ("artist", "albumCount") | ("album", "trackCount") => format!(
                "metric_primary {direction_sql}, display_name COLLATE {MUSIC_COLLATION_NAME} {tie_direction_sql}, identity_key {tie_direction_sql}"
            ),
            ("artist", "duration") | ("album", "played") => format!(
                "metric_primary {direction_sql}, display_name COLLATE {MUSIC_COLLATION_NAME} {tie_direction_sql}, identity_key {tie_direction_sql}"
            ),
            ("artist", "trackDuration") | ("album", "duration") => format!(
                "metric_primary {direction_sql}, display_name COLLATE {MUSIC_COLLATION_NAME} {tie_direction_sql}, identity_key {tie_direction_sql}"
            ),
            _ => return Err(DatabaseError::CorruptData("invalid index sort field".to_owned())),
        };
        // The unmarked entry is a synthetic "未知" title, so sorting it by its
        // display text lets its locale collation split the natural-language
        // groups. Treat it as a dedicated text-sort category instead: last in
        // ascending order and first in descending order. Metric sorts remain
        // purely metric-based, so unmarked entries still participate normally.
        if sort_field == "title"
            && matches!(
                kind,
                "artist" | "album" | "artist-navigation" | "album-navigation"
            )
        {
            order_by = format!(
                "CASE WHEN identity_key IN ('artist:unmarked', 'album:unmarked') THEN 1 ELSE 0 END {direction_sql}, {order_by}"
            );
        }
        let materialize_sql = format!(
            "INSERT INTO index_snapshot_rows(snapshot_id, kind, position, group_label, payload_json)
             SELECT snapshot_id, kind,
                    ROW_NUMBER() OVER (ORDER BY {order_by}) - 1,
                    CASE WHEN identity_key IN ('album:unmarked', 'artist:unmarked') THEN '未知'
                         ELSE {MUSIC_QUICK_INDEX_FUNCTION}(display_name) END,
                    payload_json
             FROM index_snapshot_input WHERE snapshot_id=?1"
        );
        tx.execute(&materialize_sql, [snapshot_id])?;
        let total_count = tx
            .query_row(
                "SELECT COUNT(*) FROM index_snapshot_rows WHERE snapshot_id=?1",
                [snapshot_id],
                |row| row.get::<_, i64>(0),
            )?
            .max(0) as u64;
        let mut quick_index = QuickIndexSnapshot {
            total_count,
            ..QuickIndexSnapshot::default()
        };
        if grouping_enabled && total_count > 0 {
            let mut statement = tx.prepare(
                "WITH transitions AS (
                    SELECT position, group_label,
                           LAG(group_label) OVER (ORDER BY position) AS previous_label
                    FROM index_snapshot_rows WHERE snapshot_id=?1
                 )
                 SELECT position, group_label FROM transitions
                 WHERE previous_label IS NULL OR previous_label <> group_label
                 ORDER BY position",
            )?;
            let rows = statement.query_map([snapshot_id], |row| {
                Ok(QuickIndexGroup {
                    before_index: row.get::<_, i64>(0)?.max(0) as u64,
                    label: row.get(1)?,
                })
            })?;
            for row in rows {
                quick_index.push_group(row?);
            }
        }
        Ok(IndexSnapshotInfo {
            snapshot_id: snapshot_id.to_owned(),
            quick_index,
        })
    }

    pub fn get_index_snapshot_page<T: DeserializeOwned>(
        &self,
        snapshot_id: &str,
        offset: u64,
        requested_limit: u32,
    ) -> Result<Page<T>, DatabaseError> {
        let limit = query_limit(requested_limit);
        let connection = self.lock()?;
        let total_count = connection
            .query_row(
                "SELECT COUNT(*) FROM index_snapshot_rows WHERE snapshot_id=?1",
                [snapshot_id],
                |row| row.get::<_, i64>(0),
            )?
            .max(0) as u64;
        let mut statement = connection.prepare(
            "SELECT payload_json FROM index_snapshot_rows
             WHERE snapshot_id=?1 ORDER BY position LIMIT ?2 OFFSET ?3",
        )?;
        let payloads = statement
            .query_map(
                params![snapshot_id, i64::from(limit), query_limit_i64(offset)],
                |row| row.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let items = payloads
            .into_iter()
            .map(|payload| {
                serde_json::from_str(&payload)
                    .map_err(|error| DatabaseError::CorruptData(error.to_string()))
            })
            .collect::<Result<Vec<T>, _>>()?;
        Ok(Page {
            items,
            offset,
            limit,
            total_count,
        })
    }

    /// Read a single aggregate row from a prepared snapshot without transferring the complete
    /// index to the renderer. Navigation lookups use their own snapshot kind so they cannot
    /// invalidate the visible index page.
    pub fn get_index_snapshot_entry<T: DeserializeOwned>(
        &self,
        snapshot_id: &str,
        identity_key: &str,
    ) -> Result<Option<T>, DatabaseError> {
        let connection = self.lock()?;
        let payload = connection
            .query_row(
                "SELECT payload_json FROM index_snapshot_input
             WHERE snapshot_id=?1 AND identity_key=?2 LIMIT 1",
                params![snapshot_id, identity_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        payload
            .map(|payload| {
                serde_json::from_str(&payload)
                    .map_err(|error| DatabaseError::CorruptData(error.to_string()))
            })
            .transpose()
    }

    /// Compute quick-index group boundaries from the complete filtered and sorted library
    /// result in one SQLite read. The renderer receives only group offsets, never every row's
    /// grouping key or a series of pages to scan.
    pub fn quick_index_for_tracks(
        &self,
        request: &ListTracksQuery,
        query_alternates: &[String],
    ) -> Result<QuickIndexSnapshot, DatabaseError> {
        if !matches!(
            request.sort.field,
            TrackSortField::Title | TrackSortField::Artist | TrackSortField::Album
        ) {
            return Ok(QuickIndexSnapshot::default());
        }
        let connection = self.lock()?;
        let query = request
            .query
            .as_deref()
            .filter(|text| !text.trim().is_empty());
        let query_json = search_terms_json(query, query_alternates)?;
        let (title_filter, artist_filter, album_filter) =
            field_filter_values(request.field_filter.as_ref())?;
        let order_by = tracks_order_by_sql(request.sort);
        let value_sql = match request.sort.field {
            // `sort_title` is backfilled from the same displayed file stem when the title tag is
            // missing, so grouping and sorting share one persisted key without decoding locators
            // or materializing TrackSummary rows in Rust.
            TrackSortField::Title => "t.sort_title",
            TrackSortField::Artist => "COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, '')",
            TrackSortField::Album => "COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, '')",
            _ => unreachable!("non-text sort returned above"),
        };
        let sql = format!(
            "WITH ordered AS (
                SELECT ROW_NUMBER() OVER (ORDER BY {order_by}) - 1 AS position,
                       {MUSIC_QUICK_INDEX_FUNCTION}({value_sql}) AS group_label,
                       COUNT(*) OVER () AS total_count
                FROM tracks t
                WHERE EXISTS (SELECT 1 FROM source_mappings m JOIN library_roots r
                              ON r.source_id=m.source_id AND r.enabled=1 WHERE m.track_id=t.track_id)
                  AND (?1 IS NULL OR EXISTS (SELECT 1 FROM json_each(?1) term
                       WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) LIKE '%' || term.value || '%' COLLATE NOCASE
                          OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist) LIKE '%' || term.value || '%' COLLATE NOCASE
                          OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) LIKE '%' || term.value || '%' COLLATE NOCASE
                          OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist) LIKE '%' || term.value || '%' COLLATE NOCASE))
                  AND (?4 IS NULL OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) = ?4 COLLATE BINARY)
                  AND (?5 IS NULL OR EXISTS (SELECT 1 FROM json_each(?5) requested_artist
                       WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, '') = requested_artist.value COLLATE BINARY
                          OR instr('|' || replace(replace(replace(replace(replace(replace(replace(replace(COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, ''), char(92), '|'), '/', '|'), ';', '|'), ',', '|'), '&', '|'), ' | ', '|'), '| ', '|'), ' |', '|') || '|', '|' || requested_artist.value || '|') > 0))
                  AND (?6 IS NULL OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) = ?6 COLLATE BINARY)
             ), transitions AS (
                SELECT position, group_label, total_count,
                       LAG(group_label) OVER (ORDER BY position) AS previous_label
                FROM ordered
             )
             SELECT position, group_label, total_count FROM transitions
             WHERE previous_label IS NULL OR previous_label <> group_label
             ORDER BY position"
        );
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(
            params![
                query_json,
                Option::<i64>::None,
                Option::<i64>::None,
                title_filter,
                artist_filter,
                album_filter
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?.max(0) as u64,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?.max(0) as u64,
                ))
            },
        )?;
        let mut snapshot = QuickIndexSnapshot::default();
        for row in rows {
            let (before_index, label, total_count) = row?;
            snapshot.total_count = total_count;
            snapshot.push_group(QuickIndexGroup {
                label,
                before_index,
            });
        }
        Ok(snapshot)
    }

    /// Compare two display labels using the same ICU4X category and numeric collation as SQL.
    pub fn compare_music_text(&self, left: &str, right: &str) -> Ordering {
        compare_music_names(&self.collators, left, right)
    }

    /// Normalize one display label with the same SQLite UDF used by index aggregation.
    /// This is for resolving a single navigation target; bulk identities stay in SQL.
    pub fn music_identity_key(&self, value: &str) -> Result<String, DatabaseError> {
        self.lock()?
            .query_row("SELECT music_identity_key(?1)", [value], |row| row.get(0))
            .map_err(DatabaseError::from)
    }

    pub fn journal_mode(&self) -> Result<String, DatabaseError> {
        Ok(self
            .lock()?
            .pragma_query_value(None, "journal_mode", |row| row.get(0))?)
    }

    pub fn get_theme_preferences(&self) -> Result<ThemePreferences, DatabaseError> {
        let connection = self.lock()?;
        let preferences = connection
            .query_row(
                "SELECT background_hex, accent_hex FROM theme_preferences WHERE singleton_id=1",
                [],
                |row| {
                    Ok(ThemePreferences {
                        background_hex: row.get(0)?,
                        accent_hex: row.get(1)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default();
        preferences.validate().map_err(|_| {
            DatabaseError::CorruptData(
                "stored theme colors are not six-digit hexadecimal values".to_owned(),
            )
        })?;
        Ok(preferences)
    }

    pub fn set_theme_preferences(
        &self,
        preferences: &ThemePreferences,
    ) -> Result<(), DatabaseError> {
        preferences.validate()?;
        self.lock()?.execute(
            "INSERT INTO theme_preferences (singleton_id, background_hex, accent_hex)
             VALUES (1, ?1, ?2)
             ON CONFLICT(singleton_id) DO UPDATE SET
                background_hex=excluded.background_hex,
                accent_hex=excluded.accent_hex",
            params![preferences.background_hex, preferences.accent_hex],
        )?;
        Ok(())
    }

    /// Remove the pre-JSON theme value after it has been imported into settings.json.
    /// The table remains as a schema migration compatibility shell; it is no longer
    /// an authoritative settings store.
    pub fn clear_legacy_theme_preferences(&self) -> Result<(), DatabaseError> {
        self.lock()?
            .execute("DELETE FROM theme_preferences WHERE singleton_id=1", [])?;
        Ok(())
    }

    pub fn add_library_root(
        &self,
        kind: MediaSourceKind,
        display_name: impl Into<String>,
        locator: MediaLocator,
    ) -> Result<LibraryRoot, DatabaseError> {
        let root = LibraryRoot {
            id: SourceId::new(),
            kind,
            display_name: display_name.into(),
            locator,
            enabled: true,
        };
        self.save_library_root(&root)?;
        Ok(root)
    }

    pub fn save_library_root(&self, root: &LibraryRoot) -> Result<(), DatabaseError> {
        let stored = locator::encode(&root.locator);
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO library_roots
                (source_id, source_kind, display_name, locator_kind, locator_encoding, locator_data, enabled)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(source_id) DO UPDATE SET
                source_kind=excluded.source_kind, display_name=excluded.display_name,
                locator_kind=excluded.locator_kind, locator_encoding=excluded.locator_encoding,
                locator_data=excluded.locator_data, enabled=excluded.enabled",
            params![
                root.id.to_string(),
                source_kind_name(root.kind),
                root.display_name,
                stored.kind,
                stored.encoding,
                stored.data,
                i64::from(root.enabled),
            ],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO library_sync_state (source_id) VALUES (?1)",
            [root.id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn library_roots(&self) -> Result<Vec<LibraryRoot>, DatabaseError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT source_id, source_kind, display_name, locator_kind, locator_encoding,
                    locator_data, enabled
             FROM library_roots ORDER BY display_name COLLATE NOCASE, source_id",
        )?;
        let rows = statement.query_map([], |row| {
            let source_id: String = row.get(0)?;
            let source_kind: String = row.get(1)?;
            let display_name: String = row.get(2)?;
            let locator_kind: String = row.get(3)?;
            let locator_encoding: String = row.get(4)?;
            let locator_data: Vec<u8> = row.get(5)?;
            let enabled: bool = row.get(6)?;
            Ok((
                source_id,
                source_kind,
                display_name,
                locator_kind,
                locator_encoding,
                locator_data,
                enabled,
            ))
        })?;

        rows.map(|row| {
            let (id, kind, display_name, locator_kind, encoding, data, enabled) = row?;
            Ok(LibraryRoot {
                id: SourceId::parse(&id)
                    .map_err(|error| DatabaseError::CorruptData(error.to_string()))?,
                kind: parse_source_kind(&kind)?,
                display_name,
                locator: locator::decode(&locator_kind, &encoding, &data)?,
                enabled,
            })
        })
        .collect()
    }

    /// Aggregate the artist variants and disc groups needed by one album detail view.
    /// This scans enabled-source tracks once in SQLite; the renderer never pages through the
    /// album just to rebuild these summaries.
    pub fn album_detail_facets(
        &self,
        album_names: &[String],
    ) -> Result<AlbumDetailFacets, DatabaseError> {
        let names = album_names.iter().cloned().collect::<HashSet<_>>();
        if names.is_empty() {
            return Ok(AlbumDetailFacets {
                disc_counts: Vec::new(),
                track_count: 0,
                unknown_disc_track_count: 0,
                quick_index_ascending: QuickIndexSnapshot::default(),
                quick_index_descending: QuickIndexSnapshot::default(),
            });
        }
        let mut names = names.into_iter().collect::<Vec<_>>();
        names.sort();
        let names_json = serde_json::to_string(&names)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        let connection = self.lock()?;
        let (discs_json, track_count, unknown_count, ascending_json, descending_json): (
            String,
            i64,
            i64,
            String,
            String,
        ) =
            connection.query_row(
                "WITH visible AS MATERIALIZED (
                    SELECT t.track_id,
                           t.disc_number
                    FROM tracks t
                    WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, '') COLLATE BINARY
                          IN (SELECT value FROM json_each(?1))
                      AND EXISTS (SELECT 1 FROM source_mappings m JOIN library_roots r
                                  ON r.source_id=m.source_id AND r.enabled=1 WHERE m.track_id=t.track_id)
                ), disc_counts AS (
                    SELECT disc_number, COUNT(*) AS count
                    FROM visible WHERE disc_number IS NOT NULL GROUP BY disc_number
                ), unknown_count AS (
                    SELECT COUNT(*) AS count
                    FROM visible WHERE disc_number IS NULL
                ), ascending_groups AS (
                    SELECT 'DISC ' || current.disc_number AS label,
                           (SELECT COALESCE(SUM(previous.count), 0) FROM disc_counts previous
                            WHERE previous.disc_number < current.disc_number) AS before_index,
                           0 AS group_order,
                           current.disc_number AS disc_order
                    FROM disc_counts current
                    WHERE EXISTS (SELECT 1 FROM disc_counts WHERE disc_number>=2)
                    UNION ALL
                    SELECT '未知', (SELECT COALESCE(SUM(count), 0) FROM disc_counts), 1, 0
                    WHERE EXISTS (SELECT 1 FROM disc_counts WHERE disc_number>=2)
                      AND (SELECT count FROM unknown_count) > 0
                ), descending_groups(label, before_index, group_order, disc_order) AS (
                    SELECT '未知', 0, -1, 0
                    WHERE EXISTS (SELECT 1 FROM disc_counts WHERE disc_number>=2)
                      AND (SELECT count FROM unknown_count) > 0
                    UNION ALL
                    SELECT 'DISC ' || current.disc_number AS label,
                           (SELECT COALESCE(SUM(previous.count), 0) +
                                   (SELECT count FROM unknown_count) FROM disc_counts previous
                            WHERE previous.disc_number > current.disc_number) AS before_index,
                           0 AS group_order,
                           current.disc_number AS disc_order
                    FROM disc_counts current
                    WHERE EXISTS (SELECT 1 FROM disc_counts WHERE disc_number>=2)
                )
                SELECT
                    COALESCE((SELECT json_group_array(json_object('discNumber', disc_number, 'count', count))
                              FROM (SELECT disc_number, count FROM disc_counts ORDER BY disc_number)), '[]'),
                    (SELECT COUNT(*) FROM visible),
                    (SELECT count FROM unknown_count),
                    COALESCE((SELECT json_group_array(json_object('label', label, 'beforeIndex', before_index))
                              FROM (SELECT label, before_index FROM ascending_groups ORDER BY group_order, disc_order)), '[]'),
                    COALESCE((SELECT json_group_array(json_object('label', label, 'beforeIndex', before_index))
                              FROM (SELECT label, before_index FROM descending_groups ORDER BY group_order, disc_order DESC)), '[]')",
                [&names_json],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )?;
        let disc_counts = serde_json::from_str(&discs_json)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        let track_count = track_count.max(0) as u64;
        let quick_index_ascending = album_disc_quick_index_from_json(&ascending_json, track_count)?;
        let quick_index_descending =
            album_disc_quick_index_from_json(&descending_json, track_count)?;
        Ok(AlbumDetailFacets {
            disc_counts,
            track_count,
            unknown_disc_track_count: unknown_count.max(0) as u64,
            quick_index_ascending,
            quick_index_descending,
        })
    }

    /// Merge artist spelling variants and choose the display spelling in SQLite, then sort the
    /// resulting unique names with the same ICU collation as all other music text lists.
    pub fn album_detail_artists(
        &self,
        album_names: &[String],
    ) -> Result<Vec<String>, DatabaseError> {
        let names = album_names.iter().cloned().collect::<HashSet<_>>();
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let mut names = names.into_iter().collect::<Vec<_>>();
        names.sort();
        let names_json = serde_json::to_string(&names)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        let connection = self.lock()?;
        let artists = {
            let mut statement = connection.prepare(
                "WITH visible AS MATERIALIZED (
                    SELECT t.track_id,
                           COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist) AS artist
                    FROM tracks t
                    WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, '') COLLATE BINARY
                          IN (SELECT value FROM json_each(?1))
                      AND EXISTS (SELECT 1 FROM source_mappings m JOIN library_roots r
                                  ON r.source_id=m.source_id AND r.enabled=1 WHERE m.track_id=t.track_id)
                ), variants AS (
                    SELECT DISTINCT v.track_id, trim(a.value) AS name
                    FROM visible v JOIN json_each(music_split_artists_json(v.artist)) a
                    WHERE trim(a.value)<>''
                ), mapped AS (
                    SELECT v.name, COUNT(DISTINCT v.track_id) AS count,
                            music_identity_key(v.name) AS identity_key,
                            music_identity_traditional(v.name) AS traditional
                    FROM variants v
                    GROUP BY v.name, identity_key, traditional
                ), ranked AS (
                    SELECT name, identity_key,
                           ROW_NUMBER() OVER (
                               PARTITION BY identity_key
                               ORDER BY count DESC,
                                        CASE WHEN traditional=name THEN 1 ELSE 0 END DESC,
                                        name COLLATE BINARY ASC
                           ) AS display_rank
                    FROM mapped
                )
                SELECT name FROM ranked WHERE display_rank=1
                ORDER BY name COLLATE MUSIC_LOCALE_NATURAL",
            )?;
            let rows = statement.query_map([names_json], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        Ok(artists)
    }

    pub fn create_album_index_snapshot(
        &self,
        sort_field: &str,
        direction: &str,
        grouping_enabled: bool,
    ) -> Result<IndexSnapshotInfo, DatabaseError> {
        self.create_album_index_snapshot_for_kind(sort_field, direction, grouping_enabled, "album")
    }

    pub fn create_album_navigation_snapshot(&self) -> Result<IndexSnapshotInfo, DatabaseError> {
        self.create_album_index_snapshot_for_kind("title", "asc", false, "album-navigation")
    }

    fn create_album_index_snapshot_for_kind(
        &self,
        sort_field: &str,
        direction: &str,
        grouping_enabled: bool,
        snapshot_kind: &str,
    ) -> Result<IndexSnapshotInfo, DatabaseError> {
        let metric_sql = match sort_field {
            "title" => "0",
            "duration" => "g.duration_ms",
            "played" => "g.played_ms",
            "trackCount" => "g.track_count",
            _ => {
                return Err(DatabaseError::CorruptData(
                    "invalid index sort field".to_owned(),
                ))
            }
        };
        let snapshot_id = Uuid::new_v4().to_string();
        let mut connection = self.lock()?;
        let tx = connection.transaction()?;
        tx.execute(
            "DELETE FROM index_snapshot_rows WHERE kind=?1",
            [snapshot_kind],
        )?;
        tx.execute(
            "DELETE FROM index_snapshot_input WHERE kind=?1",
            [snapshot_kind],
        )?;
        let insert_sql = format!(
            "WITH raw_facets AS (
                SELECT COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, '') AS original,
                       music_trim_whitespace(COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, '')) AS album_name,
                       COUNT(DISTINCT t.track_id) AS track_count,
                       SUM(COALESCE(t.duration_ms, 0)) AS duration_ms,
                       SUM(COALESCE(s.played_ms, 0)) AS played_ms
                FROM tracks t
                LEFT JOIN track_playback_statistics s ON s.track_id=t.track_id
                WHERE EXISTS (SELECT 1 FROM source_mappings m JOIN library_roots r
                              ON r.source_id=m.source_id AND r.enabled=1 WHERE m.track_id=t.track_id)
                GROUP BY COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, '')
             ), mapped AS (
                SELECT f.original, f.album_name,
                       CASE WHEN f.album_name='' THEN 'album:unmarked'
                             ELSE 'album:' || music_identity_key(f.album_name) END AS identity_key,
                       f.track_count, f.duration_ms, f.played_ms
                FROM raw_facets f
             ), name_counts AS (
                SELECT identity_key, album_name,
                       SUM(track_count) AS track_count
                FROM mapped GROUP BY identity_key, album_name
             ), group_metrics AS (
                SELECT identity_key, SUM(track_count) AS track_count,
                       SUM(duration_ms) AS duration_ms, SUM(played_ms) AS played_ms,
                       MAX(CASE WHEN album_name='' THEN 1 ELSE 0 END) AS unmarked
                FROM mapped GROUP BY identity_key
             ), ranked_names AS (
                SELECT n.identity_key, n.album_name,
                       ROW_NUMBER() OVER (
                           PARTITION BY n.identity_key
                           ORDER BY n.track_count DESC,
                                     CASE WHEN music_identity_traditional(n.album_name)=n.album_name THEN 1 ELSE 0 END DESC,
                                    n.album_name COLLATE BINARY ASC
                       ) AS display_rank
                FROM name_counts n
             ), query_alias_values AS (
                SELECT DISTINCT identity_key, alias.value AS alias
                FROM mapped, json_each(json_array(original, album_name)) alias
             ), query_aliases_json AS (
                SELECT identity_key, json_group_array(alias) AS query_aliases
                FROM (SELECT identity_key, alias FROM query_alias_values ORDER BY identity_key, alias COLLATE BINARY)
                GROUP BY identity_key
             )
             INSERT INTO index_snapshot_input
                (snapshot_id, kind, identity_key, display_name, metric_primary, metric_secondary, payload_json)
             SELECT ?1, ?2, g.identity_key,
                    CASE WHEN g.unmarked<>0 THEN '未知' ELSE COALESCE(r.album_name, '未知') END,
                    {metric_sql}, 0,
                    json_object(
                        'key', g.identity_key,
                        'displayName', CASE WHEN g.unmarked<>0 THEN '未知' ELSE COALESCE(r.album_name, '未知') END,
                        'aliases', json(COALESCE(q.query_aliases, '[]')),
                        'trackCount', g.track_count,
                        'durationMs', g.duration_ms,
                        'playedMs', g.played_ms,
                        'unmarked', json(CASE WHEN g.unmarked<>0 THEN 'true' ELSE 'false' END)
                    )
             FROM group_metrics g
             LEFT JOIN ranked_names r ON r.identity_key=g.identity_key AND r.display_rank=1
             LEFT JOIN query_aliases_json q ON q.identity_key=g.identity_key",
        );
        tx.execute(&insert_sql, params![snapshot_id, snapshot_kind])?;
        let info = Self::materialize_index_snapshot(
            &tx,
            snapshot_kind,
            sort_field,
            direction,
            false,
            grouping_enabled,
            &snapshot_id,
        )?;
        tx.commit()?;
        Ok(info)
    }

    /// Read a paged album track list ordered by disc, track number, then natural title.
    pub fn list_album_tracks_page(
        &self,
        album_names: &[String],
        offset: u64,
        requested_limit: u32,
    ) -> Result<Page<TrackSummary>, DatabaseError> {
        self.list_album_tracks_page_sorted(album_names, None, offset, requested_limit)
    }

    pub fn list_album_tracks_page_sorted(
        &self,
        album_names: &[String],
        sort: Option<TrackListSort>,
        offset: u64,
        requested_limit: u32,
    ) -> Result<Page<TrackSummary>, DatabaseError> {
        self.list_album_tracks_page_with_index_sort(
            album_names,
            sort,
            None,
            offset,
            requested_limit,
        )
    }

    /// Build text quick-index group offsets for an album's current SQLite text ordering. Disc
    /// index groups are supplied separately by the album detail query when sorting by `#`.
    pub fn quick_index_for_album_tracks(
        &self,
        album_names: &[String],
        sort: TrackListSort,
    ) -> Result<QuickIndexSnapshot, DatabaseError> {
        if !matches!(
            sort.field,
            TrackSortField::Title | TrackSortField::Artist | TrackSortField::Album
        ) {
            return Ok(QuickIndexSnapshot::default());
        }
        let names = album_names.iter().cloned().collect::<HashSet<_>>();
        if names.is_empty() {
            return Ok(QuickIndexSnapshot::default());
        }
        let mut names = names.into_iter().collect::<Vec<_>>();
        names.sort();
        let names_json = serde_json::to_string(&names)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        let value_sql = match sort.field {
            TrackSortField::Title => "t.sort_title",
            TrackSortField::Artist => "COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, '')",
            TrackSortField::Album => "COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, '')",
            _ => unreachable!("non-text sort returned above"),
        };
        let sql = format!(
            "WITH matching AS ({ALBUM_TRACKS_PAGE_SQL}), ordered AS (
                SELECT ROW_NUMBER() OVER (ORDER BY {order_by}) - 1 AS position,
                       {MUSIC_QUICK_INDEX_FUNCTION}({value_sql}) AS group_label,
                       COUNT(*) OVER () AS total_count
                FROM tracks t JOIN matching m ON m.track_id=t.track_id
             ), transitions AS (
                SELECT position, group_label, total_count,
                       LAG(group_label) OVER (ORDER BY position) AS previous_label
                FROM ordered
             )
             SELECT position, group_label, total_count FROM transitions
             WHERE previous_label IS NULL OR previous_label <> group_label
             ORDER BY position",
            order_by = tracks_order_by_sql(sort),
        );
        let connection = self.lock()?;
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map([names_json], |row| {
            Ok((
                row.get::<_, i64>(0)?.max(0) as u64,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?.max(0) as u64,
            ))
        })?;
        let mut snapshot = QuickIndexSnapshot::default();
        for row in rows {
            let (before_index, label, total_count) = row?;
            snapshot.total_count = total_count;
            snapshot.push_group(QuickIndexGroup {
                label,
                before_index,
            });
        }
        Ok(snapshot)
    }

    pub fn list_album_tracks_page_with_index_sort(
        &self,
        album_names: &[String],
        sort: Option<TrackListSort>,
        index_sort: Option<TrackSortDirection>,
        offset: u64,
        requested_limit: u32,
    ) -> Result<Page<TrackSummary>, DatabaseError> {
        let unique_names = album_names.iter().cloned().collect::<HashSet<_>>();
        if unique_names.is_empty() {
            return Ok(Page {
                items: Vec::new(),
                offset,
                limit: query_limit(requested_limit),
                total_count: 0,
            });
        }
        let mut names = unique_names.into_iter().collect::<Vec<_>>();
        names.sort();
        let names_json = serde_json::to_string(&names)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        let connection = self.lock()?;
        let total_count = connection.query_row(
            "SELECT COUNT(DISTINCT t.track_id)
             FROM tracks t
             WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, '') COLLATE BINARY
                   IN (SELECT value FROM json_each(?1))
               AND EXISTS (SELECT 1 FROM source_mappings m JOIN library_roots r
                           ON r.source_id=m.source_id AND r.enabled=1 WHERE m.track_id=t.track_id)",
            [&names_json],
            |row| row.get::<_, i64>(0),
        )?.max(0) as u64;
        let limit = query_limit(requested_limit);
        let database_offset = i64::try_from(offset).unwrap_or(i64::MAX);
        let has_numbered_discs = if sort.is_none() && index_sort.is_some() {
            connection.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM tracks t
                     WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, '') COLLATE BINARY
                           IN (SELECT value FROM json_each(?1))
                       AND t.disc_number >= 2
                       AND EXISTS (SELECT 1 FROM source_mappings m JOIN library_roots r
                                   ON r.source_id=m.source_id AND r.enabled=1 WHERE m.track_id=t.track_id)
                 )",
                [&names_json],
                |row| row.get::<_, bool>(0),
            )?
        } else {
            false
        };
        let order_by = sort.map(tracks_order_by_sql).unwrap_or_else(|| match index_sort {
            Some(direction) if has_numbered_discs => {
                let disc_direction = if direction == TrackSortDirection::Desc { "DESC" } else { "ASC" };
                let track_direction = disc_direction;
                let title_direction = disc_direction;
                let unknown_direction = if direction == TrackSortDirection::Desc { "DESC" } else { "ASC" };
                format!(
                    "(t.disc_number IS NULL) {unknown_direction}, t.disc_number {disc_direction}, \
                     (t.track_number IS NULL) ASC, t.track_number {track_direction}, \
                     COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title, '') COLLATE MUSIC_LOCALE_NATURAL {title_direction}, \
                     t.track_id {title_direction}"
                )
            }
            Some(TrackSortDirection::Desc) => "(t.disc_number IS NULL) ASC, t.disc_number DESC, \
                (t.track_number IS NULL) ASC, t.track_number DESC, \
                COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title, '') COLLATE MUSIC_LOCALE_NATURAL DESC, t.track_id DESC".to_owned(),
            Some(TrackSortDirection::Asc) | None => "COALESCE(t.disc_number, 2147483647), COALESCE(t.track_number, 2147483647), \
                COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title, '') COLLATE MUSIC_LOCALE_NATURAL, t.track_id".to_owned(),
        });
        let sql = format!("{ALBUM_TRACKS_PAGE_SQL} ORDER BY {order_by} LIMIT ?2 OFFSET ?3");
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(
            params![names_json, limit, database_offset],
            row_to_track_summary,
        )?;
        let mut items = rows.collect::<Result<Vec<_>, _>>()?;
        attach_file_names(&connection, &mut items)?;
        Ok(Page {
            items,
            offset,
            limit,
            total_count,
        })
    }

    /// Return only the IDs needed to find album artwork. Unlike the detail page query, this does
    /// not count the album or materialize playback statistics and file names for every row.
    pub fn list_album_artwork_candidates(
        &self,
        album_names: &[String],
        offset: u64,
        requested_limit: u32,
    ) -> Result<Vec<TrackId>, DatabaseError> {
        let unique_names = album_names.iter().cloned().collect::<HashSet<_>>();
        if unique_names.is_empty() {
            return Ok(Vec::new());
        }
        let mut names = unique_names.into_iter().collect::<Vec<_>>();
        names.sort();
        let names_json = serde_json::to_string(&names)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        let limit = query_limit(requested_limit);
        let database_offset = i64::try_from(offset).unwrap_or(i64::MAX);
        let connection = self.lock()?;
        let mut statement = connection.prepare(ALBUM_ARTWORK_CANDIDATES_SQL)?;
        let rows = statement.query_map(params![names_json, limit, database_offset], |row| {
            row.get::<_, String>(0)
        })?;
        rows.map(|row| {
            let id = row?;
            TrackId::parse(&id).map_err(|error| DatabaseError::CorruptData(error.to_string()))
        })
        .collect()
    }

    /// Return artwork candidates and their enabled native locators in album display order.
    /// This is a backend-only scan input: callers must not page it through the UI.
    pub fn album_artwork_candidate_locators(
        &self,
        album_names: &[String],
    ) -> Result<Vec<(TrackId, Vec<MediaLocator>)>, DatabaseError> {
        let unique_names = album_names.iter().cloned().collect::<HashSet<_>>();
        if unique_names.is_empty() {
            return Ok(Vec::new());
        }
        let mut names = unique_names.into_iter().collect::<Vec<_>>();
        names.sort();
        let names_json = serde_json::to_string(&names)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT t.track_id, m.locator_kind, m.locator_encoding, m.locator_data
             FROM tracks t
             JOIN source_mappings m ON m.track_id=t.track_id
             JOIN library_roots r ON r.source_id=m.source_id AND r.enabled=1
             WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, '') COLLATE BINARY
                   IN (SELECT value FROM json_each(?1))
             ORDER BY COALESCE(t.disc_number, 2147483647),
                      COALESCE(t.track_number, 2147483647),
                      COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title, '') COLLATE MUSIC_LOCALE_NATURAL,
                      t.track_id,
                      CASE WHEN m.locator_kind='filesystem_path' THEN 0 ELSE 1 END,
                      r.source_id, m.source_item_id",
        )?;
        let mut rows = statement.query([names_json])?;
        let mut candidates: Vec<(TrackId, Vec<MediaLocator>)> = Vec::new();
        while let Some(row) = rows.next()? {
            let raw_id: String = row.get(0)?;
            let track_id = TrackId::parse(&raw_id)
                .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
            let kind: String = row.get(1)?;
            let encoding: String = row.get(2)?;
            let data: Vec<u8> = row.get(3)?;
            let locator = locator::decode(&kind, &encoding, &data)?;
            if let Some((last_id, locators)) = candidates.last_mut() {
                if *last_id == track_id {
                    locators.push(locator);
                    continue;
                }
            }
            candidates.push((track_id, vec![locator]));
        }
        Ok(candidates)
    }

    pub fn create_artist_index_snapshot(
        &self,
        sort_field: &str,
        direction: &str,
        grouping_enabled: bool,
    ) -> Result<IndexSnapshotInfo, DatabaseError> {
        self.create_artist_index_snapshot_for_kind(
            sort_field,
            direction,
            grouping_enabled,
            "artist",
        )
    }

    pub fn create_artist_navigation_snapshot(&self) -> Result<IndexSnapshotInfo, DatabaseError> {
        self.create_artist_index_snapshot_for_kind("title", "asc", false, "artist-navigation")
    }

    fn create_artist_index_snapshot_for_kind(
        &self,
        sort_field: &str,
        direction: &str,
        grouping_enabled: bool,
        snapshot_kind: &str,
    ) -> Result<IndexSnapshotInfo, DatabaseError> {
        let metric_sql = match sort_field {
            "title" => "0",
            "trackCount" => "m.track_count",
            "albumCount" => "m.album_count",
            "duration" => "m.played_ms",
            "trackDuration" => "m.duration_ms",
            _ => {
                return Err(DatabaseError::CorruptData(
                    "invalid index sort field".to_owned(),
                ))
            }
        };
        let snapshot_id = Uuid::new_v4().to_string();
        let mut connection = self.lock()?;
        let tx = connection.transaction()?;
        tx.execute(
            "DELETE FROM index_snapshot_rows WHERE kind=?1",
            [snapshot_kind],
        )?;
        tx.execute(
            "DELETE FROM index_snapshot_input WHERE kind=?1",
            [snapshot_kind],
        )?;
        let insert_sql = format!(
            "WITH visible AS (
                SELECT t.track_id,
                       COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist) AS artist,
                       COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) AS album,
                       COALESCE((SELECT played_ms FROM track_playback_statistics WHERE track_id=t.track_id), 0) AS played_ms,
                       COALESCE(t.duration_ms, 0) AS duration_ms
                FROM tracks t
                WHERE EXISTS (SELECT 1 FROM source_mappings m JOIN library_roots r
                              ON r.source_id=m.source_id AND r.enabled=1 WHERE m.track_id=t.track_id)
             ), expanded AS (
                SELECT v.track_id, COALESCE(a.value, '') AS artist_name,
                       CASE WHEN a.value IS NULL THEN 'artist:unmarked'
                            ELSE music_identity_key(a.value) END AS artist_key,
                       CASE WHEN v.album IS NULL OR trim(v.album) = '' THEN NULL
                            ELSE 'album:' || music_identity_key(v.album) END AS album_key,
                       v.played_ms, v.duration_ms
                FROM visible v
                LEFT JOIN json_each(music_split_artists_json(v.artist)) a ON trim(a.value)<>''
             ), artist_tracks AS (
                SELECT DISTINCT track_id, artist_key, album_key, played_ms, duration_ms FROM expanded
             ), artist_metrics AS (
                SELECT artist_key, COUNT(*) AS track_count,
                       COUNT(DISTINCT album_key) AS album_count,
                       SUM(played_ms) AS played_ms, SUM(duration_ms) AS duration_ms
                FROM artist_tracks GROUP BY artist_key
             ), artist_name_counts AS (
                SELECT artist_key, artist_name, COUNT(DISTINCT track_id) AS track_count
                FROM expanded GROUP BY artist_key, artist_name
             ), ranked_names AS (
                SELECT n.artist_key, n.artist_name, n.track_count,
                       ROW_NUMBER() OVER (
                           PARTITION BY n.artist_key
                           ORDER BY n.track_count DESC,
                                     CASE WHEN music_identity_traditional(n.artist_name)=n.artist_name THEN 1 ELSE 0 END DESC,
                                    n.artist_name COLLATE BINARY ASC
                       ) AS display_rank
                FROM artist_name_counts n
             ), names_json AS (
                SELECT n.artist_key,
                       json_group_array(json_object('name', n.artist_name, 'count', n.track_count)) AS names,
                       json_group_array(n.artist_name) AS artist_names
                FROM (SELECT artist_key, artist_name, track_count
                      FROM artist_name_counts ORDER BY artist_key, artist_name COLLATE BINARY) n
                GROUP BY n.artist_key
             )
             INSERT INTO index_snapshot_input
                (snapshot_id, kind, identity_key, display_name, metric_primary, metric_secondary, payload_json)
             SELECT ?1, ?2, m.artist_key,
                    CASE WHEN r.artist_name='' THEN '未知' ELSE r.artist_name END,
                    {metric_sql}, 0,
                    json_object(
                        'key', m.artist_key,
                        'names', json(j.names),
                        'artistNames', json(j.artist_names),
                        'displayName', CASE WHEN r.artist_name='' THEN '未知' ELSE r.artist_name END,
                        'trackCount', m.track_count,
                        'albumCount', m.album_count,
                        'playedMs', m.played_ms,
                        'durationMs', m.duration_ms
                    )
             FROM artist_metrics m
             JOIN names_json j ON j.artist_key=m.artist_key
             JOIN ranked_names r ON r.artist_key=m.artist_key AND r.display_rank=1"
        );
        tx.execute(&insert_sql, params![snapshot_id, snapshot_kind])?;
        let info = Self::materialize_index_snapshot(
            &tx,
            snapshot_kind,
            sort_field,
            direction,
            true,
            grouping_enabled,
            &snapshot_id,
        )?;
        tx.commit()?;
        Ok(info)
    }

    pub fn playlist_file_sync_state(
        &self,
        source_id: SourceId,
    ) -> Result<Option<PlaylistFileSyncState>, DatabaseError> {
        let connection = self.lock()?;
        let stored = connection
            .query_row(
                "SELECT playlist_id, locator_kind, locator_encoding, locator_data,
                        size_bytes, modified_at_utc_ms, content_sha256
                 FROM playlist_file_sync_state WHERE source_id=?1",
                [source_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Vec<u8>>(6)?,
                    ))
                },
            )
            .optional()?;
        stored
            .map(
                |(playlist_id, kind, encoding, data, size_bytes, modified_at_utc_ms, hash)| {
                    let playlist_id = PlaylistId::parse(&playlist_id)
                        .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
                    let size_bytes = u64::try_from(size_bytes).map_err(|_| {
                        DatabaseError::CorruptData("negative playlist size".to_owned())
                    })?;
                    let content_sha256: [u8; 32] = hash.try_into().map_err(|_| {
                        DatabaseError::CorruptData(
                            "playlist source digest is not a SHA-256 value".to_owned(),
                        )
                    })?;
                    Ok(PlaylistFileSyncState {
                        playlist_id,
                        locator: locator::decode(&kind, &encoding, &data)?,
                        fingerprint: FileFingerprint {
                            size_bytes,
                            modified_at_utc_ms,
                            created_at_utc_ms: None,
                        },
                        content_sha256,
                    })
                },
            )
            .transpose()
    }

    pub fn record_playlist_file_sync_state(
        &self,
        source_id: SourceId,
        state: &PlaylistFileSyncState,
    ) -> Result<(), DatabaseError> {
        let stored = locator::encode(&state.locator);
        let size_bytes = to_sql_i64(state.fingerprint.size_bytes, "playlist size_bytes")?;
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO playlist_file_sync_state
                (source_id, playlist_id, locator_kind, locator_encoding, locator_data,
                 size_bytes, modified_at_utc_ms, content_sha256)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(source_id) DO UPDATE SET
                playlist_id=excluded.playlist_id,
                locator_kind=excluded.locator_kind,
                locator_encoding=excluded.locator_encoding,
                locator_data=excluded.locator_data,
                size_bytes=excluded.size_bytes,
                modified_at_utc_ms=excluded.modified_at_utc_ms,
                content_sha256=excluded.content_sha256",
            params![
                source_id.to_string(),
                state.playlist_id.to_string(),
                stored.kind,
                stored.encoding,
                stored.data,
                size_bytes,
                state.fingerprint.modified_at_utc_ms,
                state.content_sha256.as_slice(),
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Remove only mappings owned by a playlist-file source. Other source mappings and the
    /// shared Track rows remain intact, so a track referenced by a folder or another playlist
    /// keeps its identity and library visibility.
    /// Every `(source_item_id, locator)` mapped under one source, for path migration.
    pub fn source_item_locators(
        &self,
        source_id: SourceId,
    ) -> Result<Vec<(String, MediaLocator)>, DatabaseError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT source_item_id, locator_kind, locator_encoding, locator_data
             FROM source_mappings WHERE source_id=?1 ORDER BY source_item_id",
        )?;
        let rows = statement.query_map([source_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (item_id, kind, encoding, data) = row?;
            Ok((item_id, locator::decode(&kind, &encoding, &data)?))
        })
        .collect()
    }

    /// Rewrite one source in place onto filesystem paths (Android content URI -> real path,
    /// ADR 0014). The library root becomes `root`; each `(old item ID, new item ID, path)` mapping
    /// keeps its TrackId, so play statistics, lyrics, overrides, playlist entries, and the saved
    /// session stay attached. Items without a rewrite (or whose new ID already exists) are left
    /// untouched for the next scan to reconcile as usual. The playlist-file fingerprint is cleared
    /// so a migrated playlist source is re-read from its new path. Returns the rewritten count.
    pub fn migrate_source_to_filesystem(
        &self,
        root: &LibraryRoot,
        rewrites: &[(String, String, std::path::PathBuf)],
    ) -> Result<usize, DatabaseError> {
        let stored_root = locator::encode(&root.locator);
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let source_id = root.id.to_string();
        let updated_root = tx.execute(
            "UPDATE library_roots
                SET source_kind=?2, display_name=?3, locator_kind=?4, locator_encoding=?5,
                    locator_data=?6, enabled=?7
              WHERE source_id=?1",
            params![
                source_id,
                source_kind_name(root.kind),
                root.display_name,
                stored_root.kind,
                stored_root.encoding,
                stored_root.data,
                i64::from(root.enabled),
            ],
        )?;
        if updated_root != 1 {
            return Err(DatabaseError::CorruptData(
                "migrated library root does not exist".to_owned(),
            ));
        }
        let mut rewritten = 0;
        for (old_item_id, new_item_id, path) in rewrites {
            if old_item_id == new_item_id {
                continue;
            }
            let stored = locator::encode(&MediaLocator::FileSystem(path.clone()));
            let taken: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM source_mappings WHERE source_id=?1 AND source_item_id=?2)",
                params![source_id, new_item_id],
                |row| row.get(0),
            )?;
            if taken {
                continue;
            }
            rewritten += tx.execute(
                "UPDATE source_mappings
                    SET source_item_id=?3, locator_key=?3, locator_kind=?4,
                        locator_encoding=?5, locator_data=?6
                  WHERE source_id=?1 AND source_item_id=?2",
                params![
                    source_id,
                    old_item_id,
                    new_item_id,
                    stored.kind,
                    stored.encoding,
                    stored.data,
                ],
            )?;
        }
        tx.execute(
            "DELETE FROM playlist_file_sync_state WHERE source_id=?1",
            [&source_id],
        )?;
        tx.commit()?;
        Ok(rewritten)
    }

    /// Write a consistent standalone copy of the database (used before source migrations).
    pub fn backup_into(&self, destination: &std::path::Path) -> Result<(), DatabaseError> {
        let destination = destination.to_str().ok_or_else(|| {
            DatabaseError::CorruptData("backup path is not valid UTF-8".to_owned())
        })?;
        let connection = self.lock()?;
        connection.execute("VACUUM INTO ?1", [destination])?;
        Ok(())
    }

    pub fn remove_playlist_file_source(&self, source_id: SourceId) -> Result<(), DatabaseError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM source_mappings WHERE source_id=?1",
            [source_id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM playlist_file_sync_state WHERE source_id=?1",
            [source_id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Resolve a unique TrackId from an already-normalized media locator. Windows callers may
    /// canonicalize filesystem paths first while preserving the original locator in playlist
    /// entries.
    pub fn resolve_track_id_for_locator(
        &self,
        locator_value: &MediaLocator,
    ) -> Result<Option<TrackId>, DatabaseError> {
        let connection = self.lock()?;
        let stored = locator::encode(locator_value);
        resolve_track_id_by_locator_connection(&connection, &stored)
    }

    /// Resolve many Windows paths through the indexed canonical locator key in one SQLite
    /// statement. This is used when combining the persistent and transient catalogs.
    #[cfg(windows)]
    pub fn resolve_track_ids_for_windows_paths(
        &self,
        paths: &[std::path::PathBuf],
    ) -> Result<Vec<Option<TrackId>>, DatabaseError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let keys = paths
            .iter()
            .map(|path| player_core::windows_locator_key(path.as_path()))
            .collect::<Vec<_>>();
        let keys_json = serde_json::to_string(&keys)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "WITH requested AS (
                 SELECT CAST(key AS INTEGER) AS ordinal, value AS locator_key
                 FROM json_each(?1)
             ), matches AS (
                 SELECT requested.ordinal, mappings.track_id
                 FROM requested
                 JOIN source_mappings AS mappings
                   ON mappings.locator_key=requested.locator_key
                 GROUP BY requested.ordinal, mappings.track_id
             )
             SELECT requested.ordinal,
                    CASE WHEN COUNT(matches.track_id)=1 THEN MIN(matches.track_id) END
             FROM requested
             LEFT JOIN matches USING (ordinal)
             GROUP BY requested.ordinal
             ORDER BY requested.ordinal",
        )?;
        let rows = statement.query_map([keys_json], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let mut resolved = vec![None; paths.len()];
        for row in rows {
            let (ordinal, value) = row?;
            let ordinal = usize::try_from(ordinal).map_err(|_| {
                DatabaseError::CorruptData("bulk locator query returned a negative index".into())
            })?;
            let slot = resolved.get_mut(ordinal).ok_or_else(|| {
                DatabaseError::CorruptData("bulk locator query returned an invalid index".into())
            })?;
            *slot = value.map(|value| parse_track_id(&value)).transpose()?;
        }
        Ok(resolved)
    }

    /// Save a playlist and replace its ordered entries atomically. Entries without an explicit
    /// TrackId are matched only when their exact stored locator maps to one internal TrackId.
    pub fn save_playlist(&self, playlist: &Playlist) -> Result<(), DatabaseError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO playlists (playlist_id, name) VALUES (?1, ?2)
             ON CONFLICT(playlist_id) DO UPDATE SET name=excluded.name",
            params![playlist.id.to_string(), playlist.name],
        )?;
        tx.execute(
            "DELETE FROM playlist_entries WHERE playlist_id=?1",
            [playlist.id.to_string()],
        )?;

        for (index, entry) in playlist.entries.iter().enumerate() {
            let position = i64::try_from(index)
                .map_err(|_| DatabaseError::InvalidNumber("playlist entry position"))?;
            let duration_ms = entry
                .duration_ms
                .map(|value| to_sql_i64(value, "playlist entry duration"))
                .transpose()?;
            let stored = locator::encode(&entry.locator);
            let track_id = match entry.track_id {
                Some(track_id) => {
                    let exists: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM tracks WHERE track_id=?1)",
                        [track_id.to_string()],
                        |row| row.get(0),
                    )?;
                    if !exists {
                        return Err(DatabaseError::TrackNotFound(track_id));
                    }
                    Some(track_id)
                }
                None => resolve_track_id_by_locator(&tx, &stored)?,
            };
            tx.execute(
                "INSERT INTO playlist_entries
                    (playlist_id, position, track_id, locator_kind, locator_encoding,
                     locator_data, title, duration_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    playlist.id.to_string(),
                    position,
                    track_id.map(|id| id.to_string()),
                    stored.kind,
                    stored.encoding,
                    stored.data,
                    entry.title,
                    duration_ms,
                ],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Load a playlist for backend export or editing. Native locators remain inside Rust.
    pub fn get_playlist(&self, playlist_id: PlaylistId) -> Result<Option<Playlist>, DatabaseError> {
        let connection = self.lock()?;
        let name = connection
            .query_row(
                "SELECT name FROM playlists WHERE playlist_id=?1",
                [playlist_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(name) = name else {
            return Ok(None);
        };

        let mut statement = connection.prepare_cached(
            "SELECT track_id, locator_kind, locator_encoding, locator_data, title, duration_ms
             FROM playlist_entries WHERE playlist_id=?1 ORDER BY position",
        )?;
        let rows = statement.query_map([playlist_id.to_string()], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<i64>>(5)?,
            ))
        })?;
        let entries = rows
            .map(|row| {
                let (track_id, kind, encoding, data, title, duration_ms) = row?;
                let locator = locator::decode(&kind, &encoding, &data)?;
                let track_id = track_id.map(|value| parse_track_id(&value)).transpose()?;
                let duration_ms = duration_ms
                    .map(|value| {
                        u64::try_from(value).map_err(|_| {
                            DatabaseError::CorruptData(
                                "playlist duration is negative in the database".to_owned(),
                            )
                        })
                    })
                    .transpose()?;
                Ok(PlaylistEntry {
                    track_id,
                    locator,
                    title,
                    duration_ms,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(Some(Playlist {
            id: playlist_id,
            name,
            entries,
        }))
    }

    /// Adopt a legacy static playlist only when one same-name candidate has the exact same
    /// ordered locator sequence. This avoids replacing unrelated same-name user playlists.
    pub fn unique_playlist_id_matching_locators(
        &self,
        name: &str,
        imported_locators: &[MediaLocator],
    ) -> Result<Option<PlaylistId>, DatabaseError> {
        let connection = self.lock()?;
        let candidate_ids = {
            let mut statement =
                connection.prepare("SELECT playlist_id FROM playlists WHERE name=?1")?;
            let rows = statement.query_map([name], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut matched = Vec::new();
        for candidate_id in candidate_ids {
            let mut statement = connection.prepare(
                "SELECT locator_kind, locator_encoding, locator_data
                 FROM playlist_entries WHERE playlist_id=?1 ORDER BY position",
            )?;
            let rows = statement.query_map([&candidate_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })?;
            let stored = rows.collect::<Result<Vec<_>, _>>()?;
            if stored.len() != imported_locators.len() {
                continue;
            }
            let mut same_ordered_locators = true;
            for ((kind, encoding, data), imported) in stored.iter().zip(imported_locators) {
                let existing = locator::decode(kind, encoding, data)?;
                if !locators_have_same_identity(&existing, imported) {
                    same_ordered_locators = false;
                    break;
                }
            }
            if same_ordered_locators {
                matched.push(
                    PlaylistId::parse(&candidate_id)
                        .map_err(|error| DatabaseError::CorruptData(error.to_string()))?,
                );
            }
            if matched.len() > 1 {
                return Ok(None);
            }
        }
        Ok(matched.pop())
    }

    /// List playlist names and counts without reading entry locators.
    pub fn list_playlists(&self) -> Result<Vec<PlaylistSummary>, DatabaseError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT p.playlist_id, p.name, COUNT(e.position)
             FROM playlists p
             LEFT JOIN playlist_entries e ON e.playlist_id=p.playlist_id
             GROUP BY p.playlist_id, p.name
             ORDER BY p.name COLLATE NOCASE, p.playlist_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (id, name, count) = row?;
            let id = PlaylistId::parse(&id)
                .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
            let entry_count = u64::try_from(count)
                .map_err(|_| DatabaseError::CorruptData("negative playlist count".to_owned()))?;
            Ok(PlaylistSummary {
                id,
                name,
                entry_count,
            })
        })
        .collect()
    }

    /// Return one IPC-safe window. Missing TrackIds are repaired from exact or normalized
    /// locator identity before paging; a locator resolves only to one unique internal TrackId.
    pub fn get_playlist_page(
        &self,
        playlist_id: PlaylistId,
        offset: u64,
        limit: u32,
    ) -> Result<Option<PlaylistPage>, DatabaseError> {
        self.get_playlist_page_sorted(playlist_id, offset, limit, PlaylistListSort::default())
    }

    /// Where `track_id` is in the playlist as the sorted pages list it (0-based; the first entry when the track
    /// appears several times), or `None` when the playlist does not contain it. One statement with the same entry
    /// resolution and ordering as the pages, numbered with `ROW_NUMBER()`.
    pub fn playlist_track_position(
        &self,
        playlist_id: PlaylistId,
        sort: PlaylistListSort,
        track_id: TrackId,
    ) -> Result<Option<u64>, DatabaseError> {
        let mut connection = self.lock()?;
        reconcile_unmatched_playlist_entries(&mut connection, playlist_id)?;
        let direction = match sort.direction {
            PlaylistSortDirection::Asc => "ASC",
            PlaylistSortDirection::Desc => "DESC",
        };
        let order_by = playlist_order_by(sort, direction);
        let (resolve, select_from) = PLAYLIST_PAGE_SQL
            .split_once("SELECT e.position, e.track_id,")
            .and_then(|(resolve, rest)| {
                rest.find("FROM resolved_entries e")
                    .map(|start| (resolve, &rest[start..]))
            })
            .ok_or_else(|| {
                DatabaseError::CorruptData("playlist page query changed shape".to_owned())
            })?;
        let sql = format!(
            "{resolve} SELECT idx FROM (
                 SELECT e.track_id AS tid, ROW_NUMBER() OVER (ORDER BY {order_by}) - 1 AS idx {select_from}
             ) WHERE tid = ?2 ORDER BY idx LIMIT 1"
        );
        let position = connection
            .query_row(
                &sql,
                params![playlist_id.to_string(), track_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        Ok(position.and_then(|value| u64::try_from(value).ok()))
    }

    pub fn get_playlist_page_sorted(
        &self,
        playlist_id: PlaylistId,
        offset: u64,
        limit: u32,
        sort: PlaylistListSort,
    ) -> Result<Option<PlaylistPage>, DatabaseError> {
        let mut connection = self.lock()?;
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM playlists WHERE playlist_id=?1)",
            [playlist_id.to_string()],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(None);
        }

        let limit = query_limit(limit);
        reconcile_unmatched_playlist_entries(&mut connection, playlist_id)?;
        let total_count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM playlist_entries WHERE playlist_id=?1",
            [playlist_id.to_string()],
            |row| row.get(0),
        )?;
        let direction = match sort.direction {
            PlaylistSortDirection::Asc => "ASC",
            PlaylistSortDirection::Desc => "DESC",
        };
        let order_by = playlist_order_by(sort, direction);
        let query = format!("{PLAYLIST_PAGE_SQL} ORDER BY {order_by} LIMIT ?2 OFFSET ?3");
        let mut statement = connection.prepare(&query)?;
        let rows = statement.query_map(
            params![
                playlist_id.to_string(),
                i64::from(limit),
                query_limit_i64(offset)
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                    row.get::<_, bool>(12)?,
                    row.get::<_, i64>(13)?,
                    row.get::<_, Option<i64>>(14)?,
                    row.get::<_, String>(15)?,
                    row.get::<_, String>(16)?,
                    row.get::<_, Vec<u8>>(17)?,
                ))
            },
        )?;
        let items = rows
            .map(|row| {
                let (
                    position,
                    track_id,
                    title,
                    artist,
                    album,
                    duration_ms,
                    codec,
                    bitrate_bps,
                    sample_rate_hz,
                    year,
                    release_date,
                    bit_depth,
                    has_enabled_mapping,
                    played_ms,
                    created_at_utc_ms,
                    locator_kind,
                    locator_encoding,
                    locator_data,
                ) = row?;
                let position = u64::try_from(position).map_err(|_| {
                    DatabaseError::CorruptData("negative playlist entry position".to_owned())
                })?;
                let track_id = track_id.map(|value| parse_track_id(&value)).transpose()?;
                let duration_ms = duration_ms
                    .map(|value| {
                        u64::try_from(value).map_err(|_| {
                            DatabaseError::CorruptData(
                                "playlist duration is negative in the database".to_owned(),
                            )
                        })
                    })
                    .transpose()?;
                let played_ms = u64::try_from(played_ms.max(0)).map_err(|_| {
                    DatabaseError::CorruptData("playlist played_ms is negative".to_owned())
                })?;
                let file_name = locator::decode(&locator_kind, &locator_encoding, &locator_data)
                    .ok()
                    .and_then(|locator| player_core::locator_display_file_stem(&locator));
                let release_date = blank_to_none(release_date)
                    .and_then(|value| player_core::parse_recording_date(&value));
                let year = year
                    .and_then(|value| u16::try_from(value).ok())
                    .filter(|value| *value > 0)
                    .or_else(|| {
                        release_date
                            .as_deref()
                            .and_then(player_core::recording_date_year)
                    });
                Ok(PlaylistEntrySummary {
                    position,
                    track_id,
                    title: blank_to_none(title),
                    artist,
                    album,
                    duration_ms,
                    has_enabled_mapping,
                    codec,
                    bitrate_bps: bitrate_bps.map(|value| value.max(0) as u32),
                    sample_rate_hz: sample_rate_hz.map(|value| value.max(0) as u32),
                    release_date,
                    year,
                    created_at_utc_ms,
                    bit_depth: bit_depth
                        .and_then(|value| u8::try_from(value).ok())
                        .filter(|value| *value > 0),
                    played_ms,
                    file_name,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(Some(PlaylistPage {
            items,
            offset,
            limit,
            total_count: u64::try_from(total_count)
                .map_err(|_| DatabaseError::CorruptData("negative playlist count".to_owned()))?,
        }))
    }

    /// Compute playlist text-group boundaries from its complete SQLite-ordered result in one
    /// query. This avoids reading the playlist in repeated UI-sized batches just to discover
    /// where each ICU quick-index group begins.
    pub fn quick_index_for_playlist(
        &self,
        playlist_id: PlaylistId,
        sort: PlaylistListSort,
    ) -> Result<Option<QuickIndexSnapshot>, DatabaseError> {
        if !matches!(
            sort.field,
            PlaylistSortField::Title | PlaylistSortField::Artist | PlaylistSortField::Album
        ) {
            return Ok(Some(QuickIndexSnapshot::default()));
        }

        let connection = self.lock()?;
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM playlists WHERE playlist_id=?1)",
            [playlist_id.to_string()],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(None);
        }

        let field = match sort.field {
            PlaylistSortField::Title => "title",
            PlaylistSortField::Artist => "artist",
            PlaylistSortField::Album => "album",
            _ => unreachable!("non-text playlist sort returned above"),
        };
        let direction = match sort.direction {
            PlaylistSortDirection::Asc => "ASC",
            PlaylistSortDirection::Desc => "DESC",
        };
        let sql = format!(
            "WITH playlist_rows AS ({PLAYLIST_PAGE_SQL}), ordered AS (
                 SELECT ROW_NUMBER() OVER (
                            ORDER BY (length(music_trim_whitespace(COALESCE({field}, ''))) = 0) {direction},
                                     COALESCE({field}, '') COLLATE MUSIC_LOCALE_NATURAL {direction},
                                     position ASC
                        ) - 1 AS position,
                        {MUSIC_QUICK_INDEX_FUNCTION}(COALESCE({field}, '')) AS group_label,
                        COUNT(*) OVER () AS total_count
                 FROM playlist_rows
             ), transitions AS (
                 SELECT position, group_label, total_count,
                        LAG(group_label) OVER (ORDER BY position) AS previous_label
                 FROM ordered
             )
             SELECT position, group_label, total_count FROM transitions
             WHERE previous_label IS NULL OR previous_label <> group_label
             ORDER BY position"
        );
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map([playlist_id.to_string()], |row| {
            Ok((
                row.get::<_, i64>(0)?.max(0) as u64,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?.max(0) as u64,
            ))
        })?;
        let mut snapshot = QuickIndexSnapshot::default();
        for row in rows {
            let (before_index, label, total_count) = row?;
            snapshot.total_count = total_count;
            snapshot.push_group(QuickIndexGroup {
                label,
                before_index,
            });
        }
        Ok(Some(snapshot))
    }

    pub fn delete_playlist(&self, playlist_id: PlaylistId) -> Result<bool, DatabaseError> {
        let connection = self.lock()?;
        Ok(connection.execute(
            "DELETE FROM playlists WHERE playlist_id=?1",
            [playlist_id.to_string()],
        )? > 0)
    }

    pub fn list_tracks_page(
        &self,
        request: ListTracksQuery,
    ) -> Result<Page<TrackSummary>, DatabaseError> {
        self.list_tracks_page_with_alternates(request, &[])
    }

    /// Search the library page across the original query and its script variants.
    pub fn list_tracks_page_with_alternates(
        &self,
        request: ListTracksQuery,
        query_alternates: &[String],
    ) -> Result<Page<TrackSummary>, DatabaseError> {
        let connection = self.lock()?;
        let search = request.query.filter(|text| !text.trim().is_empty());
        let limit = query_limit(request.limit);
        let search_value = search.as_deref();
        let filter = request.field_filter.as_ref();
        let total_count =
            count_tracks_filtered_in(&connection, search_value, query_alternates, filter)?;
        let items = fetch_tracks_window_filtered(
            &connection,
            search_value,
            query_alternates,
            filter,
            request.sort,
            request.offset,
            limit,
        )?;
        Ok(Page {
            items,
            offset: request.offset,
            limit,
            total_count: total_count.max(0) as u64,
        })
    }

    /// The 0-based position of `track_id` in the filtered and sorted library list (the order `list_tracks_page`
    /// pages through), or `None` when the track is not in that list. One SQLite statement: the same filter and
    /// ordering as the pages, numbered with `ROW_NUMBER()`.
    pub fn track_position(
        &self,
        request: &ListTracksQuery,
        query_alternates: &[String],
        track_id: TrackId,
    ) -> Result<Option<u64>, DatabaseError> {
        let connection = self.lock()?;
        let query_json = search_terms_json(
            request
                .query
                .as_deref()
                .filter(|text| !text.trim().is_empty()),
            query_alternates,
        )?;
        let (title_filter, artist_filter, album_filter) =
            field_filter_values(request.field_filter.as_ref())?;
        let order_by = tracks_order_by_sql(request.sort);
        // Everything after the select list (FROM and the filters) is shared with the page query.
        let from_where = TRACKS_PAGE_SELECT_SQL
            .find("\n FROM tracks t")
            .map(|start| &TRACKS_PAGE_SELECT_SQL[start..])
            .ok_or_else(|| {
                DatabaseError::CorruptData("track page query changed shape".to_owned())
            })?;
        let sql = format!(
            "SELECT position FROM (
                 SELECT t.track_id AS id, ROW_NUMBER() OVER (ORDER BY {order_by}) - 1 AS position{from_where}
             ) WHERE id = ?7"
        );
        // ?2 and ?3 (limit and offset of the page query) are not used here.
        let position = connection
            .query_row(
                &sql,
                params![
                    query_json,
                    0_i64,
                    0_i64,
                    title_filter,
                    artist_filter,
                    album_filter,
                    track_id.to_string()
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        Ok(position.map(|position| position.max(0) as u64))
    }

    /// Read the complete filtered and sorted result with one SQLite statement. This is reserved
    /// for backend snapshot construction (for example, merging the transient Explorer catalog);
    /// renderer-facing pages must continue to use `list_tracks_page_with_alternates`.
    pub fn list_tracks_all_with_alternates(
        &self,
        request: &ListTracksQuery,
        query_alternates: &[String],
    ) -> Result<Vec<TrackSummary>, DatabaseError> {
        let connection = self.lock()?;
        fetch_tracks_window_filtered_unbounded(
            &connection,
            request
                .query
                .as_deref()
                .filter(|text| !text.trim().is_empty()),
            query_alternates,
            request.field_filter.as_ref(),
            request.sort,
            0,
            i64::MAX,
        )
    }

    /// Read a backend-selected range without the renderer page-size cap. The caller still gets
    /// one SQLite query with filtering and ordering applied before LIMIT/OFFSET.
    pub fn list_tracks_window_with_alternates(
        &self,
        request: &ListTracksQuery,
        query_alternates: &[String],
        offset: u64,
        limit: u64,
    ) -> Result<Vec<TrackSummary>, DatabaseError> {
        let connection = self.lock()?;
        fetch_tracks_window_filtered_unbounded(
            &connection,
            request
                .query
                .as_deref()
                .filter(|text| !text.trim().is_empty()),
            query_alternates,
            request.field_filter.as_ref(),
            request.sort,
            offset,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
    }

    /// Filter, count, sort, and page an artist-specific list in SQLite. Artist membership uses
    /// the same delimiter parser as the artist index, avoiding a full candidate scan in Rust.
    pub fn list_tracks_page_for_artists(
        &self,
        request: ListTracksQuery,
        query_alternates: &[String],
        artist_names: &[String],
    ) -> Result<Page<TrackSummary>, DatabaseError> {
        let artist_names = artist_names
            .iter()
            .map(|name| name.trim())
            .collect::<Vec<_>>();
        if artist_names.is_empty() {
            return Ok(Page {
                items: Vec::new(),
                offset: request.offset,
                limit: query_limit(request.limit),
                total_count: 0,
            });
        }
        let connection = self.lock()?;
        let search = request.query.filter(|text| !text.trim().is_empty());
        let query_json = search_terms_json(search.as_deref(), query_alternates)?;
        let artist_json = serde_json::to_string(&artist_names)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        let total_count: i64 = connection.query_row(
            &format!("SELECT COUNT(*) FROM ({ARTIST_TRACKS_PREFIX_SQL})"),
            params![query_json, artist_json],
            |row| row.get(0),
        )?;
        let limit = query_limit(request.limit);
        let sql = format!(
            "{} ORDER BY {} LIMIT ?3 OFFSET ?4",
            ARTIST_TRACKS_PREFIX_SQL,
            tracks_order_by_sql(request.sort)
        );
        let mut statement = connection.prepare(&sql)?;
        let mut page = statement
            .query_map(
                params![
                    query_json,
                    artist_json,
                    i64::from(limit),
                    query_limit_i64(request.offset)
                ],
                row_to_track_summary,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        attach_file_names(&connection, &mut page)?;
        Ok(Page {
            items: page,
            offset: request.offset,
            limit,
            total_count: total_count.max(0) as u64,
        })
    }

    /// Build artist-detail grouping from the same exact artist membership and SQLite order used
    /// by its page query. The complete scan remains inside the database layer; the UI receives
    /// only group offsets.
    pub fn quick_index_for_artists(
        &self,
        query: Option<&str>,
        query_alternates: &[String],
        artist_names: &[String],
        sort: TrackListSort,
    ) -> Result<QuickIndexSnapshot, DatabaseError> {
        if !matches!(
            sort.field,
            TrackSortField::Title | TrackSortField::Artist | TrackSortField::Album
        ) {
            return Ok(QuickIndexSnapshot::default());
        }
        let artist_names = artist_names
            .iter()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>();
        if artist_names.is_empty() {
            return Ok(QuickIndexSnapshot::default());
        }
        let connection = self.lock()?;
        let query_json = search_terms_json(query, query_alternates)?;
        let artist_json = serde_json::to_string(&artist_names)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        let value_sql = match sort.field {
            TrackSortField::Title => "t.sort_title",
            TrackSortField::Artist => "COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, '')",
            TrackSortField::Album => "COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, '')",
            _ => unreachable!("non-text sort returned above"),
        };
        let sql = format!(
            "WITH matching AS ({ARTIST_TRACKS_PREFIX_SQL}), ordered AS (
                SELECT ROW_NUMBER() OVER (ORDER BY {order_by}) - 1 AS position,
                       {MUSIC_QUICK_INDEX_FUNCTION}({value_sql}) AS group_label,
                       COUNT(*) OVER () AS total_count
                FROM tracks t JOIN matching m ON m.track_id=t.track_id
             ), transitions AS (
                SELECT position, group_label, total_count,
                       LAG(group_label) OVER (ORDER BY position) AS previous_label
                FROM ordered
             )
             SELECT position, group_label, total_count FROM transitions
             WHERE previous_label IS NULL OR previous_label <> group_label
             ORDER BY position",
            order_by = tracks_order_by_sql(sort),
        );
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(params![query_json, artist_json], |row| {
            Ok((
                row.get::<_, i64>(0)?.max(0) as u64,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?.max(0) as u64,
            ))
        })?;
        let mut snapshot = QuickIndexSnapshot::default();
        for row in rows {
            let (before_index, label, total_count) = row?;
            snapshot.total_count = total_count;
            snapshot.push_group(QuickIndexGroup {
                label,
                before_index,
            });
        }
        Ok(snapshot)
    }

    /// Return stable IDs in exactly the same filter and order as library pages.
    /// Playback queue setup keeps these compact IDs inside Rust.
    pub fn list_track_ids(&self, query: Option<&str>) -> Result<Vec<TrackId>, DatabaseError> {
        self.list_track_ids_filtered(query, None, TrackListSort::default())
    }

    /// Return stable IDs using the same query, field filter, and sort as library pages.
    pub fn list_track_ids_filtered(
        &self,
        query: Option<&str>,
        field_filter: Option<&TrackFieldFilter>,
        sort: TrackListSort,
    ) -> Result<Vec<TrackId>, DatabaseError> {
        self.list_track_ids_filtered_with_alternates(query, &[], field_filter, sort)
    }

    /// Return stable IDs using the same union of query terms, field filter, and sort as library pages.
    pub fn list_track_ids_filtered_with_alternates(
        &self,
        query: Option<&str>,
        query_alternates: &[String],
        field_filter: Option<&TrackFieldFilter>,
        sort: TrackListSort,
    ) -> Result<Vec<TrackId>, DatabaseError> {
        let connection = self.lock()?;
        let query = query.filter(|text| !text.trim().is_empty());
        let query_json = search_terms_json(query, query_alternates)?;
        let (title_filter, artist_filter, album_filter) = field_filter_values(field_filter)?;
        let sql = format!(
            "SELECT t.track_id
             FROM tracks t
             WHERE EXISTS (SELECT 1 FROM source_mappings m JOIN library_roots r
                           ON r.source_id=m.source_id AND r.enabled=1 WHERE m.track_id=t.track_id)
               AND (?1 IS NULL OR EXISTS (
                    SELECT 1 FROM json_each(?1) term
                    WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) LIKE '%' || term.value || '%' COLLATE NOCASE
                       OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist) LIKE '%' || term.value || '%' COLLATE NOCASE
                       OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) LIKE '%' || term.value || '%' COLLATE NOCASE
                       OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist) LIKE '%' || term.value || '%' COLLATE NOCASE
               ))
               AND (?2 IS NULL OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) = ?2 COLLATE BINARY)
               AND (?3 IS NULL OR EXISTS (SELECT 1 FROM json_each(?3) requested_artist
                   WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, '') = requested_artist.value COLLATE BINARY
                      OR instr('|' || replace(replace(replace(replace(replace(replace(replace(replace(COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, ''), char(92), '|'), '/', '|'), ';', '|'), ',', '|'), '&', '|'), ' | ', '|'), '| ', '|'), ' |', '|') || '|', '|' || requested_artist.value || '|') > 0))
               AND (?4 IS NULL OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) = ?4 COLLATE BINARY)
             ORDER BY {}",
            tracks_order_by_sql(sort)
        );
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(
            params![query_json, title_filter, artist_filter, album_filter],
            |row| row.get::<_, String>(0),
        )?;
        let ids = rows
            .map(|row| {
                let value = row?;
                parse_track_id(&value)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Return playable playlist Track IDs with their original entry positions.
    /// Duplicate Track IDs remain separate queue entries; unmatched and disabled
    /// entries are omitted while the rest of the playlist remains ordered.
    pub fn playlist_track_ids(
        &self,
        playlist_id: PlaylistId,
    ) -> Result<Option<Vec<(u64, TrackId)>>, DatabaseError> {
        let mut connection = self.lock()?;
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM playlists WHERE playlist_id=?1)",
            [playlist_id.to_string()],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        reconcile_unmatched_playlist_entries(&mut connection, playlist_id)?;
        let mut statement = connection.prepare(
            "WITH resolved_entries AS (
                 SELECT pe.position,
                        CASE WHEN pe.track_id IS NOT NULL THEN pe.track_id
                             ELSE (
                                 SELECT CASE WHEN COUNT(DISTINCT sm.track_id)=1 THEN MIN(sm.track_id) END
                                 FROM source_mappings sm
                                 WHERE sm.locator_kind=pe.locator_kind
                                   AND sm.locator_encoding=pe.locator_encoding
                                   AND sm.locator_data=pe.locator_data
                             )
                        END AS track_id
                 FROM playlist_entries pe
                 WHERE pe.playlist_id=?1
             )
             SELECT e.position, e.track_id
             FROM resolved_entries e
             WHERE e.track_id IS NOT NULL
               AND EXISTS (
                   SELECT 1 FROM source_mappings m
                   JOIN library_roots r ON r.source_id=m.source_id
                   WHERE m.track_id=e.track_id AND r.enabled=1
               )
             ORDER BY e.position",
        )?;
        let rows = statement.query_map([playlist_id.to_string()], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (position, raw_id) = row?;
            let position = u64::try_from(position).map_err(|_| {
                DatabaseError::CorruptData("negative playlist entry position".to_owned())
            })?;
            Ok((position, parse_track_id(&raw_id)?))
        })
        .collect::<Result<Vec<_>, DatabaseError>>()
        .map(Some)
    }

    pub fn count_tracks(&self, query: Option<&str>) -> Result<u64, DatabaseError> {
        let connection = self.lock()?;
        let query = query.filter(|text| !text.trim().is_empty());
        Ok(count_tracks_in(&connection, query, &[])?.max(0) as u64)
    }

    pub fn list_tracks_window(
        &self,
        query: Option<&str>,
        offset: u64,
        limit: u32,
    ) -> Result<Vec<TrackSummary>, DatabaseError> {
        let connection = self.lock()?;
        let query = query.filter(|text| !text.trim().is_empty());
        fetch_tracks_window(&connection, query, offset, query_limit(limit))
    }

    /// Return the current user-facing metadata for one internal track ID.
    /// User overrides are applied just as they are for paginated library results.
    pub fn get_track_summary(
        &self,
        track_id: TrackId,
    ) -> Result<Option<TrackSummary>, DatabaseError> {
        let connection = self.lock()?;
        let mut summary = connection
            .query_row(
                TRACK_SUMMARY_SQL,
                [track_id.to_string()],
                row_to_track_summary,
            )
            .optional()?;
        if let Some(summary) = summary.as_mut() {
            attach_file_names(&connection, std::slice::from_mut(summary))?;
        }
        Ok(summary)
    }

    /// Resolve a bounded batch of Track IDs to their current user-facing metadata.
    /// The output retains the input order, duplicate IDs, and missing-track slots.
    pub fn get_track_summaries(
        &self,
        track_ids: &[TrackId],
    ) -> Result<Vec<Option<TrackSummary>>, DatabaseError> {
        let connection = self.lock()?;
        let mut summaries = {
            let mut statement = connection.prepare_cached(TRACK_SUMMARY_SQL)?;
            let collected = track_ids
                .iter()
                .map(|track_id| {
                    statement
                        .query_row([track_id.to_string()], row_to_track_summary)
                        .optional()
                        .map_err(DatabaseError::from)
                })
                .collect::<Result<Vec<_>, _>>()?;
            collected
        };
        let ids = summaries
            .iter()
            .filter_map(|item| item.as_ref().map(|summary| summary.id))
            .collect::<Vec<_>>();
        let names = file_names_for_tracks(&connection, &ids)?;
        for summary in summaries.iter_mut().flatten() {
            summary.file_name = names.get(&summary.id).cloned();
        }
        Ok(summaries)
    }

    /// Read the selected lyrics cache for one internal track identity.
    pub fn get_track_lyrics(
        &self,
        track_id: TrackId,
    ) -> Result<Option<TrackLyrics>, DatabaseError> {
        let connection = self.lock()?;
        let json = connection
            .query_row(
                "SELECT lyrics_json FROM track_lyrics WHERE track_id=?1",
                [track_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        json.map(|value| {
            let lyrics: TrackLyrics = serde_json::from_str(&value)
                .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
            if lyrics.track_id != track_id {
                return Err(DatabaseError::CorruptData(
                    "stored lyrics TrackId does not match its database key".to_owned(),
                ));
            }
            Ok(lyrics)
        })
        .transpose()
        .map(|lyrics| {
            lyrics.map(|mut cached| {
                repair_provider_lrc_cache(cached.provider, &mut cached.lyrics);
                cached
            })
        })
    }

    /// Save a network result unless a person has already selected lyrics for this track.
    /// Automatic cache entries are LRU-evicted once their serialized size exceeds the cap.
    pub fn save_automatic_track_lyrics(&self, lyrics: &TrackLyrics) -> Result<bool, DatabaseError> {
        if lyrics.manually_selected
            || !matches!(lyrics.provider, LyricProvider::NetEase | LyricProvider::Qq)
            || !lyrics.lyrics.synced
        {
            return Err(DatabaseError::CorruptData(
                "automatic lyric cache accepts only synced provider results".to_owned(),
            ));
        }
        let json = serde_json::to_string(lyrics)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        if json.len() > MAX_TRACK_LYRICS_JSON_BYTES {
            return Err(DatabaseError::CorruptData(
                "serialized lyrics exceed the supported size limit".to_owned(),
            ));
        }

        let payload_bytes = i64::try_from(json.len())
            .map_err(|_| DatabaseError::InvalidNumber("lyric payload bytes"))?;
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let manually_selected: Option<bool> = tx
            .query_row(
                "SELECT manually_selected FROM track_lyrics WHERE track_id=?1",
                [lyrics.track_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if manually_selected == Some(true) {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO track_lyrics
                 (track_id, lyrics_json, manually_selected, updated_at_utc_ms, payload_bytes)
             VALUES (?1, ?2, 0, ?3, ?4)
             ON CONFLICT(track_id) DO UPDATE SET
                 lyrics_json=excluded.lyrics_json,
                 manually_selected=0,
                 updated_at_utc_ms=excluded.updated_at_utc_ms,
                 payload_bytes=excluded.payload_bytes",
            params![
                lyrics.track_id.to_string(),
                json,
                lyrics.updated_at_utc_ms,
                payload_bytes,
            ],
        )?;
        prune_automatic_lyrics_cache(&tx)?;
        tx.commit()?;
        Ok(true)
    }

    /// Persist an explicit candidate selection. This row is protected from automatic updates
    /// and automatic-cache eviction.
    /// Remove persisted lyrics and cached candidates for one track.
    /// Sidecar/embedded files are left untouched. Callers that want to suppress rediscovery
    /// should write an empty manual sentinel after this delete.
    pub fn clear_track_lyrics(&self, track_id: TrackId) -> Result<bool, DatabaseError> {
        let connection = self.lock()?;
        let deleted = connection.execute(
            "DELETE FROM track_lyrics WHERE track_id=?1",
            [track_id.to_string()],
        )?;
        connection.execute(
            "DELETE FROM lyric_candidates WHERE track_id=?1",
            [track_id.to_string()],
        )?;
        Ok(deleted > 0)
    }

    pub fn save_manual_track_lyrics(&self, lyrics: &TrackLyrics) -> Result<(), DatabaseError> {
        let mut selected = lyrics.clone();
        selected.manually_selected = true;
        let json = serde_json::to_string(&selected)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        if json.len() > MAX_TRACK_LYRICS_JSON_BYTES {
            return Err(DatabaseError::CorruptData(
                "serialized lyrics exceed the supported size limit".to_owned(),
            ));
        }
        let payload_bytes = i64::try_from(json.len())
            .map_err(|_| DatabaseError::InvalidNumber("lyric payload bytes"))?;
        self.lock()?.execute(
            "INSERT INTO track_lyrics
                 (track_id, lyrics_json, manually_selected, updated_at_utc_ms, payload_bytes)
             VALUES (?1, ?2, 1, ?3, ?4)
             ON CONFLICT(track_id) DO UPDATE SET
                 lyrics_json=excluded.lyrics_json,
                 manually_selected=1,
                 updated_at_utc_ms=excluded.updated_at_utc_ms,
                 payload_bytes=excluded.payload_bytes",
            params![
                selected.track_id.to_string(),
                json,
                selected.updated_at_utc_ms,
                payload_bytes,
            ],
        )?;
        Ok(())
    }

    /// Keep candidate rows only long enough for the follow-up pick command. The cache is bounded
    /// by candidate count per track, serialized payload size per candidate, and total bytes.
    pub fn cache_lyric_candidates(
        &self,
        track_id: TrackId,
        candidates: &[LyricCandidate],
        fetched_at_utc_ms: i64,
    ) -> Result<(), DatabaseError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM lyric_candidates WHERE track_id=?1",
            [track_id.to_string()],
        )?;
        insert_lyric_candidate_rows(&tx, track_id, candidates, fetched_at_utc_ms)?;
        prune_lyric_candidate_cache(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// Add candidates from a later page without dropping lyrics already shown.
    pub fn append_lyric_candidates(
        &self,
        track_id: TrackId,
        candidates: &[LyricCandidate],
        fetched_at_utc_ms: i64,
    ) -> Result<(), DatabaseError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert_lyric_candidate_rows(&tx, track_id, candidates, fetched_at_utc_ms)?;
        prune_lyric_candidate_cache(&tx)?;
        tx.commit()?;
        Ok(())
    }
}

fn split_artist_names(value: Option<&str>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    value
        .split(['|', '\\', '/', ';', ',', '&'])
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .filter(|name| seen.insert((*name).to_owned()))
        .map(str::to_owned)
        .collect()
}

fn insert_lyric_candidate_rows(
    tx: &rusqlite::Transaction<'_>,
    track_id: TrackId,
    candidates: &[LyricCandidate],
    fetched_at_utc_ms: i64,
) -> Result<(), DatabaseError> {
    for candidate in candidates {
        let json = serde_json::to_string(candidate)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        if json.len() > MAX_LYRIC_CANDIDATE_JSON_BYTES {
            continue;
        }
        let payload_bytes = i64::try_from(json.len())
            .map_err(|_| DatabaseError::InvalidNumber("lyric candidate bytes"))?;
        tx.execute(
            "INSERT INTO lyric_candidates
                     (track_id, candidate_id, candidate_json, fetched_at_utc_ms, payload_bytes)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(track_id, candidate_id) DO UPDATE SET
                     candidate_json=excluded.candidate_json,
                     fetched_at_utc_ms=excluded.fetched_at_utc_ms,
                     payload_bytes=excluded.payload_bytes",
            params![
                track_id.to_string(),
                candidate.candidate_id,
                json,
                fetched_at_utc_ms,
                payload_bytes,
            ],
        )?;
    }
    Ok(())
}

impl Database {
    /// Resolve a candidate returned by an earlier search and make it the durable manual choice.
    pub fn select_lyric_candidate(
        &self,
        track_id: TrackId,
        candidate_id: &str,
        selected_at_utc_ms: i64,
    ) -> Result<TrackLyrics, DatabaseError> {
        let mut connection = self.lock()?;
        let candidate_json = connection
            .query_row(
                "SELECT candidate_json FROM lyric_candidates
                 WHERE track_id=?1 AND candidate_id=?2",
                params![track_id.to_string(), candidate_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| {
                DatabaseError::CorruptData("lyric candidate is missing or expired".to_owned())
            })?;
        let candidate: LyricCandidate = serde_json::from_str(&candidate_json)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        if candidate.candidate_id != candidate_id {
            return Err(DatabaseError::CorruptData(
                "stored candidate ID does not match its database key".to_owned(),
            ));
        }
        let mut lyrics = candidate.lyrics;
        repair_provider_lrc_cache(candidate.provider, &mut lyrics);
        let selected = TrackLyrics {
            track_id,
            provider: candidate.provider,
            candidate_id: Some(candidate.candidate_id),
            lyrics,
            manually_selected: true,
            updated_at_utc_ms: selected_at_utc_ms,
        };
        let json = serde_json::to_string(&selected)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        if json.len() > MAX_TRACK_LYRICS_JSON_BYTES {
            return Err(DatabaseError::CorruptData(
                "serialized lyrics exceed the supported size limit".to_owned(),
            ));
        }
        let payload_bytes = i64::try_from(json.len())
            .map_err(|_| DatabaseError::InvalidNumber("lyric payload bytes"))?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO track_lyrics
                 (track_id, lyrics_json, manually_selected, updated_at_utc_ms, payload_bytes)
             VALUES (?1, ?2, 1, ?3, ?4)
             ON CONFLICT(track_id) DO UPDATE SET
                 lyrics_json=excluded.lyrics_json,
                 manually_selected=1,
                 updated_at_utc_ms=excluded.updated_at_utc_ms,
                 payload_bytes=excluded.payload_bytes",
            params![
                track_id.to_string(),
                json,
                selected_at_utc_ms,
                payload_bytes
            ],
        )?;
        tx.commit()?;
        Ok(selected)
    }

    /// Return native locators for enabled mappings of a track, with filesystem paths first.
    /// Paths stay as `PathBuf` and are decoded using the platform-native representation.
    pub fn track_locators(&self, track_id: TrackId) -> Result<Vec<MediaLocator>, DatabaseError> {
        let connection = self.lock()?;
        let track_exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM tracks WHERE track_id=?1)",
            [track_id.to_string()],
            |row| row.get(0),
        )?;
        if !track_exists {
            return Err(DatabaseError::TrackNotFound(track_id));
        }

        let mut statement = connection.prepare_cached(
            "SELECT m.locator_kind, m.locator_encoding, m.locator_data
             FROM source_mappings m
             JOIN library_roots r ON r.source_id=m.source_id
             WHERE m.track_id=?1 AND r.enabled=1
             ORDER BY CASE WHEN m.locator_kind='filesystem_path' THEN 0 ELSE 1 END,
                      r.source_id, m.source_item_id",
        )?;
        let rows = statement.query_map([track_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (kind, encoding, data) = row?;
            locator::decode(&kind, &encoding, &data)
        })
        .collect()
    }

    /// Read enabled filesystem locators for many tracks in one SQLite query. Used by catalog
    /// union/deduplication paths so they do not turn a full-list merge into N+1 locator reads.
    pub fn filesystem_locators_for_tracks(
        &self,
        track_ids: &[TrackId],
    ) -> Result<Vec<(TrackId, std::path::PathBuf)>, DatabaseError> {
        if track_ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = track_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let ids_json = serde_json::to_string(&ids)
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT m.track_id, m.locator_kind, m.locator_encoding, m.locator_data
             FROM source_mappings m
             JOIN library_roots r ON r.source_id=m.source_id AND r.enabled=1
             WHERE m.locator_kind='filesystem_path'
               AND m.track_id IN (SELECT value FROM json_each(?1))
             ORDER BY m.track_id, r.source_id, m.source_item_id",
        )?;
        let rows = statement.query_map([ids_json], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (raw_id, kind, encoding, data) = row?;
            let track_id = TrackId::parse(&raw_id)
                .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
            match locator::decode(&kind, &encoding, &data)? {
                MediaLocator::FileSystem(path) => Ok((track_id, path)),
                MediaLocator::ContentUri(_) => Err(DatabaseError::CorruptData(
                    "filesystem locator row decoded as content URI".to_owned(),
                )),
            }
        })
        .collect()
    }

    /// Stable-enough cache identity for a track's artwork, invalidated by library file updates.
    /// Some providers do not expose a modification time; source ID, item ID, and file size still
    /// provide a stable cache key and avoid re-reading the original cover on every view.
    pub fn album_artwork_cache_identity(
        &self,
        track_id: TrackId,
    ) -> Result<Option<String>, DatabaseError> {
        let connection = self.lock()?;
        let identity = connection
            .query_row(
                "SELECT m.source_id, m.source_item_id, m.size_bytes, m.modified_at_utc_ms
                 FROM source_mappings m
                 JOIN library_roots r ON r.source_id=m.source_id AND r.enabled=1
                 WHERE m.track_id=?1
                 ORDER BY CASE WHEN m.locator_kind='filesystem_path' THEN 0 ELSE 1 END,
                          r.source_id, m.source_item_id
                 LIMIT 1",
                [track_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .optional()?;
        Ok(identity.map(|(source_id, item_id, size, modified)| {
            let modified =
                modified.map_or_else(|| "mtime-unavailable".to_owned(), |value| value.to_string());
            format!("{source_id}\0{item_id}\0{size}\0{modified}")
        }))
    }

    /// Select an existing regular filesystem file from this track's enabled source mappings.
    /// The returned path remains a native `PathBuf`; a later file open can still fail if the
    /// source changes after this check.
    pub fn resolve_playable_filesystem_locator(
        &self,
        track_id: TrackId,
    ) -> Result<std::path::PathBuf, DatabaseError> {
        let locators = self.track_locators(track_id)?;
        if locators.is_empty() {
            return Err(DatabaseError::NoEnabledTrackMapping(track_id));
        }

        let mut has_filesystem_locator = false;
        for locator in locators {
            let MediaLocator::FileSystem(path) = locator else {
                continue;
            };
            has_filesystem_locator = true;
            if std::fs::metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
                return Ok(path);
            }
        }

        if has_filesystem_locator {
            Err(DatabaseError::TrackFileUnavailable(track_id))
        } else {
            Err(DatabaseError::NonFilesystemTrackLocator(track_id))
        }
    }

    /// Resolve an enabled Android playable URI for this stable TrackId: a content URI, or a
    /// `file://` URI for a filesystem mapping (All files access, ADR 0014). The URI is returned
    /// for the current command only and must never be persisted as the playback-session identity.
    pub fn resolve_playable_content_uri(&self, track_id: TrackId) -> Result<String, DatabaseError> {
        let locators = self.track_locators(track_id)?;
        if locators.is_empty() {
            return Err(DatabaseError::NoEnabledTrackMapping(track_id));
        }
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut file_uri = None;
        for locator in locators {
            match locator {
                MediaLocator::ContentUri(uri) => return Ok(uri),
                #[cfg(unix)]
                MediaLocator::FileSystem(path) => {
                    if file_uri.is_none() && path.is_file() {
                        file_uri = player_core::file_uri_for_path(&path);
                    }
                }
                #[cfg(not(unix))]
                MediaLocator::FileSystem(_) => {}
            }
        }
        file_uri.ok_or(DatabaseError::NonContentUriTrackLocator(track_id))
    }

    pub fn set_user_override(
        &self,
        track_id: TrackId,
        field: UserMetadataField,
        value: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let field = override_field_name(field);
        if let Some(value) = value {
            tx.execute(
                "INSERT INTO track_overrides (track_id, field, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT(track_id, field) DO UPDATE SET value=excluded.value",
                params![track_id.to_string(), field, value],
            )?;
        } else {
            tx.execute(
                "DELETE FROM track_overrides WHERE track_id=?1 AND field=?2",
                params![track_id.to_string(), field],
            )?;
        }
        if field == "title" {
            let stored_title: Option<String> = tx.query_row(
                "SELECT title FROM tracks WHERE track_id=?1",
                [track_id.to_string()],
                |row| row.get(0),
            )?;
            let file_name = file_names_for_tracks(&tx, &[track_id])?
                .remove(&track_id)
                .unwrap_or_default();
            let sort_title = effective_sort_title(value, stored_title.as_deref(), Some(&file_name));
            tx.execute(
                "UPDATE tracks SET sort_title=?2 WHERE track_id=?1",
                params![track_id.to_string(), sort_title],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn sync_state(
        &self,
        source_id: SourceId,
    ) -> Result<Option<LibrarySyncState>, DatabaseError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT last_attempt_utc_ms, last_success_utc_ms, last_state, error_count, last_error
                 FROM library_sync_state WHERE source_id=?1",
                [source_id.to_string()],
                |row| {
                    Ok(LibrarySyncState {
                        last_attempt_utc_ms: row.get(0)?,
                        last_success_utc_ms: row.get(1)?,
                        state: row.get(2)?,
                        error_count: row.get::<_, i64>(3)?.max(0) as u64,
                        last_error: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(DatabaseError::from)
    }

    /// Replace the exact queue traversal and its playback checkpoint atomically.
    /// Queue entries are normalized so progress checkpoints never rewrite a large queue.
    pub fn save_playback_session(
        &self,
        checkpoint: &PlaybackSessionCheckpoint,
    ) -> Result<(), DatabaseError> {
        player_core::PlaybackQueue::restore(checkpoint.queue.clone())
            .map_err(|error| DatabaseError::CorruptData(error.to_owned()))?;
        let entry_count = i64::try_from(checkpoint.queue.entries.len())
            .map_err(|_| DatabaseError::InvalidNumber("playback queue length"))?;
        let cursor = i64::try_from(checkpoint.queue.cursor)
            .map_err(|_| DatabaseError::InvalidNumber("playback queue cursor"))?;
        let position_ms = i64::try_from(checkpoint.position_ms)
            .map_err(|_| DatabaseError::InvalidNumber("playback position"))?;
        let mut traversal = vec![0_i64; checkpoint.queue.entries.len()];
        for (rank, source_index) in checkpoint.queue.play_order.iter().copied().enumerate() {
            traversal[source_index] = i64::try_from(rank)
                .map_err(|_| DatabaseError::InvalidNumber("playback traversal rank"))?;
        }
        let mut shuffle_restore_order = vec![None; checkpoint.queue.entries.len()];
        if let Some(order) = &checkpoint.queue.shuffle_restore_order {
            for (rank, source_index) in order.iter().copied().enumerate() {
                shuffle_restore_order[source_index] = Some(
                    i64::try_from(rank)
                        .map_err(|_| DatabaseError::InvalidNumber("shuffle restore rank"))?,
                );
            }
        }
        let queue_sort_json = checkpoint
            .queue
            .queue_sort
            .map(|sort| serde_json::to_string(&sort))
            .transpose()
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        let (source_kind, source_playlist_id, source_query, source_field_filter_json) =
            match &checkpoint.queue.context {
                PlaybackQueueContext::Library {
                    query,
                    field_filter,
                    sort: _,
                } => (
                    "library",
                    None,
                    query.as_deref(),
                    field_filter
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()
                        .map_err(|error| DatabaseError::CorruptData(error.to_string()))?,
                ),
                PlaybackQueueContext::Playlist { playlist_id } => {
                    ("playlist", Some(playlist_id.as_str()), None, None)
                }
            };
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM playback_session_entries", [])?;
        tx.execute(
            "INSERT INTO playback_session(
                singleton_id, source_kind, source_playlist_id, source_query,
                source_field_filter_json, cursor, position_ms, shuffle, repeat_mode,
                random_state, entry_count, queue_sort_json
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(singleton_id) DO UPDATE SET
                source_kind=excluded.source_kind,
                source_playlist_id=excluded.source_playlist_id,
                source_query=excluded.source_query,
                source_field_filter_json=excluded.source_field_filter_json,
                cursor=excluded.cursor,
                position_ms=excluded.position_ms,
                shuffle=excluded.shuffle,
                repeat_mode=excluded.repeat_mode,
                random_state=excluded.random_state,
                entry_count=excluded.entry_count,
                queue_sort_json=excluded.queue_sort_json",
            params![
                source_kind,
                source_playlist_id,
                source_query,
                source_field_filter_json,
                cursor,
                position_ms,
                checkpoint.queue.shuffle,
                repeat_mode_name(checkpoint.queue.repeat),
                checkpoint.queue.random_state.to_string(),
                entry_count,
                queue_sort_json,
            ],
        )?;
        {
            let mut statement = tx.prepare_cached(
                "INSERT INTO playback_session_entries(
                    source_index, track_id, source_position, traversal_order, shuffle_restore_order
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (source_index, entry) in checkpoint.queue.entries.iter().enumerate() {
                statement.execute(params![
                    i64::try_from(source_index)
                        .map_err(|_| DatabaseError::InvalidNumber("playback source index"))?,
                    entry.track_id.to_string(),
                    entry
                        .source_position
                        .map(i64::try_from)
                        .transpose()
                        .map_err(|_| DatabaseError::InvalidNumber("playlist entry position"))?,
                    traversal[source_index],
                    shuffle_restore_order[source_index],
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Load without resolving any media locator; unavailable sources must not erase a session.
    pub fn load_playback_session(
        &self,
    ) -> Result<Option<PlaybackSessionCheckpoint>, DatabaseError> {
        let connection = self.lock()?;
        let header = connection
            .query_row(
                "SELECT source_kind, source_playlist_id, source_query, source_field_filter_json,
                        cursor, position_ms, queue_sort_json,
                        shuffle, repeat_mode, random_state, entry_count
                 FROM playback_session WHERE singleton_id=1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, bool>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, i64>(10)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            kind,
            playlist_id,
            query,
            field_filter_json,
            cursor,
            position_ms,
            queue_sort_json,
            shuffle,
            repeat,
            random,
            count,
        )) = header
        else {
            return Ok(None);
        };
        if count <= 0 || count > 1_000_000 || cursor < 0 || position_ms < 0 {
            return Err(DatabaseError::CorruptData(
                "playback session header is out of range".into(),
            ));
        }
        let context = match kind.as_str() {
            "library" if playlist_id.is_none() => PlaybackQueueContext::Library {
                query,
                field_filter: field_filter_json
                    .map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(|error| DatabaseError::CorruptData(error.to_string()))?,
                sort: Default::default(),
            },
            "playlist" if query.is_none() && field_filter_json.is_none() => {
                PlaybackQueueContext::Playlist {
                    playlist_id: playlist_id.ok_or_else(|| {
                        DatabaseError::CorruptData("playlist session has no playlist ID".into())
                    })?,
                }
            }
            _ => {
                return Err(DatabaseError::CorruptData(
                    "playback session source is invalid".into(),
                ))
            }
        };
        let queue_sort = queue_sort_json
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
        let repeat = parse_repeat_mode(&repeat)
            .ok_or_else(|| DatabaseError::CorruptData("playback repeat mode is invalid".into()))?;
        let random_state = random
            .parse::<u64>()
            .ok()
            .filter(|value| *value != 0)
            .ok_or_else(|| DatabaseError::CorruptData("playback random state is invalid".into()))?;
        let capacity = usize::try_from(count)
            .map_err(|_| DatabaseError::InvalidNumber("playback queue length"))?;
        let mut entries = Vec::with_capacity(capacity);
        let mut play_order = vec![usize::MAX; capacity];
        let mut shuffle_restore_order = vec![usize::MAX; capacity];
        let mut restore_order_rows = 0_usize;
        {
            let mut statement = connection.prepare_cached(
                "SELECT source_index, track_id, source_position, traversal_order, shuffle_restore_order
                 FROM playback_session_entries ORDER BY source_index",
            )?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let source_index = usize::try_from(row.get::<_, i64>(0)?).map_err(|_| {
                    DatabaseError::CorruptData("playback entry index is negative".into())
                })?;
                if source_index != entries.len() || source_index >= capacity {
                    return Err(DatabaseError::CorruptData(
                        "playback entry indices are incomplete".into(),
                    ));
                }
                let id = row.get::<_, String>(1)?;
                let track_id = TrackId::parse(&id).map_err(|error| {
                    DatabaseError::CorruptData(format!("invalid playback TrackId: {error}"))
                })?;
                let source_position = row
                    .get::<_, Option<i64>>(2)?
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| {
                        DatabaseError::CorruptData("playlist entry position is negative".into())
                    })?;
                let rank = usize::try_from(row.get::<_, i64>(3)?).map_err(|_| {
                    DatabaseError::CorruptData("playback traversal rank is negative".into())
                })?;
                if rank >= capacity || play_order[rank] != usize::MAX {
                    return Err(DatabaseError::CorruptData(
                        "playback traversal is invalid".into(),
                    ));
                }
                play_order[rank] = source_index;
                if let Some(restore_rank) = row.get::<_, Option<i64>>(4)? {
                    let restore_rank = usize::try_from(restore_rank).map_err(|_| {
                        DatabaseError::CorruptData("shuffle restore rank is negative".into())
                    })?;
                    if restore_rank >= capacity || shuffle_restore_order[restore_rank] != usize::MAX
                    {
                        return Err(DatabaseError::CorruptData(
                            "shuffle restore traversal is invalid".into(),
                        ));
                    }
                    shuffle_restore_order[restore_rank] = source_index;
                    restore_order_rows += 1;
                }
                entries.push(PlaybackQueueEntry {
                    track_id,
                    source_position,
                });
            }
        }
        if entries.len() != capacity || play_order.contains(&usize::MAX) {
            return Err(DatabaseError::CorruptData(
                "playback queue entries are incomplete".into(),
            ));
        }
        let shuffle_restore_order = match restore_order_rows {
            0 => None,
            rows if rows == capacity && !shuffle_restore_order.contains(&usize::MAX) => {
                Some(shuffle_restore_order)
            }
            _ => {
                return Err(DatabaseError::CorruptData(
                    "shuffle restore traversal is incomplete".into(),
                ))
            }
        };
        let queue = PlaybackQueueSnapshot {
            context,
            entries,
            play_order,
            shuffle_restore_order,
            cursor: usize::try_from(cursor)
                .map_err(|_| DatabaseError::CorruptData("playback cursor is negative".into()))?,
            shuffle,
            queue_sort,
            repeat,
            random_state,
        };
        player_core::PlaybackQueue::restore(queue.clone())
            .map_err(|error| DatabaseError::CorruptData(error.to_owned()))?;
        Ok(Some(PlaybackSessionCheckpoint {
            queue,
            position_ms: u64::try_from(position_ms)
                .map_err(|_| DatabaseError::InvalidNumber("playback position"))?,
        }))
    }

    /// Update only cursor and position after a track transition or bounded progress interval.
    pub fn checkpoint_playback_position(
        &self,
        queue: &PlaybackQueueSnapshot,
        position_ms: u64,
    ) -> Result<(), DatabaseError> {
        self.checkpoint_playback_position_after_read(queue, position_ms, || {})
    }

    fn checkpoint_playback_position_after_read(
        &self,
        queue: &PlaybackQueueSnapshot,
        position_ms: u64,
        after_read: impl FnOnce(),
    ) -> Result<(), DatabaseError> {
        player_core::PlaybackQueue::restore(queue.clone())
            .map_err(|error| DatabaseError::CorruptData(error.to_owned()))?;
        let position_ms = i64::try_from(position_ms)
            .map_err(|_| DatabaseError::InvalidNumber("playback position"))?;
        let cursor = i64::try_from(queue.cursor)
            .map_err(|_| DatabaseError::InvalidNumber("playback queue cursor"))?;
        let source_index = queue.play_order[queue.cursor];
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let persisted: Option<(String, Option<i64>, i64)> = tx
            .query_row(
                "SELECT track_id, source_position, traversal_order
                 FROM playback_session_entries WHERE source_index=?1",
                [i64::try_from(source_index)
                    .map_err(|_| DatabaseError::InvalidNumber("playback source index"))?],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        after_read();
        let expected = &queue.entries[source_index];
        let expected_position = expected
            .source_position
            .map(i64::try_from)
            .transpose()
            .map_err(|_| DatabaseError::InvalidNumber("playlist entry position"))?;
        if !persisted.is_some_and(|(track_id, source_position, rank)| {
            track_id == expected.track_id.to_string()
                && source_position == expected_position
                && usize::try_from(rank).ok() == Some(queue.cursor)
        }) {
            return Err(DatabaseError::CorruptData(
                "playback queue changed before progress checkpoint".into(),
            ));
        }
        let changed = tx.execute(
            "UPDATE playback_session SET cursor=?1, position_ms=?2 WHERE singleton_id=1 AND entry_count=?3",
            params![cursor, position_ms, queue.entries.len() as i64],
        )?;
        if changed != 1 {
            return Err(DatabaseError::CorruptData(
                "playback session disappeared before checkpoint".into(),
            ));
        }
        tx.commit()?;
        Ok(())
    }

    /// Register one process runtime before it records any listening checkpoint.
    /// Runtime IDs must be fresh UUIDs and must not be reused after finish.
    pub fn register_playback_statistics_runtime(
        &self,
        runtime_id: Uuid,
        owner_pid: Option<u32>,
        owner_process_started_utc_ms: Option<i64>,
    ) -> Result<(), DatabaseError> {
        let registered_at_utc_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| DatabaseError::CorruptData("system clock predates Unix epoch".into()))?
            .as_millis();
        let registered_at_utc_ms = i64::try_from(registered_at_utc_ms)
            .map_err(|_| DatabaseError::InvalidNumber("runtime registration time"))?;
        if owner_pid == Some(0) {
            return Err(DatabaseError::InvalidNumber("runtime owner PID"));
        }
        let owner_pid = owner_pid.map(i64::from);
        if owner_process_started_utc_ms.is_some_and(|started| started < 0) {
            return Err(DatabaseError::InvalidNumber("process start time"));
        }

        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO playback_statistics_runtimes
                (runtime_id, owner_pid, owner_process_started_utc_ms, registered_at_utc_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                runtime_id.to_string(),
                owner_pid,
                owner_process_started_utc_ms,
                registered_at_utc_ms
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Apply a batch of monotonic per-track counters atomically. The runtime
    /// must already be registered; retries at or below the stored watermark
    /// do not add listening time again.
    pub fn record_playback_checkpoints(
        &self,
        runtime_id: Uuid,
        checkpoints: &[PlaybackCheckpoint],
    ) -> Result<(), DatabaseError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        apply_playback_checkpoints(&tx, runtime_id, checkpoints)?;
        tx.commit()?;
        Ok(())
    }

    /// Commit the final batch and remove the runtime and its watermarks in one
    /// transaction. Checkpoints arriving after this returns are rejected.
    pub fn finish_playback_statistics_runtime(
        &self,
        runtime_id: Uuid,
        final_checkpoints: &[PlaybackCheckpoint],
    ) -> Result<(), DatabaseError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        apply_playback_checkpoints(&tx, runtime_id, final_checkpoints)?;
        let deleted = tx.execute(
            "DELETE FROM playback_statistics_runtimes WHERE runtime_id=?1",
            [runtime_id.to_string()],
        )?;
        if deleted != 1 {
            return Err(DatabaseError::UnknownPlaybackStatisticsRuntime(runtime_id));
        }
        tx.commit()?;
        Ok(())
    }

    /// List active runtime registrations so the integration layer can inspect
    /// and clean up sessions whose owning process is known to have exited.
    pub fn list_playback_statistics_runtimes(
        &self,
    ) -> Result<Vec<PlaybackStatisticsRuntime>, DatabaseError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT runtime_id, owner_pid, owner_process_started_utc_ms, registered_at_utc_ms
             FROM playback_statistics_runtimes ORDER BY registered_at_utc_ms, runtime_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (runtime_id, owner_pid, owner_process_started_utc_ms, registered_at_utc_ms) = row?;
            let runtime_id = Uuid::parse_str(&runtime_id)
                .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
            let owner_pid = owner_pid
                .map(|pid| {
                    u32::try_from(pid).map_err(|_| {
                        DatabaseError::CorruptData("runtime owner PID is out of range".into())
                    })
                })
                .transpose()?;
            if registered_at_utc_ms < 0 {
                return Err(DatabaseError::CorruptData(
                    "runtime registration time is negative".into(),
                ));
            }
            if owner_process_started_utc_ms.is_some_and(|started| started < 0) {
                return Err(DatabaseError::CorruptData(
                    "runtime owner process start time is negative".into(),
                ));
            }
            Ok(PlaybackStatisticsRuntime {
                runtime_id,
                owner_pid,
                owner_process_started_utc_ms,
                registered_at_utc_ms,
            })
        })
        .collect()
    }

    /// Return statistics for a queue's Track IDs with a bounded number of
    /// parameters per query. Duplicate queue entries share one TrackId row.
    pub fn playback_statistics_for(
        &self,
        track_ids: &[TrackId],
    ) -> Result<HashMap<TrackId, PlaybackStatistics>, DatabaseError> {
        const IDS_PER_QUERY: usize = 500;
        let mut statistics = HashMap::with_capacity(track_ids.len());
        for track_id in track_ids {
            statistics
                .entry(*track_id)
                .or_insert(PlaybackStatistics::default());
        }
        if statistics.is_empty() {
            return Ok(statistics);
        }

        let connection = self.lock()?;
        let unique_ids: Vec<TrackId> = statistics.keys().copied().collect();
        for chunk in unique_ids.chunks(IDS_PER_QUERY) {
            let ids: Vec<String> = chunk.iter().map(ToString::to_string).collect();
            let sql = format!(
                "WITH requested(track_id) AS (VALUES {})
                 SELECT requested.track_id, COALESCE(s.played_ms, 0),
                        CASE WHEN t.duration_ms > 0 THEN t.duration_ms
                             WHEN s.observed_duration_ms > 0 THEN s.observed_duration_ms
                             ELSE NULL END
                 FROM requested
                 LEFT JOIN tracks t ON t.track_id=requested.track_id
                 LEFT JOIN track_playback_statistics s ON s.track_id=requested.track_id",
                ids.iter()
                    .enumerate()
                    .map(|(index, _)| format!("(?{})", index + 1))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let mut statement = connection.prepare(&sql)?;
            let rows = statement.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            })?;
            for row in rows {
                let (track_id, played_ms, duration_ms) = row?;
                let track_id = parse_track_id(&track_id)?;
                let played_ms = u64::try_from(played_ms)
                    .map_err(|_| DatabaseError::CorruptData("negative played duration".into()))?;
                let duration_ms = duration_ms
                    .filter(|duration| *duration > 0)
                    .map(|duration| {
                        u64::try_from(duration).map_err(|_| {
                            DatabaseError::CorruptData("negative track duration".into())
                        })
                    })
                    .transpose()?;
                statistics.insert(
                    track_id,
                    PlaybackStatistics {
                        played_ms,
                        duration_ms,
                    },
                );
            }
        }
        Ok(statistics)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, DatabaseError> {
        match self.connection.lock() {
            Ok(connection) => Ok(connection),
            Err(poisoned) => {
                let connection = poisoned.into_inner();
                if !connection.is_autocommit() {
                    connection.execute_batch("ROLLBACK")?;
                }
                connection.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?;
                self.connection.clear_poison();
                Ok(connection)
            }
        }
    }
}

impl LibraryRepository for Database {
    type Error = DatabaseError;

    fn track_sync_state(
        &self,
        identity: &TrackIdentity,
    ) -> Result<Option<TrackSyncState>, Self::Error> {
        let connection = self.lock()?;
        let source_id = identity.source_id.to_string();
        let exact = {
            let mut statement = connection.prepare_cached(
                "SELECT m.size_bytes, m.modified_at_utc_ms, t.metadata_loaded, t.metadata_version
                 FROM source_mappings m JOIN tracks t ON t.track_id=m.track_id
                 WHERE m.source_id=?1 AND m.source_item_id=?2",
            )?;
            statement
                .query_row(params![source_id, identity.source_item_id], |row| {
                    row_to_sync_state(row)
                })
                .optional()?
        };
        if exact.is_some() || identity.locator_key.is_none() {
            return Ok(exact);
        }
        let mut statement = connection.prepare_cached(
            "SELECT m.size_bytes, m.modified_at_utc_ms, t.metadata_loaded, t.metadata_version
                 FROM source_mappings m JOIN tracks t ON t.track_id=m.track_id
                 WHERE m.locator_key=?1 LIMIT 1",
        )?;
        statement
            .query_row(
                [identity.locator_key.as_deref().unwrap_or_default()],
                row_to_sync_state,
            )
            .optional()
            .map_err(DatabaseError::from)
    }

    fn apply_source_scan(
        &mut self,
        root: &LibraryRoot,
        state: &SourceScanState,
        observed: &[MediaTrackRecord],
        changed: &[MediaTrackRecord],
        errors: &[MediaSourceError],
        synced_at_utc_ms: i64,
    ) -> Result<SyncApplyStats, Self::Error> {
        apply_source_scan_transaction(
            self,
            SyncApplyRequest {
                root,
                state,
                observed,
                changed,
                errors,
                synced_at_utc_ms,
            },
            &mut |_| {},
            &SyncCancellation::default(),
        )
        .map(|outcome| outcome.stats)
    }

    fn apply_metadata_backfill_batch(
        &mut self,
        tracks: &[MediaTrackRecord],
    ) -> Result<(), Self::Error> {
        if tracks.is_empty() {
            return Ok(());
        }
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for record in tracks {
            let Some(metadata) = record.metadata.as_ref() else {
                continue;
            };
            let track_id = resolve_track_id(&tx, record)?;
            let current_version: Option<i64> = tx
                .query_row(
                    "SELECT metadata_version FROM tracks WHERE track_id=?1",
                    [track_id.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            if current_version.is_some_and(|version| version >= i64::from(TRACK_METADATA_VERSION)) {
                continue;
            }
            let file_name = player_core::locator_display_file_stem(&record.locator);
            persist_metadata(&tx, track_id, Some(metadata), file_name.as_deref())?;
        }
        tx.commit()?;
        Ok(())
    }

    fn apply_source_scan_with_progress(
        &mut self,
        request: SyncApplyRequest<'_>,
        progress: &mut dyn FnMut(u64),
        cancellation: &SyncCancellation,
    ) -> Result<SyncApplyOutcome, Self::Error> {
        apply_source_scan_transaction(self, request, progress, cancellation)
    }
}

fn apply_source_scan_transaction(
    database: &Database,
    request: SyncApplyRequest<'_>,
    progress: &mut dyn FnMut(u64),
    cancellation: &SyncCancellation,
) -> Result<SyncApplyOutcome, DatabaseError> {
    let SyncApplyRequest {
        root,
        state,
        observed,
        changed,
        errors,
        synced_at_utc_ms,
    } = request;
    if cancellation.is_cancelled() {
        return Ok(SyncApplyOutcome {
            cancelled: true,
            ..SyncApplyOutcome::default()
        });
    }

    let changed_items = changed
        .iter()
        .map(|record| record.identity.source_item_id.as_str())
        .collect::<HashSet<_>>();
    let mut connection = database.lock()?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM sync_seen_items", [])?;
    let mut seen_statement = tx.prepare_cached(
        "INSERT OR IGNORE INTO sync_seen_items (source_id, source_item_id) VALUES (?1, ?2)",
    )?;

    for (index, record) in observed.iter().enumerate() {
        if cancellation.is_cancelled() {
            return Ok(SyncApplyOutcome {
                cancelled: true,
                ..SyncApplyOutcome::default()
            });
        }
        if record.identity.source_id != root.id {
            return Err(DatabaseError::SourceMismatch);
        }
        seen_statement.execute(params![root.id.to_string(), record.identity.source_item_id])?;

        let track_id = resolve_track_id(&tx, record)?;
        if changed_items.contains(record.identity.source_item_id.as_str()) {
            let file_name = player_core::locator_display_file_stem(&record.locator);
            persist_metadata(
                &tx,
                track_id,
                record.metadata.as_ref(),
                file_name.as_deref(),
            )?;
        } else {
            tx.execute(
                "INSERT OR IGNORE INTO tracks (track_id) VALUES (?1)",
                [track_id.to_string()],
            )?;
        }
        persist_mapping(&tx, root.id, track_id, record)?;
        progress((index + 1) as u64);
    }
    if state.allows_reconciliation() {
        for source_item_id in errors
            .iter()
            .filter_map(|error| error.source_item_id.as_deref())
        {
            // A complete source can still contain an individually missing/unreadable item.
            // Keep its old mapping while reconciling other successfully observed items.
            seen_statement.execute(params![root.id.to_string(), source_item_id])?;
        }
    }
    drop(seen_statement);

    if cancellation.is_cancelled() {
        return Ok(SyncApplyOutcome {
            cancelled: true,
            ..SyncApplyOutcome::default()
        });
    }

    let removed_source_mappings = if state.allows_reconciliation() {
        tx.execute(
            "DELETE FROM source_mappings
             WHERE source_id=?1 AND NOT EXISTS (
                SELECT 1 FROM sync_seen_items seen
                WHERE seen.source_id=source_mappings.source_id
                  AND seen.source_item_id=source_mappings.source_item_id
             )",
            [root.id.to_string()],
        )? as u64
    } else {
        0
    };

    let state_name = scan_state_name(state);
    let last_error = errors
        .first()
        .map(|error| error.message.as_str())
        .or(match state {
            SourceScanState::Incomplete { reason }
            | SourceScanState::Unavailable { reason }
            | SourceScanState::PermissionRevoked { reason } => Some(reason.as_str()),
            SourceScanState::Complete => None,
        });
    let last_success = state.allows_reconciliation().then_some(synced_at_utc_ms);
    tx.execute(
        "INSERT INTO library_sync_state
            (source_id, last_attempt_utc_ms, last_success_utc_ms, last_state, error_count, last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(source_id) DO UPDATE SET
            last_attempt_utc_ms=excluded.last_attempt_utc_ms,
            last_success_utc_ms=COALESCE(excluded.last_success_utc_ms, library_sync_state.last_success_utc_ms),
            last_state=excluded.last_state, error_count=excluded.error_count, last_error=excluded.last_error",
        params![
            root.id.to_string(),
            synced_at_utc_ms,
            last_success,
            state_name,
            errors.len() as i64,
            last_error,
        ],
    )?;
    if cancellation.is_cancelled() {
        return Ok(SyncApplyOutcome {
            cancelled: true,
            ..SyncApplyOutcome::default()
        });
    }
    tx.commit()?;
    Ok(SyncApplyOutcome {
        stats: SyncApplyStats {
            inserted_or_updated: changed_items.len() as u64,
            removed_source_mappings,
        },
        cancelled: false,
    })
}

fn count_tracks_in(
    connection: &Connection,
    query: Option<&str>,
    query_alternates: &[String],
) -> Result<i64, DatabaseError> {
    if let Some(query) = query {
        let query_json = search_terms_json(Some(query), query_alternates)?;
        Ok(connection.query_row(COUNT_SEARCH_SQL, [query_json], |row| row.get(0))?)
    } else {
        Ok(connection.query_row(COUNT_LIBRARY_SQL, [], |row| row.get(0))?)
    }
}

fn count_tracks_filtered_in(
    connection: &Connection,
    query: Option<&str>,
    query_alternates: &[String],
    field_filter: Option<&TrackFieldFilter>,
) -> Result<i64, DatabaseError> {
    if field_filter.is_none() {
        return count_tracks_in(connection, query, query_alternates);
    }
    let query_json = search_terms_json(query, query_alternates)?;
    let (title_filter, artist_filter, album_filter) = field_filter_values(field_filter)?;
    Ok(connection.query_row(
        COUNT_FIELD_FILTER_SQL,
        params![query_json, title_filter, artist_filter, album_filter],
        |row| row.get(0),
    )?)
}

fn fetch_tracks_window(
    connection: &Connection,
    query: Option<&str>,
    offset: u64,
    limit: u32,
) -> Result<Vec<TrackSummary>, DatabaseError> {
    fetch_tracks_window_filtered(
        connection,
        query,
        &[],
        None,
        TrackListSort::default(),
        offset,
        limit,
    )
}

fn tracks_order_by_sql(sort: TrackListSort) -> String {
    let asc = matches!(sort.direction, TrackSortDirection::Asc);
    match (sort.field, asc) {
        (TrackSortField::Title, true) => {
            "(length(trim(COALESCE(t.sort_title, ''), char(9)||char(10)||char(11)||char(12)||char(13)||' ')) = 0) ASC, \
             t.sort_title COLLATE MUSIC_LOCALE_NATURAL ASC, t.track_id ASC"
        }
        (TrackSortField::Title, false) => {
            "(length(trim(COALESCE(t.sort_title, ''), char(9)||char(10)||char(11)||char(12)||char(13)||' ')) = 0) DESC, \
             t.sort_title COLLATE MUSIC_LOCALE_NATURAL DESC, t.track_id DESC"
        }
        (TrackSortField::Artist, true) => {
            "(length(trim(COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, ''), char(9)||char(10)||char(11)||char(12)||char(13)||' ')) = 0) ASC, \
             COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist) COLLATE MUSIC_LOCALE_NATURAL ASC, t.track_id ASC"
        }
        (TrackSortField::Artist, false) => {
            "(length(trim(COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist, ''), char(9)||char(10)||char(11)||char(12)||char(13)||' ')) = 0) DESC, \
             COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist) COLLATE MUSIC_LOCALE_NATURAL DESC, t.track_id DESC"
        }
        (TrackSortField::Album, true) => {
            "(length(trim(COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, ''), char(9)||char(10)||char(11)||char(12)||char(13)||' ')) = 0) ASC, \
             COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) COLLATE MUSIC_LOCALE_NATURAL ASC, t.track_id ASC"
        }
        (TrackSortField::Album, false) => {
            "(length(trim(COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album, ''), char(9)||char(10)||char(11)||char(12)||char(13)||' ')) = 0) DESC, \
             COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) COLLATE MUSIC_LOCALE_NATURAL DESC, t.track_id DESC"
        }
        (TrackSortField::Year, true) => {
            "(t.release_date IS NULL OR length(trim(t.release_date))=0) ASC, t.release_date ASC, (t.year IS NULL) ASC, t.year ASC, t.track_id ASC"
        }
        (TrackSortField::Year, false) => {
            "(t.release_date IS NULL OR length(trim(t.release_date))=0) ASC, t.release_date DESC, (t.year IS NULL) ASC, t.year DESC, t.track_id DESC"
        }
        (TrackSortField::CreatedAt, true) => {
            "((SELECT MIN(m.created_at_utc_ms) FROM source_mappings m JOIN library_roots r ON r.source_id=m.source_id WHERE m.track_id=t.track_id AND r.enabled=1) IS NULL) ASC, (SELECT MIN(m.created_at_utc_ms) FROM source_mappings m JOIN library_roots r ON r.source_id=m.source_id WHERE m.track_id=t.track_id AND r.enabled=1) ASC, t.track_id ASC"
        }
        (TrackSortField::CreatedAt, false) => {
            "((SELECT MIN(m.created_at_utc_ms) FROM source_mappings m JOIN library_roots r ON r.source_id=m.source_id WHERE m.track_id=t.track_id AND r.enabled=1) IS NULL) ASC, (SELECT MIN(m.created_at_utc_ms) FROM source_mappings m JOIN library_roots r ON r.source_id=m.source_id WHERE m.track_id=t.track_id AND r.enabled=1) DESC, t.track_id DESC"
        }
        (TrackSortField::AudioFormat, true) => {
            return audio_format_order_by("ASC", "t.track_id");
        }
        (TrackSortField::AudioFormat, false) => {
            return audio_format_order_by("DESC", "t.track_id");
        }
        (TrackSortField::Duration, true) => {
            "(t.duration_ms IS NULL) ASC, t.duration_ms ASC, t.track_id ASC"
        }
        (TrackSortField::Duration, false) => {
            "(t.duration_ms IS NULL) ASC, t.duration_ms DESC, t.track_id DESC"
        }
        // Match the list "播放次數" = played_ms / duration_ms (unknown duration last).
        (TrackSortField::PlayCount, true) => {
            "(t.duration_ms IS NULL OR t.duration_ms <= 0) ASC, \
             CASE WHEN t.duration_ms IS NULL OR t.duration_ms <= 0 THEN 0 \
                  ELSE CAST(COALESCE((SELECT s.played_ms FROM track_playback_statistics s WHERE s.track_id=t.track_id), 0) AS REAL) / t.duration_ms \
             END ASC, t.track_id ASC"
        }
        (TrackSortField::PlayCount, false) => {
            "(t.duration_ms IS NULL OR t.duration_ms <= 0) ASC, \
             CASE WHEN t.duration_ms IS NULL OR t.duration_ms <= 0 THEN 0 \
                  ELSE CAST(COALESCE((SELECT s.played_ms FROM track_playback_statistics s WHERE s.track_id=t.track_id), 0) AS REAL) / t.duration_ms \
             END DESC, t.track_id DESC"
        }
    }
    .to_owned()
}

fn queue_sort_order_by(sort: TrackListSort) -> String {
    let direction = if sort.direction == TrackSortDirection::Asc {
        "ASC"
    } else {
        "DESC"
    };
    match sort.field {
        TrackSortField::Title => format!(
            "(length(trim(COALESCE(t.sort_title, ''))) = 0) {direction}, \
             t.sort_title COLLATE MUSIC_LOCALE_NATURAL {direction}, t.track_id {direction}"
        ),
        TrackSortField::Artist => format!(
            "(length(trim(COALESCE(t.artist, ''))) = 0) {direction}, \
             COALESCE(t.artist, '') COLLATE MUSIC_LOCALE_NATURAL {direction}, t.track_id {direction}"
        ),
        TrackSortField::Album => format!(
            "(length(trim(COALESCE(t.album, ''))) = 0) {direction}, \
             COALESCE(t.album, '') COLLATE MUSIC_LOCALE_NATURAL {direction}, t.track_id {direction}"
        ),
        TrackSortField::Year => format!(
            "(t.release_date IS NULL OR length(trim(t.release_date))=0) ASC, \
             t.release_date {direction}, (t.year IS NULL) ASC, t.year {direction}, t.track_id {direction}"
        ),
        TrackSortField::CreatedAt => format!(
            "(t.created_at_utc_ms IS NULL) ASC, t.created_at_utc_ms {direction}, t.track_id {direction}"
        ),
        TrackSortField::AudioFormat => audio_format_order_by(direction, "t.track_id"),
        TrackSortField::Duration => format!(
            "(t.duration_ms IS NULL) ASC, t.duration_ms {direction}, t.track_id {direction}"
        ),
        TrackSortField::PlayCount => format!(
            "(t.duration_ms IS NULL OR t.duration_ms <= 0) ASC, \
             CASE WHEN t.duration_ms IS NULL OR t.duration_ms <= 0 THEN 0 \
                  ELSE CAST(t.played_ms AS REAL) / t.duration_ms END {direction}, \
             t.track_id {direction}"
        ),
    }
}

fn fetch_tracks_window_filtered(
    connection: &Connection,
    query: Option<&str>,
    query_alternates: &[String],
    field_filter: Option<&TrackFieldFilter>,
    sort: TrackListSort,
    offset: u64,
    limit: u32,
) -> Result<Vec<TrackSummary>, DatabaseError> {
    fetch_tracks_window_filtered_unbounded(
        connection,
        query,
        query_alternates,
        field_filter,
        sort,
        offset,
        i64::from(limit),
    )
}

fn fetch_tracks_window_filtered_unbounded(
    connection: &Connection,
    query: Option<&str>,
    query_alternates: &[String],
    field_filter: Option<&TrackFieldFilter>,
    sort: TrackListSort,
    offset: u64,
    limit: i64,
) -> Result<Vec<TrackSummary>, DatabaseError> {
    let query_json = search_terms_json(query, query_alternates)?;
    let (title_filter, artist_filter, album_filter) = field_filter_values(field_filter)?;
    let order_by = tracks_order_by_sql(sort);
    let sql = format!(
        "{} ORDER BY {} LIMIT ?2 OFFSET ?3",
        TRACKS_PAGE_SELECT_SQL, order_by
    );
    let mut items = {
        let mut statement = connection.prepare(&sql)?;
        let collected = statement
            .query_map(
                params![
                    query_json,
                    limit.max(0),
                    query_limit_i64(offset),
                    title_filter,
                    artist_filter,
                    album_filter
                ],
                row_to_track_summary,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        collected
    };
    attach_file_names(connection, &mut items)?;
    Ok(items)
}

fn search_terms_json(
    query: Option<&str>,
    query_alternates: &[String],
) -> Result<Option<String>, DatabaseError> {
    let mut terms = Vec::<String>::new();
    if let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) {
        terms.push(query.to_owned());
    }
    for alternate in query_alternates.iter().take(8) {
        let alternate = alternate.trim();
        if !alternate.is_empty() && !terms.iter().any(|term| term == alternate) {
            terms.push(alternate.to_owned());
        }
    }
    if terms.is_empty() {
        return Ok(None);
    }
    serde_json::to_string(&terms)
        .map(Some)
        .map_err(|error| DatabaseError::CorruptData(error.to_string()))
}

type FieldFilterSqlValues = (Option<String>, Option<String>, Option<String>);

fn field_filter_values(
    filter: Option<&TrackFieldFilter>,
) -> Result<FieldFilterSqlValues, DatabaseError> {
    Ok(match filter {
        Some(TrackFieldFilter {
            field: TrackField::Title,
            value,
        }) => (Some(value.clone()), None, None),
        Some(TrackFieldFilter {
            field: TrackField::Artist,
            value,
        }) => (
            None,
            Some(
                serde_json::to_string(&split_artist_names(Some(value)))
                    .map_err(|error| DatabaseError::CorruptData(error.to_string()))?,
            ),
            None,
        ),
        Some(TrackFieldFilter {
            field: TrackField::Album,
            value,
        }) => (None, None, Some(value.clone())),
        None => (None, None, None),
    })
}

fn row_to_track_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrackSummary> {
    let raw_id: String = row.get(0)?;
    let id = TrackId::parse(&raw_id).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let release_date = blank_to_none(row.get::<_, Option<String>>(12)?)
        .and_then(|value| player_core::parse_recording_date(&value));
    let year = row
        .get::<_, Option<i64>>(11)?
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0)
        .or_else(|| {
            release_date
                .as_deref()
                .and_then(player_core::recording_date_year)
        });
    Ok(TrackSummary {
        id,
        title: blank_to_none(row.get(1)?),
        artist: row.get(2)?,
        album: row.get(3)?,
        album_artist: row.get(4)?,
        track_number: row
            .get::<_, Option<i64>>(5)?
            .map(|value| value.max(0) as u32),
        disc_number: row
            .get::<_, Option<i64>>(6)?
            .map(|value| value.max(0) as u32),
        duration_ms: row
            .get::<_, Option<i64>>(7)?
            .map(|value| value.max(0) as u64),
        codec: row.get(8)?,
        bitrate_bps: row
            .get::<_, Option<i64>>(9)?
            .map(|value| value.max(0) as u32),
        sample_rate_hz: row
            .get::<_, Option<i64>>(10)?
            .map(|value| value.max(0) as u32),
        release_date,
        year,
        created_at_utc_ms: row.get(15)?,
        bit_depth: row
            .get::<_, Option<i64>>(13)?
            .and_then(|value| u8::try_from(value).ok())
            .filter(|value| *value > 0),
        played_ms: row.get::<_, i64>(14)?.max(0) as u64,
        file_name: None,
    })
}

fn blank_to_none(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

fn attach_file_names(
    connection: &Connection,
    items: &mut [TrackSummary],
) -> Result<(), DatabaseError> {
    if items.is_empty() {
        return Ok(());
    }
    let ids = items.iter().map(|item| item.id).collect::<Vec<_>>();
    let names = file_names_for_tracks(connection, &ids)?;
    for item in items {
        item.file_name = names.get(&item.id).cloned();
    }
    Ok(())
}

fn file_names_for_tracks(
    connection: &Connection,
    track_ids: &[TrackId],
) -> Result<HashMap<TrackId, String>, DatabaseError> {
    if track_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let ids = track_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let ids_json = serde_json::to_string(&ids)
        .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
    let mut names = HashMap::new();
    let mut statement = connection.prepare(
        "SELECT m.track_id, m.locator_kind, m.locator_encoding, m.locator_data
         FROM source_mappings m
         JOIN library_roots r ON r.source_id = m.source_id
         WHERE m.track_id IN (SELECT value FROM json_each(?1))
         ORDER BY r.enabled DESC,
                  CASE m.locator_kind WHEN 'filesystem_path' THEN 0 ELSE 1 END,
                  m.source_id",
    )?;
    let mut rows = statement.query([ids_json])?;
    while let Some(row) = rows.next()? {
        let id = parse_track_id(&row.get::<_, String>(0)?)?;
        if names.contains_key(&id) {
            continue;
        }
        let kind: String = row.get(1)?;
        let encoding: String = row.get(2)?;
        let data: Vec<u8> = row.get(3)?;
        let Ok(locator) = locator::decode(&kind, &encoding, &data) else {
            continue;
        };
        if let Some(stem) = player_core::locator_display_file_stem(&locator) {
            names.insert(id, stem);
        }
    }
    Ok(names)
}

fn resolve_track_id(
    tx: &Transaction<'_>,
    record: &MediaTrackRecord,
) -> Result<TrackId, DatabaseError> {
    let exact = {
        let mut statement = tx.prepare_cached(
            "SELECT track_id FROM source_mappings WHERE source_id=?1 AND source_item_id=?2",
        )?;
        statement
            .query_row(
                params![
                    record.identity.source_id.to_string(),
                    record.identity.source_item_id
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?
    };
    let mapped = if exact.is_some() {
        exact
    } else if let Some(locator_key) = &record.identity.locator_key {
        {
            let mut statement = tx.prepare_cached(
                "SELECT track_id FROM source_mappings WHERE locator_key=?1 LIMIT 1",
            )?;
            statement
                .query_row([locator_key], |row| row.get::<_, String>(0))
                .optional()?
        }
    } else {
        None
    };
    match mapped {
        Some(value) => parse_track_id(&value),
        None => Ok(TrackId::new()),
    }
}

fn persist_metadata(
    tx: &Transaction<'_>,
    track_id: TrackId,
    metadata: Option<&TrackMetadata>,
    file_name: Option<&str>,
) -> Result<(), DatabaseError> {
    let Some(metadata) = metadata else {
        let user_title: Option<String> = tx
            .query_row(
                "SELECT value FROM track_overrides WHERE track_id=?1 AND field='title'",
                [track_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let stored_title: Option<String> = tx
            .query_row(
                "SELECT title FROM tracks WHERE track_id=?1",
                [track_id.to_string()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        let sort_title =
            effective_sort_title(user_title.as_deref(), stored_title.as_deref(), file_name);
        tx.prepare_cached(
            "INSERT INTO tracks (track_id, metadata_loaded, sort_title) VALUES (?1, 0, ?2)
             ON CONFLICT(track_id) DO UPDATE SET metadata_loaded=0, sort_title=excluded.sort_title",
        )?
        .execute(params![track_id.to_string(), sort_title])?;
        return Ok(());
    };

    let user_title: Option<String> = {
        let mut statement = tx.prepare_cached(
            "SELECT value FROM track_overrides WHERE track_id=?1 AND field='title'",
        )?;
        statement
            .query_row([track_id.to_string()], |row| row.get(0))
            .optional()?
    };
    let sort_title =
        effective_sort_title(user_title.as_deref(), metadata.title.as_deref(), file_name);
    let release_date = metadata
        .release_date
        .as_deref()
        .and_then(player_core::parse_recording_date)
        .or_else(|| {
            metadata
                .year
                .filter(|year| *year > 0 && *year <= 9999)
                .map(|year| format!("{year:04}"))
        });
    let year = release_date
        .as_deref()
        .and_then(player_core::recording_date_year)
        .or(metadata.year.filter(|value| *value > 0 && *value <= 9999));
    if metadata
        .year
        .is_some_and(|value| value == 0 || value > 9999)
    {
        return Err(DatabaseError::InvalidNumber("year"));
    }
    if metadata
        .bit_depth
        .is_some_and(|depth| depth == 0 || depth > 64)
    {
        return Err(DatabaseError::InvalidNumber("bit_depth"));
    }

    tx.prepare_cached(
        "INSERT INTO tracks
            (track_id, metadata_loaded, sort_title, title, artist, album, album_artist, track_number,
             disc_number, duration_ms, codec, bitrate_bps, sample_rate_hz, year, release_date, bit_depth, metadata_version)
         VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
         ON CONFLICT(track_id) DO UPDATE SET
            metadata_loaded=1, sort_title=excluded.sort_title, title=excluded.title,
            artist=excluded.artist, album=excluded.album,
            album_artist=excluded.album_artist, track_number=excluded.track_number,
            disc_number=excluded.disc_number, duration_ms=excluded.duration_ms, codec=excluded.codec,
            bitrate_bps=excluded.bitrate_bps, sample_rate_hz=excluded.sample_rate_hz,
            year=excluded.year, release_date=excluded.release_date, bit_depth=excluded.bit_depth,
            metadata_version=excluded.metadata_version",
    )?
    .execute(params![
            track_id.to_string(),
            sort_title,
            metadata.title,
            metadata.artist,
            metadata.album,
            metadata.album_artist,
            metadata.track_number.map(i64::from),
            metadata.disc_number.map(i64::from),
            metadata.duration_ms.map(|value| to_sql_i64(value, "duration_ms")).transpose()?,
            metadata.codec,
            metadata.bitrate_bps.map(i64::from),
            metadata.sample_rate_hz.map(i64::from),
            year.map(i64::from),
            release_date,
            metadata.bit_depth.map(i64::from),
            i64::from(TRACK_METADATA_VERSION),
        ])?;
    Ok(())
}

fn effective_sort_title(
    override_title: Option<&str>,
    metadata_title: Option<&str>,
    file_name: Option<&str>,
) -> String {
    override_title
        .filter(|value| !value.trim().is_empty())
        .or_else(|| metadata_title.filter(|value| !value.trim().is_empty()))
        .or_else(|| file_name.filter(|value| !value.trim().is_empty()))
        .unwrap_or_default()
        .to_lowercase()
}

fn persist_mapping(
    tx: &Transaction<'_>,
    source_id: SourceId,
    track_id: TrackId,
    record: &MediaTrackRecord,
) -> Result<(), DatabaseError> {
    let stored = locator::encode(&record.locator);
    tx.prepare_cached(
        "INSERT INTO source_mappings
            (source_id, source_item_id, locator_key, locator_kind, locator_encoding,
             locator_data, size_bytes, modified_at_utc_ms, created_at_utc_ms, track_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(source_id, source_item_id) DO UPDATE SET
            locator_key=excluded.locator_key, locator_kind=excluded.locator_kind,
            locator_encoding=excluded.locator_encoding, locator_data=excluded.locator_data,
            size_bytes=excluded.size_bytes, modified_at_utc_ms=excluded.modified_at_utc_ms,
            created_at_utc_ms=excluded.created_at_utc_ms,
            track_id=excluded.track_id
         WHERE source_mappings.locator_key IS NOT excluded.locator_key
            OR source_mappings.locator_kind IS NOT excluded.locator_kind
            OR source_mappings.locator_encoding IS NOT excluded.locator_encoding
            OR source_mappings.locator_data IS NOT excluded.locator_data
            OR source_mappings.size_bytes IS NOT excluded.size_bytes
            OR source_mappings.modified_at_utc_ms IS NOT excluded.modified_at_utc_ms
            OR source_mappings.created_at_utc_ms IS NOT excluded.created_at_utc_ms
            OR source_mappings.track_id IS NOT excluded.track_id",
    )?
    .execute(params![
        source_id.to_string(),
        record.identity.source_item_id,
        record.identity.locator_key,
        stored.kind,
        stored.encoding,
        stored.data,
        to_sql_i64(record.fingerprint.size_bytes, "size_bytes")?,
        record.fingerprint.modified_at_utc_ms,
        record.fingerprint.created_at_utc_ms,
        track_id.to_string(),
    ])?;
    Ok(())
}

fn row_to_sync_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrackSyncState> {
    Ok(TrackSyncState {
        fingerprint: FileFingerprint {
            size_bytes: row.get::<_, i64>(0)?.max(0) as u64,
            modified_at_utc_ms: row.get(1)?,
            created_at_utc_ms: None,
        },
        metadata_loaded: row.get(2)?,
        metadata_version: row.get::<_, i64>(3)?.max(0) as u32,
    })
}

fn apply_playback_checkpoints(
    tx: &Transaction<'_>,
    runtime_id: Uuid,
    checkpoints: &[PlaybackCheckpoint],
) -> Result<(), DatabaseError> {
    let runtime_key = runtime_id.to_string();
    let registered: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM playback_statistics_runtimes WHERE runtime_id=?1)",
        [&runtime_key],
        |row| row.get(0),
    )?;
    if !registered {
        return Err(DatabaseError::UnknownPlaybackStatisticsRuntime(runtime_id));
    }

    let mut seen = HashSet::with_capacity(checkpoints.len());
    for checkpoint in checkpoints {
        if !seen.insert(checkpoint.track_id) {
            return Err(DatabaseError::CorruptData(
                "a playback checkpoint batch contains a duplicate TrackId".into(),
            ));
        }
        let counter_ms = i64::try_from(checkpoint.played_ms)
            .map_err(|_| DatabaseError::InvalidNumber("played duration checkpoint"))?;
        let duration_ms = checkpoint
            .duration_ms
            .filter(|duration| *duration > 0)
            .map(|duration| {
                i64::try_from(duration)
                    .map_err(|_| DatabaseError::InvalidNumber("observed track duration"))
            })
            .transpose()?;
        let track_key = checkpoint.track_id.to_string();
        let previous_counter_ms: Option<i64> = tx
            .query_row(
                "SELECT played_ms FROM playback_statistics_checkpoints
                 WHERE runtime_id=?1 AND track_id=?2",
                params![runtime_key, track_key],
                |row| row.get(0),
            )
            .optional()?;
        let previous_counter_ms = previous_counter_ms.unwrap_or(0);
        if counter_ms < previous_counter_ms {
            return Err(DatabaseError::CorruptData(format!(
                "playback checkpoint for TrackId {} moved backwards",
                checkpoint.track_id
            )));
        }
        let delta_ms = counter_ms - previous_counter_ms;
        let previous_total_ms: Option<i64> = tx
            .query_row(
                "SELECT played_ms FROM track_playback_statistics WHERE track_id=?1",
                [&track_key],
                |row| row.get(0),
            )
            .optional()?;
        let total_ms = previous_total_ms
            .unwrap_or(0)
            .checked_add(delta_ms)
            .ok_or(DatabaseError::InvalidNumber("total played duration"))?;

        tx.execute(
            "INSERT INTO track_playback_statistics
                (track_id, played_ms, observed_duration_ms)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(track_id) DO UPDATE SET
                played_ms=excluded.played_ms,
                observed_duration_ms=COALESCE(
                    excluded.observed_duration_ms,
                    track_playback_statistics.observed_duration_ms
                )",
            params![track_key, total_ms, duration_ms],
        )?;
        tx.execute(
            "INSERT INTO playback_statistics_checkpoints (runtime_id, track_id, played_ms)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(runtime_id, track_id) DO UPDATE SET played_ms=excluded.played_ms",
            params![runtime_key, track_key, counter_ms],
        )?;
    }
    Ok(())
}

fn parse_track_id(value: &str) -> Result<TrackId, DatabaseError> {
    TrackId::parse(value).map_err(|error| DatabaseError::CorruptData(error.to_string()))
}

fn resolve_track_id_by_locator(
    tx: &Transaction<'_>,
    stored: &locator::StoredLocator,
) -> Result<Option<TrackId>, DatabaseError> {
    resolve_track_id_by_locator_connection(tx, stored)
}

fn locators_have_same_identity(left: &MediaLocator, right: &MediaLocator) -> bool {
    if left == right {
        return true;
    }
    #[cfg(windows)]
    if let (MediaLocator::FileSystem(left), MediaLocator::FileSystem(right)) = (left, right) {
        return player_core::windows_locator_key(left) == player_core::windows_locator_key(right);
    }
    false
}

fn resolve_track_id_by_locator_connection(
    connection: &Connection,
    stored: &locator::StoredLocator,
) -> Result<Option<TrackId>, DatabaseError> {
    let matched = connection.query_row(
        "SELECT CASE WHEN COUNT(DISTINCT track_id)=1 THEN MIN(track_id) END
         FROM source_mappings
         WHERE locator_kind=?1 AND locator_encoding=?2 AND locator_data=?3",
        params![stored.kind, stored.encoding, stored.data],
        |row| row.get::<_, Option<String>>(0),
    )?;
    if matched.is_some() {
        return matched.map(|value| parse_track_id(&value)).transpose();
    }

    #[cfg(windows)]
    if stored.kind == "filesystem_path" && stored.encoding == "windows_utf16le" {
        let MediaLocator::FileSystem(path) =
            locator::decode(stored.kind, stored.encoding, &stored.data)?
        else {
            return Ok(None);
        };
        let lookup_key = player_core::windows_locator_key(&path);
        let matched = connection.query_row(
            "SELECT CASE WHEN COUNT(DISTINCT track_id)=1 THEN MIN(track_id) END
             FROM source_mappings WHERE locator_key=?1",
            [lookup_key],
            |row| row.get::<_, Option<String>>(0),
        )?;
        return matched.map(|value| parse_track_id(&value)).transpose();
    }

    Ok(None)
}

/// Repair all unresolved entries in one playlist after exact or platform-normalized locator
/// matching finds one unique TrackId. Existing mappings and per-entry metadata stay as-is.
fn reconcile_unmatched_playlist_entries(
    connection: &mut Connection,
    playlist_id: PlaylistId,
) -> Result<(), DatabaseError> {
    let playlist_id_text = playlist_id.to_string();
    let has_unmatched = connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM playlist_entries
             WHERE playlist_id=?1 AND track_id IS NULL
         )",
        [playlist_id_text.as_str()],
        |row| row.get::<_, bool>(0),
    )?;
    if !has_unmatched {
        return Ok(());
    }

    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let candidates = {
        let mut statement = tx.prepare_cached(
            "SELECT position, locator_kind, locator_encoding, locator_data
             FROM playlist_entries
             WHERE playlist_id=?1 AND track_id IS NULL
             ORDER BY position",
        )?;
        let rows = statement.query_map([playlist_id_text.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })?;
        let candidates = rows.collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        candidates
    };

    for (position, kind, encoding, data) in candidates {
        let locator_value = locator::decode(&kind, &encoding, &data)?;
        let stored = locator::encode(&locator_value);
        let Some(track_id) = resolve_track_id_by_locator(&tx, &stored)? else {
            continue;
        };
        tx.execute(
            "UPDATE playlist_entries SET track_id=?1
             WHERE playlist_id=?2 AND position=?3 AND track_id IS NULL",
            params![track_id.to_string(), playlist_id_text, position],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn source_kind_name(kind: MediaSourceKind) -> &'static str {
    match kind {
        MediaSourceKind::WindowsSystemIndex => "windows_system_index",
        MediaSourceKind::WindowsFilesystem => "windows_filesystem",
        MediaSourceKind::PlaylistFile => "playlist_file",
        MediaSourceKind::AndroidMediaStore => "android_media_store",
        MediaSourceKind::AndroidSaf => "android_saf",
        MediaSourceKind::AndroidFilesystem => "android_filesystem",
        MediaSourceKind::Other => "other",
    }
}

fn parse_source_kind(value: &str) -> Result<MediaSourceKind, DatabaseError> {
    match value {
        "windows_system_index" => Ok(MediaSourceKind::WindowsSystemIndex),
        "windows_filesystem" => Ok(MediaSourceKind::WindowsFilesystem),
        "playlist_file" => Ok(MediaSourceKind::PlaylistFile),
        "android_media_store" => Ok(MediaSourceKind::AndroidMediaStore),
        "android_saf" => Ok(MediaSourceKind::AndroidSaf),
        "android_filesystem" => Ok(MediaSourceKind::AndroidFilesystem),
        "other" => Ok(MediaSourceKind::Other),
        _ => Err(DatabaseError::CorruptData(format!(
            "unknown media source kind: {value}"
        ))),
    }
}

fn scan_state_name(state: &SourceScanState) -> &'static str {
    match state {
        SourceScanState::Complete => "complete",
        SourceScanState::Incomplete { .. } => "incomplete",
        SourceScanState::Unavailable { .. } => "unavailable",
        SourceScanState::PermissionRevoked { .. } => "permission_revoked",
    }
}

fn override_field_name(field: UserMetadataField) -> &'static str {
    match field {
        UserMetadataField::Title => "title",
        UserMetadataField::Artist => "artist",
        UserMetadataField::Album => "album",
        UserMetadataField::AlbumArtist => "album_artist",
    }
}

fn query_limit(limit: u32) -> u32 {
    limit.clamp(1, MAX_PAGE_SIZE)
}

fn prune_automatic_lyrics_cache(tx: &Transaction<'_>) -> Result<(), DatabaseError> {
    loop {
        let cached_bytes: i64 = tx.query_row(
            "SELECT COALESCE(SUM(payload_bytes), 0) FROM track_lyrics WHERE manually_selected=0",
            [],
            |row| row.get(0),
        )?;
        if cached_bytes <= MAX_AUTOMATIC_LYRICS_CACHE_BYTES {
            return Ok(());
        }
        let removed = tx.execute(
            "DELETE FROM track_lyrics WHERE track_id=(
                 SELECT track_id FROM track_lyrics
                 WHERE manually_selected=0
                 ORDER BY updated_at_utc_ms ASC, track_id ASC LIMIT 1
             ) AND manually_selected=0",
            [],
        )?;
        if removed == 0 {
            return Ok(());
        }
    }
}

fn prune_lyric_candidate_cache(tx: &Transaction<'_>) -> Result<(), DatabaseError> {
    loop {
        let cached_bytes: i64 = tx.query_row(
            "SELECT COALESCE(SUM(payload_bytes), 0) FROM lyric_candidates",
            [],
            |row| row.get(0),
        )?;
        if cached_bytes <= MAX_LYRIC_CANDIDATE_CACHE_BYTES {
            return Ok(());
        }
        let removed = tx.execute(
            "DELETE FROM lyric_candidates WHERE (track_id, candidate_id)=(
                 SELECT track_id, candidate_id FROM lyric_candidates
                 ORDER BY fetched_at_utc_ms ASC, track_id ASC, candidate_id ASC LIMIT 1
             )",
            [],
        )?;
        if removed == 0 {
            return Ok(());
        }
    }
}

fn validate_theme_color(value: &str, field: &'static str) -> Result<(), DatabaseError> {
    let bytes = value.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' || !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err(DatabaseError::InvalidThemeColor(field));
    }
    Ok(())
}

fn query_limit_i64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn album_disc_quick_index_from_json(
    groups_json: &str,
    total_count: u64,
) -> Result<QuickIndexSnapshot, DatabaseError> {
    let groups: Vec<QuickIndexGroup> = serde_json::from_str(groups_json)
        .map_err(|error| DatabaseError::CorruptData(error.to_string()))?;
    let mut snapshot = QuickIndexSnapshot {
        total_count,
        ..QuickIndexSnapshot::default()
    };
    for group in groups {
        snapshot.push_group(group);
    }
    Ok(snapshot)
}

fn to_sql_i64(value: u64, field: &'static str) -> Result<i64, DatabaseError> {
    i64::try_from(value).map_err(|_| DatabaseError::InvalidNumber(field))
}

#[cfg(test)]
mod tests {
    use super::{AlbumDiscCount, AlbumIndexEntry, ArtistIndexEntry, QuickIndexGroup};
    use std::{
        fs,
        io::{Seek, SeekFrom, Write},
        path::{Path, PathBuf},
        process::{Command, Stdio},
        sync::mpsc,
        thread,
        time::{Duration, SystemTime},
    };

    use player_core::{
        parse_lrc, FileFingerprint, LibraryRoot, ListTracksQuery, LyricCandidate, LyricProvider,
        MediaIndex, MediaLocator, MediaSourceError, MediaSourceKind, MediaTrackRecord,
        PlaybackCheckpoint, PlaybackQueue, PlaybackQueueContext, PlaybackQueueEntry,
        PlaybackStatistics, Playlist, PlaylistEntry, PlaylistId, PlaylistListSort,
        PlaylistSortDirection, PlaylistSortField, QueueRepeatMode, SourceId, SourceScan,
        SourceScanState, SyncApplyRequest, SyncCancellation, SyncEngine, TrackField,
        TrackFieldFilter, TrackId, TrackIdentity, TrackListSort, TrackLyrics, TrackMetadata,
        TrackMetadataError, TrackSortDirection, TrackSortField, UserMetadataField,
        TRACK_METADATA_VERSION,
    };
    use rusqlite::{params, Connection, OptionalExtension};

    use super::{
        natural_title_cmp, query_limit, CollationType, Database, DatabaseError, LibraryRepository,
        MusicCollationSpec, Ordering, PlaybackSessionCheckpoint, PlaylistFileSyncState,
        QueueSortCandidate, ThemePreferences, COUNT_LIBRARY_SQL, SCHEMA_V1, SCHEMA_V2, SCHEMA_V3,
        SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7, SCHEMA_V8, SCHEMA_V9, TRACKS_PAGE_SQL,
    };

    fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
        let mut value = path.as_os_str().to_os_string();
        value.push(suffix);
        value.into()
    }

    #[test]
    fn renderer_page_size_is_capped_at_one_thousand() {
        assert_eq!(query_limit(0), 1);
        assert_eq!(query_limit(1000), 1000);
        assert_eq!(query_limit(1001), 1000);
        assert_eq!(query_limit(u32::MAX), 1000);
    }

    #[cfg(windows)]
    #[test]
    fn bulk_windows_locator_resolution_returns_ordered_unique_matches_in_one_query() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "bulk locator source",
        );
        let path = PathBuf::from(r"C:\Music\same.flac");
        let mut item = record(
            &root,
            "same",
            &player_core::windows_locator_key(&path),
            "same",
            10,
            100,
        );
        item.locator = MediaLocator::FileSystem(path.clone());
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&item),
            std::slice::from_ref(&item),
            1000,
        );
        let expected = db
            .resolve_track_id_for_locator(&MediaLocator::FileSystem(path.clone()))
            .expect("resolve path")
            .expect("track exists");

        let result = db
            .resolve_track_ids_for_windows_paths(&[
                path.clone(),
                PathBuf::from(r"C:\Music\missing.flac"),
                path,
            ])
            .expect("resolve paths in one query");
        assert_eq!(result, vec![Some(expected), None, Some(expected)]);
    }

    fn add_root(db: &Database, kind: MediaSourceKind, name: &str) -> LibraryRoot {
        db.add_library_root(
            kind,
            name,
            MediaLocator::FileSystem(PathBuf::from(format!(r"C:\Music\{name}"))),
        )
        .expect("add root")
    }

    #[test]
    fn poisoned_connection_mutex_does_not_permanently_disable_database() {
        let db = Database::open_in_memory().expect("open isolated database");
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let connection = db.connection.lock().expect("initial connection lock");
            connection
                .execute_batch("BEGIN IMMEDIATE")
                .expect("start transaction held across the simulated panic");
            panic!("simulate a panic while a database operation holds the connection");
        }));
        assert!(panic.is_err(), "fixture must poison the connection mutex");

        assert!(
            db.library_roots().is_ok(),
            "a valid SQLite connection should remain usable after a lock-holder panic"
        );
        assert!(
            db.lock()
                .expect("recovered connection lock")
                .is_autocommit(),
            "recovery must roll back a transaction left open by the panic"
        );
    }

    fn record(
        root: &LibraryRoot,
        source_item_id: &str,
        locator_key: &str,
        title: &str,
        size_bytes: u64,
        modified_at_utc_ms: i64,
    ) -> MediaTrackRecord {
        MediaTrackRecord {
            identity: TrackIdentity {
                source_id: root.id,
                source_item_id: source_item_id.to_owned(),
                locator_key: Some(locator_key.to_owned()),
            },
            locator: MediaLocator::FileSystem(PathBuf::from(format!(
                r"C:\Music\{source_item_id}.flac"
            ))),
            fingerprint: FileFingerprint {
                size_bytes,
                modified_at_utc_ms: Some(modified_at_utc_ms),
                created_at_utc_ms: None,
            },
            metadata: Some(TrackMetadata {
                title: Some(title.to_owned()),
                artist: Some("artist".to_owned()),
                album: Some("album".to_owned()),
                album_artist: None,
                track_number: Some(1),
                disc_number: Some(1),
                duration_ms: Some(215_321),
                codec: Some("FLAC".to_owned()),
                bitrate_bps: Some(900_000),
                sample_rate_hz: Some(48_000),
                release_date: Some("2001".to_owned()),
                year: Some(2001),
                bit_depth: Some(24),
            }),
        }
    }

    fn apply(
        db: &mut Database,
        root: &LibraryRoot,
        state: SourceScanState,
        observed: &[MediaTrackRecord],
        changed: &[MediaTrackRecord],
        at: i64,
    ) {
        db.apply_source_scan(root, &state, observed, changed, &[], at)
            .expect("apply source scan");
    }

    #[cfg(unix)]
    #[test]
    fn android_source_migration_to_paths_keeps_track_ids_and_rescans_onto_them() {
        let mut db = Database::open_in_memory().expect("database");
        let mut root = db
            .add_library_root(
                MediaSourceKind::AndroidMediaStore,
                "共用儲存空間",
                MediaLocator::ContentUri("content://media/external_primary/audio/media".into()),
            )
            .expect("add MediaStore root");
        let mut legacy = record(&root, "external_primary:7", "unused", "Song", 10, 1_000);
        legacy.identity.locator_key = None;
        legacy.locator =
            MediaLocator::ContentUri("content://media/external_primary/audio/media/7".into());
        let mut collision = record(&root, "external_primary:8", "unused", "Dup", 10, 1_000);
        collision.identity.locator_key = None;
        collision.locator =
            MediaLocator::ContentUri("content://media/external_primary/audio/media/8".into());
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &[legacy.clone(), collision.clone()],
            &[legacy.clone(), collision.clone()],
            1,
        );
        let track_id = db
            .resolve_track_id_for_locator(&legacy.locator)
            .unwrap()
            .expect("legacy track");

        let path = PathBuf::from("/storage/emulated/0/Music/Song.flac");
        let key = player_core::unix_path_key(&path);
        root.kind = MediaSourceKind::AndroidFilesystem;
        root.locator = MediaLocator::FileSystem(PathBuf::from("/storage/emulated/0/Music"));
        let rewritten = db
            .migrate_source_to_filesystem(
                &root,
                &[
                    ("external_primary:7".into(), key.clone(), path.clone()),
                    // Same target path: must not steal or merge the first mapping.
                    ("external_primary:8".into(), key.clone(), path.clone()),
                ],
            )
            .expect("migrate");
        assert_eq!(rewritten, 1);
        let roots = db.library_roots().unwrap();
        let migrated = roots.iter().find(|item| item.id == root.id).unwrap();
        assert_eq!(migrated.kind, MediaSourceKind::AndroidFilesystem);
        assert_eq!(
            db.resolve_track_id_for_locator(&MediaLocator::FileSystem(path.clone()))
                .unwrap(),
            Some(track_id)
        );

        // A path scan of the same file updates that mapping instead of creating a new TrackId.
        let mut rescanned = record(&root, &key, &key, "Song", 11, 2_000);
        rescanned.locator = MediaLocator::FileSystem(path.clone());
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&rescanned),
            std::slice::from_ref(&rescanned),
            2,
        );
        assert_eq!(
            db.resolve_track_id_for_locator(&MediaLocator::FileSystem(path))
                .unwrap(),
            Some(track_id)
        );
        assert!(db.get_track_summary(track_id).unwrap().is_some());
    }

    #[test]
    fn album_artwork_cache_identity_changes_with_library_file_fingerprint() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "album-art-cache");
        let first = record(&root, "one", "one", "One", 100, 1_000);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&first),
            std::slice::from_ref(&first),
            2,
        );
        let track_id = db
            .resolve_track_id_for_locator(&first.locator)
            .expect("resolve track")
            .expect("track exists");
        let initial = db
            .album_artwork_cache_identity(track_id)
            .expect("cache identity")
            .expect("fingerprinted mapping");

        let updated = record(&root, "one", "one", "One", 101, 2_000);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&updated),
            std::slice::from_ref(&updated),
            3,
        );
        let changed = db
            .album_artwork_cache_identity(track_id)
            .expect("updated cache identity")
            .expect("fingerprinted mapping");
        assert_ne!(
            initial, changed,
            "a rescanned file must not reuse its stale cover thumbnail"
        );
    }

    #[test]
    fn album_artwork_cache_identity_is_stable_when_file_mtime_is_unavailable() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "album-art-cache-no-mtime",
        );
        let mut first = record(&root, "one", "one", "One", 100, 1_000);
        first.fingerprint.modified_at_utc_ms = None;
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&first),
            std::slice::from_ref(&first),
            2,
        );
        let track_id = db
            .resolve_track_id_for_locator(&first.locator)
            .expect("resolve track")
            .expect("track exists");
        let initial = db
            .album_artwork_cache_identity(track_id)
            .expect("cache identity")
            .expect("source mapping remains cacheable without mtime");

        let mut same_file = first.clone();
        same_file.fingerprint.modified_at_utc_ms = None;
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&same_file),
            &[],
            3,
        );
        assert_eq!(
            db.album_artwork_cache_identity(track_id)
                .expect("stable cache identity")
                .as_deref(),
            Some(initial.as_str()),
            "missing mtime must not turn every thumbnail lookup into a cache miss"
        );

        let mut resized_file = same_file;
        resized_file.fingerprint.size_bytes = 101;
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&resized_file),
            std::slice::from_ref(&resized_file),
            4,
        );
        assert_ne!(
            initial,
            db.album_artwork_cache_identity(track_id)
                .expect("resized cache identity")
                .expect("cache identity after size change"),
            "file size still invalidates a cache identity when mtime is absent"
        );
    }

    #[test]
    fn album_artwork_candidates_match_detail_page_order_without_count_or_summaries() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "album-candidates");
        let records = ["Track 10", "Track 2", "Track 1", "Track 100"]
            .into_iter()
            .enumerate()
            .map(|(index, title)| {
                record(
                    &root,
                    &format!("item-{index}"),
                    &format!("path:item-{index}"),
                    title,
                    100 + index as u64,
                    1_000 + index as i64,
                )
            })
            .collect::<Vec<_>>();
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            2_000,
        );
        let detail_page = db
            .list_album_tracks_page(&["album".to_owned()], 0, 10)
            .expect("album detail page");
        let candidate_ids = db
            .list_album_artwork_candidates(&["album".to_owned()], 0, 10)
            .expect("album artwork candidates");
        assert_eq!(
            candidate_ids,
            detail_page
                .items
                .iter()
                .map(|track| track.id)
                .collect::<Vec<_>>()
        );
        let candidate_locators = db
            .album_artwork_candidate_locators(&["album".to_owned()])
            .expect("album artwork candidates with locators");
        assert_eq!(
            candidate_locators
                .iter()
                .map(|(track_id, _)| *track_id)
                .collect::<Vec<_>>(),
            candidate_ids
        );
        assert!(candidate_locators
            .iter()
            .all(|(_, locators)| !locators.is_empty()));
        assert_eq!(
            db.list_album_artwork_candidates(&["album".to_owned()], 1, 2)
                .expect("paged candidates"),
            candidate_ids[1..3]
        );
    }

    #[test]
    #[ignore = "manual isolated synthetic benchmark for album card query lock time"]
    fn benchmark_album_card_query_cost_on_synthetic_library() {
        use std::time::Instant;

        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "album-benchmark");
        let records = (0..5_364)
            .map(|index| {
                let mut track = record(
                    &root,
                    &format!("item-{index}"),
                    &format!("path:item-{index}"),
                    &format!("Track {index}"),
                    128 + index,
                    1_000 + index as i64,
                );
                track.metadata.as_mut().expect("metadata").album =
                    Some(format!("Album {:02}", index % 24));
                track
            })
            .collect::<Vec<_>>();
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            2_000,
        );
        let album_names = (0..24)
            .map(|index| format!("Album {index:02}"))
            .collect::<Vec<_>>();

        let started = Instant::now();
        for album in &album_names {
            db.list_album_tracks_page(std::slice::from_ref(album), 0, 8)
                .expect("full album page");
        }
        let full_page_elapsed = started.elapsed();

        let started = Instant::now();
        for album in &album_names {
            db.album_artwork_candidate_locators(std::slice::from_ref(album))
                .expect("artwork candidates");
        }
        let candidate_elapsed = started.elapsed();
        eprintln!(
            "5364 synthetic tracks / 24 albums: full detail pages={full_page_elapsed:?}, ID-only candidate pages={candidate_elapsed:?}"
        );
    }

    #[test]
    fn artist_index_uses_enabled_tracks_distinct_artists_albums_and_full_play_time() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "artist-index");
        let mut first = record(&root, "one", "one", "One", 1, 1);
        let mut second = record(&root, "two", "two", "Two", 1, 2);
        let mut third = record(&root, "three", "three", "Three", 1, 3);
        let disabled_root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "artist-index-disabled",
        );
        first.metadata.as_mut().unwrap().artist = Some("Hanser / 歌手甲".to_owned());
        first.metadata.as_mut().unwrap().album = Some("專輯 A".to_owned());
        second.metadata.as_mut().unwrap().artist = Some("hanser & 歌手乙".to_owned());
        second.metadata.as_mut().unwrap().album = Some("专辑 a".to_owned());
        third.metadata.as_mut().unwrap().artist = Some("HANSer".to_owned());
        third.metadata.as_mut().unwrap().album = Some("   ".to_owned());
        first.metadata.as_mut().unwrap().duration_ms = Some(10_000);
        second.metadata.as_mut().unwrap().duration_ms = Some(20_000);
        third.metadata.as_mut().unwrap().duration_ms = Some(30_000);
        let mut disabled = record(&disabled_root, "hidden", "hidden", "Hidden", 1, 4);
        disabled.metadata.as_mut().unwrap().artist = Some("停用演出者".to_owned());
        let records = vec![first, second, third];
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            4,
        );
        apply(
            &mut db,
            &disabled_root,
            SourceScanState::Complete,
            &[disabled.clone()],
            &[disabled],
            5,
        );
        let mut disabled_root_state = disabled_root.clone();
        disabled_root_state.enabled = false;
        db.save_library_root(&disabled_root_state)
            .expect("disable source");
        let track_ids = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
                field_filter: None,
                sort: TrackListSort::default(),
            })
            .expect("tracks")
            .items;
        {
            let connection = db.lock().expect("database lock");
            for (track, played_ms) in track_ids.iter().zip([1_000_i64, 2_000, 3_000]) {
                connection
                    .execute(
                        "INSERT INTO track_playback_statistics(track_id, played_ms) VALUES (?1, ?2)",
                        params![track.id.to_string(), played_ms],
                    )
                    .expect("seed play duration");
            }
        }

        assert_eq!(
            db.music_identity_key("hanser").expect("identity key"),
            "hanser"
        );
        assert_eq!(
            db.music_identity_key("歌手简")
                .expect("simplified identity"),
            "歌手簡"
        );
        assert_eq!(
            db.music_identity_key("歌手简かな")
                .expect("Japanese kana identity"),
            "歌手简かな",
            "Japanese kana keeps the original Han spelling"
        );
        assert_eq!(
            db.music_identity_key(" ＨＡＮＳＥＲ ")
                .expect("compatibility and case identity"),
            "hanser"
        );

        let snapshot = db
            .create_artist_index_snapshot("title", "asc", true)
            .expect("create SQLite artist snapshot");
        let all_artists = db
            .get_index_snapshot_page::<ArtistIndexEntry>(&snapshot.snapshot_id, 0, 100)
            .expect("read artist snapshot");
        let hanser = all_artists
            .items
            .iter()
            .find(|entry| entry.key == "hanser")
            .expect("Hanser");
        assert_eq!(hanser.track_count, 3);
        assert_eq!(hanser.album_count, 1);
        assert_eq!(hanser.names.iter().map(|item| item.count).sum::<u64>(), 3);
        assert_eq!(hanser.played_ms, 6_000);
        assert_eq!(hanser.duration_ms, 60_000);
        assert!(all_artists
            .items
            .iter()
            .all(|entry| entry.key != "停用演出者"));
        let singer_a = all_artists
            .items
            .iter()
            .find(|entry| entry.key == "歌手甲")
            .expect("singer A");
        assert_eq!(singer_a.track_count, 1);
        assert_eq!(singer_a.played_ms, 1_000);
        assert_eq!(singer_a.duration_ms, 10_000);
        assert_eq!(snapshot.quick_index.total_count, all_artists.total_count);
        assert!(!snapshot.quick_index.groups.is_empty());
        assert_eq!(snapshot.quick_index.groups[0].before_index, 0);
        let first_page = db
            .get_index_snapshot_page::<ArtistIndexEntry>(&snapshot.snapshot_id, 0, 2)
            .expect("first artist snapshot page");
        assert_eq!(first_page.total_count, all_artists.total_count);
        assert_eq!(first_page.items.len(), 2);
        assert!(first_page.items[0].display_name <= first_page.items[1].display_name);
        let navigation = db
            .create_artist_navigation_snapshot()
            .expect("create isolated artist navigation snapshot");
        let resolved_hanser = db
            .get_index_snapshot_entry::<ArtistIndexEntry>(&navigation.snapshot_id, "hanser")
            .expect("resolve artist from backend snapshot")
            .expect("artist exists");
        assert_eq!(resolved_hanser.key, "hanser");
        assert!(
            db.get_index_snapshot_page::<ArtistIndexEntry>(&snapshot.snapshot_id, 0, 2)
                .expect("navigation leaves visible artist snapshot intact")
                .total_count
                > 0
        );
        let last_page = db
            .get_index_snapshot_page::<ArtistIndexEntry>(&snapshot.snapshot_id, 2, 2)
            .expect("second artist snapshot page");
        assert_eq!(last_page.items.len(), 1);
        let ranked = db
            .create_artist_index_snapshot("trackCount", "desc", false)
            .expect("replace artist snapshot with metric ordering");
        let ranked_page = db
            .get_index_snapshot_page::<ArtistIndexEntry>(&ranked.snapshot_id, 0, 10)
            .expect("ranked artist page");
        assert!(ranked_page
            .items
            .windows(2)
            .all(|pair| pair[0].track_count >= pair[1].track_count));
        assert_eq!(
            db.get_index_snapshot_page::<ArtistIndexEntry>(&snapshot.snapshot_id, 0, 2)
                .expect("old artist snapshot was replaced")
                .total_count,
            0,
        );

        let artist_page = db
            .list_tracks_page_for_artists(
                ListTracksQuery {
                    offset: 0,
                    limit: 10,
                    query: None,
                    field_filter: None,
                    sort: TrackListSort::default(),
                },
                &[],
                &[
                    "Hanser".to_owned(),
                    "hanser".to_owned(),
                    "HANSer".to_owned(),
                ],
            )
            .expect("artist track page");
        assert_eq!(artist_page.total_count, 3);
        let ampersand_artist_page = db
            .list_tracks_page_for_artists(
                ListTracksQuery {
                    offset: 0,
                    limit: 10,
                    query: None,
                    field_filter: None,
                    sort: TrackListSort::default(),
                },
                &[],
                &["歌手乙".to_owned()],
            )
            .expect("ampersand-separated artist track page");
        assert_eq!(ampersand_artist_page.total_count, 1);
        let substring_collision = db
            .list_tracks_page_for_artists(
                ListTracksQuery {
                    offset: 0,
                    limit: 10,
                    query: None,
                    field_filter: None,
                    sort: TrackListSort::default(),
                },
                &[],
                &["anser".to_owned()],
            )
            .expect("artist collision page");
        assert_eq!(substring_collision.total_count, 0);
    }

    #[test]
    fn artist_index_includes_navigable_unknown_entry_for_tracks_without_artist() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "artist-unknown-index",
        );
        let mut missing = record(&root, "missing-artist", "missing-artist", "No Artist", 1, 1);
        missing.metadata.as_mut().unwrap().artist = None;
        let mut blank = record(&root, "blank-artist", "blank-artist", "Blank Artist", 1, 2);
        blank.metadata.as_mut().unwrap().artist = Some(" / & ".to_owned());
        let mut real_unknown = record(
            &root,
            "real-unknown",
            "real-unknown",
            "Artist Named Unknown",
            1,
            3,
        );
        real_unknown.metadata.as_mut().unwrap().artist = Some("未知".to_owned());
        let records = [missing, blank, real_unknown];
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            3,
        );

        let snapshot = db
            .create_artist_index_snapshot("title", "asc", true)
            .expect("create artist snapshot");
        let page = db
            .get_index_snapshot_page::<ArtistIndexEntry>(&snapshot.snapshot_id, 0, 10)
            .expect("read artist snapshot");
        let unknown = page
            .items
            .iter()
            .find(|entry| entry.key == "artist:unmarked")
            .expect("unmarked artist entry");
        assert_eq!(unknown.display_name, "未知");
        assert_eq!(unknown.artist_names, vec![String::new()]);
        assert_eq!(unknown.track_count, 2);
        assert!(page.items.iter().any(|entry| entry.key == "未知"));
        assert_eq!(
            page.items.last().map(|entry| entry.key.as_str()),
            Some("artist:unmarked")
        );
        assert_eq!(
            snapshot
                .quick_index
                .groups
                .last()
                .map(|group| group.label.as_str()),
            Some("未知")
        );
        assert_eq!(
            snapshot
                .quick_index
                .groups
                .last()
                .map(|group| group.before_index),
            Some(1)
        );

        let tracks = db
            .list_tracks_page_for_artists(
                ListTracksQuery {
                    offset: 0,
                    limit: 10,
                    query: None,
                    field_filter: None,
                    sort: TrackListSort::default(),
                },
                &[],
                &unknown.artist_names,
            )
            .expect("query unmarked artist detail");
        assert_eq!(tracks.total_count, 2);
        assert_eq!(
            tracks
                .items
                .iter()
                .map(|track| track.title.as_deref().unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["Blank Artist", "No Artist"]
        );
    }

    #[test]
    fn unmarked_album_entry_aliases_load_their_tracks() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "album-unknown-index",
        );
        let mut missing = record(&root, "missing-album", "missing-album", "No Album", 1, 1);
        missing.metadata.as_mut().unwrap().album = None;
        let mut blank = record(&root, "blank-album", "blank-album", "Blank Album", 1, 2);
        blank.metadata.as_mut().unwrap().album = Some("  ".to_owned());
        let mut real_unknown = record(
            &root,
            "real-unknown-album",
            "real-unknown-album",
            "Named Unknown",
            1,
            3,
        );
        real_unknown.metadata.as_mut().unwrap().album = Some("未知".to_owned());
        let records = [missing, blank, real_unknown];
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            3,
        );

        let snapshot = db
            .create_album_index_snapshot("title", "asc", true)
            .expect("create album snapshot");
        let page = db
            .get_index_snapshot_page::<AlbumIndexEntry>(&snapshot.snapshot_id, 0, 10)
            .expect("read album snapshot");
        let unknown = page
            .items
            .iter()
            .find(|entry| entry.key == "album:unmarked")
            .expect("unmarked album entry");
        assert_eq!(unknown.display_name, "未知");
        assert_eq!(unknown.aliases, vec![String::new(), "  ".to_owned()]);
        assert_eq!(unknown.track_count, 2);
        assert_eq!(
            page.items
                .iter()
                .map(|entry| entry.key.as_str())
                .collect::<Vec<_>>(),
            vec!["album:未知", "album:unmarked"],
            "the named album remains in its locale group; the unmarked album is the final unknown category"
        );
        assert_eq!(
            snapshot
                .quick_index
                .groups
                .iter()
                .map(|group| (group.label.as_str(), group.before_index))
                .collect::<Vec<_>>(),
            [("ㄨ", 0), ("未知", 1)]
        );

        let tracks = db
            .list_album_tracks_page(&unknown.aliases, 0, 10)
            .expect("query unmarked album detail");
        assert_eq!(tracks.total_count, 2);
        assert_eq!(
            tracks
                .items
                .iter()
                .map(|track| track.title.as_deref().unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["Blank Album", "No Album"]
        );
        assert!(page.items.iter().any(|entry| entry.key == "album:未知"));
    }

    #[test]
    fn unmarked_album_participates_in_selected_album_index_sort() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "album-sort-unknown",
        );
        let mut unknown_a = record(&root, "unknown-a", "unknown-a", "Unknown A", 1, 1);
        unknown_a.metadata.as_mut().unwrap().album = None;
        unknown_a.metadata.as_mut().unwrap().duration_ms = Some(20_000);
        let mut unknown_b = record(&root, "unknown-b", "unknown-b", "Unknown B", 1, 2);
        unknown_b.metadata.as_mut().unwrap().album = Some("   ".to_owned());
        unknown_b.metadata.as_mut().unwrap().duration_ms = Some(20_000);
        let mut album_a = record(&root, "album-a", "album-a", "Album A", 1, 3);
        album_a.metadata.as_mut().unwrap().album = Some("Alpha".to_owned());
        album_a.metadata.as_mut().unwrap().duration_ms = Some(100_000);
        let mut album_b_a = record(&root, "album-b-a", "album-b-a", "Album B 1", 1, 4);
        album_b_a.metadata.as_mut().unwrap().album = Some("Beta".to_owned());
        album_b_a.metadata.as_mut().unwrap().duration_ms = Some(30_000);
        let mut album_b_b = record(&root, "album-b-b", "album-b-b", "Album B 2", 1, 5);
        album_b_b.metadata.as_mut().unwrap().album = Some("Beta".to_owned());
        album_b_b.metadata.as_mut().unwrap().duration_ms = Some(30_000);
        let mut album_b_c = record(&root, "album-b-c", "album-b-c", "Album B 3", 1, 6);
        album_b_c.metadata.as_mut().unwrap().album = Some("Beta".to_owned());
        album_b_c.metadata.as_mut().unwrap().duration_ms = Some(30_000);
        let records = [
            unknown_a, unknown_b, album_a, album_b_a, album_b_b, album_b_c,
        ];
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            records.len() as i64,
        );

        let track_ids = records
            .iter()
            .map(|item| {
                db.resolve_track_id_for_locator(&item.locator)
                    .expect("resolve track")
                    .expect("track exists")
            })
            .collect::<Vec<_>>();
        {
            let connection = db.lock().expect("database connection");
            connection
                .execute(
                    "INSERT INTO track_playback_statistics(track_id, played_ms) VALUES (?1, 80000), (?2, 80000), (?3, 300000), (?4, 10000), (?5, 10000), (?6, 10000)",
                    params![
                        track_ids[0].to_string(), track_ids[1].to_string(),
                        track_ids[2].to_string(), track_ids[3].to_string(),
                        track_ids[4].to_string(), track_ids[5].to_string(),
                    ],
                )
                .expect("insert playback statistics");
        }

        let sorted_keys = |field: &str, direction: &str| {
            let snapshot = db
                .create_album_index_snapshot(field, direction, false)
                .expect("create album snapshot");
            db.get_index_snapshot_page::<AlbumIndexEntry>(&snapshot.snapshot_id, 0, 10)
                .expect("read album snapshot")
                .items
                .into_iter()
                .map(|entry| entry.key)
                .collect::<Vec<_>>()
        };

        assert_eq!(
            sorted_keys("trackCount", "asc"),
            vec!["album:alpha", "album:unmarked", "album:beta"]
        );
        assert_eq!(
            sorted_keys("trackCount", "desc"),
            vec!["album:beta", "album:unmarked", "album:alpha"]
        );
        assert_eq!(
            sorted_keys("duration", "asc"),
            vec!["album:unmarked", "album:beta", "album:alpha"]
        );
        assert_eq!(
            sorted_keys("duration", "desc"),
            vec!["album:alpha", "album:beta", "album:unmarked"]
        );
        assert_eq!(
            sorted_keys("played", "asc"),
            vec!["album:beta", "album:unmarked", "album:alpha"]
        );
        assert_eq!(
            sorted_keys("played", "desc"),
            vec!["album:alpha", "album:unmarked", "album:beta"]
        );
    }

    #[test]
    fn album_facets_keep_untagged_tracks_separate_and_album_pages_merge_exact_aliases() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "album-index");
        let mut disc_two = record(&root, "track-10", "track-10", "Song 10", 1, 1);
        let mut disc_one = record(&root, "track-2", "track-2", "Song 2", 1, 2);
        let mut unmarked = record(&root, "untagged", "untagged", "Unmarked", 1, 3);
        let mut titled_other = record(&root, "other", "other", "Tagged Other", 1, 4);
        let mut padded_name = record(&root, "padded", "padded", "Padded Album", 1, 5);
        disc_two.metadata.as_mut().unwrap().album = Some("专辑 A".to_owned());
        disc_two.metadata.as_mut().unwrap().duration_ms = Some(20_000);
        disc_two.metadata.as_mut().unwrap().disc_number = Some(2);
        disc_two.metadata.as_mut().unwrap().track_number = Some(1);
        disc_one.metadata.as_mut().unwrap().album = Some("專輯 A".to_owned());
        disc_one.metadata.as_mut().unwrap().duration_ms = Some(10_000);
        disc_one.metadata.as_mut().unwrap().disc_number = Some(1);
        disc_one.metadata.as_mut().unwrap().track_number = Some(10);
        unmarked.metadata.as_mut().unwrap().album = Some("  ".to_owned());
        titled_other.metadata.as_mut().unwrap().album = Some("其它".to_owned());
        padded_name.metadata.as_mut().unwrap().album = Some("\u{3000}專輯 A\u{3000}".to_owned());
        padded_name.metadata.as_mut().unwrap().disc_number = Some(1);
        padded_name.metadata.as_mut().unwrap().track_number = Some(11);
        padded_name.metadata.as_mut().unwrap().duration_ms = Some(5_000);
        let disc_two_locator = disc_two.locator.clone();
        let disc_one_locator = disc_one.locator.clone();
        let padded_locator = padded_name.locator.clone();
        let records = [disc_two, disc_one, unmarked, titled_other, padded_name];
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            5,
        );

        let disc_two_id = db
            .resolve_track_id_for_locator(&disc_two_locator)
            .expect("resolve disc two")
            .expect("disc two exists");
        let disc_one_id = db
            .resolve_track_id_for_locator(&disc_one_locator)
            .expect("resolve disc one")
            .expect("disc one exists");
        let padded_id = db
            .resolve_track_id_for_locator(&padded_locator)
            .expect("resolve padded album")
            .expect("padded album exists");
        {
            let connection = db.lock().expect("database connection");
            connection
                .execute(
                    "INSERT INTO track_playback_statistics (track_id, played_ms) VALUES (?1, ?2), (?3, ?4), (?5, ?6)",
                    params![disc_two_id.to_string(), 900, disc_one_id.to_string(), 300, padded_id.to_string(), 100],
                )
                .expect("insert playback statistics");
        }

        let page = db
            .list_album_tracks_page(
                &[
                    "專輯 A".into(),
                    "专辑 A".into(),
                    "\u{3000}專輯 A\u{3000}".into(),
                ],
                0,
                10,
            )
            .expect("album page");
        assert_eq!(page.total_count, 3);
        assert_eq!(page.items[0].track_number, Some(10));
        assert_eq!(page.items[1].track_number, Some(11));
        assert_eq!(page.items[2].disc_number, Some(2));
        let descending = db
            .list_album_tracks_page_with_index_sort(
                &[
                    "專輯 A".into(),
                    "专辑 A".into(),
                    "\u{3000}專輯 A\u{3000}".into(),
                ],
                None,
                Some(TrackSortDirection::Desc),
                0,
                10,
            )
            .expect("descending album track-number page");
        assert_eq!(descending.items[0].disc_number, Some(2));
        assert_eq!(descending.items[0].track_number, Some(1));
        assert_eq!(descending.items[1].disc_number, Some(1));
        assert_eq!(descending.items[1].track_number, Some(11));
        assert_eq!(descending.items[2].track_number, Some(10));
        assert_eq!(
            db.list_album_tracks_page(&[], 0, 10)
                .expect("empty aliases")
                .total_count,
            0
        );

        let album_snapshot = db
            .create_album_index_snapshot("trackCount", "desc", false)
            .expect("create SQLite album snapshot");
        let album_page = db
            .get_index_snapshot_page::<AlbumIndexEntry>(&album_snapshot.snapshot_id, 0, 2)
            .expect("first album snapshot page");
        assert_eq!(album_page.total_count, 3, "two tagged groups plus unmarked");
        assert_eq!(album_page.items.len(), 2);
        assert!(album_page.items[0].track_count >= album_page.items[1].track_count);
        let all_albums = db
            .get_index_snapshot_page::<AlbumIndexEntry>(&album_snapshot.snapshot_id, 0, 10)
            .expect("all album index cards");
        let merged = all_albums
            .items
            .iter()
            .find(|item| item.key == "album:專輯 a")
            .expect("merged aliases");
        assert_eq!(merged.track_count, 3);
        assert_eq!(merged.display_name, "專輯 A");
        assert!(merged.aliases.contains(&"專輯 A".to_owned()));
        assert!(merged.aliases.contains(&"专辑 A".to_owned()));
        assert!(merged
            .aliases
            .contains(&"\u{3000}專輯 A\u{3000}".to_owned()));
        assert!(all_albums
            .items
            .iter()
            .any(|item| item.unmarked && item.display_name == "未知"));
        assert_eq!(all_albums.items[0].track_count, 3);
        assert!(all_albums.items[0].key != "album:unmarked");
        let navigation = db
            .create_album_navigation_snapshot()
            .expect("create isolated album navigation snapshot");
        let resolved_album = db
            .get_index_snapshot_entry::<AlbumIndexEntry>(&navigation.snapshot_id, "album:專輯 a")
            .expect("resolve album from backend snapshot")
            .expect("album exists");
        assert_eq!(resolved_album.track_count, 3);
        assert!(
            db.get_index_snapshot_page::<AlbumIndexEntry>(&album_snapshot.snapshot_id, 0, 2)
                .expect("navigation leaves visible album snapshot intact")
                .total_count
                > 0
        );
    }

    #[test]
    fn album_detail_artist_and_disc_facets_are_aggregated_and_grouped_in_sqlite() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "album-detail-facets",
        );
        let mut records = Vec::new();
        for (item, artist, disc) in [
            ("simple-1", "歌手简", Some(2)),
            ("simple-2", "歌手简", Some(2)),
            ("traditional-1", "歌手簡", Some(3)),
            ("traditional-2", "歌手簡", None),
        ] {
            let mut track = record(&root, item, item, item, 1, 1);
            let metadata = track.metadata.as_mut().expect("metadata");
            metadata.album = Some("Shared Album".to_owned());
            metadata.artist = Some(artist.to_owned());
            metadata.disc_number = disc;
            records.push(track);
        }
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            2,
        );

        let facets = db
            .album_detail_facets(&["Shared Album".to_owned()])
            .expect("aggregate album detail facets");
        assert_eq!(facets.track_count, 4);
        assert_eq!(facets.unknown_disc_track_count, 1);
        assert_eq!(
            facets.disc_counts,
            vec![
                AlbumDiscCount {
                    disc_number: 2,
                    count: 2
                },
                AlbumDiscCount {
                    disc_number: 3,
                    count: 1
                },
            ]
        );
        assert_eq!(
            db.album_detail_artists(&["Shared Album".to_owned()])
                .expect("merge and sort album artists"),
            vec!["歌手簡".to_owned()],
        );
    }

    #[test]
    fn album_disc_index_groups_each_number_and_places_unknown_by_sort_direction() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "album-disc-index");
        let fixtures = [
            ("disc2-track1", Some(2), 1),
            ("disc2-track2", Some(2), 2),
            ("disc3-track1", Some(3), 1),
            ("disc1-track9", Some(1), 9),
            ("disc0-track2", Some(0), 2),
            ("unknown-track1", None, 1),
        ];
        let records = fixtures
            .iter()
            .enumerate()
            .map(|(index, (title, disc_number, track_number))| {
                let mut item = record(
                    &root,
                    &format!("{title}.flac"),
                    &format!("{title}.flac"),
                    title,
                    1,
                    1_800_000_000_100 + index as i64,
                );
                let metadata = item.metadata.as_mut().expect("metadata");
                metadata.album = Some("Multi-disc".to_owned());
                metadata.disc_number = *disc_number;
                metadata.track_number = Some(*track_number);
                item
            })
            .collect::<Vec<_>>();
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            1_800_000_000_200,
        );

        let aliases = ["Multi-disc".to_owned()];
        let facets = db
            .album_detail_facets(&aliases)
            .expect("album detail facets and SQLite disc group snapshots");
        let ascending_groups = facets
            .quick_index_ascending
            .groups
            .iter()
            .map(|group| (group.label.as_str(), group.before_index))
            .collect::<Vec<_>>();
        assert_eq!(
            ascending_groups,
            [
                ("DISC 0", 0),
                ("DISC 1", 1),
                ("DISC 2", 2),
                ("DISC 3", 4),
                ("未知", 5)
            ]
        );
        let descending_groups = facets
            .quick_index_descending
            .groups
            .iter()
            .map(|group| (group.label.as_str(), group.before_index))
            .collect::<Vec<_>>();
        assert_eq!(
            descending_groups,
            [
                ("未知", 0),
                ("DISC 3", 1),
                ("DISC 2", 2),
                ("DISC 1", 4),
                ("DISC 0", 5)
            ]
        );
        let ascending = db
            .list_album_tracks_page_with_index_sort(
                &aliases,
                None,
                Some(TrackSortDirection::Asc),
                0,
                20,
            )
            .expect("ascending disc index");
        assert_eq!(
            ascending
                .items
                .iter()
                .map(|item| item.title.as_deref().unwrap_or_default())
                .collect::<Vec<_>>(),
            [
                "disc0-track2",
                "disc1-track9",
                "disc2-track1",
                "disc2-track2",
                "disc3-track1",
                "unknown-track1"
            ]
        );

        let descending = db
            .list_album_tracks_page_with_index_sort(
                &aliases,
                None,
                Some(TrackSortDirection::Desc),
                0,
                20,
            )
            .expect("descending disc index");
        assert_eq!(
            descending
                .items
                .iter()
                .map(|item| item.title.as_deref().unwrap_or_default())
                .collect::<Vec<_>>(),
            [
                "unknown-track1",
                "disc3-track1",
                "disc2-track2",
                "disc2-track1",
                "disc1-track9",
                "disc0-track2"
            ]
        );

        let mut unclassified_db = Database::open_in_memory().expect("database");
        let root = add_root(
            &unclassified_db,
            MediaSourceKind::WindowsFilesystem,
            "album-disc-without-multiple-discs",
        );
        let records = [("only-disc0", Some(0)), ("only-disc1", Some(1))]
            .iter()
            .enumerate()
            .map(|(index, (title, disc_number))| {
                let mut item = record(
                    &root,
                    &format!("{title}.flac"),
                    &format!("{title}.flac"),
                    title,
                    1,
                    1_800_000_000_300 + index as i64,
                );
                let metadata = item.metadata.as_mut().expect("metadata");
                metadata.album = Some("Only low disc numbers".to_owned());
                metadata.disc_number = *disc_number;
                item
            })
            .collect::<Vec<_>>();
        apply(
            &mut unclassified_db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            1_800_000_000_400,
        );
        let facets = unclassified_db
            .album_detail_facets(&["Only low disc numbers".to_owned()])
            .expect("album without disc classification");
        assert!(facets.quick_index_ascending.groups.is_empty());
        assert!(facets.quick_index_descending.groups.is_empty());
    }

    #[test]
    fn unmarked_album_and_artist_text_groups_follow_sort_direction() {
        let mut db = Database::from_connection_with_collation(
            Connection::open_in_memory().expect("connection"),
            MusicCollationSpec::from_locale_parts("zh-TW", Some(CollationType::Zhuyin)),
        )
        .expect("Taiwanese locale database");
        let root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "unknown-text-groups",
        );
        let mut records = [
            record(&root, "w1", "path:w1", "無謬の花", 1, 1_800_000_000_100),
            record(&root, "w2", "path:w2", "文明EP", 1, 1_800_000_000_101),
            record(
                &root,
                "unmarked",
                "path:unmarked",
                "Unmarked",
                1,
                1_800_000_000_102,
            ),
        ];
        records[0].metadata.as_mut().expect("metadata").album = Some("無謬の花".to_owned());
        records[0].metadata.as_mut().expect("metadata").artist = Some("無謬の花".to_owned());
        records[1].metadata.as_mut().expect("metadata").album = Some("文明EP".to_owned());
        records[1].metadata.as_mut().expect("metadata").artist = Some("文明EP".to_owned());
        records[2].metadata.as_mut().expect("metadata").album = None;
        records[2].metadata.as_mut().expect("metadata").artist = None;
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            1_800_000_000_200,
        );

        let album_ascending = db
            .create_album_index_snapshot("title", "asc", true)
            .expect("ascending album snapshot");
        let albums: player_core::Page<AlbumIndexEntry> = db
            .get_index_snapshot_page(&album_ascending.snapshot_id, 0, 20)
            .expect("ascending albums");
        assert_eq!(
            albums.items.last().map(|item| item.display_name.as_str()),
            Some("未知")
        );
        assert_eq!(
            album_ascending
                .quick_index
                .groups
                .last()
                .map(|group| group.before_index),
            Some(albums.total_count - 1),
            "ascending album quick index must put the unknown group after every locale group"
        );
        assert_eq!(
            album_ascending
                .quick_index
                .groups
                .iter()
                .map(|group| (group.label.as_str(), group.before_index))
                .collect::<Vec<_>>(),
            [("ㄨ", 0), ("未知", 2)],
            "unknown albums must not split one ICU Zhuyin category"
        );
        assert_eq!(
            album_ascending
                .quick_index
                .groups
                .last()
                .map(|group| group.label.as_str()),
            Some("未知")
        );

        let album_descending = db
            .create_album_index_snapshot("title", "desc", true)
            .expect("descending album snapshot");
        let albums: player_core::Page<AlbumIndexEntry> = db
            .get_index_snapshot_page(&album_descending.snapshot_id, 0, 20)
            .expect("descending albums");
        assert_eq!(
            albums.items.first().map(|item| item.display_name.as_str()),
            Some("未知")
        );
        assert_eq!(
            album_descending
                .quick_index
                .groups
                .first()
                .map(|group| group.label.as_str()),
            Some("未知")
        );
        assert_eq!(
            album_descending
                .quick_index
                .groups
                .first()
                .map(|group| group.before_index),
            Some(0)
        );
        assert_eq!(
            album_descending
                .quick_index
                .groups
                .iter()
                .map(|group| (group.label.as_str(), group.before_index))
                .collect::<Vec<_>>(),
            [("未知", 0), ("ㄨ", 1)]
        );

        let artist_ascending = db
            .create_artist_index_snapshot("title", "asc", true)
            .expect("ascending artist snapshot");
        let artists: player_core::Page<ArtistIndexEntry> = db
            .get_index_snapshot_page(&artist_ascending.snapshot_id, 0, 20)
            .expect("ascending artists");
        assert_eq!(
            artists.items.last().map(|item| item.display_name.as_str()),
            Some("未知")
        );
        assert_eq!(
            artist_ascending
                .quick_index
                .groups
                .last()
                .map(|group| group.label.as_str()),
            Some("未知")
        );
        assert_eq!(
            artist_ascending
                .quick_index
                .groups
                .iter()
                .map(|group| (group.label.as_str(), group.before_index))
                .collect::<Vec<_>>(),
            [("ㄨ", 0), ("未知", 2)]
        );

        let artist_descending = db
            .create_artist_index_snapshot("title", "desc", true)
            .expect("descending artist snapshot");
        let artists: player_core::Page<ArtistIndexEntry> = db
            .get_index_snapshot_page(&artist_descending.snapshot_id, 0, 20)
            .expect("descending artists");
        assert_eq!(
            artists.items.first().map(|item| item.display_name.as_str()),
            Some("未知")
        );
        assert_eq!(
            artist_descending
                .quick_index
                .groups
                .first()
                .map(|group| group.label.as_str()),
            Some("未知")
        );
        assert_eq!(
            artist_descending
                .quick_index
                .groups
                .iter()
                .map(|group| (group.label.as_str(), group.before_index))
                .collect::<Vec<_>>(),
            [("未知", 0), ("ㄨ", 1)]
        );
    }

    #[test]
    fn cancelling_during_persistence_rolls_back_without_reconciling_mappings() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "cancel rollback");
        let old_one = record(&root, "one", "key-one", "before", 10, 1_800_000_000_001);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&old_one),
            std::slice::from_ref(&old_one),
            1,
        );

        let updated_one = record(&root, "one", "key-one", "after", 11, 1_800_000_000_002);
        let new_two = record(&root, "two", "key-two", "new", 12, 1_800_000_000_003);
        let cancellation = SyncCancellation::default();
        let outcome = db
            .apply_source_scan_with_progress(
                SyncApplyRequest {
                    root: &root,
                    state: &SourceScanState::Complete,
                    observed: &[updated_one.clone(), new_two.clone()],
                    changed: &[updated_one, new_two],
                    errors: &[],
                    synced_at_utc_ms: 2,
                },
                &mut |processed| {
                    if processed == 1 {
                        cancellation.cancel();
                    }
                },
                &cancellation,
            )
            .expect("cancellation is a reported outcome");

        assert!(outcome.cancelled);
        assert_eq!(outcome.stats, player_core::SyncApplyStats::default());
        let page = list(&db, 0, 10);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].title.as_deref(), Some("before"));
    }

    fn list(
        db: &Database,
        offset: u64,
        limit: u32,
    ) -> player_core::Page<player_core::TrackSummary> {
        db.list_tracks_page(ListTracksQuery {
            offset,
            limit,
            query: None,
            field_filter: None,
            sort: Default::default(),
        })
        .expect("list tracks")
    }

    fn track_id_for_query(db: &Database, query: &str) -> TrackId {
        db.list_tracks_page(ListTracksQuery {
            offset: 0,
            limit: 20,
            query: Some(query.to_owned()),
            field_filter: None,
            sort: Default::default(),
        })
        .expect("find track")
        .items
        .into_iter()
        .next()
        .expect("matching track")
        .id
    }

    #[test]
    fn disabling_a_source_hides_its_tracks_but_keeps_them_for_reenable() {
        let mut db = Database::open_in_memory().expect("database");
        let mut root = add_root(&db, MediaSourceKind::WindowsFilesystem, "toggle source");
        let record = record(&root, "only-item", "only-key", "Source-only song", 12, 100);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&record),
            std::slice::from_ref(&record),
            100,
        );
        let track_id = db
            .resolve_track_id_for_locator(&record.locator)
            .expect("resolve indexed track")
            .expect("track remains stored");

        assert_eq!(db.count_tracks(None).expect("count enabled tracks"), 1);
        assert_eq!(
            db.count_tracks(Some("Source-only"))
                .expect("search enabled tracks"),
            1
        );

        root.enabled = false;
        db.save_library_root(&root).expect("disable source");

        let hidden = list(&db, 0, 10);
        assert!(hidden.items.is_empty());
        assert_eq!(hidden.total_count, 0);
        assert_eq!(db.count_tracks(None).expect("count after disabling"), 0);
        assert_eq!(
            db.count_tracks(Some("Source-only"))
                .expect("search after disabling"),
            0
        );
        assert!(db
            .list_track_ids_filtered(
                None,
                Some(&TrackFieldFilter {
                    field: TrackField::Title,
                    value: "Source-only song".to_owned(),
                }),
                Default::default(),
            )
            .expect("filtered queue IDs")
            .is_empty());
        assert!(db
            .get_track_summary(track_id)
            .expect("stored row remains readable")
            .is_some());

        root.enabled = true;
        db.save_library_root(&root).expect("reenable source");
        assert_eq!(list(&db, 0, 10).total_count, 1);
    }

    #[test]
    fn migration_is_idempotent_and_file_database_uses_wal() {
        let unique = player_core::TrackId::new().to_string();
        let path = std::env::temp_dir().join(format!("moemusic-player-{unique}.sqlite"));
        {
            let db = Database::open(&path).expect("open file database");
            assert_eq!(
                db.journal_mode()
                    .expect("journal mode")
                    .to_ascii_lowercase(),
                "wal"
            );
            db.migrate().expect("idempotent migration");
            let version: i64 = db
                .lock()
                .expect("connection")
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .expect("schema version");
            assert_eq!(version, 15);
        }
        {
            let db = Database::open_checked(&path).expect("read-only validate migrated database");
            assert!(db.library_roots().expect("read roots").is_empty());
            assert_eq!(
                db.get_theme_preferences().expect("theme preferences"),
                ThemePreferences::default()
            );
        }
        for suffix in ["", "-wal", "-shm"] {
            let mut file = path.as_os_str().to_os_string();
            file.push(suffix);
            let file = PathBuf::from(file);
            if file.exists() {
                std::fs::remove_file(file).expect("remove generated test database");
            }
        }
    }

    #[test]
    fn version_one_database_migrates_additively_to_current_schema() {
        let connection = rusqlite::Connection::open_in_memory().expect("legacy database");
        connection
            .execute_batch(SCHEMA_V1)
            .expect("create version one schema");
        connection
            .execute(
                "INSERT INTO library_roots
                    (source_id, source_kind, display_name, locator_kind, locator_encoding, locator_data)
                 VALUES ('legacy-source', 'other', 'legacy', 'content_uri', 'utf8', X'6C6567616379')",
                [],
            )
            .expect("insert legacy root");
        connection
            .pragma_update(None, "user_version", 1)
            .expect("mark legacy schema version");

        let db = Database::from_connection(connection).expect("migrate version one");
        let current_version: i64 = db
            .lock()
            .expect("connection")
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("current schema version");
        assert_eq!(current_version, 15);
        let preserved_roots: i64 = db
            .lock()
            .expect("connection")
            .query_row("SELECT COUNT(*) FROM library_roots", [], |row| row.get(0))
            .expect("legacy roots remain");
        assert_eq!(preserved_roots, 1);
        let release_date_column: i64 = db
            .lock()
            .expect("connection")
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tracks') WHERE name='release_date'",
                [],
                |row| row.get(0),
            )
            .expect("release_date column");
        assert_eq!(release_date_column, 1);
        let locator_index: Option<String> = db
            .lock()
            .expect("connection")
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='index' AND name='source_mappings_locator_exact'",
                [],
                |row| row.get(0),
            )
            .optional()
            .expect("locator index lookup");
        assert_eq!(
            locator_index.as_deref(),
            Some("source_mappings_locator_exact")
        );
        assert_eq!(
            db.get_theme_preferences().expect("theme preferences"),
            ThemePreferences::default()
        );
    }

    #[test]
    fn version_two_database_migrates_theme_preferences_with_defaults() {
        let connection = rusqlite::Connection::open_in_memory().expect("legacy database");
        connection
            .execute_batch(SCHEMA_V1)
            .expect("create schema v1");
        connection
            .pragma_update(None, "user_version", 1)
            .expect("mark schema v1");
        connection
            .execute_batch(SCHEMA_V2)
            .expect("create schema v2");
        connection
            .pragma_update(None, "user_version", 2)
            .expect("mark schema v2");

        let db = Database::from_connection(connection).expect("migrate schema v2");
        let version: i64 = db
            .lock()
            .expect("connection")
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, 15);
        assert_eq!(
            db.get_theme_preferences().expect("theme preferences"),
            ThemePreferences::default()
        );
    }

    #[test]
    fn version_three_migration_adds_playlist_reconciliation_index() {
        let connection = rusqlite::Connection::open_in_memory().expect("legacy database");
        connection
            .execute_batch(SCHEMA_V1)
            .expect("create schema v1");
        connection
            .execute_batch(SCHEMA_V2)
            .expect("create schema v2");
        connection
            .execute_batch(SCHEMA_V3)
            .expect("create schema v3");
        connection
            .pragma_update(None, "user_version", 3)
            .expect("mark schema v3");

        let db = Database::from_connection(connection).expect("migrate schema v3");
        let (version, index): (i64, Option<String>) = {
            let connection = db.lock().expect("database connection");
            (
                connection
                    .pragma_query_value(None, "user_version", |row| row.get(0))
                    .expect("schema version"),
                connection
                    .query_row(
                        "SELECT name FROM sqlite_master WHERE type='index' AND name='playlist_entries_unmatched'",
                        [],
                        |row| row.get(0),
                    )
                    .optional()
                    .expect("reconciliation index lookup"),
            )
        };
        assert_eq!(version, 15);
        assert_eq!(index.as_deref(), Some("playlist_entries_unmatched"));
    }

    #[test]
    fn version_four_migration_adds_playlist_file_sync_state() {
        let connection = rusqlite::Connection::open_in_memory().expect("legacy database");
        connection
            .execute_batch(SCHEMA_V1)
            .expect("create schema v1");
        connection
            .execute_batch(SCHEMA_V2)
            .expect("create schema v2");
        connection
            .execute_batch(SCHEMA_V3)
            .expect("create schema v3");
        connection
            .execute_batch(SCHEMA_V4)
            .expect("create schema v4");
        connection
            .pragma_update(None, "user_version", 4)
            .expect("mark schema v4");

        let db = Database::from_connection(connection).expect("migrate schema v4");
        let (version, table): (i64, Option<String>) = {
            let connection = db.lock().expect("database connection");
            (
                connection
                    .pragma_query_value(None, "user_version", |row| row.get(0))
                    .expect("schema version"),
                connection
                    .query_row(
                        "SELECT name FROM sqlite_master WHERE type='table' AND name='playlist_file_sync_state'",
                        [],
                        |row| row.get(0),
                    )
                    .optional()
                    .expect("sync-state table lookup"),
            )
        };
        assert_eq!(version, 15);
        assert_eq!(table.as_deref(), Some("playlist_file_sync_state"));
    }

    #[test]
    fn version_five_metadata_migration_backfills_once_and_projects_to_playlist_pages() {
        let connection = rusqlite::Connection::open_in_memory().expect("legacy database");
        connection
            .execute_batch(SCHEMA_V1)
            .expect("create schema v1");
        connection
            .execute_batch(SCHEMA_V2)
            .expect("create schema v2");
        connection
            .execute_batch(SCHEMA_V3)
            .expect("create schema v3");
        connection
            .execute_batch(SCHEMA_V4)
            .expect("create schema v4");
        connection
            .execute_batch(SCHEMA_V5)
            .expect("create schema v5");
        let track_id = TrackId::new();
        connection
            .execute(
                "INSERT INTO tracks (track_id, metadata_loaded, sort_title, title)
                 VALUES (?1, 1, 'old title', 'old title')",
                [track_id.to_string()],
            )
            .expect("insert legacy track");
        connection
            .pragma_update(None, "user_version", 5)
            .expect("mark schema v5");

        let mut db = Database::from_connection(connection).expect("migrate schema v5");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "metadata backfill");
        let record = record(
            &root,
            "legacy-item",
            "path-key:legacy",
            "New title",
            12,
            345,
        );
        db.lock()
            .expect("database lock")
            .execute(
                "INSERT INTO source_mappings
                    (source_id, source_item_id, locator_key, locator_kind, locator_encoding,
                     locator_data, size_bytes, modified_at_utc_ms, track_id)
                 VALUES (?1, ?2, ?3, 'filesystem', 'utf8', X'6C6567616379', 12, 345, ?4)",
                params![
                    root.id.to_string(),
                    record.identity.source_item_id,
                    record.identity.locator_key,
                    track_id.to_string(),
                ],
            )
            .expect("map legacy track");

        let legacy_state = db
            .track_sync_state(&record.identity)
            .expect("read old state")
            .expect("legacy mapping exists");
        assert_eq!(legacy_state.metadata_version, 0);
        db.apply_metadata_backfill_batch(std::slice::from_ref(&record))
            .expect("commit backfill batch");
        let summary = db
            .get_track_summary(track_id)
            .expect("query summary")
            .unwrap();
        assert_eq!(summary.title.as_deref(), Some("New title"));
        assert_eq!(summary.year, Some(2001));
        assert_eq!(summary.bit_depth, Some(24));
        assert_eq!(summary.artist.as_deref(), Some("artist"));

        let mut playlist = Playlist::new("metadata projection");
        playlist.entries = vec![
            PlaylistEntry {
                track_id: Some(track_id),
                locator: record.locator.clone(),
                title: Some("entry override".to_owned()),
                duration_ms: Some(9_876),
            },
            PlaylistEntry {
                track_id: None,
                locator: MediaLocator::FileSystem(PathBuf::from(r"C:\missing\future.flac")),
                title: Some("unmatched".to_owned()),
                duration_ms: None,
            },
        ];
        db.save_playlist(&playlist).expect("save playlist");
        let playlist_page = db
            .get_playlist_page(playlist.id, 0, 10)
            .expect("project playlist page")
            .expect("playlist exists");
        assert_eq!(
            playlist_page.items[0].title.as_deref(),
            Some("entry override")
        );
        assert_eq!(playlist_page.items[0].duration_ms, Some(9_876));
        assert_eq!(playlist_page.items[0].year, Some(2001));
        assert_eq!(playlist_page.items[0].bit_depth, Some(24));
        assert_eq!(playlist_page.items[1].track_id, None);
        assert_eq!(playlist_page.items[1].year, None);
        assert_eq!(playlist_page.items[1].bit_depth, None);

        let mut repeat = record.clone();
        repeat.metadata.as_mut().expect("metadata").year = Some(1999);
        db.apply_metadata_backfill_batch(&[repeat])
            .expect("idempotent repeat");
        assert_eq!(
            db.get_track_summary(track_id)
                .expect("summary after repeated batch")
                .unwrap()
                .year,
            Some(2001)
        );
    }

    #[test]
    fn version_six_migration_adds_track_lyrics_and_candidate_cache_tables() {
        let connection = rusqlite::Connection::open_in_memory().expect("legacy database");
        connection
            .execute_batch(SCHEMA_V1)
            .expect("create schema v1");
        connection
            .execute_batch(SCHEMA_V2)
            .expect("create schema v2");
        connection
            .execute_batch(SCHEMA_V3)
            .expect("create schema v3");
        connection
            .execute_batch(SCHEMA_V4)
            .expect("create schema v4");
        connection
            .execute_batch(SCHEMA_V5)
            .expect("create schema v5");
        connection
            .execute_batch(SCHEMA_V6)
            .expect("create schema v6");
        let track_id = TrackId::new();
        connection
            .execute(
                "INSERT INTO tracks (track_id, metadata_loaded, sort_title, title)
                 VALUES (?1, 1, 'persisted title', 'persisted title')",
                [track_id.to_string()],
            )
            .expect("insert v6 track");
        connection
            .pragma_update(None, "user_version", 6)
            .expect("mark schema v6");

        let db = Database::from_connection(connection).expect("migrate schema v6");
        let (version, lyric_table, candidate_table) = {
            let connection = db.lock().expect("database lock");
            (
                connection
                    .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                    .expect("schema version"),
                connection
                    .query_row(
                        "SELECT name FROM sqlite_master WHERE type='table' AND name='track_lyrics'",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .expect("track lyrics table"),
                connection
                    .query_row(
                        "SELECT name FROM sqlite_master WHERE type='table' AND name='lyric_candidates'",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .expect("lyric candidates table"),
            )
        };
        assert_eq!(version, 15);
        assert_eq!(lyric_table, "track_lyrics");
        assert_eq!(candidate_table, "lyric_candidates");
        assert_eq!(
            db.get_track_summary(track_id)
                .expect("query migrated track")
                .expect("track remains")
                .title
                .as_deref(),
            Some("persisted title")
        );
    }

    #[test]
    fn version_seven_migration_adds_normalized_playback_session_tables() {
        let connection = rusqlite::Connection::open_in_memory().expect("schema v7 database");
        connection.execute_batch(SCHEMA_V1).expect("schema v1");
        connection.execute_batch(SCHEMA_V2).expect("schema v2");
        connection.execute_batch(SCHEMA_V3).expect("schema v3");
        connection.execute_batch(SCHEMA_V4).expect("schema v4");
        connection.execute_batch(SCHEMA_V5).expect("schema v5");
        connection.execute_batch(SCHEMA_V6).expect("schema v6");
        connection.execute_batch(SCHEMA_V7).expect("schema v7");
        connection
            .pragma_update(None, "user_version", 7)
            .expect("mark schema v7");

        let db = Database::from_connection(connection).expect("migrate schema v7");
        let connection = db.lock().expect("connection");
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("schema version");
        let tables: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('playback_session', 'playback_session_entries')",
            [],
            |row| row.get(0),
        ).expect("playback session tables");
        assert_eq!(version, 15);
        assert_eq!(tables, 2);
    }

    #[test]
    fn selected_lyric_candidate_is_persisted_and_blocks_later_automatic_results() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "lyrics");
        let track = record(&root, "lyrics-track", "lyrics-path", "Song", 10, 1);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&track),
            std::slice::from_ref(&track),
            1,
        );
        let track_id = track_id_for_query(&db, "Song");
        let initial = TrackLyrics {
            track_id,
            provider: LyricProvider::NetEase,
            candidate_id: Some("net-1".into()),
            lyrics: parse_lrc("[00:01.00]automatic").expect("automatic LRC"),
            manually_selected: false,
            updated_at_utc_ms: 10,
        };
        assert!(db
            .save_automatic_track_lyrics(&initial)
            .expect("save automatic result"));

        let candidate = LyricCandidate {
            candidate_id: "qq-2".into(),
            provider: LyricProvider::Qq,
            provider_track_id: Some("2".into()),
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            album: None,
            duration_ms: Some(180_000),
            lyrics: parse_lrc("[00:02.00]manually selected").expect("candidate LRC"),
            score: 0.91,
        };
        db.cache_lyric_candidates(track_id, std::slice::from_ref(&candidate), 20)
            .expect("cache candidate");
        let selected = db
            .select_lyric_candidate(track_id, "qq-2", 30)
            .expect("select candidate");
        assert!(selected.manually_selected);
        assert_eq!(selected.provider, LyricProvider::Qq);
        assert_eq!(selected.lyrics.lines[0].text, "manually selected");

        let future_automatic = TrackLyrics {
            track_id,
            provider: LyricProvider::NetEase,
            candidate_id: Some("net-3".into()),
            lyrics: parse_lrc("[00:03.00]later result").expect("later LRC"),
            manually_selected: false,
            updated_at_utc_ms: 40,
        };
        assert!(!db
            .save_automatic_track_lyrics(&future_automatic)
            .expect("preserve manual result"));
        let stored = db
            .get_track_lyrics(track_id)
            .expect("read lyrics")
            .expect("manual lyrics remain");
        assert!(stored.manually_selected);
        assert_eq!(stored.candidate_id.as_deref(), Some("qq-2"));
        assert_eq!(stored.lyrics.lines[0].text, "manually selected");
    }

    #[test]
    fn persisted_provider_lrc_repairs_auto_read_and_manual_candidate_selection() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "colon-lrc-cache");
        let track = record(&root, "colon-lrc-track", "colon-lrc-path", "Song", 10, 1);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&track),
            std::slice::from_ref(&track),
            1,
        );
        let track_id = track_id_for_query(&db, "Song");
        let raw_text = (0..43)
            .map(|second| {
                let separator = if second < 3 { '.' } else { ':' };
                format!("[00:{second:02}{separator}25]synthetic line {second}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut legacy_parse = parse_lrc(&raw_text).expect("current LRC parse");
        legacy_parse.lines.truncate(3);
        assert_eq!(
            legacy_parse.lines.len(),
            3,
            "fixture represents the stored bad parse"
        );

        let automatic = TrackLyrics {
            track_id,
            provider: LyricProvider::NetEase,
            candidate_id: Some("netease:colon-lrc".into()),
            lyrics: legacy_parse.clone(),
            manually_selected: false,
            updated_at_utc_ms: 10,
        };
        assert!(db
            .save_automatic_track_lyrics(&automatic)
            .expect("save legacy automatic cache"));

        let automatic_read = db
            .get_track_lyrics(track_id)
            .expect("read automatic cache")
            .expect("automatic cache exists");
        assert_eq!(automatic_read.lyrics.lines.len(), 43);
        assert!(automatic_read
            .lyrics
            .lines
            .iter()
            .all(|line| line.start_ms.is_some()));
        assert_eq!(automatic_read.lyrics.raw_text, raw_text);
        assert!(!automatic_read.manually_selected);
        {
            let connection = db.lock().expect("database lock");
            let stored_json: String = connection
                .query_row(
                    "SELECT lyrics_json FROM track_lyrics WHERE track_id=?1",
                    [track_id.to_string()],
                    |row| row.get(0),
                )
                .expect("automatic row remains stored");
            let stored: TrackLyrics = serde_json::from_str(&stored_json).expect("stored JSON");
            assert_eq!(
                stored.lyrics.lines.len(),
                3,
                "read repair must not rewrite SQLite"
            );
        }

        let candidate = LyricCandidate {
            candidate_id: "netease:manual-colon-lrc".into(),
            provider: LyricProvider::NetEase,
            provider_track_id: Some("3340112782".into()),
            title: Some("Song".into()),
            artist: Some("artist".into()),
            album: Some("album".into()),
            duration_ms: Some(43_000),
            lyrics: legacy_parse,
            score: 1.0,
        };
        db.cache_lyric_candidates(track_id, std::slice::from_ref(&candidate), 20)
            .expect("cache manual candidate");
        let selected = db
            .select_lyric_candidate(track_id, &candidate.candidate_id, 30)
            .expect("select manual candidate");
        assert!(selected.manually_selected);
        assert_eq!(selected.lyrics.lines.len(), 43);
        assert!(selected
            .lyrics
            .lines
            .iter()
            .all(|line| line.start_ms.is_some()));

        let manual_read = db
            .get_track_lyrics(track_id)
            .expect("read manual cache")
            .expect("manual cache exists");
        assert!(manual_read.manually_selected);
        assert_eq!(manual_read.lyrics.lines.len(), 43);
        assert!(manual_read
            .lyrics
            .lines
            .iter()
            .all(|line| line.start_ms.is_some()));
        assert_eq!(manual_read.lyrics.raw_text, raw_text);
        let mut legacy_manual_parse = parse_lrc(&raw_text).expect("parse legacy manual LRC");
        legacy_manual_parse.lines.truncate(3);
        let legacy_manual = TrackLyrics {
            track_id,
            provider: LyricProvider::NetEase,
            candidate_id: Some(candidate.candidate_id.clone()),
            lyrics: legacy_manual_parse,
            manually_selected: true,
            updated_at_utc_ms: 30,
        };
        {
            let connection = db.lock().expect("database lock");
            connection
                .execute(
                    "UPDATE track_lyrics SET lyrics_json=?2 WHERE track_id=?1",
                    params![
                        track_id.to_string(),
                        serde_json::to_string(&legacy_manual).expect("legacy manual JSON")
                    ],
                )
                .expect("simulate previously persisted bad manual parse");
        }
        let repaired_manual_read = db
            .get_track_lyrics(track_id)
            .expect("read legacy manual cache")
            .expect("legacy manual cache exists");
        assert!(repaired_manual_read.manually_selected);
        assert_eq!(repaired_manual_read.lyrics.lines.len(), 43);
        assert!(repaired_manual_read
            .lyrics
            .lines
            .iter()
            .all(|line| line.start_ms.is_some()));
        let connection = db.lock().expect("database lock");
        let stored_json: String = connection
            .query_row(
                "SELECT lyrics_json FROM track_lyrics WHERE track_id=?1",
                [track_id.to_string()],
                |row| row.get(0),
            )
            .expect("manual row remains stored");
        let stored: TrackLyrics = serde_json::from_str(&stored_json).expect("stored JSON");
        assert!(stored.manually_selected);
        assert_eq!(
            stored.lyrics.lines.len(),
            3,
            "legacy manual cache is repaired only on read"
        );
    }

    #[test]
    fn legacy_plain_provider_lrc_cache_is_reparsed_into_timed_lines() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "legacy-plain-lrc");
        let track = record(
            &root,
            "legacy-plain-track",
            "legacy-plain-path",
            "Song",
            10,
            1,
        );
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&track),
            std::slice::from_ref(&track),
            1,
        );
        let track_id = track_id_for_query(&db, "Song");
        let raw_text = (0..43)
            .map(|second| format!("[02:{second:02}:04]synthetic line {second}"))
            .collect::<Vec<_>>()
            .join("\n");
        let legacy_lyrics = player_core::ParsedLyrics {
            format: player_core::LyricFormat::Plain,
            lines: raw_text
                .lines()
                .enumerate()
                .map(|(index, text)| player_core::LyricLine {
                    start_ms: None,
                    text: text.to_owned(),
                    translation: (index == 18).then(|| "legacy translation".to_owned()),
                    romanization: (index == 20).then(|| "legacy romanization".to_owned()),
                })
                .collect(),
            raw_text: raw_text.clone(),
            raw_karaoke: Some("retained raw karaoke".to_owned()),
            synced: false,
        };

        for provider in [LyricProvider::NetEase, LyricProvider::Qq] {
            for manually_selected in [false, true] {
                let legacy = TrackLyrics {
                    track_id,
                    provider,
                    candidate_id: Some("legacy:plain-lrc".to_owned()),
                    lyrics: legacy_lyrics.clone(),
                    manually_selected,
                    updated_at_utc_ms: 10,
                };
                let json = serde_json::to_string(&legacy).expect("legacy plain cache JSON");
                let connection = db.lock().expect("database lock");
                connection
                    .execute(
                        "INSERT INTO track_lyrics
                             (track_id, lyrics_json, manually_selected, updated_at_utc_ms, payload_bytes)
                         VALUES (?1, ?2, ?3, ?4, ?5)
                         ON CONFLICT(track_id) DO UPDATE SET
                             lyrics_json=excluded.lyrics_json,
                             manually_selected=excluded.manually_selected,
                             updated_at_utc_ms=excluded.updated_at_utc_ms,
                             payload_bytes=excluded.payload_bytes",
                        params![
                            track_id.to_string(),
                            json,
                            manually_selected,
                            10_i64,
                            json.len() as i64
                        ],
                    )
                    .expect("insert old serialized Plain cache");
                drop(connection);

                let repaired = db
                    .get_track_lyrics(track_id)
                    .expect("read old Plain cache")
                    .expect("cache exists");
                assert_eq!(repaired.provider, provider);
                assert_eq!(repaired.manually_selected, manually_selected);
                assert_eq!(repaired.lyrics.format, player_core::LyricFormat::Lrc);
                assert!(repaired.lyrics.synced);
                assert_eq!(repaired.lyrics.lines.len(), 43);
                assert!(repaired
                    .lyrics
                    .lines
                    .iter()
                    .all(|line| line.start_ms.is_some() && !line.text.starts_with('[')));
                assert_eq!(
                    repaired
                        .lyrics
                        .lines
                        .iter()
                        .map(|line| line.text.clone())
                        .collect::<Vec<_>>(),
                    (0..43)
                        .map(|second| format!("synthetic line {second}"))
                        .collect::<Vec<_>>()
                );
                assert_eq!(repaired.lyrics.lines[0].start_ms, Some(120_040));
                assert_eq!(repaired.lyrics.lines[18].start_ms, Some(138_040));
                assert_eq!(
                    repaired.lyrics.lines[18].translation.as_deref(),
                    Some("legacy translation")
                );
                assert_eq!(
                    repaired.lyrics.lines[20].romanization.as_deref(),
                    Some("legacy romanization")
                );
                assert_eq!(
                    repaired.lyrics.raw_karaoke.as_deref(),
                    Some("retained raw karaoke")
                );
                assert_eq!(repaired.lyrics.raw_text, raw_text);
                let connection = db.lock().expect("database lock");
                let stored_json: String = connection
                    .query_row(
                        "SELECT lyrics_json FROM track_lyrics WHERE track_id=?1",
                        [track_id.to_string()],
                        |row| row.get(0),
                    )
                    .expect("legacy cache remains stored");
                let stored: TrackLyrics = serde_json::from_str(&stored_json).expect("stored JSON");
                assert_eq!(stored.lyrics.format, player_core::LyricFormat::Plain);
                assert!(
                    !stored.lyrics.synced,
                    "read repair must not migrate the row"
                );
                assert!(stored.lyrics.lines[0].text.starts_with("[02:00:04]"));
            }
        }

        let candidate = LyricCandidate {
            candidate_id: "legacy:plain-candidate".to_owned(),
            provider: LyricProvider::NetEase,
            provider_track_id: Some("3340112782".to_owned()),
            title: Some("Song".to_owned()),
            artist: Some("artist".to_owned()),
            album: Some("album".to_owned()),
            duration_ms: Some(43_000),
            lyrics: legacy_lyrics,
            score: 1.0,
        };
        db.cache_lyric_candidates(track_id, std::slice::from_ref(&candidate), 20)
            .expect("cache legacy Plain candidate");
        let selected = db
            .select_lyric_candidate(track_id, &candidate.candidate_id, 30)
            .expect("select legacy Plain candidate");
        assert_eq!(selected.lyrics.format, player_core::LyricFormat::Lrc);
        assert!(selected.lyrics.synced);
        assert_eq!(selected.lyrics.lines.len(), 43);
        assert!(selected
            .lyrics
            .lines
            .iter()
            .all(|line| line.start_ms.is_some()));
    }

    #[test]
    fn metadata_backfill_keeps_committed_batches_across_database_reopen() {
        struct InterruptingIndex {
            source_id: SourceId,
            records: Vec<MediaTrackRecord>,
            reads: usize,
            cancel: SyncCancellation,
            cancel_at: Option<usize>,
        }

        impl MediaIndex for InterruptingIndex {
            fn scan(&mut self, _root: &LibraryRoot) -> SourceScan {
                SourceScan {
                    source_id: self.source_id,
                    state: SourceScanState::Complete,
                    tracks: self.records.clone(),
                    errors: Vec::new(),
                }
            }

            fn read_metadata(
                &mut self,
                _track: &MediaTrackRecord,
            ) -> Result<TrackMetadata, TrackMetadataError> {
                self.reads += 1;
                if self.cancel_at == Some(self.reads) {
                    self.cancel.cancel();
                }
                Ok(TrackMetadata {
                    title: Some(format!("backfilled {}", self.reads)),
                    year: Some(2024),
                    bit_depth: Some(24),
                    ..TrackMetadata::default()
                })
            }
        }

        let path =
            std::env::temp_dir().join(format!("moemusic-backfill-{}.sqlite", TrackId::new()));
        let root;
        let mut records;
        {
            let mut db = Database::open(&path).expect("open database");
            root = add_root(&db, MediaSourceKind::WindowsFilesystem, "backfill restart");
            records = (0..130)
                .map(|index| {
                    record(
                        &root,
                        &format!("legacy-{index:03}"),
                        &format!("path-key:{index:03}"),
                        "old metadata",
                        100,
                        1_800_000_000_000,
                    )
                })
                .collect::<Vec<_>>();
            apply(
                &mut db,
                &root,
                SourceScanState::Complete,
                &records,
                &records,
                1_800_000_000_001,
            );
            for record in &mut records {
                record.metadata = None;
            }
            db.lock()
                .expect("database lock")
                .execute(
                    "UPDATE tracks SET metadata_version=?1",
                    [i64::from(TRACK_METADATA_VERSION - 1)],
                )
                .expect("mark rows for migration backfill");

            let cancel = SyncCancellation::default();
            let mut index = InterruptingIndex {
                source_id: root.id,
                records: records.clone(),
                reads: 0,
                cancel: cancel.clone(),
                cancel_at: Some(128),
            };
            let report = SyncEngine::sync_cancellable_with_progress(
                &root,
                &mut index,
                &mut db,
                1_800_000_000_002,
                &cancel,
                |_| {},
            )
            .expect("partial backfill cancellation");
            assert!(report.cancelled);
            assert_eq!(index.reads, 128);
            assert_eq!(
                db.lock()
                    .expect("database lock")
                    .query_row(
                        "SELECT COUNT(*) FROM tracks WHERE metadata_version=?1",
                        [i64::from(TRACK_METADATA_VERSION)],
                        |row| row.get::<_, i64>(0),
                    )
                    .expect("count committed versions"),
                128
            );
            assert_eq!(
                db.lock()
                    .expect("database lock")
                    .query_row(
                        "SELECT COUNT(*) FROM tracks WHERE metadata_version=?1 AND year=2024",
                        [i64::from(TRACK_METADATA_VERSION)],
                        |row| row.get::<_, i64>(0),
                    )
                    .expect("count updated recording dates"),
                128
            );
        }

        {
            let mut db = Database::open(&path).expect("reopen after interruption");
            let mut index = InterruptingIndex {
                source_id: root.id,
                records: records.clone(),
                reads: 0,
                cancel: SyncCancellation::default(),
                cancel_at: None,
            };
            let report = SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_000_003)
                .expect("resume remaining metadata");
            assert!(!report.cancelled);
            assert_eq!(index.reads, 2);
            assert_eq!(
                db.lock()
                    .expect("database lock")
                    .query_row(
                        "SELECT COUNT(*) FROM tracks WHERE metadata_version=?1",
                        [i64::from(TRACK_METADATA_VERSION)],
                        |row| row.get::<_, i64>(0),
                    )
                    .expect("count completed versions"),
                130
            );
            assert_eq!(
                db.lock()
                    .expect("database lock")
                    .query_row(
                        "SELECT COUNT(*) FROM tracks WHERE metadata_version=?1 AND year=2024",
                        [i64::from(TRACK_METADATA_VERSION)],
                        |row| row.get::<_, i64>(0),
                    )
                    .expect("count updated recording dates"),
                130
            );
        }
        for suffix in ["", "-wal", "-shm"] {
            let mut file = path.as_os_str().to_os_string();
            file.push(suffix);
            let file = PathBuf::from(file);
            if file.exists() {
                std::fs::remove_file(file).expect("remove generated test database");
            }
        }
    }

    #[test]
    fn playlist_file_sync_state_round_trips_native_locator_and_digest() {
        let db = Database::open_in_memory().expect("database");
        let source_id = SourceId::new();
        let state = PlaylistFileSyncState {
            playlist_id: PlaylistId::new(),
            locator: MediaLocator::FileSystem(PathBuf::from(r"C:\音樂\清單.m3u8")),
            fingerprint: FileFingerprint {
                size_bytes: 11_487,
                modified_at_utc_ms: Some(1_800_000_000_001),
                created_at_utc_ms: None,
            },
            content_sha256: [0x5a; 32],
        };
        assert!(db
            .playlist_file_sync_state(source_id)
            .expect("no prior state")
            .is_none());
        db.record_playlist_file_sync_state(source_id, &state)
            .expect("save state");
        assert_eq!(
            db.playlist_file_sync_state(source_id).expect("load state"),
            Some(state)
        );
    }

    #[test]
    fn theme_preferences_round_trip_after_reopening_database() {
        let path = std::env::temp_dir().join(format!(
            "moemusic-theme-preferences-{}.sqlite",
            player_core::TrackId::new()
        ));
        let preferences = ThemePreferences {
            background_hex: "#102030".to_owned(),
            accent_hex: "#A0b1C2".to_owned(),
        };
        {
            let db = Database::open(&path).expect("open theme database");
            assert_eq!(
                db.get_theme_preferences().expect("default theme"),
                ThemePreferences::default()
            );
            db.set_theme_preferences(&preferences)
                .expect("save theme preferences");
            assert_eq!(
                db.get_theme_preferences().expect("saved theme"),
                preferences
            );
        }
        {
            let db = Database::open(&path).expect("reopen theme database");
            assert_eq!(
                db.get_theme_preferences().expect("reopened theme"),
                preferences
            );
        }
        for suffix in ["", "-wal", "-shm"] {
            let mut file = path.as_os_str().to_os_string();
            file.push(suffix);
            let file = PathBuf::from(file);
            if file.exists() {
                std::fs::remove_file(file).expect("remove generated theme database");
            }
        }
    }

    #[test]
    fn theme_preferences_reject_invalid_hex_without_changing_saved_values() {
        let db = Database::open_in_memory().expect("database");
        let original = db.get_theme_preferences().expect("default theme");
        let invalid = ThemePreferences {
            background_hex: "#000000".to_owned(),
            accent_hex: "#55D9FG".to_owned(),
        };
        assert!(matches!(
            db.set_theme_preferences(&invalid),
            Err(DatabaseError::InvalidThemeColor("accentHex"))
        ));
        assert_eq!(
            db.get_theme_preferences().expect("unchanged theme"),
            original
        );
    }

    #[test]
    fn clearing_legacy_theme_preferences_keeps_music_source_projection() {
        let db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "legacy-theme-test");
        db.set_theme_preferences(&ThemePreferences {
            background_hex: "#102030".to_owned(),
            accent_hex: "#A0B1C2".to_owned(),
        })
        .expect("save legacy preference");

        db.clear_legacy_theme_preferences()
            .expect("clear migrated preference");

        assert_eq!(
            db.get_theme_preferences().expect("empty legacy preference"),
            ThemePreferences::default()
        );
        assert_eq!(
            db.library_roots().expect("music source remains"),
            vec![root]
        );
    }

    #[test]
    fn playlist_save_matches_only_exact_locator_and_returns_ipc_safe_pages() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "source");
        let record = record(&root, "one", "path-key:one", "曲庫標題", 10, 100);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&record),
            std::slice::from_ref(&record),
            1000,
        );

        let mut playlist = player_core::Playlist::new("中文清單 🎵");
        playlist.entries = vec![
            PlaylistEntry {
                track_id: None,
                locator: record.locator.clone(),
                title: Some("清單顯示標題".to_owned()),
                duration_ms: Some(1_234),
            },
            PlaylistEntry {
                track_id: None,
                locator: MediaLocator::FileSystem(PathBuf::from(
                    r"C:\音樂\未索引 專輯 🎧\曲目 02.flac",
                )),
                title: Some("曲庫標題".to_owned()),
                duration_ms: None,
            },
        ];
        db.save_playlist(&playlist).expect("save playlist");

        let summaries = db.list_playlists().expect("list playlists");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, playlist.id);
        assert_eq!(summaries[0].name, "中文清單 🎵");
        assert_eq!(summaries[0].entry_count, 2);

        let page = db
            .get_playlist_page(playlist.id, 0, 1)
            .expect("load first page")
            .expect("playlist exists");
        assert_eq!(page.offset, 0);
        assert_eq!(page.limit, 1);
        assert_eq!(page.total_count, 2);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].track_id, Some(list(&db, 0, 10).items[0].id));
        assert_eq!(page.items[0].title.as_deref(), Some("清單顯示標題"));
        assert_eq!(page.items[0].artist.as_deref(), Some("artist"));
        assert_eq!(page.items[0].album.as_deref(), Some("album"));
        assert_eq!(page.items[0].duration_ms, Some(1_234));
        assert_eq!(page.items[0].codec.as_deref(), Some("FLAC"));
        assert_eq!(page.items[0].bitrate_bps, Some(900_000));
        assert_eq!(page.items[0].sample_rate_hz, Some(48_000));
        assert_eq!(page.items[0].year, Some(2001));
        assert_eq!(page.items[0].bit_depth, Some(24));
        assert!(page.items[0].has_enabled_mapping);

        let second_page = db
            .get_playlist_page(playlist.id, 1, 10)
            .expect("load second page")
            .expect("playlist exists");
        assert_eq!(second_page.total_count, 2);
        assert_eq!(second_page.items.len(), 1);
        assert_eq!(second_page.items[0].position, 1);
        assert_eq!(second_page.items[0].track_id, None);
        assert_eq!(second_page.items[0].title.as_deref(), Some("曲庫標題"));
        assert_eq!(second_page.items[0].year, None);
        assert_eq!(second_page.items[0].bit_depth, None);
        assert!(!second_page.items[0].has_enabled_mapping);

        let stored = db
            .get_playlist(playlist.id)
            .expect("load playlist for export")
            .expect("playlist exists");
        assert_eq!(stored.entries[0].track_id, page.items[0].track_id);
        assert_eq!(stored.entries[0].locator, playlist.entries[0].locator);
        assert_eq!(stored.entries[0].title, playlist.entries[0].title);
        assert_eq!(
            stored.entries[0].duration_ms,
            playlist.entries[0].duration_ms
        );
        assert_eq!(stored.entries[1].track_id, None);
        assert_eq!(stored.entries[1].locator, playlist.entries[1].locator);
        assert!(db
            .get_playlist_page(PlaylistId::new(), 0, 10)
            .expect("missing playlist query")
            .is_none());
        assert!(db.delete_playlist(playlist.id).expect("delete playlist"));
        assert!(db.list_playlists().expect("list after delete").is_empty());
    }

    #[test]
    fn playlist_page_sort_is_paged_and_keeps_source_positions_for_rows() {
        let db = Database::open_in_memory().expect("database");
        let mut playlist = Playlist::new("sortable");
        playlist.entries = vec![
            PlaylistEntry {
                track_id: None,
                locator: MediaLocator::FileSystem(PathBuf::from(r"C:\Music\z.flac")),
                title: Some("Zulu".to_owned()),
                duration_ms: None,
            },
            PlaylistEntry {
                track_id: None,
                locator: MediaLocator::FileSystem(PathBuf::from(r"C:\Music\a.flac")),
                title: Some("Alpha".to_owned()),
                duration_ms: None,
            },
            PlaylistEntry {
                track_id: None,
                locator: MediaLocator::FileSystem(PathBuf::from(r"C:\Music\m.flac")),
                title: Some("Mike".to_owned()),
                duration_ms: None,
            },
        ];
        db.save_playlist(&playlist).expect("save playlist");

        let by_title = db
            .get_playlist_page_sorted(
                playlist.id,
                0,
                2,
                PlaylistListSort {
                    field: PlaylistSortField::Title,
                    direction: PlaylistSortDirection::Asc,
                },
            )
            .expect("sort playlist")
            .expect("playlist exists");
        assert_eq!(
            by_title
                .items
                .iter()
                .map(|item| item.title.as_deref().unwrap())
                .collect::<Vec<_>>(),
            vec!["Alpha", "Mike"]
        );
        assert_eq!(
            by_title
                .items
                .iter()
                .map(|item| item.position)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(by_title.total_count, 3);
        assert_eq!(by_title.offset, 0);

        let grouped = db
            .quick_index_for_playlist(
                playlist.id,
                PlaylistListSort {
                    field: PlaylistSortField::Title,
                    direction: PlaylistSortDirection::Asc,
                },
            )
            .expect("build playlist group boundaries")
            .expect("playlist exists");
        assert_eq!(grouped.total_count, 3);
        assert_eq!(
            grouped
                .groups
                .iter()
                .map(|group| (group.label.as_str(), group.before_index))
                .collect::<Vec<_>>(),
            vec![("A", 0), ("M", 1), ("Z", 2)]
        );

        let by_position = db
            .get_playlist_page_sorted(
                playlist.id,
                0,
                3,
                PlaylistListSort {
                    field: PlaylistSortField::Position,
                    direction: PlaylistSortDirection::Desc,
                },
            )
            .expect("sort by source position")
            .expect("playlist exists");
        assert_eq!(
            by_position
                .items
                .iter()
                .map(|item| item.position)
                .collect::<Vec<_>>(),
            vec![2, 1, 0]
        );

        // Sorting is a page view only; the persisted playlist sequence remains unchanged.
        let original = db
            .get_playlist_page(playlist.id, 0, 3)
            .expect("default page")
            .expect("playlist exists");
        assert_eq!(
            original
                .items
                .iter()
                .map(|item| item.position)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(original.items[0].title.as_deref(), Some("Zulu"));
    }

    #[test]
    fn playlist_title_sort_uses_natural_numeric_order_in_both_directions() {
        let db = Database::open_in_memory().expect("database");
        let mut playlist = Playlist::new("natural title order");
        playlist.entries = ["Track 100", "Track 10", "Track 2", "Track 1"]
            .into_iter()
            .map(|title| PlaylistEntry {
                track_id: None,
                locator: MediaLocator::FileSystem(PathBuf::from(format!(r"C:\Music\{title}.flac"))),
                title: Some(title.to_owned()),
                duration_ms: None,
            })
            .collect();
        db.save_playlist(&playlist).expect("save playlist");

        for (direction, expected) in [
            (
                PlaylistSortDirection::Asc,
                vec!["Track 1", "Track 2", "Track 10", "Track 100"],
            ),
            (
                PlaylistSortDirection::Desc,
                vec!["Track 100", "Track 10", "Track 2", "Track 1"],
            ),
        ] {
            let page = db
                .get_playlist_page_sorted(
                    playlist.id,
                    0,
                    10,
                    PlaylistListSort {
                        field: PlaylistSortField::Title,
                        direction,
                    },
                )
                .expect("sort playlist")
                .expect("playlist exists");
            assert_eq!(
                page.items
                    .iter()
                    .map(|item| item.title.as_deref().unwrap())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn natural_title_comparison_handles_long_numbers_case_and_leading_zeroes() {
        let ascending = [
            "Disc 2",
            "Disc 02",
            "Disc 10",
            "Disc 999999999999999999999999999999999999999999",
        ];
        let mut actual = ascending.to_vec();
        actual.sort_by(|left, right| natural_title_cmp(left, right));
        assert_eq!(actual, ascending);
        assert_eq!(natural_title_cmp("song 3", "Song 12"), Ordering::Less);
        assert_eq!(natural_title_cmp("Song 12", "song 3"), Ordering::Greater);
    }

    #[test]
    fn music_collation_uses_explicit_zhuyin_and_natural_numbers() {
        let db = Database::from_connection_with_collation(
            Connection::open_in_memory().expect("connection"),
            MusicCollationSpec::from_locale_parts("zh-TW", Some(CollationType::Zhuyin)),
        )
        .expect("Taiwanese Zhuyin database");

        let names = vec![
            "字 2".to_owned(),
            "中 10".to_owned(),
            "中 2".to_owned(),
            "Track 100000000000000000000000000000000000000000".to_owned(),
            "Track 10".to_owned(),
            "Track 2".to_owned(),
        ];
        let indices = db
            .sort_text_indices(&names)
            .expect("sort names through SQLite");
        let sorted = indices
            .iter()
            .map(|index| names[*index].as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            sorted,
            [
                "Track 2",
                "Track 10",
                "Track 100000000000000000000000000000000000000000",
                "中 2",
                "中 10",
                "字 2",
            ],
            "symbols, numbers and Latin precede Taiwanese Zhuyin groups while numeric runs stay natural",
        );
    }

    #[test]
    fn locale_sort_categories_and_quick_index_share_the_same_icu_groups() {
        for (locale, collation_type, names, expected_order, expected_groups) in [
            (
                "zh-TW",
                Some(CollationType::Zhuyin),
                vec!["漢", "A", "10", "-", "1", "B", "不", "比", "爬", "迷"],
                vec!["-", "1", "10", "A", "B", "比", "不", "爬", "迷", "漢"],
                vec!["ㄅ", "ㄅ", "ㄆ", "ㄇ"],
            ),
            (
                "zh-CN",
                Some(CollationType::Pinyin),
                vec!["漢", "B", "10", "-", "1", "A", "不", "趴", "迷"],
                vec!["-", "1", "10", "A", "B", "不", "漢", "迷", "趴"],
                vec!["B", "P", "M"],
            ),
            (
                "ja-JP",
                None,
                vec!["東京", "カナ", "10", "-", "1", "かな", "A", "B", "漢"],
                vec!["-", "1", "10", "A", "B", "かな", "カナ", "漢", "東京"],
                vec!["か", "か", "漢字", "漢字"],
            ),
        ] {
            let db = Database::from_connection_with_collation(
                Connection::open_in_memory().expect("connection"),
                MusicCollationSpec::from_locale_parts(locale, collation_type),
            )
            .expect("locale database");
            let values = names
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>();
            let sorted = db
                .sort_text_indices(&values)
                .expect("sort values through SQLite")
                .into_iter()
                .map(|index| values[index].as_str().to_owned())
                .collect::<Vec<_>>();
            assert_eq!(sorted, expected_order, "sort category order for {locale}");

            let index_names = if locale == "zh-TW" {
                vec!["不", "比", "爬", "迷"]
            } else if locale == "zh-CN" {
                vec!["不", "趴", "迷"]
            } else {
                vec!["かな", "カナ", "漢", "東京"]
            };
            let (labels, groups) = db.quick_index_groupings(
                &index_names
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect::<Vec<_>>(),
            );
            assert_eq!(groups, expected_groups, "index groups for {locale}");
            let expected_labels =
                expected_groups
                    .iter()
                    .fold(Vec::<String>::new(), |mut labels, label| {
                        let label = (*label).to_owned();
                        if !labels.contains(&label) {
                            labels.push(label);
                        }
                        labels
                    });
            assert_eq!(labels, expected_labels);
        }
    }

    #[test]
    fn quick_index_snapshot_labels_are_unique_when_a_group_reappears() {
        let db = Database::from_connection_with_collation(
            Connection::open_in_memory().expect("connection"),
            MusicCollationSpec::from_locale_parts("zh-TW", Some(CollationType::Zhuyin)),
        )
        .expect("Taiwanese locale database");
        let values = ["無謬の花", "", "文明EP"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();

        let snapshot = db
            .quick_index_for_ordered_values(&values)
            .expect("quick-index snapshot");

        assert_eq!(
            snapshot
                .groups
                .iter()
                .map(|group| (group.label.as_str(), group.before_index))
                .collect::<Vec<_>>(),
            [("ㄨ", 0), ("未知", 1), ("ㄨ", 2)],
            "group rows preserve each real transition in the sorted list"
        );
        assert_eq!(
            snapshot.labels,
            ["ㄨ", "未知"],
            "the quick-index picker exposes each category once even when the list is interrupted by an unknown row"
        );
    }

    #[test]
    fn music_collation_ignores_padding_spaces_before_zhuyin_ordering() {
        let db = Database::from_connection_with_collation(
            Connection::open_in_memory().expect("connection"),
            MusicCollationSpec::from_locale_parts("zh-TW", Some(CollationType::Zhuyin)),
        )
        .expect("Taiwanese Zhuyin database");
        let names = [
            "不負韶光",
            "比浪水更輕的 Lighter Than Tears",
            " 蘋果糖漿 Apple Syrup",
            " 迷失鴿哨 Lost Pigeon Whistle",
            " 道中酣戰 Gruelling Battle",
            " 對手戲 Duodrama",
            "八珍玉食",
            "八重櫻",
        ];
        let sorted = db
            .sort_text_indices(&names.map(str::to_owned))
            .expect("sort names through SQLite")
            .into_iter()
            .map(|index| names[index])
            .collect::<Vec<_>>();
        assert_eq!(
            sorted,
            [
                "八珍玉食",
                "八重櫻",
                "比浪水更輕的 Lighter Than Tears",
                "不負韶光",
                " 蘋果糖漿 Apple Syrup",
                " 迷失鴿哨 Lost Pigeon Whistle",
                " 道中酣戰 Gruelling Battle",
                " 對手戲 Duodrama",
            ],
            "padding spaces must not move later Bopomofo initials ahead of the ㄅ group",
        );
    }

    #[test]
    fn playlist_track_position_matches_the_sorted_page_order() {
        let path = std::env::temp_dir().join(format!(
            "moemusic-playlist-position-{}.sqlite3",
            TrackId::new()
        ));
        let mut db = Database::open(&path).expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "playlist-position");
        let records: Vec<_> = ["delta", "alpha", "charlie", "bravo", "echo"]
            .iter()
            .enumerate()
            .map(|(index, name)| record(&root, name, name, name, 100 + index as u64, 1_000 + index as i64))
            .collect();
        apply(&mut db, &root, SourceScanState::Complete, &records, &records, 2_000);
        let tracks = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 100,
                query: None,
                field_filter: None,
                sort: Default::default(),
            })
            .expect("tracks")
            .items;
        // The playlist holds every track but the last one, in reverse order, and the first one twice.
        let mut playlist = Playlist::new("positions");
        let members: Vec<_> = tracks.iter().take(tracks.len() - 1).rev().collect();
        for track in members.iter().copied().chain(std::iter::once(members[members.len() - 1])) {
            playlist.entries.push(PlaylistEntry {
                track_id: Some(track.id),
                locator: MediaLocator::FileSystem(PathBuf::from(format!(r"C:\Music\{}.flac", track.title.clone().unwrap_or_default()))),
                title: None,
                duration_ms: None,
            });
        }
        db.save_playlist(&playlist).expect("save playlist");

        for field in [PlaylistSortField::Position, PlaylistSortField::Title, PlaylistSortField::Artist] {
            for direction in [PlaylistSortDirection::Asc, PlaylistSortDirection::Desc] {
                let sort = PlaylistListSort { field, direction };
                let page = db
                    .get_playlist_page_sorted(playlist.id, 0, 100, sort)
                    .expect("page")
                    .expect("playlist exists");
                for track in &members {
                    let first = page
                        .items
                        .iter()
                        .position(|item| item.track_id == Some(track.id))
                        .expect("in the page") as u64;
                    assert_eq!(
                        db.playlist_track_position(playlist.id, sort, track.id).expect("position"),
                        Some(first),
                        "{field:?} {direction:?}: {:?}",
                        track.title
                    );
                }
            }
        }
        let outside = tracks.last().expect("a track outside the playlist").id;
        assert_eq!(
            db.playlist_track_position(playlist.id, PlaylistListSort::default(), outside).unwrap(),
            None
        );
    }

    #[test]
    fn track_position_matches_the_page_order_for_every_sort_and_filter() {
        let path = std::env::temp_dir().join(format!(
            "moemusic-track-position-{}.sqlite3",
            TrackId::new()
        ));
        let mut db = Database::open(&path).expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "track-position");
        let records: Vec<_> = ["delta", "alpha", "charlie", "bravo", "echo"]
            .iter()
            .enumerate()
            .map(|(index, name)| {
                record(
                    &root,
                    name,
                    name,
                    name,
                    100 + index as u64,
                    1_000 + index as i64,
                )
            })
            .collect();
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            2_000,
        );

        for field in [
            TrackSortField::Title,
            TrackSortField::Artist,
            TrackSortField::Album,
        ] {
            for direction in [TrackSortDirection::Asc, TrackSortDirection::Desc] {
                for query in [None, Some("a".to_owned())] {
                    let request = ListTracksQuery {
                        offset: 0,
                        limit: 100,
                        query: query.clone(),
                        field_filter: None,
                        sort: TrackListSort { field, direction },
                    };
                    let page = db.list_tracks_page(request.clone()).expect("page");
                    assert!(!page.items.is_empty());
                    for (index, track) in page.items.iter().enumerate() {
                        assert_eq!(
                            db.track_position(&request, &[], track.id)
                                .expect("position"),
                            Some(index as u64),
                            "{field:?} {direction:?} {query:?}: {:?}",
                            track.title
                        );
                    }
                }
            }
        }
        // A track the filter hides, and one that does not exist, have no position.
        let request = ListTracksQuery {
            offset: 0,
            limit: 100,
            query: Some("charlie".to_owned()),
            field_filter: None,
            sort: Default::default(),
        };
        let hidden = track_id_for_query(&db, "alpha");
        assert_eq!(db.track_position(&request, &[], hidden).unwrap(), None);
        assert_eq!(
            db.track_position(&request, &[], TrackId::new()).unwrap(),
            None
        );
    }

    #[test]
    fn missing_track_title_uses_displayed_file_name_for_title_sorting() {
        let path = std::env::temp_dir().join(format!(
            "moemusic-missing-title-sort-{}.sqlite3",
            TrackId::new()
        ));
        let collation = MusicCollationSpec::from_locale_parts("zh-TW", Some(CollationType::Zhuyin));
        let mut db = Database::open_with_collation(&path, collation.clone())
            .expect("Taiwanese Zhuyin database");
        let root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "missing-title-sort",
        );
        let mut untagged = record(&root, "untagged", "untagged", "", 100, 1_000);
        untagged.locator = MediaLocator::FileSystem(PathBuf::from(r"C:\Music\不負韶光 伴奏.wav"));
        untagged.metadata.as_mut().expect("metadata").title = None;
        let tagged = record(
            &root,
            "tagged",
            "tagged",
            "比浪水更輕的 Lighter Than Tears",
            101,
            1_001,
        );
        let records = [untagged, tagged];
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            2_000,
        );

        let page = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
                field_filter: None,
                sort: TrackListSort {
                    field: TrackSortField::Title,
                    direction: TrackSortDirection::Asc,
                },
            })
            .expect("sorted page");
        let displayed_titles = page
            .items
            .iter()
            .map(|track| {
                player_core::display_track_title(track.title.as_deref(), track.file_name.as_deref())
                    .expect("visible title")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            displayed_titles,
            ["比浪水更輕的 Lighter Than Tears", "不負韶光 伴奏",],
            "a missing title tag must sort by the file name shown in the list",
        );

        db.lock()
            .expect("database lock")
            .execute("UPDATE tracks SET sort_title='' WHERE title IS NULL", [])
            .expect("simulate a pre-fix database row");
        drop(db);
        let repaired = Database::open_with_collation(&path, collation)
            .expect("reopen database and repair missing title sort keys");
        let repaired_page = repaired
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
                field_filter: None,
                sort: TrackListSort {
                    field: TrackSortField::Title,
                    direction: TrackSortDirection::Asc,
                },
            })
            .expect("page after startup repair");
        let repaired_sort_title: String = repaired
            .lock()
            .expect("database lock")
            .query_row(
                "SELECT sort_title FROM tracks WHERE title IS NULL LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("repaired title sort key");
        assert_eq!(repaired_sort_title, "不負韶光 伴奏");
        assert_eq!(
            repaired_page
                .items
                .iter()
                .map(|track| {
                    player_core::display_track_title(
                        track.title.as_deref(),
                        track.file_name.as_deref(),
                    )
                    .expect("visible title")
                })
                .collect::<Vec<_>>(),
            displayed_titles,
            "startup repair restores the correct order for existing databases",
        );
        drop(repaired);
        for suffix in ["", "-wal", "-shm"] {
            let file = if suffix.is_empty() {
                path.clone()
            } else {
                sidecar_path(&path, suffix)
            };
            let _ = std::fs::remove_file(file);
        }
    }

    #[test]
    fn zhuyin_order_places_bi_before_bu_in_library_sql() {
        let mut db = Database::from_connection_with_collation(
            Connection::open_in_memory().expect("connection"),
            MusicCollationSpec::from_locale_parts("zh-TW", Some(CollationType::Zhuyin)),
        )
        .expect("Taiwanese Zhuyin database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "zhuyin-bi-bu");
        let records = ["不負韶光 無駕帶伴奏", "比浪水更輕的 Lighter Than Tears"]
            .iter()
            .enumerate()
            .map(|(index, title)| {
                record(
                    &root,
                    &format!("item-{index}"),
                    &format!("path:item-{index}"),
                    title,
                    100 + index as u64,
                    1_000 + index as i64,
                )
            })
            .collect::<Vec<_>>();
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            2_000,
        );
        let page = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
                field_filter: None,
                sort: TrackListSort {
                    field: TrackSortField::Title,
                    direction: TrackSortDirection::Asc,
                },
            })
            .expect("sorted page");
        assert_eq!(
            page.items
                .iter()
                .map(|track| track.title.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["比浪水更輕的 Lighter Than Tears", "不負韶光 無駕帶伴奏"],
            "Taiwan Zhuyin order puts ㄅㄧ before ㄅㄨ",
        );
    }

    #[test]
    fn reopening_rebuilds_music_index_when_collation_callback_changed() {
        let path = std::env::temp_dir().join(format!(
            "moemusic-stale-collation-{}.sqlite3",
            TrackId::new()
        ));
        let collation = MusicCollationSpec::from_locale_parts("zh-TW", Some(CollationType::Zhuyin));
        let mut db = Database::open_with_collation(&path, collation.clone())
            .expect("create isolated database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "stale-collation");
        let records = ["不負韶光 無駕帶伴奏", "比浪水更輕的 Lighter Than Tears"]
            .iter()
            .enumerate()
            .map(|(index, title)| {
                record(
                    &root,
                    &format!("item-{index}"),
                    &format!("path:item-{index}"),
                    title,
                    100 + index as u64,
                    1_000 + index as i64,
                )
            })
            .collect::<Vec<_>>();
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            2_000,
        );
        drop(db);

        let legacy_connection = Connection::open(&path).expect("open legacy fixture");
        legacy_connection
            .create_collation(super::MUSIC_COLLATION_NAME, |left, right| left.cmp(right))
            .expect("register legacy binary-like comparator");
        legacy_connection
            .execute_batch(
                "DROP INDEX tracks_sort_title;
                 CREATE INDEX tracks_sort_title
                     ON tracks(sort_title COLLATE MUSIC_LOCALE_NATURAL, track_id);",
            )
            .expect("build index with old comparator");
        legacy_connection
            .execute(
                "UPDATE music_collation_metadata SET signature=?1 WHERE singleton=1",
                [collation
                    .signature()
                    .replace("|comparator=4", "|comparator=3")],
            )
            .expect("mark the legacy comparator signature");
        drop(legacy_connection);

        let reopened = Database::open_checked(&path)
            .expect("startup checks and reopen using the current ICU comparator");
        let page = reopened
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
                field_filter: None,
                sort: TrackListSort {
                    field: TrackSortField::Title,
                    direction: TrackSortDirection::Asc,
                },
            })
            .expect("read sorted page");
        let titles = [
            "不負韶光 無駕帶伴奏".to_owned(),
            "比浪水更輕的 Lighter Than Tears".to_owned(),
        ];
        let expected = reopened
            .sort_text_indices(&titles)
            .expect("sort titles through SQLite")
            .into_iter()
            .map(|index| titles[index].as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            page.items
                .iter()
                .map(|track| track.title.as_deref().unwrap())
                .collect::<Vec<_>>(),
            expected,
            "reopen must rebuild indexes produced by a prior comparator implementation",
        );

        drop(reopened);
        for suffix in ["", "-wal", "-shm"] {
            let file = if suffix.is_empty() {
                path.clone()
            } else {
                sidecar_path(&path, suffix)
            };
            let _ = std::fs::remove_file(file);
        }
    }

    #[test]
    fn track_title_artist_and_album_sql_sorts_share_icu_collation() {
        let mut db = Database::from_connection_with_collation(
            Connection::open_in_memory().expect("connection"),
            MusicCollationSpec::from_locale_parts("zh-TW", Some(CollationType::Zhuyin)),
        )
        .expect("Taiwanese Zhuyin database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "icu-sort-source");
        let fields = [
            ("字 2", "字 2", "專輯 10"),
            ("中 10", "中 10", "專輯 2"),
            ("中 2", "中 2", "專輯 1"),
        ];
        let records = fields
            .iter()
            .enumerate()
            .map(|(index, (title, artist, album))| {
                let mut item = record(
                    &root,
                    &format!("item-{index}"),
                    &format!("path:item-{index}"),
                    title,
                    100 + index as u64,
                    1_000 + index as i64,
                );
                let metadata = item.metadata.as_mut().expect("metadata");
                metadata.artist = Some((*artist).to_owned());
                metadata.album = Some((*album).to_owned());
                item
            })
            .collect::<Vec<_>>();
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            2_000,
        );

        for (field, expected) in [
            (TrackSortField::Title, vec!["中 2", "中 10", "字 2"]),
            (TrackSortField::Artist, vec!["中 2", "中 10", "字 2"]),
            (TrackSortField::Album, vec!["中 2", "中 10", "字 2"]),
        ] {
            let page = db
                .list_tracks_page(ListTracksQuery {
                    offset: 0,
                    limit: 10,
                    query: None,
                    field_filter: None,
                    sort: TrackListSort {
                        field,
                        direction: TrackSortDirection::Asc,
                    },
                })
                .expect("sorted page");
            assert_eq!(
                page.items
                    .iter()
                    .map(|track| track.title.as_deref().unwrap())
                    .collect::<Vec<_>>(),
                expected,
                "all text sort fields use the same ICU4X collation",
            );
        }
    }

    #[test]
    fn unknown_text_values_reverse_with_text_sort_direction() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "unknown-sort-source",
        );
        let records = [
            ("Alpha", Some("Alpha"), Some("Alpha album")),
            ("Zulu", Some("Zulu"), Some("Zulu album")),
            ("Missing", None, None),
            ("Blank", Some("  \t"), Some("  \t")),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (title, artist, album))| {
            let mut item = record(
                &root,
                &format!("unknown-{index}"),
                &format!("unknown-path-{index}"),
                title,
                100 + index as u64,
                1_000 + index as i64,
            );
            let metadata = item.metadata.as_mut().expect("metadata");
            metadata.artist = artist.map(str::to_owned);
            metadata.album = album.map(str::to_owned);
            item
        })
        .collect::<Vec<_>>();
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            2_000,
        );

        let tracks = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
                field_filter: None,
                sort: TrackListSort {
                    field: TrackSortField::Title,
                    direction: TrackSortDirection::Asc,
                },
            })
            .expect("read tracks")
            .items;
        let expected_named = [
            (TrackSortField::Artist, ["Alpha", "Zulu"]),
            (TrackSortField::Album, ["Alpha", "Zulu"]),
        ];

        for (field, asc_named) in expected_named {
            for (direction, named, unknown_first) in [
                (TrackSortDirection::Asc, asc_named, false),
                (TrackSortDirection::Desc, [asc_named[1], asc_named[0]], true),
            ] {
                let sort = TrackListSort { field, direction };
                let page = db
                    .list_tracks_page(ListTracksQuery {
                        offset: 0,
                        limit: 10,
                        query: None,
                        field_filter: None,
                        sort,
                    })
                    .expect("sort library page");
                let titles = page
                    .items
                    .iter()
                    .map(|track| track.title.as_deref().unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(titles.len(), 4);
                if unknown_first {
                    assert!(titles[..2]
                        .iter()
                        .all(|title| matches!(*title, "Missing" | "Blank")));
                    assert_eq!(&titles[2..], &named);
                } else {
                    assert_eq!(&titles[..2], &named);
                    assert!(titles[2..]
                        .iter()
                        .all(|title| matches!(*title, "Missing" | "Blank")));
                }
            }
        }

        let mut playlist = Playlist::new("unknown values");
        playlist.entries = tracks
            .iter()
            .map(|track| PlaylistEntry {
                track_id: Some(track.id),
                locator: MediaLocator::FileSystem(PathBuf::from(format!(
                    r"C:\Music\{}.flac",
                    track.title.as_deref().unwrap()
                ))),
                title: track.title.clone(),
                duration_ms: track.duration_ms,
            })
            .collect();
        db.save_playlist(&playlist).expect("save playlist");

        for field in [PlaylistSortField::Artist, PlaylistSortField::Album] {
            for (direction, named, unknown_first) in [
                (PlaylistSortDirection::Asc, ["Alpha", "Zulu"], false),
                (PlaylistSortDirection::Desc, ["Zulu", "Alpha"], true),
            ] {
                let page = db
                    .get_playlist_page_sorted(
                        playlist.id,
                        0,
                        10,
                        PlaylistListSort { field, direction },
                    )
                    .expect("read sorted playlist")
                    .expect("playlist exists");
                let titles = page
                    .items
                    .iter()
                    .map(|item| item.title.as_deref().unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(titles.len(), 4);
                if unknown_first {
                    assert!(titles[..2]
                        .iter()
                        .all(|title| matches!(*title, "Missing" | "Blank")));
                    assert_eq!(&titles[2..], &named);
                } else {
                    assert_eq!(&titles[..2], &named);
                    assert!(titles[2..]
                        .iter()
                        .all(|title| matches!(*title, "Missing" | "Blank")));
                }
            }
        }
    }

    #[test]
    fn collation_data_signature_change_rebuilds_the_title_index() {
        let path =
            std::env::temp_dir().join(format!("moemusic-collation-{}.sqlite3", TrackId::new()));
        let db = Database::open_with_collation(
            &path,
            MusicCollationSpec::from_locale_parts("zh-TW", Some(CollationType::Zhuyin)),
        )
        .expect("create database");
        let previous_signature = db.collation_spec.signature();
        drop(db);

        let connection = Connection::open(&path).expect("open fixture");
        connection
            .execute(
                "UPDATE music_collation_metadata SET signature=?1 WHERE singleton=1",
                [format!("outdated-{previous_signature}")],
            )
            .expect("mark stale ICU data signature");
        drop(connection);

        let reopened = Database::open_with_collation(
            &path,
            MusicCollationSpec::from_locale_parts("zh-TW", Some(CollationType::Zhuyin)),
        )
        .expect("rebuild collation index");
        let stored_signature: String = reopened
            .lock()
            .expect("database lock")
            .query_row(
                "SELECT signature FROM music_collation_metadata WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .expect("updated signature");
        assert_eq!(stored_signature, previous_signature);
        drop(reopened);
        for suffix in ["", "-wal", "-shm"] {
            let file = if suffix.is_empty() {
                path.clone()
            } else {
                sidecar_path(&path, suffix)
            };
            let _ = std::fs::remove_file(file);
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_sort_locale_names_map_to_icu_collation_types() {
        use super::split_windows_sort_name;

        assert_eq!(
            split_windows_sort_name("zh-TW_pronun"),
            ("zh-TW".to_owned(), Some(CollationType::Zhuyin)),
        );
        assert_eq!(
            split_windows_sort_name("zh-CN_stroke"),
            ("zh-CN".to_owned(), Some(CollationType::Stroke)),
        );
        assert_eq!(split_windows_sort_name("en-US"), ("en-US".to_owned(), None));
    }

    #[test]
    fn audio_quality_sort_direction_applies_to_codec_rate_depth_and_bitrate() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "audio-sort");
        let definitions = [
            ("mp3", "MP3", 48_000, None, Some(320_000)),
            ("mp4", "Mp4", 96_000, None, Some(256_000)),
            ("opus160", "Opus", 48_000, None, Some(160_000)),
            ("opus96", "Opus", 48_000, None, Some(96_000)),
            ("opus96k", "Opus", 96_000, None, Some(128_000)),
            ("vorbis", "Vorbis", 48_000, None, Some(192_000)),
            ("flac24", "Flac", 96_000, Some(24), None),
            ("flac16", "Flac", 96_000, Some(16), None),
            ("flac44", "Flac", 44_100, Some(24), None),
            ("wav", "Wav", 44_100, Some(16), None),
            ("dsd64", "DSD64", 2_822_400, Some(1), Some(5_644_800)),
            ("dsd128", "DSD128", 5_644_800, Some(1), Some(11_289_600)),
        ];
        let records = definitions
            .iter()
            .map(|(name, codec, sample_rate, bit_depth, bitrate)| {
                let mut item = record(&root, name, name, name, 10, 1_000);
                let metadata = item.metadata.as_mut().expect("metadata");
                metadata.codec = Some((*codec).to_owned());
                metadata.sample_rate_hz = Some(*sample_rate);
                metadata.bit_depth = *bit_depth;
                metadata.bitrate_bps = *bitrate;
                item
            })
            .collect::<Vec<_>>();
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            1_000,
        );

        let mut playlist = Playlist::new("audio format order");
        playlist.entries = records
            .iter()
            .map(|item| PlaylistEntry {
                track_id: None,
                locator: item.locator.clone(),
                title: None,
                duration_ms: None,
            })
            .collect();
        db.save_playlist(&playlist).expect("save playlist");

        let sorted = db
            .get_playlist_page_sorted(
                playlist.id,
                0,
                20,
                PlaylistListSort {
                    field: PlaylistSortField::AudioFormat,
                    direction: PlaylistSortDirection::Asc,
                },
            )
            .expect("sort by audio format")
            .expect("playlist exists");
        let codecs = sorted
            .items
            .iter()
            .map(|item| item.codec.as_deref().expect("codec"))
            .collect::<Vec<_>>();
        assert_eq!(
            codecs,
            vec![
                "MP3", "Mp4", "Opus", "Opus", "Opus", "Vorbis", "Flac", "Flac", "Flac", "Wav",
                "DSD64", "DSD128"
            ]
        );
        assert_eq!(sorted.items[2].bitrate_bps, Some(96_000));
        assert_eq!(sorted.items[3].bitrate_bps, Some(160_000));
        assert_eq!(sorted.items[4].sample_rate_hz, Some(96_000));
        assert_eq!(sorted.items[6].sample_rate_hz, Some(44_100));
        assert_eq!(sorted.items[7].bit_depth, Some(16));
        assert_eq!(sorted.items[8].bit_depth, Some(24));

        let library_sorted = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 20,
                query: None,
                field_filter: None,
                sort: player_core::TrackListSort {
                    field: player_core::TrackSortField::AudioFormat,
                    direction: player_core::TrackSortDirection::Asc,
                },
            })
            .expect("sort library by audio format");
        assert_eq!(
            library_sorted
                .items
                .iter()
                .map(|item| item.title.as_deref().expect("title"))
                .collect::<Vec<_>>(),
            vec![
                "mp3", "mp4", "opus96", "opus160", "opus96k", "vorbis", "flac44", "flac16",
                "flac24", "wav", "dsd64", "dsd128"
            ]
        );
        assert_eq!(library_sorted.items[2].bitrate_bps, Some(96_000));
        assert_eq!(library_sorted.items[3].bitrate_bps, Some(160_000));
        assert_eq!(library_sorted.items[4].sample_rate_hz, Some(96_000));
        assert_eq!(library_sorted.items[6].sample_rate_hz, Some(44_100));
        assert_eq!(library_sorted.items[7].bit_depth, Some(16));
        assert_eq!(library_sorted.items[8].bit_depth, Some(24));
        let queue_track_ids = db
            .list_track_ids_filtered(
                None,
                None,
                player_core::TrackListSort {
                    field: player_core::TrackSortField::AudioFormat,
                    direction: player_core::TrackSortDirection::Asc,
                },
            )
            .expect("sort queue track IDs by audio format");
        assert_eq!(
            queue_track_ids,
            library_sorted
                .items
                .iter()
                .map(|track| track.id)
                .collect::<Vec<_>>()
        );

        let playlist_desc = db
            .get_playlist_page_sorted(
                playlist.id,
                0,
                20,
                PlaylistListSort {
                    field: PlaylistSortField::AudioFormat,
                    direction: PlaylistSortDirection::Desc,
                },
            )
            .expect("sort playlist quality descending")
            .expect("playlist exists");
        assert_eq!(
            playlist_desc
                .items
                .iter()
                .map(|item| item.title.as_deref().expect("title"))
                .collect::<Vec<_>>(),
            vec![
                "dsd128", "dsd64", "wav", "flac24", "flac16", "flac44", "vorbis", "opus96k",
                "opus160", "opus96", "mp4", "mp3"
            ]
        );
        assert_eq!(playlist_desc.items[3].bit_depth, Some(24));
        assert_eq!(playlist_desc.items[4].bit_depth, Some(16));
        assert_eq!(playlist_desc.items[5].sample_rate_hz, Some(44_100));
        assert_eq!(playlist_desc.items[7].sample_rate_hz, Some(96_000));
        assert_eq!(playlist_desc.items[8].bitrate_bps, Some(160_000));
        assert_eq!(playlist_desc.items[9].bitrate_bps, Some(96_000));
        let library_desc = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 20,
                query: None,
                field_filter: None,
                sort: player_core::TrackListSort {
                    field: player_core::TrackSortField::AudioFormat,
                    direction: player_core::TrackSortDirection::Desc,
                },
            })
            .expect("sort library quality descending");
        assert_eq!(
            library_desc
                .items
                .iter()
                .map(|item| item.title.as_deref().expect("title"))
                .collect::<Vec<_>>(),
            vec![
                "dsd128", "dsd64", "wav", "flac24", "flac16", "flac44", "vorbis", "opus96k",
                "opus160", "opus96", "mp4", "mp3"
            ]
        );
        assert_eq!(library_desc.items[3].bit_depth, Some(24));
        assert_eq!(library_desc.items[4].bit_depth, Some(16));
        assert_eq!(library_desc.items[5].sample_rate_hz, Some(44_100));
        assert_eq!(library_desc.items[7].sample_rate_hz, Some(96_000));
        assert_eq!(library_desc.items[8].bitrate_bps, Some(160_000));
        assert_eq!(library_desc.items[9].bitrate_bps, Some(96_000));
        let queue_track_ids_desc = db
            .list_track_ids_filtered(
                None,
                None,
                player_core::TrackListSort {
                    field: player_core::TrackSortField::AudioFormat,
                    direction: player_core::TrackSortDirection::Desc,
                },
            )
            .expect("sort queue track IDs by audio format descending");
        assert_eq!(
            queue_track_ids_desc,
            library_desc
                .items
                .iter()
                .map(|track| track.id)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn missing_title_summary_uses_file_stem_from_the_locator() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "untitled");
        let mut record = record(&root, "blank", "blank-key", "ignored", 10, 100);
        record.metadata.as_mut().expect("metadata").title = Some("   ".to_owned());
        record.locator = MediaLocator::FileSystem(if cfg!(windows) {
            PathBuf::from(r"\\?\D:\Music\資料夾\曲 目.flac")
        } else {
            PathBuf::from("資料夾").join("曲 目.flac")
        });
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&record),
            std::slice::from_ref(&record),
            1000,
        );
        let page = list(&db, 0, 10);
        assert_eq!(page.total_count, 1);
        assert_eq!(page.items[0].title, None);
        assert_eq!(page.items[0].file_name.as_deref(), Some("曲 目"));
        assert_eq!(page.items[0].artist.as_deref(), Some("artist"));
        let summary = db
            .get_track_summary(page.items[0].id)
            .expect("summary")
            .expect("track");
        assert_eq!(summary.file_name.as_deref(), Some("曲 目"));
        let batch = db.get_track_summaries(&[page.items[0].id]).expect("batch");
        assert_eq!(
            batch[0]
                .as_ref()
                .and_then(|item| item.file_name.clone())
                .as_deref(),
            Some("曲 目")
        );

        let mut playlist = player_core::Playlist::new("untitled entries");
        playlist.entries = vec![
            PlaylistEntry {
                track_id: None,
                locator: MediaLocator::ContentUri(
                    "content://com.android.externalstorage.documents/document/primary%3AMusic%2F%E6%99%B4%E5%A4%A9.flac?displayName=secret.mp3".into(),
                ),
                title: Some("  ".to_owned()),
                duration_ms: None,
            },
            PlaylistEntry {
                track_id: None,
                locator: MediaLocator::ContentUri(
                    "content://media/external/audio/media/42?title=Nope".into(),
                ),
                title: None,
                duration_ms: None,
            },
        ];
        db.save_playlist(&playlist).expect("save playlist");
        let entries = db
            .get_playlist_page(playlist.id, 0, 10)
            .expect("page")
            .expect("playlist");
        assert_eq!(entries.items[0].title, None);
        assert_eq!(entries.items[0].file_name.as_deref(), Some("晴天"));
        assert_eq!(entries.items[0].track_id, None);
        assert_eq!(entries.items[1].title, None);
        assert_eq!(entries.items[1].file_name, None);

        let id = page.items[0].id;
        let json = format!(
            r#"{{"id":"{id}","title":"kept","artist":null,"album":null,"albumArtist":null,"trackNumber":null,"discNumber":null,"durationMs":null,"codec":null,"bitrateBps":null,"sampleRateHz":null,"year":null,"bitDepth":null,"playedMs":0}}"#
        );
        let loaded: player_core::TrackSummary = serde_json::from_str(&json).expect("old summary");
        assert_eq!(loaded.title.as_deref(), Some("kept"));
        assert_eq!(loaded.file_name, None);
    }

    #[test]
    fn queue_ids_match_library_page_order_and_playlist_order_keeps_duplicates() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "queue source");
        let first = record(&root, "one", "queue-one", "Beta", 10, 100);
        let second = record(&root, "two", "queue-two", "Alpha", 10, 100);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &[first.clone(), second.clone()],
            &[first.clone(), second.clone()],
            1000,
        );

        let page_ids = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 20,
                query: Some("a".to_owned()),
                field_filter: None,
                sort: Default::default(),
            })
            .expect("query matching library page")
            .items
            .into_iter()
            .map(|track| track.id)
            .collect::<Vec<_>>();
        assert_eq!(db.list_track_ids(Some("a")).expect("queue query"), page_ids);
        assert_eq!(
            db.list_track_ids(Some("  ")).expect("all queue IDs").len(),
            2
        );

        let mut playlist = player_core::Playlist::new("duplicates");
        playlist.entries = vec![
            PlaylistEntry {
                track_id: None,
                locator: first.locator.clone(),
                title: None,
                duration_ms: None,
            },
            PlaylistEntry {
                track_id: None,
                locator: MediaLocator::FileSystem(PathBuf::from(r"C:\Missing\not-indexed.flac")),
                title: None,
                duration_ms: None,
            },
            PlaylistEntry {
                track_id: None,
                locator: first.locator,
                title: None,
                duration_ms: None,
            },
            PlaylistEntry {
                track_id: None,
                locator: second.locator,
                title: None,
                duration_ms: None,
            },
        ];
        db.save_playlist(&playlist)
            .expect("save duplicate playlist");
        let queue = db
            .playlist_track_ids(playlist.id)
            .expect("playlist queue query")
            .expect("playlist exists");
        assert_eq!(queue.len(), 3);
        assert_eq!(queue[0].0, 0);
        assert_eq!(queue[0].1, queue[1].1);
        assert_eq!(queue[1].0, 2);
        assert_eq!(queue[2].0, 3);
        assert!(db
            .playlist_track_ids(PlaylistId::new())
            .expect("unknown playlist query")
            .is_none());
    }

    #[test]
    fn track_summary_batch_preserves_order_duplicate_ids_and_missing_slots() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "summary source");
        let track = record(
            &root,
            "summary-item",
            "summary-track",
            "Summary Song",
            10,
            100,
        );
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&track),
            std::slice::from_ref(&track),
            1000,
        );

        let track_id = db
            .list_track_ids(None)
            .expect("list inserted track")
            .into_iter()
            .next()
            .expect("inserted track ID");
        let missing_id = TrackId::new();
        let summaries = db
            .get_track_summaries(&[track_id, missing_id, track_id])
            .expect("resolve metadata batch");

        assert_eq!(summaries.len(), 3);
        assert_eq!(
            summaries[0].as_ref().map(|summary| summary.id),
            Some(track_id)
        );
        assert_eq!(
            summaries[0]
                .as_ref()
                .and_then(|summary| summary.title.as_deref()),
            Some("Summary Song")
        );
        assert!(
            summaries[1].is_none(),
            "missing Track IDs keep an empty slot"
        );
        assert_eq!(
            summaries[2].as_ref().map(|summary| summary.id),
            Some(track_id)
        );
    }

    #[test]
    fn ambiguous_exact_locator_is_not_guessed_from_title_or_source_order() {
        let mut db = Database::open_in_memory().expect("database");
        let first_root = add_root(&db, MediaSourceKind::WindowsFilesystem, "first");
        let second_root = add_root(&db, MediaSourceKind::WindowsFilesystem, "second");
        let path = MediaLocator::FileSystem(PathBuf::from(r"C:\Music\same.flac"));
        let mut first = record(&first_root, "one", "path-key:first", "same title", 10, 100);
        first.locator = path.clone();
        let mut second = record(
            &second_root,
            "two",
            "path-key:second",
            "same title",
            10,
            100,
        );
        second.locator = path.clone();
        apply(
            &mut db,
            &first_root,
            SourceScanState::Complete,
            std::slice::from_ref(&first),
            std::slice::from_ref(&first),
            1000,
        );
        apply(
            &mut db,
            &second_root,
            SourceScanState::Complete,
            std::slice::from_ref(&second),
            std::slice::from_ref(&second),
            1000,
        );

        let mut playlist = player_core::Playlist::new("ambiguous");
        playlist.entries.push(PlaylistEntry {
            track_id: None,
            locator: path,
            title: Some("same title".to_owned()),
            duration_ms: None,
        });
        db.save_playlist(&playlist).expect("save unresolved entry");
        let page = db
            .get_playlist_page(playlist.id, 0, 10)
            .expect("page query")
            .expect("playlist exists");
        assert_eq!(page.items[0].track_id, None);
        assert!(!page.items[0].has_enabled_mapping);
    }

    #[test]
    fn unresolved_playlist_locator_resolves_after_a_later_library_sync() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "source");
        let path = PathBuf::from(r"C:\音樂\稍後索引 🎧\曲目.flac");
        let mut playlist = player_core::Playlist::new("later sync");
        playlist.entries.push(PlaylistEntry {
            track_id: None,
            locator: MediaLocator::FileSystem(path.clone()),
            title: Some("清單標題".to_owned()),
            duration_ms: None,
        });
        db.save_playlist(&playlist)
            .expect("save before library sync");

        let unresolved = db
            .get_playlist_page(playlist.id, 0, 10)
            .expect("read unresolved page")
            .expect("playlist exists");
        assert_eq!(unresolved.items[0].track_id, None);
        assert!(!unresolved.items[0].has_enabled_mapping);

        let mut record = record(&root, "later", "later-path-key", "曲庫標題", 10, 100);
        record.locator = MediaLocator::FileSystem(path.clone());
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&record),
            std::slice::from_ref(&record),
            2000,
        );

        let resolved = db
            .get_playlist_page(playlist.id, 0, 10)
            .expect("read resolved page")
            .expect("playlist exists");
        assert!(resolved.items[0].track_id.is_some());
        assert_eq!(resolved.items[0].title.as_deref(), Some("清單標題"));
        assert!(resolved.items[0].has_enabled_mapping);
        let stored = db
            .get_playlist(playlist.id)
            .expect("load playlist")
            .expect("playlist exists");
        assert_eq!(stored.entries[0].locator, MediaLocator::FileSystem(path));
        assert_eq!(stored.entries[0].track_id, resolved.items[0].track_id);
    }

    #[test]
    fn file_database_reopens_unicode_playlist_entries() {
        let path =
            std::env::temp_dir().join(format!("moemusic-playlist-{}.sqlite", PlaylistId::new()));
        let mut playlist = player_core::Playlist::new("重開後保留 🎵");
        playlist.entries.push(PlaylistEntry {
            track_id: None,
            locator: MediaLocator::FileSystem(PathBuf::from(r"C:\音樂\長路徑 專輯 🎧\曲目.flac")),
            title: Some("手動清單標題".to_owned()),
            duration_ms: Some(12_345),
        });
        {
            let db = Database::open(&path).expect("open file database");
            db.save_playlist(&playlist).expect("save playlist");
        }
        {
            let db = Database::open(&path).expect("reopen file database");
            let restored = db
                .get_playlist(playlist.id)
                .expect("load playlist")
                .expect("playlist exists");
            assert_eq!(restored.id, playlist.id);
            assert_eq!(restored.name, playlist.name);
            assert_eq!(restored.entries, playlist.entries);
        }
        for suffix in ["", "-wal", "-shm"] {
            let mut file = path.as_os_str().to_os_string();
            file.push(suffix);
            let file = PathBuf::from(file);
            if file.exists() {
                std::fs::remove_file(file).expect("remove temporary playlist database");
            }
        }
    }

    #[test]
    fn checked_open_rejects_corrupt_main_with_a_pending_wal_without_writing_files() {
        assert_eq!(rusqlite::version(), "3.53.2");

        let directory = std::env::temp_dir().join(format!(
            "moemusic-wal-mismatch-{}-{}",
            std::process::id(),
            PlaylistId::new()
        ));
        fs::create_dir_all(&directory).expect("create isolated fixture directory");
        let source_path = directory.join("source.sqlite3");
        let checked_path = directory.join("checked.sqlite3");
        {
            let source = Connection::open(&source_path).expect("create WAL source database");
            source
                .pragma_update(None, "journal_mode", "WAL")
                .expect("enable WAL for source");
            source
                .pragma_update(None, "wal_autocheckpoint", 0)
                .expect("disable automatic checkpoint for fixture");
            source
                .execute_batch(
                    "CREATE TABLE entries(id INTEGER PRIMARY KEY, body TEXT NOT NULL);
                     WITH RECURSIVE n(value) AS (
                         SELECT 1 UNION ALL SELECT value + 1 FROM n WHERE value < 4000
                     ) INSERT INTO entries(id, body) SELECT value, printf('entry-%d', value) FROM n;",
                )
                .expect("seed source pages");
            let checkpoint: (i64, i64, i64) = source
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .expect("checkpoint the source baseline");
            assert_eq!(checkpoint.0, 0, "source baseline checkpoint was busy");

            source
                .pragma_update(None, "user_version", 99)
                .expect("write page-one frame into source WAL");
            let source_wal = sidecar_path(&source_path, "-wal");
            let source_shm = sidecar_path(&source_path, "-shm");
            assert!(fs::metadata(&source_wal).expect("source WAL exists").len() > 0);
            fs::copy(&source_path, &checked_path).expect("copy SQLite main file");
            fs::copy(&source_wal, sidecar_path(&checked_path, "-wal"))
                .expect("copy committed WAL frame");
            if source_shm.exists() {
                fs::copy(&source_shm, sidecar_path(&checked_path, "-shm"))
                    .expect("copy WAL index sidecar");
            }
        }

        // Page 2 is not represented by the copied page-one WAL frame. Make the
        // main/WAL pair inconsistent while keeping a syntactically valid WAL.
        let mut checked_main = fs::OpenOptions::new()
            .write(true)
            .open(&checked_path)
            .expect("open isolated main file for fixture corruption");
        checked_main
            .seek(SeekFrom::Start(4096))
            .expect("seek to page two");
        checked_main
            .write_all(&[0xff])
            .expect("damage a b-tree page not present in WAL");
        checked_main.flush().expect("flush fixture corruption");
        drop(checked_main);

        let main_before = fs::read(&checked_path).expect("capture isolated main file");
        let wal_path = sidecar_path(&checked_path, "-wal");
        let wal_before = fs::read(&wal_path).expect("capture isolated WAL");
        let result = Database::open_checked(&checked_path);
        let error = result.expect_err("checked open must reject the damaged pair");
        assert!(
            error.to_string().contains("startup quick_check failed"),
            "startup must reject the pair at the pre-migration guard: {error}"
        );
        assert_eq!(
            fs::read(&checked_path).expect("read main after rejected open"),
            main_before,
            "failed checked open must not rewrite the main database"
        );
        assert_eq!(
            fs::read(&wal_path).expect("read WAL after rejected open"),
            wal_before,
            "failed checked open must not checkpoint, truncate, or replace the WAL"
        );
        fs::remove_dir_all(&directory).expect("remove isolated fixture directory");
    }

    #[test]
    fn forced_process_exit_recovers_committed_wal_and_rolls_back_open_transaction() {
        assert_eq!(rusqlite::version(), "3.53.2");
        let directory = std::env::temp_dir().join(format!(
            "moemusic-wal-kill-{}-{}",
            std::process::id(),
            PlaylistId::new()
        ));
        fs::create_dir_all(&directory).expect("create isolated crash-test directory");
        let path = directory.join("crash.sqlite3");
        let status = Command::new(std::env::current_exe().expect("test executable path"))
            .arg("--exact")
            .arg("database::tests::sqlite_crash_child_writer")
            .arg("--nocapture")
            .env("MOE_DB_CRASH_TEST_PATH", &path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("start isolated SQLite child process");
        assert_eq!(
            status.code(),
            Some(73),
            "child exited inside an open transaction"
        );
        assert!(
            fs::metadata(sidecar_path(&path, "-wal"))
                .expect("committed WAL survives process exit")
                .len()
                > 0
        );

        let database = Database::open_checked(&path).expect("recover committed WAL on reopen");
        let connection = database.lock().expect("database mutex");
        let rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM crash_rows", [], |row| row.get(0))
            .expect("read recovered rows");
        let committed: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM crash_rows WHERE value='committed'",
                [],
                |row| row.get(0),
            )
            .expect("read committed row");
        assert_eq!(rows, 1, "the uncommitted transaction must be rolled back");
        assert_eq!(
            committed, 1,
            "the committed row must survive the process exit"
        );
        drop(connection);
        database
            .checkpoint_wal()
            .expect("checkpoint after recovery");
        drop(database);
        fs::remove_dir_all(&directory).expect("remove isolated crash-test directory");
    }

    #[test]
    fn sqlite_crash_child_writer() {
        let Some(path) = std::env::var_os("MOE_DB_CRASH_TEST_PATH") else {
            return;
        };
        assert_eq!(rusqlite::version(), "3.53.2");
        let mut connection = Connection::open(path).expect("open isolated child database");
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .expect("enable WAL");
        connection
            .pragma_update(None, "wal_autocheckpoint", 0)
            .expect("leave committed transaction in WAL");
        connection
            .pragma_update(None, "cache_size", 1)
            .expect("force small page cache");
        connection
            .execute_batch(
                "CREATE TABLE crash_rows(value TEXT NOT NULL);
                 INSERT INTO crash_rows(value) VALUES('committed');",
            )
            .expect("commit durable baseline");
        let transaction = connection.transaction().expect("begin open transaction");
        for index in 0..2_000 {
            transaction
                .execute(
                    "INSERT INTO crash_rows(value) VALUES(?1)",
                    [format!("uncommitted-{index}")],
                )
                .expect("write uncommitted rows");
        }
        std::process::exit(73);
    }

    #[test]
    #[ignore = "manual startup-check timing with 100,000 synthetic tracks"]
    fn measure_checked_open_100k_tracks_with_pending_wal() {
        assert_eq!(rusqlite::version(), "3.53.2");
        let directory = std::env::temp_dir().join(format!(
            "moemusic-open-check-100k-{}-{}",
            std::process::id(),
            PlaylistId::new()
        ));
        fs::create_dir_all(&directory).expect("create isolated benchmark directory");
        let path = directory.join("library.sqlite3");
        let mut database = Database::open(&path).expect("open isolated benchmark database");
        let root = add_root(&database, MediaSourceKind::WindowsFilesystem, "check-100k");
        let records = (0..100_000)
            .map(|index| {
                let item = format!("check-100k-{index}");
                record(
                    &root,
                    &item,
                    &format!("locator-check-100k-{index}"),
                    &format!("Track {index}"),
                    3_000_000 + index as u64,
                    1_800_000_000_000,
                )
            })
            .collect::<Vec<_>>();
        database
            .apply_source_scan(
                &root,
                &SourceScanState::Complete,
                &records,
                &records,
                &[],
                1_800_000_000_000,
            )
            .expect("insert synthetic track rows");
        drop(database);

        let writer = Connection::open(&path).expect("open isolated WAL writer");
        writer
            .pragma_update(None, "journal_mode", "WAL")
            .expect("enable WAL for timing fixture");
        writer
            .pragma_update(None, "wal_autocheckpoint", 0)
            .expect("disable timing-fixture auto checkpoint");
        writer
            .pragma_update(None, "application_id", 5_063_493_i64)
            .expect("leave one valid page in WAL");
        let started = std::time::Instant::now();
        let checked = Database::open_checked(&path).expect("checked open 100k fixture");
        let elapsed = started.elapsed();
        assert_eq!(checked.count_tracks(None).expect("count tracks"), 100_000);
        let database_bytes = fs::metadata(&path).expect("database file metadata").len();
        eprintln!(
            "sqlite_version={} tracks=100000 main_database_bytes={} checked_open_with_pending_wal_ms={}",
            rusqlite::version(),
            database_bytes,
            elapsed.as_millis()
        );
        drop(checked);
        drop(writer);

        let started = std::time::Instant::now();
        let reopened = Database::open_checked(&path).expect("checked open without pending WAL");
        let clean_open_elapsed = started.elapsed();
        assert_eq!(reopened.count_tracks(None).expect("count tracks"), 100_000);
        eprintln!(
            "sqlite_version={} tracks=100000 main_database_bytes={} checked_open_without_wal_ms={}",
            rusqlite::version(),
            fs::metadata(&path).expect("clean database metadata").len(),
            clean_open_elapsed.as_millis()
        );
        drop(reopened);
        fs::remove_dir_all(&directory).expect("remove isolated benchmark directory");
    }

    #[test]
    fn exact_locator_resolution_uses_the_migration_index() {
        let db = Database::open_in_memory().expect("database");
        let connection = db.lock().expect("connection");
        let mut statement = connection
            .prepare(
                "EXPLAIN QUERY PLAN
                 SELECT CASE WHEN COUNT(DISTINCT track_id)=1 THEN MIN(track_id) END
                 FROM source_mappings
                 WHERE locator_kind='filesystem_path'
                   AND locator_encoding='windows_utf16le'
                   AND locator_data=X'0000'",
            )
            .expect("query plan");
        let details = statement
            .query_map([], |row| row.get::<_, String>(3))
            .expect("query plan rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("query plan details")
            .join(" ");
        assert!(
            details.contains("source_mappings_locator_exact"),
            "{details}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn playlist_locator_matches_extended_windows_identity_and_preserves_imported_path() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "locator normalization",
        );
        let playlist_path =
            PathBuf::from(r"C:\Music\locator normalization\漢字 東京 🌸\長檔名 空白 123.flac");
        let indexed_path = PathBuf::from(format!(r"\\?\{}", playlist_path.display()));
        let mut indexed = record(
            &root,
            "unicode-long-path",
            &player_core::windows_locator_key(&indexed_path),
            "indexed title",
            12,
            100,
        );
        indexed.locator = MediaLocator::FileSystem(indexed_path);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&indexed),
            std::slice::from_ref(&indexed),
            1_000,
        );
        let indexed_id = list(&db, 0, 10).items[0].id;

        let mut playlist = player_core::Playlist::new("Windows locator identity");
        playlist.entries.push(PlaylistEntry {
            track_id: None,
            locator: MediaLocator::FileSystem(playlist_path.clone()),
            title: Some("imported title".to_owned()),
            duration_ms: Some(123_456),
        });
        db.save_playlist(&playlist).expect("save imported playlist");

        let page = db
            .get_playlist_page(playlist.id, 0, 10)
            .expect("read imported playlist page")
            .expect("playlist exists");
        assert_eq!(page.total_count, 1);
        assert_eq!(page.items[0].position, 0);
        assert_eq!(page.items[0].track_id, Some(indexed_id));
        assert!(page.items[0].has_enabled_mapping);

        let saved = db
            .get_playlist(playlist.id)
            .expect("read saved playlist")
            .expect("playlist exists");
        assert_eq!(saved.entries[0].track_id, Some(indexed_id));
        assert_eq!(
            saved.entries[0].locator,
            MediaLocator::FileSystem(playlist_path)
        );
        assert_eq!(saved.entries[0].title.as_deref(), Some("imported title"));
    }

    #[cfg(windows)]
    #[test]
    fn playlist_page_repairs_only_unmatched_entries_and_persists_after_reopen() {
        let path = std::env::temp_dir().join(format!(
            "moemusic-playlist-reconcile-{}.sqlite",
            PlaylistId::new()
        ));
        let root_path = PathBuf::from(r"C:\Music\reconcile root\漢字 東京 🌸\長檔名 空白 123.flac");
        let extended_path = PathBuf::from(format!(r"\\?\{}", root_path.display()));
        let mut playlist = player_core::Playlist::new("existing unresolved Hanser list");
        playlist.entries = vec![
            PlaylistEntry {
                track_id: None,
                locator: MediaLocator::FileSystem(root_path.clone()),
                title: Some("keep manual title".to_owned()),
                duration_ms: Some(100_000),
            },
            PlaylistEntry {
                track_id: None,
                locator: MediaLocator::FileSystem(root_path.clone()),
                title: Some("already assigned title".to_owned()),
                duration_ms: Some(200_000),
            },
        ];

        let (indexed_track_id, preassigned_track_id) = {
            let mut db = Database::open(&path).expect("open reconciliation database");
            let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "reconcile root");
            db.save_playlist(&playlist)
                .expect("save pre-existing unresolved playlist");
            let mut indexed = record(
                &root,
                "extended-unicode-item",
                &player_core::windows_locator_key(&extended_path),
                "A indexed track",
                12,
                100,
            );
            indexed.locator = MediaLocator::FileSystem(extended_path);
            let other = record(
                &root,
                "other-item",
                "other-locator-key",
                "B other track",
                13,
                101,
            );
            apply(
                &mut db,
                &root,
                SourceScanState::Complete,
                &[indexed.clone(), other.clone()],
                &[indexed, other],
                1_000,
            );
            let tracks = list(&db, 0, 10).items;
            let indexed_track_id = tracks
                .iter()
                .find(|track| track.title.as_deref() == Some("A indexed track"))
                .expect("indexed track")
                .id;
            let preassigned_track_id = tracks
                .iter()
                .find(|track| track.title.as_deref() == Some("B other track"))
                .expect("second track")
                .id;
            {
                let connection = db.lock().expect("database connection");
                connection
                    .execute(
                        "UPDATE playlist_entries SET track_id=?1
                         WHERE playlist_id=?2 AND position=1",
                        params![preassigned_track_id.to_string(), playlist.id.to_string()],
                    )
                    .expect("set the pre-existing second mapping");
            }
            let before = db
                .get_playlist(playlist.id)
                .expect("read existing playlist before repair")
                .expect("playlist exists");
            assert_eq!(before.entries[0].track_id, None);
            assert_eq!(before.entries[1].track_id, Some(preassigned_track_id));

            assert_eq!(
                db.playlist_track_ids(playlist.id)
                    .expect("repair queue items")
                    .expect("playlist exists"),
                vec![(0, indexed_track_id), (1, preassigned_track_id)]
            );
            let page = db
                .get_playlist_page(playlist.id, 0, 10)
                .expect("query existing playlist page")
                .expect("playlist exists");
            assert_eq!(page.items[0].track_id, Some(indexed_track_id));
            assert_eq!(page.items[1].track_id, Some(preassigned_track_id));
            assert!(page.items.iter().all(|item| item.has_enabled_mapping));
            let repaired = db
                .get_playlist(playlist.id)
                .expect("read repaired playlist")
                .expect("playlist exists");
            assert_eq!(repaired.entries[0].track_id, Some(indexed_track_id));
            assert_eq!(repaired.entries[1].track_id, Some(preassigned_track_id));
            assert_eq!(repaired.entries[0].locator, playlist.entries[0].locator);
            assert_eq!(
                repaired.entries[0].title.as_deref(),
                Some("keep manual title")
            );
            assert_eq!(repaired.entries[0].duration_ms, Some(100_000));
            assert_eq!(
                repaired.entries[1].title.as_deref(),
                Some("already assigned title")
            );
            (indexed_track_id, preassigned_track_id)
        };

        let reopened = Database::open(&path).expect("reopen reconciliation database");
        let restored = reopened
            .get_playlist(playlist.id)
            .expect("read playlist after reopen")
            .expect("playlist exists");
        assert_eq!(restored.entries[0].track_id, Some(indexed_track_id));
        assert_eq!(restored.entries[1].track_id, Some(preassigned_track_id));
        assert_eq!(restored.entries[0].locator, playlist.entries[0].locator);
        assert_eq!(
            restored.entries[0].title.as_deref(),
            Some("keep manual title")
        );
        let page = reopened
            .get_playlist_page(playlist.id, 0, 10)
            .expect("read page after reopen")
            .expect("playlist exists");
        assert_eq!(page.items[0].track_id, Some(indexed_track_id));
        assert_eq!(page.items[1].track_id, Some(preassigned_track_id));
        drop(reopened);

        for suffix in ["", "-wal", "-shm"] {
            let mut file = path.as_os_str().to_os_string();
            file.push(suffix);
            let file = PathBuf::from(file);
            if file.exists() {
                std::fs::remove_file(file).expect("remove temporary reconciliation database");
            }
        }
    }

    #[test]
    fn complete_scan_preserves_only_individually_failed_items_until_they_are_removed() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::PlaylistFile, "playlist source");
        let kept = record(&root, "kept", "kept-key", "kept", 10, 1_800_000_000_001);
        let temporarily_missing = record(
            &root,
            "missing",
            "missing-key",
            "missing",
            11,
            1_800_000_000_002,
        );
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &[kept.clone(), temporarily_missing.clone()],
            &[kept.clone(), temporarily_missing.clone()],
            1,
        );

        db.apply_source_scan(
            &root,
            &SourceScanState::Complete,
            std::slice::from_ref(&kept),
            &[],
            &[MediaSourceError {
                source_item_id: Some("missing".to_owned()),
                message: "one playlist track is temporarily unavailable".to_owned(),
            }],
            2,
        )
        .expect("apply partial per-item result");
        assert_eq!(list(&db, 0, 10).items.len(), 2);

        db.apply_source_scan(
            &root,
            &SourceScanState::Complete,
            std::slice::from_ref(&kept),
            &[],
            &[],
            3,
        )
        .expect("remove item after it disappears from the playlist");
        let tracks = list(&db, 0, 10).items;
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title.as_deref(), Some("kept"));
    }

    #[test]
    fn removing_playlist_source_keeps_track_when_another_source_still_maps_it() {
        let mut db = Database::open_in_memory().expect("database");
        let playlist_root = add_root(&db, MediaSourceKind::PlaylistFile, "playlist");
        let second_playlist_root = add_root(&db, MediaSourceKind::PlaylistFile, "second playlist");
        let folder_root = add_root(&db, MediaSourceKind::WindowsFilesystem, "folder");
        let playlist_record = record(
            &playlist_root,
            "same-song",
            "same-file",
            "same-song",
            10,
            1_800_000_000_001,
        );
        let mut folder_record = record(
            &folder_root,
            "same-song",
            "same-file",
            "same-song",
            10,
            1_800_000_000_001,
        );
        let playable_path = std::env::temp_dir().join(format!(
            "moemusic-shared-playlist-track-{}.mp3",
            SourceId::new()
        ));
        std::fs::write(&playable_path, b"playable test placeholder")
            .expect("create remaining source media file");
        folder_record.locator = MediaLocator::FileSystem(playable_path.clone());
        let second_playlist_record = record(
            &second_playlist_root,
            "same-song",
            "same-file",
            "same-song",
            10,
            1_800_000_000_001,
        );
        apply(
            &mut db,
            &playlist_root,
            SourceScanState::Complete,
            std::slice::from_ref(&playlist_record),
            std::slice::from_ref(&playlist_record),
            1,
        );
        apply(
            &mut db,
            &folder_root,
            SourceScanState::Complete,
            std::slice::from_ref(&folder_record),
            std::slice::from_ref(&folder_record),
            2,
        );
        apply(
            &mut db,
            &second_playlist_root,
            SourceScanState::Complete,
            std::slice::from_ref(&second_playlist_record),
            std::slice::from_ref(&second_playlist_record),
            3,
        );
        let shared_track = list(&db, 0, 10).items[0].id;
        db.remove_playlist_file_source(playlist_root.id)
            .expect("remove playlist source mappings");
        let page = list(&db, 0, 10);
        assert_eq!(page.total_count, 1);
        assert_eq!(page.items[0].id, shared_track);
        let mappings: i64 = db
            .lock()
            .expect("connection")
            .query_row(
                "SELECT COUNT(*) FROM source_mappings WHERE track_id=?1",
                [shared_track.to_string()],
                |row| row.get(0),
            )
            .expect("remaining mappings");
        assert_eq!(
            mappings, 2,
            "folder and second playlist still map the track"
        );
        assert_eq!(
            db.resolve_playable_filesystem_locator(shared_track)
                .expect("resolve remaining enabled source"),
            playable_path
        );
        std::fs::remove_file(playable_path).expect("remove test media file");
    }

    #[test]
    fn unavailable_incomplete_and_revoked_scans_never_remove_mappings() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "source");
        let record = record(
            &root,
            "one",
            "windows:path:c:/music/one.flac",
            "one",
            10,
            100,
        );
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&record),
            std::slice::from_ref(&record),
            1000,
        );
        assert_eq!(list(&db, 0, 10).total_count, 1);

        for state in [
            SourceScanState::Unavailable {
                reason: "NAS offline".to_owned(),
            },
            SourceScanState::Incomplete {
                reason: "directory could not be read".to_owned(),
            },
            SourceScanState::PermissionRevoked {
                reason: "permission revoked".to_owned(),
            },
        ] {
            apply(&mut db, &root, state, &[], &[], 2000);
            assert_eq!(list(&db, 0, 10).total_count, 1);
        }

        let sync_state = db
            .sync_state(root.id)
            .expect("read sync state")
            .expect("state row");
        assert_eq!(sync_state.state, "permission_revoked");
        assert_eq!(sync_state.last_success_utc_ms, Some(1000));

        apply(&mut db, &root, SourceScanState::Complete, &[], &[], 3000);
        assert_eq!(list(&db, 0, 10).total_count, 0);
    }

    #[test]
    fn system_index_and_filesystem_keys_resolve_to_one_track_and_keep_override() {
        let mut db = Database::open_in_memory().expect("database");
        let index_root = add_root(&db, MediaSourceKind::WindowsSystemIndex, "index");
        let index_record = record(
            &index_root,
            "system-item-42",
            "win-path-key:c:\\music\\album\\track.flac",
            "index title",
            10,
            100,
        );
        apply(
            &mut db,
            &index_root,
            SourceScanState::Complete,
            std::slice::from_ref(&index_record),
            std::slice::from_ref(&index_record),
            1000,
        );
        let first = list(&db, 0, 10).items.remove(0);
        db.set_user_override(first.id, UserMetadataField::Title, Some("manual title"))
            .expect("save override");

        let filesystem_root = add_root(&db, MediaSourceKind::WindowsFilesystem, "fallback");
        let fallback_record = record(
            &filesystem_root,
            "normalized-path-item",
            "win-path-key:c:\\music\\album\\track.flac",
            "fallback title",
            10,
            100,
        );
        apply(
            &mut db,
            &filesystem_root,
            SourceScanState::Complete,
            std::slice::from_ref(&fallback_record),
            std::slice::from_ref(&fallback_record),
            2000,
        );

        let after_fallback = list(&db, 0, 10);
        assert_eq!(after_fallback.total_count, 1);
        assert_eq!(after_fallback.items[0].id, first.id);
        assert_eq!(
            after_fallback.items[0].title.as_deref(),
            Some("manual title")
        );

        let updated_index_record = record(
            &index_root,
            "system-item-42",
            "win-path-key:c:\\music\\album\\track.flac",
            "new automatic title",
            11,
            200,
        );
        apply(
            &mut db,
            &index_root,
            SourceScanState::Complete,
            std::slice::from_ref(&updated_index_record),
            std::slice::from_ref(&updated_index_record),
            3000,
        );
        let after_metadata_update = list(&db, 0, 10);
        assert_eq!(after_metadata_update.items[0].id, first.id);
        assert_eq!(
            after_metadata_update.items[0].title.as_deref(),
            Some("manual title")
        );
    }

    #[test]
    fn library_pages_and_queue_ids_share_explicit_sort_order() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "sort-source");
        let alpha = record(&root, "a", "path:a", "Alpha", 10, 100);
        let beta = record(&root, "b", "path:b", "Beta", 20, 200);
        let gamma = record(&root, "c", "path:c", "Gamma", 30, 300);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &[alpha.clone(), beta.clone(), gamma.clone()],
            &[alpha.clone(), beta.clone(), gamma.clone()],
            1000,
        );
        let desc = TrackListSort {
            field: TrackSortField::Title,
            direction: TrackSortDirection::Desc,
        };
        let page = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
                field_filter: None,
                sort: desc,
            })
            .expect("title desc page");
        let ids = db
            .list_track_ids_filtered(None, None, desc)
            .expect("title desc queue ids");
        assert_eq!(
            page.items
                .iter()
                .map(|track| track.title.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("Gamma"), Some("Beta"), Some("Alpha")]
        );
        assert_eq!(
            ids,
            page.items.iter().map(|track| track.id).collect::<Vec<_>>()
        );

        let queue_candidates = vec![
            QueueSortCandidate {
                entry_index: 0,
                summary: Some(page.items[0].clone()),
            },
            QueueSortCandidate {
                entry_index: 1,
                summary: Some(page.items[1].clone()),
            },
            QueueSortCandidate {
                entry_index: 2,
                summary: Some(page.items[2].clone()),
            },
            QueueSortCandidate {
                entry_index: 3,
                summary: Some(page.items[0].clone()),
            },
            QueueSortCandidate {
                entry_index: 4,
                summary: None,
            },
        ];
        assert_eq!(
            db.sort_queue_candidates(
                &queue_candidates,
                TrackListSort {
                    field: TrackSortField::Title,
                    direction: TrackSortDirection::Asc,
                },
            )
            .expect("sort duplicate queue entries in SQLite"),
            [2, 1, 0, 3, 4]
        );
    }

    #[test]
    fn quick_index_for_tracks_uses_the_complete_filtered_sql_order() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "quick-index-source",
        );
        let records = [
            record(&root, "symbol", "path:symbol", "-Opening", 10, 100),
            record(&root, "number", "path:number", "10 Opening", 20, 200),
            record(&root, "latin", "path:latin", "Alpha", 30, 300),
            record(&root, "han-b", "path:han-b", "不負韶光", 40, 400),
            record(&root, "han-p", "path:han-p", "比翼", 50, 500),
        ];
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            1000,
        );
        let request = ListTracksQuery {
            offset: 0,
            limit: 1,
            query: None,
            field_filter: None,
            sort: TrackListSort {
                field: TrackSortField::Title,
                direction: TrackSortDirection::Asc,
            },
        };
        let snapshot = db
            .quick_index_for_tracks(&request, &[])
            .expect("quick index snapshot");
        assert_eq!(snapshot.total_count, 5);
        assert_eq!(
            snapshot.groups,
            vec![
                QuickIndexGroup {
                    label: "&".to_owned(),
                    before_index: 0
                },
                QuickIndexGroup {
                    label: "#".to_owned(),
                    before_index: 1
                },
                QuickIndexGroup {
                    label: "A".to_owned(),
                    before_index: 2
                },
                QuickIndexGroup {
                    label: "ㄅ".to_owned(),
                    before_index: 3
                },
            ]
        );
        let artist_snapshot = db
            .quick_index_for_artists(None, &[], &["artist".to_owned()], request.sort)
            .expect("artist detail quick index");
        let album_snapshot = db
            .quick_index_for_album_tracks(&["album".to_owned()], request.sort)
            .expect("album detail quick index");
        assert_eq!(artist_snapshot, snapshot);
        assert_eq!(album_snapshot, snapshot);
    }

    #[test]
    fn created_at_is_projected_from_file_fingerprints_and_sorted_by_full_timestamp() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "created-at-source");
        let mut records = [
            record(&root, "late", "path:late", "Late", 10, 100),
            record(&root, "early", "path:early", "Early", 20, 200),
            record(&root, "date-only", "path:date-only", "Unknown", 30, 300),
        ];
        records[0].fingerprint.created_at_utc_ms = Some(1_798_013_925_321);
        records[1].fingerprint.created_at_utc_ms = Some(1_798_013_925_123);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            1000,
        );

        let ascending = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
                field_filter: None,
                sort: TrackListSort {
                    field: TrackSortField::CreatedAt,
                    direction: TrackSortDirection::Asc,
                },
            })
            .expect("ascending creation timestamp sort");
        assert_eq!(
            ascending
                .items
                .iter()
                .map(|track| track.title.as_deref())
                .collect::<Vec<_>>(),
            [Some("Early"), Some("Late"), Some("Unknown")],
        );
        assert_eq!(
            ascending.items[0].created_at_utc_ms,
            Some(1_798_013_925_123)
        );
        assert_eq!(
            ascending.items[1].created_at_utc_ms,
            Some(1_798_013_925_321)
        );
        assert_eq!(ascending.items[2].created_at_utc_ms, None);

        let descending = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
                field_filter: None,
                sort: TrackListSort {
                    field: TrackSortField::CreatedAt,
                    direction: TrackSortDirection::Desc,
                },
            })
            .expect("descending creation timestamp sort");
        assert_eq!(
            descending
                .items
                .iter()
                .map(|track| track.title.as_deref())
                .collect::<Vec<_>>(),
            [Some("Late"), Some("Early"), Some("Unknown")],
            "missing creation dates stay last in either sort direction",
        );
    }

    #[test]
    fn library_title_sort_uses_natural_numeric_order_in_both_directions() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "natural-sort-source",
        );
        let records = ["Track 100", "Track 10", "Track 2", "Track 1"]
            .into_iter()
            .enumerate()
            .map(|(index, title)| {
                record(
                    &root,
                    &format!("item-{index}"),
                    &format!("path:item-{index}"),
                    title,
                    10 + index as u64,
                    100 + index as i64,
                )
            })
            .collect::<Vec<_>>();
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            1000,
        );

        for (direction, expected) in [
            (
                TrackSortDirection::Asc,
                vec!["Track 1", "Track 2", "Track 10", "Track 100"],
            ),
            (
                TrackSortDirection::Desc,
                vec!["Track 100", "Track 10", "Track 2", "Track 1"],
            ),
        ] {
            let page = db
                .list_tracks_page(ListTracksQuery {
                    offset: 0,
                    limit: 10,
                    query: None,
                    field_filter: None,
                    sort: TrackListSort {
                        field: TrackSortField::Title,
                        direction,
                    },
                })
                .expect("sort library");
            assert_eq!(
                page.items
                    .iter()
                    .map(|item| item.title.as_deref().unwrap())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn play_count_sort_uses_played_over_duration_not_raw_played_ms() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "play-count-sort");
        let mut short = record(&root, "short", "path:short", "Short", 10, 100);
        short.metadata.as_mut().unwrap().duration_ms = Some(100_000);
        let mut long = record(&root, "long", "path:long", "Long", 20, 200);
        long.metadata.as_mut().unwrap().duration_ms = Some(1_000_000);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &[short.clone(), long.clone()],
            &[short, long],
            1000,
        );
        let page = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
                field_filter: None,
                sort: Default::default(),
            })
            .expect("base page");
        let short_id = page
            .items
            .iter()
            .find(|track| track.title.as_deref() == Some("Short"))
            .expect("short track")
            .id;
        let long_id = page
            .items
            .iter()
            .find(|track| track.title.as_deref() == Some("Long"))
            .expect("long track")
            .id;
        db.lock()
            .expect("connection")
            .execute(
                "INSERT INTO track_playback_statistics(track_id, played_ms) VALUES (?1, ?2), (?3, ?4)",
                params![
                    short_id.to_string(),
                    200_000_i64,
                    long_id.to_string(),
                    500_000_i64
                ],
            )
            .expect("seed listening totals");

        let desc = TrackListSort {
            field: TrackSortField::PlayCount,
            direction: TrackSortDirection::Desc,
        };
        let sorted = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
                field_filter: None,
                sort: desc,
            })
            .expect("play-count desc page");
        assert_eq!(
            sorted
                .items
                .iter()
                .map(|track| track.title.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("Short"), Some("Long")],
            "2.0 plays should rank above 0.5 plays even when raw played_ms is lower"
        );
        assert_eq!(
            db.list_track_ids_filtered(None, None, desc)
                .expect("matching queue order"),
            sorted
                .items
                .iter()
                .map(|track| track.id)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn paginated_query_returns_total_without_loading_the_entire_library() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "source");
        let first = record(&root, "a", "path:a", "Alpha", 10, 100);
        let second = record(&root, "b", "path:b", "Beta", 20, 200);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &[first.clone(), second.clone()],
            &[first, second],
            1000,
        );

        let page = db
            .list_tracks_page(ListTracksQuery {
                offset: 1,
                limit: 1,
                query: None,
                field_filter: None,
                sort: Default::default(),
            })
            .expect("page query");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.offset, 1);
        assert_eq!(page.limit, 1);
        assert_eq!(page.total_count, 2);

        let filtered = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 50,
                query: Some("alpha".to_owned()),
                field_filter: None,
                sort: Default::default(),
            })
            .expect("text filter");
        assert_eq!(filtered.total_count, 1);
        assert_eq!(filtered.items[0].title.as_deref(), Some("Alpha"));
    }

    #[test]
    fn backend_snapshot_reads_filtered_sorted_results_without_renderer_page_cap() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "snapshot-source");
        let records = (0..1_205)
            .map(|index| {
                record(
                    &root,
                    &format!("item-{index}"),
                    &format!("path:{index}"),
                    &format!("Snapshot {index:04}"),
                    index as u64,
                    100,
                )
            })
            .collect::<Vec<_>>();
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            1_000,
        );
        let query = ListTracksQuery {
            query: Some("snapshot".to_owned()),
            sort: TrackListSort {
                field: TrackSortField::Title,
                direction: TrackSortDirection::Desc,
            },
            ..ListTracksQuery::default()
        };

        let all = db
            .list_tracks_all_with_alternates(&query, &[])
            .expect("one unbounded SQLite result");
        assert_eq!(all.len(), 1_205);
        assert_eq!(all[0].title.as_deref(), Some("Snapshot 1204"));
        assert_eq!(
            all.last().and_then(|track| track.title.as_deref()),
            Some("Snapshot 0000")
        );

        let backend_range = db
            .list_tracks_window_with_alternates(&query, &[], 1_100, 105)
            .expect("backend range beyond renderer cap");
        assert_eq!(backend_range.len(), 105);
        assert_eq!(
            backend_range.first().map(|track| track.id),
            Some(all[1_100].id)
        );
        assert_eq!(
            backend_range.last().map(|track| track.id),
            Some(all[1_204].id)
        );

        let renderer_page = db
            .list_tracks_page_with_alternates(
                ListTracksQuery {
                    limit: 5_000,
                    ..query
                },
                &[],
            )
            .expect("renderer page retains its cap");
        assert_eq!(renderer_page.items.len(), 1_000);
        assert_eq!(renderer_page.total_count, 1_205);
    }

    #[test]
    fn exact_field_filter_matches_page_count_and_queue_ids_with_unicode_and_overrides() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "field-filter");
        let mut first = record(&root, "first", "path:first", "Needle Song", 10, 100);
        let mut second = record(&root, "second", "path:second", "Second Track", 20, 200);
        let mut third = record(&root, "third", "path:third", "Needle / Third", 30, 300);
        for (track, artist, album) in [
            (&mut first, "hanser", "雨夜・專輯"),
            (&mut second, "hanser", "Needle Collection"),
            (&mut third, "漢ser", "雨夜・專輯"),
        ] {
            let metadata = track.metadata.as_mut().expect("metadata");
            metadata.artist = Some(artist.to_owned());
            metadata.album = Some(album.to_owned());
        }
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &[first.clone(), second.clone(), third.clone()],
            &[first, second, third],
            1_000,
        );

        let all = list(&db, 0, 20).items;
        let id_for = |title: &str| {
            all.iter()
                .find(|track| track.title.as_deref() == Some(title))
                .expect("fixture track")
                .id
        };
        let first_id = id_for("Needle Song");
        let second_id = id_for("Second Track");
        let third_id = id_for("Needle / Third");

        for (filter, expected) in [
            (
                TrackFieldFilter {
                    field: TrackField::Title,
                    value: "Needle Song".to_owned(),
                },
                vec![first_id],
            ),
            (
                TrackFieldFilter {
                    field: TrackField::Artist,
                    value: "hanser".to_owned(),
                },
                vec![first_id, second_id],
            ),
            (
                TrackFieldFilter {
                    field: TrackField::Album,
                    value: "雨夜・專輯".to_owned(),
                },
                vec![third_id, first_id],
            ),
        ] {
            let page = db
                .list_tracks_page(ListTracksQuery {
                    offset: 0,
                    limit: 1,
                    query: Some("needle".to_owned()),
                    field_filter: Some(filter.clone()),
                    sort: Default::default(),
                })
                .expect("exact field filtered page");
            let queue_ids = db
                .list_track_ids_filtered(Some("needle"), Some(&filter), Default::default())
                .expect("same exact field filter for queue");
            assert_eq!(queue_ids, expected);
            assert_eq!(page.total_count, expected.len() as u64);
            assert_eq!(
                page.items.first().map(|track| track.id),
                expected.first().copied()
            );
            let second_page = db
                .list_tracks_page(ListTracksQuery {
                    offset: 1,
                    limit: 1,
                    query: Some("needle".to_owned()),
                    field_filter: Some(filter),
                    sort: Default::default(),
                })
                .expect("second exact field filtered page");
            assert_eq!(second_page.total_count, expected.len() as u64);
            assert_eq!(
                second_page.items.first().map(|track| track.id),
                expected.get(1).copied()
            );
        }

        let binary_case_mismatch = TrackFieldFilter {
            field: TrackField::Artist,
            value: "Hanser".to_owned(),
        };
        assert!(db
            .list_track_ids_filtered(None, Some(&binary_case_mismatch), Default::default())
            .expect("binary case-sensitive value")
            .is_empty());

        db.set_user_override(third_id, UserMetadataField::Title, Some("手動・夜曲"))
            .expect("save effective title override");
        let overridden = TrackFieldFilter {
            field: TrackField::Title,
            value: "手動・夜曲".to_owned(),
        };
        assert_eq!(
            db.list_track_ids_filtered(None, Some(&overridden), Default::default())
                .expect("filter current user-facing title"),
            vec![third_id]
        );
    }

    #[test]
    fn simplified_traditional_query_union_matches_pages_and_queue_without_duplicate_tracks() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "script-search");
        let records = ["頭髮歌曲", "头发歌曲", "頭髮與头发"]
            .into_iter()
            .enumerate()
            .map(|(index, title)| {
                record(
                    &root,
                    &format!("script-{index}"),
                    &format!("path:script-{index}"),
                    title,
                    10 + index as u64,
                    100 + index as i64,
                )
            })
            .collect::<Vec<_>>();
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &records,
            1000,
        );

        let query = Some("頭髮".to_owned());
        let alternates = vec!["头发".to_owned()];
        let mut paged_ids = Vec::new();
        for offset in 0..3 {
            let page = db
                .list_tracks_page_with_alternates(
                    ListTracksQuery {
                        offset,
                        limit: 1,
                        query: query.clone(),
                        field_filter: None,
                        sort: Default::default(),
                    },
                    &alternates,
                )
                .expect("search script variants page");
            assert_eq!(page.total_count, 3);
            paged_ids.extend(page.items.into_iter().map(|track| track.id));
        }
        assert_eq!(paged_ids.len(), 3);
        assert_eq!(
            paged_ids
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3
        );

        let queue_ids = db
            .list_track_ids_filtered_with_alternates(
                query.as_deref(),
                &alternates,
                None,
                Default::default(),
            )
            .expect("queue uses both script variants");
        assert_eq!(queue_ids, paged_ids);
    }

    #[test]
    fn artist_filter_matches_each_name_separated_by_the_supported_delimiters() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "artist-split");
        let tags = [
            ("solo", "hanser"),
            ("slash", "hanser/yousa"),
            ("pipe", "hanser | yousa"),
            ("backslash", "hanser\\yousa"),
            ("semicolon", "hanser; yousa"),
            ("comma", "hanser,yousa"),
            ("foreign-name", "John Lennon"),
            ("foreign-collab", "John Lennon,hanser"),
            ("other", "yousa"),
            ("prefix", "hanser2"),
            ("suffix", "xyousa"),
        ];
        let mut records = Vec::new();
        for (index, (title, artist)) in tags.iter().enumerate() {
            let mut track = record(
                &root,
                title,
                &format!("path:{title}"),
                title,
                (index as u64 + 1) * 10,
                100,
            );
            track.metadata.as_mut().expect("metadata").artist = Some((*artist).to_owned());
            records.push(track);
        }
        let saved = records.clone();
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &saved,
            1_000,
        );
        let all = list(&db, 0, 20).items;
        let id_for = |title: &str| {
            all.iter()
                .find(|track| track.title.as_deref() == Some(title))
                .expect("fixture track")
                .id
        };
        let hanser_ids = db
            .list_track_ids_filtered(
                None,
                Some(&TrackFieldFilter {
                    field: TrackField::Artist,
                    value: "hanser".to_owned(),
                }),
                Default::default(),
            )
            .expect("hanser membership");
        assert_eq!(
            hanser_ids,
            vec![
                id_for("backslash"),
                id_for("comma"),
                id_for("foreign-collab"),
                id_for("pipe"),
                id_for("semicolon"),
                id_for("slash"),
                id_for("solo"),
            ]
        );
        let yousa_page = db
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 20,
                query: None,
                field_filter: Some(TrackFieldFilter {
                    field: TrackField::Artist,
                    value: "yousa".to_owned(),
                }),
                sort: Default::default(),
            })
            .expect("yousa page");
        let yousa_ids = db
            .list_track_ids_filtered(
                None,
                Some(&TrackFieldFilter {
                    field: TrackField::Artist,
                    value: "yousa".to_owned(),
                }),
                Default::default(),
            )
            .expect("yousa membership");
        assert_eq!(yousa_page.total_count, yousa_ids.len() as u64);
        assert_eq!(
            yousa_ids,
            vec![
                id_for("backslash"),
                id_for("comma"),
                id_for("other"),
                id_for("pipe"),
                id_for("semicolon"),
                id_for("slash"),
            ]
        );
        let john_ids = db
            .list_track_ids_filtered(
                None,
                Some(&TrackFieldFilter {
                    field: TrackField::Artist,
                    value: "John".to_owned(),
                }),
                Default::default(),
            )
            .expect("foreign-name first name must not match as a separate artist");
        assert!(john_ids.is_empty());
        let john_lennon_ids = db
            .list_track_ids_filtered(
                None,
                Some(&TrackFieldFilter {
                    field: TrackField::Artist,
                    value: "John Lennon".to_owned(),
                }),
                Default::default(),
            )
            .expect("complete artist name");
        assert_eq!(
            john_lennon_ids,
            vec![id_for("foreign-collab"), id_for("foreign-name")]
        );
        assert!(db
            .list_track_ids_filtered(
                None,
                Some(&TrackFieldFilter {
                    field: TrackField::Artist,
                    value: "Hanser".to_owned(),
                }),
                Default::default(),
            )
            .expect("case sensitive artist token")
            .is_empty());
    }

    #[test]
    fn artist_filter_matches_multiple_original_case_variants() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "artist-case-variants",
        );
        let mut records = Vec::new();
        for (item, artist) in [("upper", "REOL"), ("title-case", "Reol"), ("lower", "reol")] {
            let mut track = record(&root, item, &format!("path:{item}"), item, 1, 100);
            track.metadata.as_mut().expect("metadata").artist = Some(artist.to_owned());
            records.push(track);
        }
        let saved = records.clone();
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            &records,
            &saved,
            1_000,
        );

        let all = list(&db, 0, 10).items;
        let expected = all
            .iter()
            .filter(|track| matches!(track.title.as_deref(), Some("upper" | "title-case")))
            .map(|track| track.id)
            .collect::<Vec<_>>();
        let filtered = db
            .list_track_ids_filtered(
                None,
                Some(&TrackFieldFilter {
                    field: TrackField::Artist,
                    value: "REOL|Reol".to_owned(),
                }),
                Default::default(),
            )
            .expect("artist names from a merged identity");
        assert_eq!(filtered, expected);
    }

    #[test]
    fn track_locator_candidates_are_enabled_filesystem_first_and_keep_manual_metadata() {
        let mut db = Database::open_in_memory().expect("database");
        let fs_a_path = PathBuf::from(r"C:\音樂\甲\曲目 🎧.flac");
        let fs_b_path = PathBuf::from(r"D:\Music\乙\Track.flac");
        let uri_value = "content://media/external/audio/42".to_owned();
        let fs_a_root = db
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                "filesystem A",
                MediaLocator::FileSystem(PathBuf::from(r"C:\音樂\甲")),
            )
            .expect("add filesystem A root");
        let fs_b_root = db
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                "filesystem B",
                MediaLocator::FileSystem(PathBuf::from(r"D:\Music\乙")),
            )
            .expect("add filesystem B root");
        let uri_root = db
            .add_library_root(
                MediaSourceKind::AndroidMediaStore,
                "MediaStore",
                MediaLocator::ContentUri(uri_value.clone()),
            )
            .expect("add content URI root");
        let mut disabled_root = db
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                "disabled filesystem",
                MediaLocator::FileSystem(PathBuf::from(r"E:\Music\disabled")),
            )
            .expect("add disabled root");
        disabled_root.enabled = false;
        db.save_library_root(&disabled_root).expect("disable root");

        let shared_key = "shared-canonical-track-key";
        for (root, item, title, locator) in [
            (
                &fs_a_root,
                "filesystem-a-item",
                "automatic title A",
                MediaLocator::FileSystem(fs_a_path.clone()),
            ),
            (
                &fs_b_root,
                "filesystem-b-item",
                "automatic title B",
                MediaLocator::FileSystem(fs_b_path.clone()),
            ),
            (
                &uri_root,
                "mediastore-item",
                "automatic title URI",
                MediaLocator::ContentUri(uri_value.clone()),
            ),
            (
                &disabled_root,
                "disabled-item",
                "automatic title disabled",
                MediaLocator::FileSystem(PathBuf::from(r"E:\Music\disabled\track.flac")),
            ),
        ] {
            let mut mapped = record(root, item, shared_key, title, 20, 100);
            mapped.locator = locator;
            apply(
                &mut db,
                root,
                SourceScanState::Complete,
                std::slice::from_ref(&mapped),
                std::slice::from_ref(&mapped),
                1000,
            );
        }

        let track_id = track_id_for_query(&db, "automatic title disabled");
        assert_eq!(list(&db, 0, 20).total_count, 1);
        db.set_user_override(track_id, UserMetadataField::Title, Some("手動標題"))
            .expect("set manual title");

        let locators = db.track_locators(track_id).expect("read locators");
        let mut expected_files = [
            (fs_a_root.id.to_string(), fs_a_path),
            (fs_b_root.id.to_string(), fs_b_path),
        ];
        expected_files.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(
            locators,
            vec![
                MediaLocator::FileSystem(expected_files[0].1.clone()),
                MediaLocator::FileSystem(expected_files[1].1.clone()),
                MediaLocator::ContentUri(uri_value),
            ]
        );
        assert_eq!(
            db.get_track_summary(track_id)
                .expect("read track summary")
                .expect("track summary")
                .title
                .as_deref(),
            Some("手動標題")
        );
        assert!(db
            .get_track_summary(TrackId::new())
            .expect("unknown track summary")
            .is_none());
        assert!(matches!(
            db.track_locators(TrackId::new()),
            Err(DatabaseError::TrackNotFound(_))
        ));
    }

    #[test]
    fn filesystem_locator_resolution_preserves_unicode_and_reports_unavailable_sources() {
        let mut db = Database::open_in_memory().expect("database");
        let root_path = std::env::temp_dir()
            .join(format!("moemusic-resolve-{}", TrackId::new()))
            .join("音樂 資料夾 🎧");
        std::fs::create_dir_all(&root_path).expect("create Unicode source directory");
        let audio_path = root_path.join("夜色 - 曲目 🎵.flac");
        std::fs::write(&audio_path, b"fixture").expect("create local fixture file");

        let local_root = db
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                "Unicode filesystem",
                MediaLocator::FileSystem(root_path.clone()),
            )
            .expect("add local root");
        let mut local_record = record(
            &local_root,
            "unicode-local-item",
            "unicode-local-key",
            "Unicode local track",
            7,
            100,
        );
        local_record.locator = MediaLocator::FileSystem(audio_path.clone());
        apply(
            &mut db,
            &local_root,
            SourceScanState::Complete,
            std::slice::from_ref(&local_record),
            std::slice::from_ref(&local_record),
            1000,
        );
        let local_track_id = track_id_for_query(&db, "Unicode local track");
        assert_eq!(
            db.resolve_playable_filesystem_locator(local_track_id)
                .expect("resolve Unicode local path"),
            audio_path
        );

        let uri_value = "content://media/external/audio/99".to_owned();
        let uri_root = db
            .add_library_root(
                MediaSourceKind::AndroidMediaStore,
                "URI only source",
                MediaLocator::ContentUri(uri_value.clone()),
            )
            .expect("add URI source");
        let mut uri_record = record(
            &uri_root,
            "uri-only-item",
            "uri-only-key",
            "URI only track",
            5,
            100,
        );
        uri_record.locator = MediaLocator::ContentUri(uri_value);
        apply(
            &mut db,
            &uri_root,
            SourceScanState::Complete,
            std::slice::from_ref(&uri_record),
            std::slice::from_ref(&uri_record),
            1000,
        );
        let uri_track_id = track_id_for_query(&db, "URI only track");
        assert!(matches!(
            db.resolve_playable_filesystem_locator(uri_track_id),
            Err(DatabaseError::NonFilesystemTrackLocator(id)) if id == uri_track_id
        ));

        let mut disabled_root = db
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                "disabled source",
                MediaLocator::FileSystem(root_path.clone()),
            )
            .expect("add disabled source");
        disabled_root.enabled = false;
        db.save_library_root(&disabled_root)
            .expect("disable filesystem root");
        let mut disabled_record = record(
            &disabled_root,
            "disabled-only-item",
            "disabled-only-key",
            "Disabled only track",
            8,
            100,
        );
        disabled_record.locator = MediaLocator::FileSystem(root_path.join("disabled.flac"));
        apply(
            &mut db,
            &disabled_root,
            SourceScanState::Complete,
            std::slice::from_ref(&disabled_record),
            std::slice::from_ref(&disabled_record),
            1000,
        );
        let disabled_track_id = db
            .resolve_track_id_for_locator(&disabled_record.locator)
            .expect("resolve disabled track locator")
            .expect("disabled track metadata remains stored");
        assert_eq!(
            db.count_tracks(Some("Disabled only track"))
                .expect("disabled track is hidden from search"),
            0
        );
        assert!(matches!(
            db.resolve_playable_filesystem_locator(disabled_track_id),
            Err(DatabaseError::NoEnabledTrackMapping(id)) if id == disabled_track_id
        ));

        let offline_root = db
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                "offline source",
                MediaLocator::FileSystem(root_path.clone()),
            )
            .expect("add offline source");
        let missing_path = root_path.join("不存在的檔案.flac");
        assert!(!missing_path.exists());
        let mut offline_record = record(
            &offline_root,
            "offline-item",
            "offline-key",
            "Offline track",
            9,
            100,
        );
        offline_record.locator = MediaLocator::FileSystem(missing_path);
        apply(
            &mut db,
            &offline_root,
            SourceScanState::Complete,
            std::slice::from_ref(&offline_record),
            std::slice::from_ref(&offline_record),
            1000,
        );
        let offline_track_id = track_id_for_query(&db, "Offline track");
        assert!(matches!(
            db.resolve_playable_filesystem_locator(offline_track_id),
            Err(DatabaseError::TrackFileUnavailable(id)) if id == offline_track_id
        ));

        std::fs::remove_dir_all(root_path.parent().expect("temporary parent"))
            .expect("remove temporary Unicode fixture");
    }

    #[test]
    fn unfiltered_page_plan_uses_sort_and_mapping_indexes() {
        let db = Database::open_in_memory().expect("database");
        let connection = db.lock().expect("connection");
        let explain_page_sql = format!("EXPLAIN QUERY PLAN {TRACKS_PAGE_SQL}");
        let plan = connection
            .prepare(&explain_page_sql)
            .expect("explain page query")
            .query_map(
                rusqlite::params![
                    Option::<&str>::None,
                    100_i64,
                    0_i64,
                    Option::<&str>::None,
                    Option::<&str>::None,
                    Option::<&str>::None
                ],
                |row| row.get::<_, String>(3),
            )
            .expect("read query plan")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect query plan");
        let explain_count_sql = format!("EXPLAIN QUERY PLAN {COUNT_LIBRARY_SQL}");
        let count_plan = connection
            .prepare(&explain_count_sql)
            .expect("explain count query")
            .query_map([], |row| row.get::<_, String>(3))
            .expect("read count query plan")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect count query plan");
        println!("unfiltered page query plan: {plan:?}");
        println!("unfiltered count query plan: {count_plan:?}");
        assert!(
            plan.iter().any(|line| line.contains("tracks_sort_title")),
            "sort index missing from plan: {plan:?}"
        );
        assert!(
            plan.iter()
                .any(|line| line.contains("source_mappings_track_id")),
            "mapping index missing from plan: {plan:?}"
        );
    }

    struct CountingIndex {
        source_id: SourceId,
        records: Vec<MediaTrackRecord>,
        metadata_reads: usize,
    }

    impl MediaIndex for CountingIndex {
        fn scan(&mut self, _root: &LibraryRoot) -> SourceScan {
            SourceScan {
                source_id: self.source_id,
                state: SourceScanState::Complete,
                tracks: self.records.clone(),
                errors: Vec::new(),
            }
        }

        fn read_metadata(
            &mut self,
            track: &MediaTrackRecord,
        ) -> Result<TrackMetadata, TrackMetadataError> {
            self.metadata_reads += 1;
            Ok(TrackMetadata {
                title: Some(format!("parsed {}", track.identity.source_item_id)),
                ..TrackMetadata::default()
            })
        }
    }

    #[test]
    fn sync_engine_reads_only_new_or_changed_metadata_and_keeps_track_id() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::WindowsFilesystem, "source");
        let mut original = record(&root, "one", "path-key:one", "ignored", 10, 100);
        original.metadata = None;
        let mut index = CountingIndex {
            source_id: root.id,
            records: vec![original.clone()],
            metadata_reads: 0,
        };

        let first_report =
            SyncEngine::sync(&root, &mut index, &mut db, 1000).expect("initial sync");
        let track_id = list(&db, 0, 10).items[0].id;
        assert_eq!(first_report.metadata_reads, 1);
        assert_eq!(
            list(&db, 0, 10).items[0].title.as_deref(),
            Some("parsed one")
        );
        db.set_user_override(track_id, UserMetadataField::Title, Some("manual"))
            .expect("set user override");

        let second_report =
            SyncEngine::sync(&root, &mut index, &mut db, 2000).expect("unchanged sync");
        assert_eq!(second_report.metadata_reads, 0);
        assert_eq!(index.metadata_reads, 1);
        assert_eq!(list(&db, 0, 10).items[0].id, track_id);

        let mut changed = original;
        changed.fingerprint.size_bytes += 1;
        index.records = vec![changed];
        let third_report =
            SyncEngine::sync(&root, &mut index, &mut db, 3000).expect("changed sync");
        let after_change = list(&db, 0, 10);
        assert_eq!(third_report.metadata_reads, 1);
        assert_eq!(index.metadata_reads, 2);
        assert_eq!(after_change.items[0].id, track_id);
        assert_eq!(after_change.items[0].title.as_deref(), Some("manual"));
    }

    #[test]
    fn sync_timestamps_are_utc_epoch_milliseconds() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, MediaSourceKind::Other, "source");
        let at = SystemTime::UNIX_EPOCH
            .elapsed()
            .expect("current time after Unix epoch")
            .as_millis() as i64;
        apply(&mut db, &root, SourceScanState::Complete, &[], &[], at);
        assert_eq!(
            db.sync_state(root.id)
                .expect("sync state")
                .unwrap()
                .last_success_utc_ms,
            Some(at)
        );
    }

    #[test]
    fn playback_session_reopens_exact_hundred_thousand_entry_traversal_and_position() {
        let path =
            std::env::temp_dir().join(format!("moe-playback-session-{}.sqlite3", TrackId::new()));
        let duplicate = TrackId::new();
        let entries = (0..100_000)
            .map(|position| PlaybackQueueEntry {
                track_id: if position == 71 || position == 72 {
                    duplicate
                } else {
                    TrackId::new()
                },
                source_position: Some(position),
            })
            .collect();
        let mut queue = PlaybackQueue::with_entries(
            entries,
            PlaybackQueueContext::Playlist {
                playlist_id: "playlist-that-may-be-offline".to_owned(),
            },
            71,
        )
        .expect("queue");
        queue.set_repeat_mode(QueueRepeatMode::All);
        queue
            .set_traversal_order(
                (0..100_000).rev().collect(),
                TrackListSort {
                    field: TrackSortField::PlayCount,
                    direction: TrackSortDirection::Desc,
                },
            )
            .expect("set persisted queue sort");
        queue.set_shuffle(true);
        for _ in 0..173 {
            queue.next(true);
        }
        let expected = PlaybackSessionCheckpoint {
            queue: queue.snapshot(),
            position_ms: 93_417,
        };
        {
            let db = Database::open(&path).expect("isolated file database");
            db.save_playback_session(&expected)
                .expect("save exact session");
            db.checkpoint_playback_position(&expected.queue, 93_417)
                .expect("checkpoint position");
        }
        {
            let db = Database::open(&path).expect("reopen isolated database");
            let actual = db
                .load_playback_session()
                .expect("load session")
                .expect("saved session");
            assert_eq!(actual, expected);
            assert_eq!(
                actual.queue.entries[71].track_id,
                actual.queue.entries[72].track_id
            );
            assert_ne!(
                actual.queue.entries[71].source_position,
                actual.queue.entries[72].source_position
            );
            let current_index = actual.queue.play_order[actual.queue.cursor];
            assert_eq!(
                actual.queue.entries[current_index].track_id,
                expected.queue.entries[expected.queue.play_order[expected.queue.cursor]].track_id
            );
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn playback_checkpoint_rejects_corrupt_cursor_without_deleting_saved_queue() {
        let db = Database::open_in_memory().expect("database");
        let queue = PlaybackQueue::new(vec![TrackId::new(), TrackId::new()], 1).expect("queue");
        db.save_playback_session(&PlaybackSessionCheckpoint {
            queue: queue.snapshot(),
            position_ms: 6_000,
        })
        .expect("save session");
        db.lock()
            .expect("connection")
            .execute("UPDATE playback_session SET cursor=2", [])
            .expect("corrupt cursor fixture");
        assert!(matches!(
            db.load_playback_session(),
            Err(DatabaseError::CorruptData(_))
        ));
        let count: i64 = db
            .lock()
            .expect("connection")
            .query_row("SELECT COUNT(*) FROM playback_session_entries", [], |row| {
                row.get(0)
            })
            .expect("preserved queue entries");
        assert_eq!(count, 2);
    }

    #[test]
    fn pause_position_checkpoint_serializes_with_statistics_writer_without_losing_either() {
        let path =
            std::env::temp_dir().join(format!("moe-pause-stats-lock-{}.sqlite3", TrackId::new()));
        let session_db = Database::open(&path).expect("session connection");
        let stats_db = Database::open(&path).expect("statistics connection");
        let track_id = TrackId::new();
        let queue = PlaybackQueue::new(vec![track_id], 0).expect("queue");
        session_db
            .save_playback_session(&PlaybackSessionCheckpoint {
                queue: queue.snapshot(),
                position_ms: 10,
            })
            .expect("initial session checkpoint");
        let runtime_id = uuid::Uuid::new_v4();
        stats_db
            .register_playback_statistics_runtime(runtime_id, None, None)
            .expect("register statistics writer");

        stats_db
            .lock()
            .expect("statistics connection")
            .busy_timeout(Duration::ZERO)
            .expect("make first competing write report immediately");

        let (read_tx, read_rx) = mpsc::channel();
        let (continue_tx, continue_rx) = mpsc::channel();
        let (session_tx, session_rx) = mpsc::channel();
        let snapshot = queue.snapshot();
        let session_worker = thread::spawn(move || {
            let result =
                session_db.checkpoint_playback_position_after_read(&snapshot, 4_321, || {
                    read_tx.send(()).expect("signal session read snapshot");
                    continue_rx.recv().expect("continue session writer");
                });
            session_tx
                .send(result.map_err(|error| format!("{error:?}")))
                .expect("report session checkpoint");
        });
        read_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("session read snapshot before competing writer");

        let (writer_done_tx, writer_done_rx) = mpsc::channel();
        let (first_attempt_tx, first_attempt_rx) = mpsc::channel();
        let (retry_tx, retry_rx) = mpsc::channel();
        let writer = thread::spawn(move || {
            let first_result = stats_db
                .record_playback_checkpoints(
                    runtime_id,
                    &[PlaybackCheckpoint {
                        track_id,
                        played_ms: 1_234,
                        duration_ms: Some(5_000),
                    }],
                )
                .map_err(|error| format!("{error:?}"));
            first_attempt_tx
                .send(first_result.clone())
                .expect("report deterministic first stats attempt");
            let result = if first_result.is_ok() {
                first_result
            } else {
                retry_rx.recv().expect("retry after session checkpoint");
                stats_db
                    .lock()
                    .expect("statistics connection")
                    .busy_timeout(Duration::from_secs(5))
                    .expect("restore normal statistics busy timeout");
                stats_db
                    .record_playback_checkpoints(
                        runtime_id,
                        &[PlaybackCheckpoint {
                            track_id,
                            played_ms: 1_234,
                            duration_ms: Some(5_000),
                        }],
                    )
                    .map_err(|error| format!("{error:?}"))
            };
            writer_done_tx.send(result).expect("return stats writer");
        });

        let first_stats_attempt = first_attempt_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("statistics writer either commits or encounters the database lock");
        continue_tx.send(()).expect("release session checkpoint");

        let session_result = session_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("session checkpoint result");
        if first_stats_attempt.is_err() {
            retry_tx.send(()).expect("allow stats writer retry");
        }
        let stats_result = writer_done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("statistics writer completes after session writer");
        session_worker.join().expect("session worker joins");
        writer.join().expect("statistics worker joins");

        assert!(
            session_result.is_ok(),
            "pause must not fail with `保存播放狀態失敗：SQLite error: database is locked`: {session_result:?}; competing stats attempt: {first_stats_attempt:?}"
        );
        stats_result.expect("statistics checkpoint commits");
        let session_verify = Database::open(&path).expect("reopen session for verification");
        let stats_verify = Database::open(&path).expect("reopen stats for verification");
        assert_eq!(
            session_verify
                .load_playback_session()
                .expect("read saved session")
                .expect("session remains")
                .position_ms,
            4_321
        );
        let checkpoint = PlaybackCheckpoint {
            track_id,
            played_ms: 1_234,
            duration_ms: Some(5_000),
        };
        stats_verify
            .record_playback_checkpoints(runtime_id, &[checkpoint])
            .expect("idempotent checkpoint retry");
        assert_eq!(
            stats_verify
                .playback_statistics_for(&[track_id])
                .expect("read listening stats")[&track_id]
                .played_ms,
            1_234
        );

        drop(session_verify);
        drop(stats_verify);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn unavailable_track_ids_are_preserved_without_stored_paths_or_track_rows() {
        let db = Database::open_in_memory().expect("database");
        let offline_id = TrackId::new();
        let queue = PlaybackQueue::with_entries(
            vec![PlaybackQueueEntry {
                track_id: offline_id,
                source_position: Some(9),
            }],
            PlaybackQueueContext::Playlist {
                playlist_id: "offline-playlist".to_owned(),
            },
            0,
        )
        .expect("queue");
        db.save_playback_session(&PlaybackSessionCheckpoint {
            queue: queue.snapshot(),
            position_ms: 4_321,
        })
        .expect("save while offline");
        let restored = db
            .load_playback_session()
            .expect("load while offline")
            .expect("session retained");
        assert_eq!(restored.queue.entries[0].track_id, offline_id);
        assert_eq!(restored.position_ms, 4_321);
        let columns: Vec<String> = {
            let connection = db.lock().expect("connection");
            let mut statement = connection
                .prepare("PRAGMA table_info(playback_session_entries)")
                .expect("columns");
            statement
                .query_map([], |row| row.get(1))
                .expect("column names")
                .collect::<Result<_, _>>()
                .expect("column list")
        };
        assert!(!columns
            .iter()
            .any(|column| column.contains("path") || column.contains("locator")));
    }

    #[test]
    fn schema_v8_migrates_to_v15_without_requiring_track_rows_for_statistics() {
        let connection = rusqlite::Connection::open_in_memory().expect("legacy v8 database");
        connection.execute_batch(SCHEMA_V1).expect("schema v1");
        connection.execute_batch(SCHEMA_V2).expect("schema v2");
        connection.execute_batch(SCHEMA_V3).expect("schema v3");
        connection.execute_batch(SCHEMA_V4).expect("schema v4");
        connection.execute_batch(SCHEMA_V5).expect("schema v5");
        connection.execute_batch(SCHEMA_V6).expect("schema v6");
        connection.execute_batch(SCHEMA_V7).expect("schema v7");
        connection.execute_batch(SCHEMA_V8).expect("schema v8");
        connection
            .pragma_update(None, "user_version", 8)
            .expect("mark v8");

        let db = Database::from_connection(connection).expect("migrate to v14");
        let version: i64 = db
            .lock()
            .expect("connection")
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, 15);
        let table_count: i64 = db
            .lock()
            .expect("connection")
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'
                 AND name IN ('track_playback_statistics',
                              'playback_statistics_runtimes',
                              'playback_statistics_checkpoints')",
                [],
                |row| row.get(0),
            )
            .expect("statistics tables");
        assert_eq!(table_count, 3);
        let release_date_column: i64 = db
            .lock()
            .expect("connection")
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tracks') WHERE name='release_date'",
                [],
                |row| row.get(0),
            )
            .expect("release_date column");
        assert_eq!(release_date_column, 1);
        let queue_restore_columns: Vec<String> = {
            let connection = db.lock().expect("connection");
            let mut statement = connection
                .prepare("PRAGMA table_info(playback_session_entries)")
                .expect("queue entry columns");
            statement
                .query_map([], |row| row.get(1))
                .expect("queue entry column names")
                .collect::<Result<_, _>>()
                .expect("queue entry column list")
        };
        assert!(queue_restore_columns
            .iter()
            .any(|column| column == "shuffle_restore_order"));
        let sort_column: i64 = db
            .lock()
            .expect("connection")
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('playback_session') WHERE name='queue_sort_json'",
                [],
                |row| row.get(0),
            )
            .expect("queue sort column");
        assert_eq!(sort_column, 1);
    }

    #[test]
    fn schema_v9_queue_migrates_and_preserves_legacy_context_without_filter() {
        let connection = rusqlite::Connection::open_in_memory().expect("legacy v9 database");
        for schema in [
            SCHEMA_V1, SCHEMA_V2, SCHEMA_V3, SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7, SCHEMA_V8,
            SCHEMA_V9,
        ] {
            connection.execute_batch(schema).expect("legacy schema");
        }
        let track_id = TrackId::new();
        connection
            .execute(
                "INSERT INTO playback_session(
                    singleton_id, source_kind, source_playlist_id, source_query,
                    cursor, position_ms, shuffle, repeat_mode, random_state, entry_count
                 ) VALUES (1, 'library', NULL, 'ambient', 0, 735, 0, 'off', '1', 1)",
                [],
            )
            .expect("insert legacy queue header");
        connection
            .execute(
                "INSERT INTO playback_session_entries(
                    source_index, track_id, source_position, traversal_order
                 ) VALUES (0, ?1, NULL, 0)",
                [track_id.to_string()],
            )
            .expect("insert legacy queue entry");
        connection
            .pragma_update(None, "user_version", 9)
            .expect("mark legacy v9");

        let db = Database::from_connection(connection).expect("apply v10 migration");
        let session = db
            .load_playback_session()
            .expect("read migrated session")
            .expect("legacy queue remains");
        assert_eq!(session.position_ms, 735);
        assert_eq!(
            session.queue.entries[session.queue.play_order[session.queue.cursor]].track_id,
            track_id
        );
        assert_eq!(
            session.queue.context,
            PlaybackQueueContext::Library {
                query: Some("ambient".to_owned()),
                field_filter: None,
                sort: Default::default(),
            }
        );
    }

    #[test]
    fn playback_session_field_filter_survives_database_reopen() {
        let path = std::env::temp_dir().join(format!(
            "moe-session-field-filter-{}.sqlite3",
            TrackId::new()
        ));
        let track_id = TrackId::new();
        let filter = TrackFieldFilter {
            field: TrackField::Artist,
            value: "hanser 夜曲".to_owned(),
        };
        let queue = PlaybackQueue::with_entries(
            vec![PlaybackQueueEntry {
                track_id,
                source_position: None,
            }],
            PlaybackQueueContext::Library {
                query: Some("ambient".to_owned()),
                field_filter: Some(filter),
                sort: Default::default(),
            },
            0,
        )
        .expect("queue with exact field context");
        let expected = PlaybackSessionCheckpoint {
            queue: queue.snapshot(),
            position_ms: 1_234,
        };
        {
            let db = Database::open(&path).expect("isolated session database");
            db.save_playback_session(&expected)
                .expect("save filter context");
        }
        {
            let db = Database::open(&path).expect("reopen isolated session database");
            assert_eq!(
                db.load_playback_session()
                    .expect("load reopened session")
                    .expect("saved session"),
                expected
            );
        }
        for suffix in ["", "-wal", "-shm"] {
            let mut file = path.as_os_str().to_os_string();
            file.push(suffix);
            let file = PathBuf::from(file);
            if file.exists() {
                std::fs::remove_file(file).expect("remove isolated session database");
            }
        }
    }

    #[test]
    fn playback_statistics_checkpoints_are_idempotent_atomic_and_finishable() {
        let unique = TrackId::new().to_string();
        let path = std::env::temp_dir().join(format!("moemusic-statistics-{unique}.sqlite"));
        let first_track = TrackId::new();
        let second_track = TrackId::new();
        let first_runtime = uuid::Uuid::new_v4();
        let second_runtime = uuid::Uuid::new_v4();
        {
            let db = Database::open(&path).expect("open statistics database");
            db.register_playback_statistics_runtime(
                first_runtime,
                Some(101),
                Some(1_700_000_000_000),
            )
            .expect("register first runtime");
            db.register_playback_statistics_runtime(
                second_runtime,
                Some(202),
                Some(1_700_000_000_001),
            )
            .expect("register second runtime");
            let runtimes = db
                .list_playback_statistics_runtimes()
                .expect("list runtimes");
            let registered = runtimes
                .iter()
                .find(|runtime| runtime.runtime_id == first_runtime)
                .expect("first runtime registration");
            assert_eq!(registered.owner_pid, Some(101));
            assert_eq!(
                registered.owner_process_started_utc_ms,
                Some(1_700_000_000_000)
            );
            let first_batch = [
                PlaybackCheckpoint {
                    track_id: first_track,
                    played_ms: 500,
                    duration_ms: Some(1_000),
                },
                PlaybackCheckpoint {
                    track_id: second_track,
                    played_ms: 100,
                    duration_ms: None,
                },
            ];
            db.record_playback_checkpoints(first_runtime, &first_batch)
                .expect("record initial batch");
            db.record_playback_checkpoints(first_runtime, &first_batch)
                .expect("retry initial batch");
            db.record_playback_checkpoints(
                second_runtime,
                &[PlaybackCheckpoint {
                    track_id: first_track,
                    played_ms: 200,
                    duration_ms: Some(2_000),
                }],
            )
            .expect("record parallel runtime");

            let atomic_failure = [
                PlaybackCheckpoint {
                    track_id: first_track,
                    played_ms: 700,
                    duration_ms: Some(1_000),
                },
                PlaybackCheckpoint {
                    track_id: second_track,
                    played_ms: 99,
                    duration_ms: None,
                },
            ];
            assert!(matches!(
                db.record_playback_checkpoints(first_runtime, &atomic_failure),
                Err(DatabaseError::CorruptData(_))
            ));
            assert_eq!(
                db.playback_statistics_for(&[first_track, second_track])
                    .expect("read after rollback"),
                [
                    (
                        first_track,
                        PlaybackStatistics {
                            played_ms: 700,
                            duration_ms: Some(2_000)
                        }
                    ),
                    (
                        second_track,
                        PlaybackStatistics {
                            played_ms: 100,
                            duration_ms: None
                        }
                    ),
                ]
                .into_iter()
                .collect::<std::collections::HashMap<_, _>>()
            );
            assert_eq!(
                db.list_playback_statistics_runtimes()
                    .expect("list runtimes")
                    .len(),
                2
            );
        }

        {
            let db = Database::open(&path).expect("reopen statistics database");
            db.record_playback_checkpoints(
                first_runtime,
                &[PlaybackCheckpoint {
                    track_id: first_track,
                    played_ms: 500,
                    duration_ms: Some(2_100),
                }],
            )
            .expect("retry an acknowledged checkpoint after reopen");
            db.record_playback_checkpoints(
                first_runtime,
                &[PlaybackCheckpoint {
                    track_id: first_track,
                    played_ms: 700,
                    duration_ms: Some(2_100),
                }],
            )
            .expect("advance checkpoint after reopen");
            assert!(matches!(
                db.finish_playback_statistics_runtime(
                    first_runtime,
                    &[PlaybackCheckpoint {
                        track_id: first_track,
                        played_ms: 600,
                        duration_ms: Some(2_200),
                    }]
                ),
                Err(DatabaseError::CorruptData(_))
            ));
            assert!(db
                .list_playback_statistics_runtimes()
                .expect("runtime remains active after failed finish")
                .iter()
                .any(|runtime| runtime.runtime_id == first_runtime));
            db.finish_playback_statistics_runtime(
                first_runtime,
                &[
                    PlaybackCheckpoint {
                        track_id: first_track,
                        played_ms: 1_000,
                        duration_ms: Some(2_200),
                    },
                    PlaybackCheckpoint {
                        track_id: second_track,
                        played_ms: 150,
                        duration_ms: None,
                    },
                ],
            )
            .expect("finish with last batch");
            assert!(matches!(
                db.record_playback_checkpoints(
                    first_runtime,
                    &[PlaybackCheckpoint {
                        track_id: first_track,
                        played_ms: 1_500,
                        duration_ms: Some(2_200),
                    }]
                ),
                Err(DatabaseError::UnknownPlaybackStatisticsRuntime(id)) if id == first_runtime
            ));
            assert_eq!(
                db.playback_statistics_for(&[first_track, second_track])
                    .expect("read final totals"),
                [
                    (
                        first_track,
                        PlaybackStatistics {
                            played_ms: 1_200,
                            duration_ms: Some(2_200)
                        }
                    ),
                    (
                        second_track,
                        PlaybackStatistics {
                            played_ms: 150,
                            duration_ms: None
                        }
                    ),
                ]
                .into_iter()
                .collect::<std::collections::HashMap<_, _>>()
            );
            db.finish_playback_statistics_runtime(second_runtime, &[])
                .expect("finish parallel runtime");
            assert!(db
                .list_playback_statistics_runtimes()
                .expect("runtimes cleaned")
                .is_empty());
        }

        for suffix in ["", "-wal", "-shm"] {
            let mut file = path.as_os_str().to_os_string();
            file.push(suffix);
            let file = PathBuf::from(file);
            if file.exists() {
                std::fs::remove_file(file).expect("remove test database");
            }
        }
    }

    #[test]
    fn playback_statistics_survive_metadata_changes_and_track_removal() {
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(
            &db,
            MediaSourceKind::WindowsFilesystem,
            "statistics metadata",
        );
        let original = record(&root, "song", "song-key", "Before", 10, 1_800_000_000_001);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&original),
            std::slice::from_ref(&original),
            1,
        );
        let track_id = track_id_for_query(&db, "Before");
        let runtime_id = uuid::Uuid::new_v4();
        db.register_playback_statistics_runtime(runtime_id, None, None)
            .expect("register runtime");
        db.record_playback_checkpoints(
            runtime_id,
            &[PlaybackCheckpoint {
                track_id,
                played_ms: 400,
                duration_ms: Some(230_000),
            }],
        )
        .expect("record listening");

        let mut changed = record(&root, "song", "song-key", "After", 11, 1_800_000_000_002);
        changed.metadata.as_mut().expect("metadata").duration_ms = Some(100_000);
        apply(
            &mut db,
            &root,
            SourceScanState::Complete,
            std::slice::from_ref(&changed),
            std::slice::from_ref(&changed),
            2,
        );
        assert_eq!(
            db.playback_statistics_for(&[track_id])
                .expect("metadata duration wins")[&track_id],
            PlaybackStatistics {
                played_ms: 400,
                duration_ms: Some(100_000)
            }
        );

        {
            let connection = db.lock().expect("connection");
            connection
                .execute(
                    "DELETE FROM source_mappings WHERE track_id=?1",
                    [track_id.to_string()],
                )
                .expect("remove mapping fixture");
            connection
                .execute(
                    "DELETE FROM tracks WHERE track_id=?1",
                    [track_id.to_string()],
                )
                .expect("remove offline track fixture");
        }
        assert_eq!(
            db.playback_statistics_for(&[track_id])
                .expect("statistics without track row")[&track_id],
            PlaybackStatistics {
                played_ms: 400,
                duration_ms: Some(230_000)
            }
        );
    }

    #[test]
    fn playback_statistics_batch_query_handles_one_hundred_thousand_ids() {
        let db = Database::open_in_memory().expect("database");
        let track_ids: Vec<TrackId> = (0..100_000).map(|_| TrackId::new()).collect();
        let mut query_ids = track_ids.clone();
        query_ids.push(track_ids[0]);
        let statistics = db
            .playback_statistics_for(&query_ids)
            .expect("read large batch");
        assert_eq!(statistics.len(), 100_000);
        assert_eq!(
            statistics[&track_ids[0]],
            PlaybackStatistics {
                played_ms: 0,
                duration_ms: None
            }
        );
    }

    #[test]
    fn playback_statistics_overflow_rejects_batch_without_changing_watermarks() {
        let db = Database::open_in_memory().expect("database");
        let track_id = TrackId::new();
        let runtime_id = uuid::Uuid::new_v4();
        db.register_playback_statistics_runtime(runtime_id, None, None)
            .expect("register runtime");
        db.lock()
            .expect("connection")
            .execute(
                "INSERT INTO track_playback_statistics(track_id, played_ms)
                 VALUES (?1, ?2)",
                params![track_id.to_string(), i64::MAX],
            )
            .expect("set total at representable limit");
        let result = db.record_playback_checkpoints(
            runtime_id,
            &[PlaybackCheckpoint {
                track_id,
                played_ms: 1,
                duration_ms: Some(0),
            }],
        );
        assert!(matches!(result, Err(DatabaseError::InvalidNumber(_))));
        assert_eq!(
            db.playback_statistics_for(&[track_id]).expect("read total")[&track_id],
            PlaybackStatistics {
                played_ms: i64::MAX as u64,
                duration_ms: None,
            }
        );
        let watermark_count: i64 = db
            .lock()
            .expect("connection")
            .query_row(
                "SELECT COUNT(*) FROM playback_statistics_checkpoints
                 WHERE runtime_id=?1 AND track_id=?2",
                params![runtime_id.to_string(), track_id.to_string()],
                |row| row.get(0),
            )
            .expect("checkpoint count");
        assert_eq!(watermark_count, 0);
    }
}
