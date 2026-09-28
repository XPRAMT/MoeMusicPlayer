use std::{
    collections::HashSet,
    error::Error,
    fmt,
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use player_core::{
    FileFingerprint, LibraryRepository, LibraryRoot, ListTracksQuery, LyricCandidate,
    LyricProvider, MediaLocator, MediaSourceError, MediaSourceKind, MediaTrackRecord, Page,
    PlaybackQueueContext, PlaybackQueueEntry, PlaybackQueueSnapshot, Playlist, PlaylistEntry,
    PlaylistEntrySummary, PlaylistId, PlaylistPage, PlaylistSummary, QueueRepeatMode, SourceId,
    SourceScanState, SyncApplyOutcome, SyncApplyRequest, SyncApplyStats, SyncCancellation, TrackId,
    TrackIdentity, TrackLyrics, TrackMetadata, TrackSummary, TrackSyncState, UserMetadataField,
    TRACK_METADATA_VERSION,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::locator;

const SCHEMA_VERSION: i64 = 8;
const MAX_PAGE_SIZE: u32 = 500;
const MAX_TRACK_LYRICS_JSON_BYTES: usize = 6 * 1024 * 1024;
const MAX_AUTOMATIC_LYRICS_CACHE_BYTES: i64 = 64 * 1024 * 1024;
const MAX_LYRIC_CANDIDATE_JSON_BYTES: usize = 4 * 1024 * 1024;
const MAX_LYRIC_CANDIDATES_PER_TRACK: usize = 20;
const MAX_LYRIC_CANDIDATE_CACHE_BYTES: i64 = 16 * 1024 * 1024;
const COUNT_LIBRARY_SQL: &str = "SELECT COUNT(DISTINCT track_id) FROM source_mappings";
const COUNT_SEARCH_SQL: &str = "SELECT COUNT(DISTINCT m.track_id) FROM source_mappings m
    JOIN tracks t ON t.track_id=m.track_id
    WHERE COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) LIKE '%' || ?1 || '%' COLLATE NOCASE
       OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist) LIKE '%' || ?1 || '%' COLLATE NOCASE
       OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) LIKE '%' || ?1 || '%' COLLATE NOCASE
       OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist) LIKE '%' || ?1 || '%' COLLATE NOCASE";
const TRACKS_PAGE_SQL: &str = "SELECT t.track_id,
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist),
    t.track_number, t.disc_number, t.duration_ms, t.codec, t.bitrate_bps, t.sample_rate_hz,
    t.year, t.bit_depth
 FROM tracks t
 WHERE EXISTS (SELECT 1 FROM source_mappings m WHERE m.track_id=t.track_id)
   AND (?1 IS NULL
        OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) LIKE '%' || ?1 || '%' COLLATE NOCASE
        OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist) LIKE '%' || ?1 || '%' COLLATE NOCASE
        OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) LIKE '%' || ?1 || '%' COLLATE NOCASE
        OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist) LIKE '%' || ?1 || '%' COLLATE NOCASE)
 ORDER BY t.sort_title, t.track_id
 LIMIT ?2 OFFSET ?3";
const TRACK_SUMMARY_SQL: &str = "SELECT t.track_id,
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album),
    COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist),
    t.track_number, t.disc_number, t.duration_ms, t.codec, t.bitrate_bps, t.sample_rate_hz,
    t.year, t.bit_depth
 FROM tracks t WHERE t.track_id=?1";
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
           pe.duration_ms AS entry_duration_ms
    FROM playlist_entries pe
    WHERE pe.playlist_id=?1
)
SELECT e.position, e.track_id,
       COALESCE(e.entry_title,
           (SELECT value FROM track_overrides WHERE track_id=e.track_id AND field='title'),
           t.title),
       COALESCE((SELECT value FROM track_overrides WHERE track_id=e.track_id AND field='artist'), t.artist),
       COALESCE((SELECT value FROM track_overrides WHERE track_id=e.track_id AND field='album'), t.album),
       COALESCE(e.entry_duration_ms, t.duration_ms),
       t.codec, t.bitrate_bps, t.sample_rate_hz, t.year, t.bit_depth,
       EXISTS (
           SELECT 1 FROM source_mappings m
           JOIN library_roots r ON r.source_id=m.source_id
           WHERE m.track_id=e.track_id AND r.enabled=1
       )
FROM resolved_entries e
LEFT JOIN tracks t ON t.track_id=e.track_id
ORDER BY e.position
LIMIT ?2 OFFSET ?3
"#;

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

pub struct Database {
    connection: Mutex<Connection>,
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
    InvalidNumber(&'static str),
    SourceMismatch,
    TrackNotFound(TrackId),
    NoEnabledTrackMapping(TrackId),
    NonFilesystemTrackLocator(TrackId),
    TrackFileUnavailable(TrackId),
    InvalidThemeColor(&'static str),
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
            Self::TrackFileUnavailable(track_id) => write!(
                f,
                "track {track_id} has no currently accessible filesystem file; its source may be offline or the file may have been removed"
            ),
            Self::InvalidThemeColor(field) => write!(
                f,
                "theme preference {field} must be a six-digit hexadecimal color such as #55D9FF"
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
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DatabaseError> {
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
        Self::from_connection(connection)
    }

    pub fn open_in_memory() -> Result<Self, DatabaseError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self, DatabaseError> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        let database = Self {
            connection: Mutex::new(connection),
        };
        database.migrate()?;
        Ok(database)
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
        }
        connection.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS sync_seen_items (
                source_id TEXT NOT NULL, source_item_id TEXT NOT NULL,
                PRIMARY KEY (source_id, source_item_id)
            );",
        )?;
        Ok(())
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
        let mut statement = connection.prepare_cached(PLAYLIST_PAGE_SQL)?;
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
                    row.get::<_, Option<i64>>(10)?,
                    row.get::<_, bool>(11)?,
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
                    bit_depth,
                    has_enabled_mapping,
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
                Ok(PlaylistEntrySummary {
                    position,
                    track_id,
                    title,
                    artist,
                    album,
                    duration_ms,
                    has_enabled_mapping,
                    codec,
                    bitrate_bps: bitrate_bps.map(|value| value.max(0) as u32),
                    sample_rate_hz: sample_rate_hz.map(|value| value.max(0) as u32),
                    year: year
                        .and_then(|value| u16::try_from(value).ok())
                        .filter(|value| *value > 0),
                    bit_depth: bit_depth
                        .and_then(|value| u8::try_from(value).ok())
                        .filter(|value| *value > 0),
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
        let connection = self.lock()?;
        let search = request.query.filter(|text| !text.trim().is_empty());
        let limit = query_limit(request.limit);
        let search_value = search.as_deref();
        let total_count = count_tracks_in(&connection, search_value)?;
        let items = fetch_tracks_window(&connection, search_value, request.offset, limit)?;
        Ok(Page {
            items,
            offset: request.offset,
            limit,
            total_count: total_count.max(0) as u64,
        })
    }

    /// Return stable IDs in exactly the same filter and order as library pages.
    /// Playback queue setup keeps these compact IDs inside Rust.
    pub fn list_track_ids(&self, query: Option<&str>) -> Result<Vec<TrackId>, DatabaseError> {
        let connection = self.lock()?;
        let query = query.filter(|text| !text.trim().is_empty());
        let mut statement = connection.prepare(
            "SELECT t.track_id
             FROM tracks t
             WHERE EXISTS (SELECT 1 FROM source_mappings m WHERE m.track_id=t.track_id)
               AND (?1 IS NULL
                    OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='title'), t.title) LIKE '%' || ?1 || '%' COLLATE NOCASE
                    OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='artist'), t.artist) LIKE '%' || ?1 || '%' COLLATE NOCASE
                    OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album'), t.album) LIKE '%' || ?1 || '%' COLLATE NOCASE
                    OR COALESCE((SELECT value FROM track_overrides WHERE track_id=t.track_id AND field='album_artist'), t.album_artist) LIKE '%' || ?1 || '%' COLLATE NOCASE)
             ORDER BY t.sort_title, t.track_id",
        )?;
        let rows = statement.query_map([query], |row| row.get::<_, String>(0))?;
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
        Ok(count_tracks_in(&connection, query)?.max(0) as u64)
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
        Ok(connection
            .query_row(
                TRACK_SUMMARY_SQL,
                [track_id.to_string()],
                row_to_track_summary,
            )
            .optional()?)
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
        for candidate in candidates.iter().take(MAX_LYRIC_CANDIDATES_PER_TRACK) {
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
        prune_lyric_candidate_cache(&tx)?;
        tx.commit()?;
        Ok(())
    }

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
        let selected = TrackLyrics {
            track_id,
            provider: candidate.provider,
            candidate_id: Some(candidate.candidate_id),
            lyrics: candidate.lyrics,
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
            let sort_title = if let Some(value) = value {
                value.to_lowercase()
            } else {
                tx.query_row(
                    "SELECT lower(COALESCE(title, '')) FROM tracks WHERE track_id=?1",
                    [track_id.to_string()],
                    |row| row.get::<_, String>(0),
                )?
            };
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
        let (source_kind, source_playlist_id, source_query) = match &checkpoint.queue.context {
            PlaybackQueueContext::Library { query } => ("library", None, query.as_deref()),
            PlaybackQueueContext::Playlist { playlist_id } => {
                ("playlist", Some(playlist_id.as_str()), None)
            }
        };
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM playback_session_entries", [])?;
        tx.execute(
            "INSERT INTO playback_session(
                singleton_id, source_kind, source_playlist_id, source_query, cursor,
                position_ms, shuffle, repeat_mode, random_state, entry_count
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(singleton_id) DO UPDATE SET
                source_kind=excluded.source_kind,
                source_playlist_id=excluded.source_playlist_id,
                source_query=excluded.source_query,
                cursor=excluded.cursor,
                position_ms=excluded.position_ms,
                shuffle=excluded.shuffle,
                repeat_mode=excluded.repeat_mode,
                random_state=excluded.random_state,
                entry_count=excluded.entry_count",
            params![
                source_kind,
                source_playlist_id,
                source_query,
                cursor,
                position_ms,
                checkpoint.queue.shuffle,
                repeat_mode_name(checkpoint.queue.repeat),
                checkpoint.queue.random_state.to_string(),
                entry_count,
            ],
        )?;
        {
            let mut statement = tx.prepare_cached(
                "INSERT INTO playback_session_entries(source_index, track_id, source_position, traversal_order)
                 VALUES (?1, ?2, ?3, ?4)",
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
                "SELECT source_kind, source_playlist_id, source_query, cursor, position_ms,
                        shuffle, repeat_mode, random_state, entry_count
                 FROM playback_session WHERE singleton_id=1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, bool>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .optional()?;
        let Some((kind, playlist_id, query, cursor, position_ms, shuffle, repeat, random, count)) =
            header
        else {
            return Ok(None);
        };
        if count <= 0 || count > 1_000_000 || cursor < 0 || position_ms < 0 {
            return Err(DatabaseError::CorruptData(
                "playback session header is out of range".into(),
            ));
        }
        let context = match kind.as_str() {
            "library" if playlist_id.is_none() => PlaybackQueueContext::Library { query },
            "playlist" if query.is_none() => PlaybackQueueContext::Playlist {
                playlist_id: playlist_id.ok_or_else(|| {
                    DatabaseError::CorruptData("playlist session has no playlist ID".into())
                })?,
            },
            _ => {
                return Err(DatabaseError::CorruptData(
                    "playback session source is invalid".into(),
                ))
            }
        };
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
        {
            let mut statement = connection.prepare_cached(
                "SELECT source_index, track_id, source_position, traversal_order
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
        let queue = PlaybackQueueSnapshot {
            context,
            entries,
            play_order,
            cursor: usize::try_from(cursor)
                .map_err(|_| DatabaseError::CorruptData("playback cursor is negative".into()))?,
            shuffle,
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
        player_core::PlaybackQueue::restore(queue.clone())
            .map_err(|error| DatabaseError::CorruptData(error.to_owned()))?;
        let position_ms = i64::try_from(position_ms)
            .map_err(|_| DatabaseError::InvalidNumber("playback position"))?;
        let cursor = i64::try_from(queue.cursor)
            .map_err(|_| DatabaseError::InvalidNumber("playback queue cursor"))?;
        let source_index = queue.play_order[queue.cursor];
        let connection = self.lock()?;
        let tx = connection.unchecked_transaction()?;
        let persisted: Option<(String, Option<i64>, i64)> = tx
            .query_row(
                "SELECT track_id, source_position, traversal_order
                 FROM playback_session_entries WHERE source_index=?1",
                [i64::try_from(source_index)
                    .map_err(|_| DatabaseError::InvalidNumber("playback source index"))?],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
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

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, DatabaseError> {
        self.connection.lock().map_err(|_| DatabaseError::Poisoned)
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
            persist_metadata(&tx, track_id, Some(metadata))?;
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
            persist_metadata(&tx, track_id, record.metadata.as_ref())?;
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

fn count_tracks_in(connection: &Connection, query: Option<&str>) -> Result<i64, DatabaseError> {
    if let Some(query) = query {
        Ok(connection.query_row(COUNT_SEARCH_SQL, [query], |row| row.get(0))?)
    } else {
        Ok(connection.query_row(COUNT_LIBRARY_SQL, [], |row| row.get(0))?)
    }
}

fn fetch_tracks_window(
    connection: &Connection,
    query: Option<&str>,
    offset: u64,
    limit: u32,
) -> Result<Vec<TrackSummary>, DatabaseError> {
    let mut statement = connection.prepare(TRACKS_PAGE_SQL)?;
    let items = statement
        .query_map(
            params![query, i64::from(limit), query_limit_i64(offset)],
            row_to_track_summary,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(items)
}

fn row_to_track_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrackSummary> {
    let raw_id: String = row.get(0)?;
    let id = TrackId::parse(&raw_id).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(TrackSummary {
        id,
        title: row.get(1)?,
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
        year: row
            .get::<_, Option<i64>>(11)?
            .and_then(|value| u16::try_from(value).ok())
            .filter(|value| *value > 0),
        bit_depth: row
            .get::<_, Option<i64>>(12)?
            .and_then(|value| u8::try_from(value).ok())
            .filter(|value| *value > 0),
    })
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
) -> Result<(), DatabaseError> {
    let Some(metadata) = metadata else {
        tx.prepare_cached(
            "INSERT INTO tracks (track_id, metadata_loaded) VALUES (?1, 0)
             ON CONFLICT(track_id) DO UPDATE SET metadata_loaded=0",
        )?
        .execute([track_id.to_string()])?;
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
    let sort_title = user_title
        .or_else(|| metadata.title.clone())
        .unwrap_or_default()
        .to_lowercase();
    if metadata.year.is_some_and(|year| year == 0 || year > 9999) {
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
             disc_number, duration_ms, codec, bitrate_bps, sample_rate_hz, year, bit_depth, metadata_version)
         VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(track_id) DO UPDATE SET
            metadata_loaded=1, sort_title=excluded.sort_title, title=excluded.title,
            artist=excluded.artist, album=excluded.album,
            album_artist=excluded.album_artist, track_number=excluded.track_number,
            disc_number=excluded.disc_number, duration_ms=excluded.duration_ms, codec=excluded.codec,
            bitrate_bps=excluded.bitrate_bps, sample_rate_hz=excluded.sample_rate_hz,
            year=excluded.year, bit_depth=excluded.bit_depth,
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
            metadata.year.map(i64::from),
            metadata.bit_depth.map(i64::from),
            i64::from(TRACK_METADATA_VERSION),
        ])?;
    Ok(())
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
             locator_data, size_bytes, modified_at_utc_ms, track_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(source_id, source_item_id) DO UPDATE SET
            locator_key=excluded.locator_key, locator_kind=excluded.locator_kind,
            locator_encoding=excluded.locator_encoding, locator_data=excluded.locator_data,
            size_bytes=excluded.size_bytes, modified_at_utc_ms=excluded.modified_at_utc_ms,
            track_id=excluded.track_id
         WHERE source_mappings.locator_key IS NOT excluded.locator_key
            OR source_mappings.locator_kind IS NOT excluded.locator_kind
            OR source_mappings.locator_encoding IS NOT excluded.locator_encoding
            OR source_mappings.locator_data IS NOT excluded.locator_data
            OR source_mappings.size_bytes IS NOT excluded.size_bytes
            OR source_mappings.modified_at_utc_ms IS NOT excluded.modified_at_utc_ms
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
        track_id.to_string(),
    ])?;
    Ok(())
}

fn row_to_sync_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrackSyncState> {
    Ok(TrackSyncState {
        fingerprint: FileFingerprint {
            size_bytes: row.get::<_, i64>(0)?.max(0) as u64,
            modified_at_utc_ms: row.get(1)?,
        },
        metadata_loaded: row.get(2)?,
        metadata_version: row.get::<_, i64>(3)?.max(0) as u32,
    })
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

fn to_sql_i64(value: u64, field: &'static str) -> Result<i64, DatabaseError> {
    i64::try_from(value).map_err(|_| DatabaseError::InvalidNumber(field))
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::SystemTime};

    use player_core::{
        parse_lrc, FileFingerprint, LibraryRoot, ListTracksQuery, LyricCandidate, LyricProvider,
        MediaIndex, MediaLocator, MediaSourceError, MediaSourceKind, MediaTrackRecord,
        PlaybackQueue, PlaybackQueueContext, PlaybackQueueEntry, Playlist, PlaylistEntry,
        PlaylistId, QueueRepeatMode, SourceId, SourceScan, SourceScanState, SyncApplyRequest,
        SyncCancellation, SyncEngine, TrackId, TrackIdentity, TrackLyrics, TrackMetadata,
        TrackMetadataError, UserMetadataField, TRACK_METADATA_VERSION,
    };
    use rusqlite::{params, OptionalExtension};

    use super::{
        Database, DatabaseError, LibraryRepository, PlaybackSessionCheckpoint,
        PlaylistFileSyncState, ThemePreferences, COUNT_LIBRARY_SQL, SCHEMA_V1, SCHEMA_V2,
        SCHEMA_V3, SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7, TRACKS_PAGE_SQL,
    };

    fn add_root(db: &Database, kind: MediaSourceKind, name: &str) -> LibraryRoot {
        db.add_library_root(
            kind,
            name,
            MediaLocator::FileSystem(PathBuf::from(format!(r"C:\Music\{name}"))),
        )
        .expect("add root")
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
        })
        .expect("list tracks")
    }

    fn track_id_for_query(db: &Database, query: &str) -> TrackId {
        db.list_tracks_page(ListTracksQuery {
            offset: 0,
            limit: 20,
            query: Some(query.to_owned()),
        })
        .expect("find track")
        .items
        .into_iter()
        .next()
        .expect("matching track")
        .id
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
            assert_eq!(version, 8);
        }
        {
            let db = Database::open(&path).expect("reopen migrated database");
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
        assert_eq!(current_version, 8);
        let preserved_roots: i64 = db
            .lock()
            .expect("connection")
            .query_row("SELECT COUNT(*) FROM library_roots", [], |row| row.get(0))
            .expect("legacy roots remain");
        assert_eq!(preserved_roots, 1);
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
        assert_eq!(version, 8);
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
        assert_eq!(version, 8);
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
        assert_eq!(version, 8);
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
        assert_eq!(version, 8);
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
        assert_eq!(version, 8);
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
            })
            .expect("text filter");
        assert_eq!(filtered.total_count, 1);
        assert_eq!(filtered.items[0].title.as_deref(), Some("Alpha"));
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
        let disabled_track_id = track_id_for_query(&db, "Disabled only track");
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
                rusqlite::params![Option::<&str>::None, 100_i64, 0_i64],
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
}
