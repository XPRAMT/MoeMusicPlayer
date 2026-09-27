use std::{
    collections::HashSet,
    error::Error,
    fmt,
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use player_core::{
    FileFingerprint, LibraryRepository, LibraryRoot, ListTracksQuery, MediaLocator,
    MediaSourceError, MediaSourceKind, MediaTrackRecord, Page, SourceId, SourceScanState,
    SyncApplyStats, TrackId, TrackIdentity, TrackMetadata, TrackSummary, TrackSyncState,
    UserMetadataField,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::locator;

const SCHEMA_VERSION: i64 = 1;
const MAX_PAGE_SIZE: u32 = 500;
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
    t.track_number, t.disc_number, t.duration_ms, t.codec, t.bitrate_bps, t.sample_rate_hz
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
    t.track_number, t.disc_number, t.duration_ms, t.codec, t.bitrate_bps, t.sample_rate_hz
 FROM tracks t WHERE t.track_id=?1";

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
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(DatabaseError::UnsupportedSchemaVersion(version));
        }
        if version == SCHEMA_VERSION {
            connection.execute_batch(
                "CREATE TEMP TABLE IF NOT EXISTS sync_seen_items (
                    source_id TEXT NOT NULL, source_item_id TEXT NOT NULL,
                    PRIMARY KEY (source_id, source_item_id)
                );",
            )?;
            return Ok(());
        }

        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(SCHEMA_V1)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
        Ok(())
    }

    pub fn journal_mode(&self) -> Result<String, DatabaseError> {
        Ok(self
            .lock()?
            .pragma_query_value(None, "journal_mode", |row| row.get(0))?)
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
                "SELECT m.size_bytes, m.modified_at_utc_ms, t.metadata_loaded
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
            "SELECT m.size_bytes, m.modified_at_utc_ms, t.metadata_loaded
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
        let changed_items = changed
            .iter()
            .map(|record| record.identity.source_item_id.as_str())
            .collect::<HashSet<_>>();
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM sync_seen_items", [])?;
        let mut seen_statement = tx.prepare_cached(
            "INSERT OR IGNORE INTO sync_seen_items (source_id, source_item_id) VALUES (?1, ?2)",
        )?;

        for record in observed {
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
        }
        drop(seen_statement);

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
        tx.commit()?;
        Ok(SyncApplyStats {
            inserted_or_updated: changed_items.len() as u64,
            removed_source_mappings,
        })
    }
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

    tx.prepare_cached(
        "INSERT INTO tracks
            (track_id, metadata_loaded, sort_title, title, artist, album, album_artist, track_number,
             disc_number, duration_ms, codec, bitrate_bps, sample_rate_hz)
         VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(track_id) DO UPDATE SET
            metadata_loaded=1, sort_title=excluded.sort_title, title=excluded.title,
            artist=excluded.artist, album=excluded.album,
            album_artist=excluded.album_artist, track_number=excluded.track_number,
            disc_number=excluded.disc_number, duration_ms=excluded.duration_ms, codec=excluded.codec,
            bitrate_bps=excluded.bitrate_bps, sample_rate_hz=excluded.sample_rate_hz",
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
    })
}

fn parse_track_id(value: &str) -> Result<TrackId, DatabaseError> {
    TrackId::parse(value).map_err(|error| DatabaseError::CorruptData(error.to_string()))
}

fn source_kind_name(kind: MediaSourceKind) -> &'static str {
    match kind {
        MediaSourceKind::WindowsSystemIndex => "windows_system_index",
        MediaSourceKind::WindowsFilesystem => "windows_filesystem",
        MediaSourceKind::AndroidMediaStore => "android_media_store",
        MediaSourceKind::AndroidSaf => "android_saf",
        MediaSourceKind::Other => "other",
    }
}

fn parse_source_kind(value: &str) -> Result<MediaSourceKind, DatabaseError> {
    match value {
        "windows_system_index" => Ok(MediaSourceKind::WindowsSystemIndex),
        "windows_filesystem" => Ok(MediaSourceKind::WindowsFilesystem),
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
        FileFingerprint, LibraryRoot, ListTracksQuery, MediaIndex, MediaLocator, MediaSourceKind,
        MediaTrackRecord, SourceId, SourceScan, SourceScanState, SyncEngine, TrackId,
        TrackIdentity, TrackMetadata, TrackMetadataError, UserMetadataField,
    };

    use super::{Database, DatabaseError, LibraryRepository, COUNT_LIBRARY_SQL, TRACKS_PAGE_SQL};

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
            assert_eq!(version, 1);
        }
        {
            let db = Database::open(&path).expect("reopen migrated database");
            assert!(db.library_roots().expect("read roots").is_empty());
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
}
