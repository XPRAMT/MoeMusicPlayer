use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_THUMBNAIL_BYTES: usize = 2 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 512 * 1024 * 1024;
const PRUNE_EVERY_WRITES: usize = 32;
const CACHE_DATABASE_NAME: &str = "cache.sqlite3";
const LEGACY_MIGRATION_KEY: &str = "legacy_index_files_migrated";
static CACHE_WRITES: AtomicUsize = AtomicUsize::new(0);
static DATABASES: OnceLock<Mutex<HashMap<PathBuf, Connection>>> = OnceLock::new();

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AlbumArtworkSelection {
    pub track_count: u64,
    pub track_ids: Vec<String>,
    pub representative: bool,
}

fn database_path(directory: &Path) -> PathBuf {
    directory.join(CACHE_DATABASE_NAME)
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_selection(selection: &AlbumArtworkSelection) -> Result<(), String> {
    if selection.track_ids.is_empty()
        || selection.track_ids.len() > 4
        || selection
            .track_ids
            .iter()
            .any(|id| id.is_empty() || id.len() > 128)
    {
        return Err("專輯封面索引的曲目清單無效。".to_owned());
    }
    Ok(())
}

fn initialize_database(connection: &mut Connection, directory: &Path) -> Result<(), String> {
    connection
        .execute_batch(
            "PRAGMA journal_mode = DELETE;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS cache_metadata (
                 key TEXT PRIMARY KEY NOT NULL,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS track_artwork_refs (
                 identity_sha256 TEXT PRIMARY KEY NOT NULL,
                 artwork_sha256 TEXT
             );
             CREATE TABLE IF NOT EXISTS album_artwork_selections (
                 selection_key TEXT PRIMARY KEY NOT NULL,
                 track_count INTEGER NOT NULL,
                 track_ids_json TEXT NOT NULL,
                 representative INTEGER NOT NULL CHECK (representative IN (0, 1))
             );",
        )
        .map_err(|error| format!("建立專輯封面快取資料庫失敗：{error}"))?;

    let migrated = connection
        .query_row(
            "SELECT value FROM cache_metadata WHERE key = ?1",
            [LEGACY_MIGRATION_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("讀取專輯封面快取狀態失敗：{error}"))?
        .is_some();

    if !migrated {
        migrate_legacy_indices(connection, directory)?;
        connection
            .execute(
                "INSERT OR REPLACE INTO cache_metadata(key, value) VALUES (?1, '1')",
                [LEGACY_MIGRATION_KEY],
            )
            .map_err(|error| format!("保存專輯封面快取遷移狀態失敗：{error}"))?;
    }

    // Cleanup is repeated even after the migration marker is committed so an
    // interrupted cleanup cannot leave the old per-item files behind forever.
    remove_legacy_indices(directory);
    Ok(())
}

fn migrate_legacy_indices(connection: &mut Connection, directory: &Path) -> Result<(), String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("讀取舊專輯封面索引失敗：{error}")),
    };
    let transaction = connection
        .transaction()
        .map_err(|error| format!("開始專輯封面索引遷移失敗：{error}"))?;

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if let Some(key) = name
            .strip_prefix("track-")
            .and_then(|name| name.strip_suffix(".ref"))
        {
            if !valid_hash(key) {
                continue;
            }
            let Ok(value) = fs::read_to_string(&path) else {
                continue;
            };
            let artwork_hash = if value == "none" {
                None
            } else if valid_hash(&value) {
                Some(value.to_ascii_lowercase())
            } else {
                continue;
            };
            transaction
                .execute(
                    "INSERT OR IGNORE INTO track_artwork_refs(identity_sha256, artwork_sha256)
                     VALUES (?1, ?2)",
                    params![key.to_ascii_lowercase(), artwork_hash],
                )
                .map_err(|error| format!("匯入曲目封面索引失敗：{error}"))?;
            continue;
        }

        let Some(key) = name
            .strip_prefix("selection-")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        if !valid_hash(key) {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        if bytes.len() > 4096 {
            continue;
        }
        let Ok(selection) = serde_json::from_slice::<AlbumArtworkSelection>(&bytes) else {
            continue;
        };
        if validate_selection(&selection).is_err() {
            continue;
        }
        let Ok(track_count) = i64::try_from(selection.track_count) else {
            continue;
        };
        let Ok(track_ids_json) = serde_json::to_string(&selection.track_ids) else {
            continue;
        };
        let representative = if selection.representative {
            1_i64
        } else {
            0_i64
        };
        transaction
            .execute(
                "INSERT OR IGNORE INTO album_artwork_selections
                 (selection_key, track_count, track_ids_json, representative)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    key.to_ascii_lowercase(),
                    track_count,
                    track_ids_json,
                    representative
                ],
            )
            .map_err(|error| format!("匯入專輯代表封面索引失敗：{error}"))?;
    }

    transaction
        .commit()
        .map_err(|error| format!("完成專輯封面索引遷移失敗：{error}"))
}

fn remove_legacy_indices(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let is_legacy_reference = name
            .strip_prefix("track-")
            .and_then(|name| name.strip_suffix(".ref"))
            .is_some_and(valid_hash);
        let is_legacy_selection = name
            .strip_prefix("selection-")
            .and_then(|name| name.strip_suffix(".json"))
            .is_some_and(valid_hash);
        if is_legacy_reference || is_legacy_selection {
            let _ = fs::remove_file(path);
        }
    }
}

fn with_database<T>(
    directory: &Path,
    operation: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    fs::create_dir_all(directory).map_err(|error| format!("建立專輯圖片快取目錄失敗：{error}"))?;
    let path = database_path(directory);
    let databases = DATABASES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut databases = databases
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !databases.contains_key(&path) {
        let mut connection = Connection::open(&path)
            .map_err(|error| format!("開啟專輯封面快取資料庫失敗：{error}"))?;
        connection
            .busy_timeout(std::time::Duration::from_secs(3))
            .map_err(|error| format!("設定專輯封面快取資料庫逾時失敗：{error}"))?;
        initialize_database(&mut connection, directory)?;
        databases.insert(path.clone(), connection);
    }
    let connection = databases.get(&path).expect("cache database inserted above");
    operation(connection)
}

fn selection_key(key: &str) -> Result<String, String> {
    if !valid_hash(key) {
        return Err("專輯封面索引無效。".to_owned());
    }
    Ok(key.to_ascii_lowercase())
}

pub(crate) fn get_selection(
    directory: &Path,
    key: &str,
) -> Result<Option<AlbumArtworkSelection>, String> {
    let key = selection_key(key)?;
    with_database(directory, |connection| {
        let stored = connection
            .query_row(
                "SELECT track_count, track_ids_json, representative
                 FROM album_artwork_selections WHERE selection_key = ?1",
                [&key],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("讀取專輯封面索引失敗：{error}"))?;
        let Some((track_count, track_ids_json, representative)) = stored else {
            return Ok(None);
        };
        let selection = AlbumArtworkSelection {
            track_count: u64::try_from(track_count)
                .map_err(|_| "專輯封面索引的曲目數量無效。".to_owned())?,
            track_ids: serde_json::from_str(&track_ids_json)
                .map_err(|error| format!("專輯封面索引格式無效：{error}"))?,
            representative,
        };
        validate_selection(&selection)?;
        Ok(Some(selection))
    })
}

pub(crate) fn put_selection(
    directory: &Path,
    key: &str,
    selection: &AlbumArtworkSelection,
) -> Result<(), String> {
    let key = selection_key(key)?;
    validate_selection(selection)?;
    let track_count = i64::try_from(selection.track_count)
        .map_err(|_| "專輯封面索引的曲目數量無效。".to_owned())?;
    let track_ids_json = serde_json::to_string(&selection.track_ids)
        .map_err(|error| format!("序列化專輯封面索引失敗：{error}"))?;
    let representative = if selection.representative {
        1_i64
    } else {
        0_i64
    };
    let directory = directory.to_path_buf();
    with_database(&directory, |connection| {
        connection
            .execute(
                "INSERT INTO album_artwork_selections
                 (selection_key, track_count, track_ids_json, representative)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(selection_key) DO UPDATE SET
                    track_count = excluded.track_count,
                    track_ids_json = excluded.track_ids_json,
                    representative = excluded.representative",
                params![key, track_count, track_ids_json, representative],
            )
            .map_err(|error| format!("儲存專輯封面索引失敗：{error}"))?;
        Ok(())
    })
}

fn cache_path(directory: &Path, sha256: &str) -> Result<PathBuf, String> {
    if !valid_hash(sha256) {
        return Err("專輯圖片快取索引無效。".to_owned());
    }
    Ok(directory.join(format!("{}.jpg", sha256.to_ascii_lowercase())))
}

fn track_reference_key(identity: &str) -> String {
    Sha256::digest(identity.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Empty means not indexed yet; `[0]` means this track was checked and has no artwork.
pub(crate) fn get_for_track(directory: &Path, identity: &str) -> Result<Vec<u8>, String> {
    let key = track_reference_key(identity);
    let reference = with_database(directory, |connection| {
        connection
            .query_row(
                "SELECT artwork_sha256 FROM track_artwork_refs WHERE identity_sha256 = ?1",
                [&key],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|error| format!("讀取曲目縮圖索引失敗：{error}"))
    })?;
    match reference {
        None => Ok(Vec::new()),
        Some(None) => Ok(vec![0]),
        Some(Some(hash)) => get(directory, &hash),
    }
}

pub(crate) fn put_track_reference(
    directory: &Path,
    identity: &str,
    sha256: Option<&str>,
) -> Result<(), String> {
    let artwork_hash = sha256.map(str::to_ascii_lowercase);
    if let Some(hash) = artwork_hash.as_deref() {
        cache_path(directory, hash)?;
    }
    let key = track_reference_key(identity);
    with_database(directory, |connection| {
        connection
            .execute(
                "INSERT INTO track_artwork_refs(identity_sha256, artwork_sha256)
                 VALUES (?1, ?2)
                 ON CONFLICT(identity_sha256) DO UPDATE SET artwork_sha256 = excluded.artwork_sha256",
                params![key, artwork_hash],
            )
            .map_err(|error| format!("寫入曲目縮圖索引失敗：{error}"))?;
        Ok(())
    })
}

pub(crate) fn get(directory: &Path, sha256: &str) -> Result<Vec<u8>, String> {
    let path = cache_path(directory, sha256)?;
    match fs::read(path) {
        Ok(bytes)
            if bytes.len() <= MAX_THUMBNAIL_BYTES && bytes.starts_with(&[0xff, 0xd8, 0xff]) =>
        {
            Ok(bytes)
        }
        Ok(_) => Err("專輯圖片縮圖快取內容無效。".to_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("讀取專輯圖片快取失敗：{error}")),
    }
}

fn prune(directory: &Path, max_bytes: u64) -> Result<(), String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("讀取專輯圖片快取目錄失敗：{error}")),
    };
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".jpg") else {
            continue;
        };
        if !valid_hash(stem) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        total_bytes = total_bytes.saturating_add(metadata.len());
        files.push((
            metadata.modified().unwrap_or(UNIX_EPOCH),
            metadata.len(),
            path,
        ));
    }
    if total_bytes <= max_bytes {
        return Ok(());
    }
    files.sort_by_key(|(modified, _, _)| *modified);
    for (_, size, path) in files {
        if total_bytes <= max_bytes {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total_bytes = total_bytes.saturating_sub(size);
        }
    }
    Ok(())
}

pub(crate) fn put(directory: &Path, sha256: &str, data_base64: &str) -> Result<(), String> {
    let bytes = STANDARD
        .decode(data_base64)
        .map_err(|error| format!("專輯縮圖資料無效：{error}"))?;
    if bytes.is_empty()
        || bytes.len() > MAX_THUMBNAIL_BYTES
        || !bytes.starts_with(&[0xff, 0xd8, 0xff])
    {
        return Err("專輯縮圖必須是有效且不超過 2 MiB 的 JPEG。".to_owned());
    }
    let path = cache_path(directory, sha256)?;
    fs::create_dir_all(directory).map_err(|error| format!("建立專輯圖片快取目錄失敗：{error}"))?;
    if let Ok(existing) = fs::read(&path) {
        if existing.len() <= MAX_THUMBNAIL_BYTES && existing.starts_with(&[0xff, 0xd8, 0xff]) {
            return Ok(());
        }
        fs::remove_file(&path).map_err(|error| format!("移除損壞的專輯縮圖快取失敗：{error}"))?;
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_path = directory.join(format!(".album-art-{nonce}.tmp"));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|error| format!("建立專輯縮圖暫存檔失敗：{error}"))?;
        file.write_all(&bytes)
            .map_err(|error| format!("寫入專輯縮圖快取失敗：{error}"))?;
        file.sync_all()
            .map_err(|error| format!("同步專輯縮圖快取失敗：{error}"))?;
        drop(file);
        match fs::rename(&temp_path, &path) {
            Ok(()) => Ok(()),
            Err(_) if path.is_file() => Ok(()),
            Err(error) => Err(format!("完成專輯縮圖快取失敗：{error}")),
        }
    })();
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    result?;
    if CACHE_WRITES
        .fetch_add(1, Ordering::Relaxed)
        .is_multiple_of(PRUNE_EVERY_WRITES)
    {
        let _ = prune(directory, MAX_CACHE_BYTES);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn close_for_test(directory: &Path) {
    if let Some(databases) = DATABASES.get() {
        let path = database_path(directory);
        databases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "moe-album-cache-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ))
    }

    fn cleanup(directory: &Path) {
        close_for_test(directory);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn cache_is_content_addressed_bounded_and_round_trips_jpeg() {
        let directory = temp_dir();
        let hash = "a".repeat(64);
        assert!(get(&directory, &hash).expect("cache miss").is_empty());
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0x01, 0x02, 0x03];
        put(&directory, &hash, &STANDARD.encode(&jpeg)).expect("save thumbnail");
        assert_eq!(get(&directory, &hash).expect("read thumbnail"), jpeg);
        jpeg[3] = 0x08;
        put(&directory, &hash, &STANDARD.encode(&jpeg)).expect("existing cache is immutable");
        assert_eq!(
            get(&directory, &hash).expect("read existing"),
            vec![0xff, 0xd8, 0xff, 0x01, 0x02, 0x03]
        );
        cleanup(&directory);
    }

    #[test]
    fn cache_rejects_path_injection_and_non_jpeg_payloads() {
        let directory = temp_dir();
        assert!(get(&directory, "../outside").is_err());
        assert!(put(
            &directory,
            &"b".repeat(64),
            &STANDARD.encode(b"not an image")
        )
        .is_err());
        assert!(!directory.exists());
        cleanup(&directory);
    }

    #[test]
    fn per_track_reference_returns_cached_jpeg_and_remembers_missing_artwork() {
        let directory = temp_dir();
        let identity = "source-a\0song-1\01024\0200";
        let hash = "c".repeat(64);
        assert!(get_for_track(&directory, identity)
            .expect("cache miss")
            .is_empty());
        put(&directory, &hash, &STANDARD.encode([0xff, 0xd8, 0xff, 1])).expect("save image");
        put_track_reference(&directory, identity, Some(&hash)).expect("save mapping");
        assert_eq!(
            get_for_track(&directory, identity).expect("read cached image"),
            [0xff, 0xd8, 0xff, 1]
        );

        let missing_identity = "source-a\0song-2\01024\0200";
        put_track_reference(&directory, missing_identity, None).expect("save missing marker");
        assert_eq!(
            get_for_track(&directory, missing_identity).expect("read missing marker"),
            [0]
        );
        cleanup(&directory);
    }

    #[test]
    fn album_selection_is_persisted_and_rejects_invalid_keys_or_track_lists() {
        let directory = temp_dir();
        let key = "d".repeat(64);
        let selection = AlbumArtworkSelection {
            track_count: 12,
            track_ids: vec!["track-a".to_owned(), "track-b".to_owned()],
            representative: true,
        };
        assert_eq!(get_selection(&directory, &key).expect("cache miss"), None);
        put_selection(&directory, &key, &selection).expect("save album selection");
        assert_eq!(
            get_selection(&directory, &key).expect("restore album selection"),
            Some(selection)
        );
        assert!(get_selection(&directory, "../outside").is_err());
        assert!(put_selection(
            &directory,
            &key,
            &AlbumArtworkSelection {
                track_count: 12,
                track_ids: vec!["track-a".to_owned(); 5],
                representative: false,
            }
        )
        .is_err());
        cleanup(&directory);
    }

    #[test]
    fn legacy_ref_and_json_files_are_imported_once_then_removed() {
        let directory = temp_dir();
        fs::create_dir_all(&directory).expect("create legacy cache");
        let identity = "legacy-source\0legacy-item\01024\0200";
        let ref_key = track_reference_key(identity);
        let image_hash = "e".repeat(64);
        fs::write(directory.join(format!("track-{ref_key}.ref")), &image_hash)
            .expect("write old track reference");
        let missing_identity = "legacy-source\0missing-item\01024\0200";
        let missing_ref_key = track_reference_key(missing_identity);
        fs::write(
            directory.join(format!("track-{missing_ref_key}.ref")),
            "none",
        )
        .expect("write old missing-artwork reference");
        let selection_key = "f".repeat(64);
        let selection = AlbumArtworkSelection {
            track_count: 3,
            track_ids: vec!["legacy-track".to_owned()],
            representative: true,
        };
        fs::write(
            directory.join(format!("selection-{selection_key}.json")),
            serde_json::to_vec(&selection).expect("encode selection"),
        )
        .expect("write old selection");
        fs::write(
            cache_path(&directory, &image_hash).expect("thumbnail path"),
            [0xff, 0xd8, 0xff, 1],
        )
        .expect("write existing thumbnail");

        assert_eq!(
            get_for_track(&directory, identity).expect("migrated ref"),
            [0xff, 0xd8, 0xff, 1]
        );
        assert_eq!(
            get_for_track(&directory, missing_identity).expect("migrated missing ref"),
            [0]
        );
        assert_eq!(
            get_selection(&directory, &selection_key).expect("migrated json"),
            Some(selection)
        );
        assert!(directory.join(CACHE_DATABASE_NAME).is_file());
        assert!(!directory.join(format!("track-{ref_key}.ref")).exists());
        assert!(!directory
            .join(format!("track-{missing_ref_key}.ref"))
            .exists());
        assert!(!directory
            .join(format!("selection-{selection_key}.json"))
            .exists());
        assert!(cache_path(&directory, &image_hash)
            .expect("thumbnail path")
            .is_file());
        cleanup(&directory);
    }

    #[test]
    fn cache_pruning_evicts_oldest_content_addressed_jpegs_to_the_budget() {
        let directory = temp_dir();
        let first_hash = "a".repeat(64);
        let second_hash = "b".repeat(64);
        let jpeg = [0xff, 0xd8, 0xff, 0x01, 0x02, 0x03];
        fs::create_dir_all(&directory).expect("create cache directory");
        fs::write(
            cache_path(&directory, &first_hash).expect("first path"),
            jpeg,
        )
        .expect("write first cache");
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            cache_path(&directory, &second_hash).expect("second path"),
            jpeg,
        )
        .expect("write second cache");
        prune(&directory, jpeg.len() as u64).expect("prune cache");
        assert!(!cache_path(&directory, &first_hash)
            .expect("first path")
            .exists());
        assert!(cache_path(&directory, &second_hash)
            .expect("second path")
            .exists());
        cleanup(&directory);
    }
}
