use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_THUMBNAIL_BYTES: usize = 2 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 512 * 1024 * 1024;
const PRUNE_EVERY_WRITES: usize = 32;
static CACHE_WRITES: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AlbumArtworkSelection {
    pub track_count: u64,
    pub track_ids: Vec<String>,
    pub representative: bool,
}

fn selection_path(directory: &Path, key: &str) -> Result<PathBuf, String> {
    if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("專輯封面索引無效。".to_owned());
    }
    Ok(directory.join(format!("selection-{}.json", key.to_ascii_lowercase())))
}

pub(crate) fn get_selection(
    directory: &Path,
    key: &str,
) -> Result<Option<AlbumArtworkSelection>, String> {
    let path = selection_path(directory, key)?;
    let json = match fs::read(path) {
        Ok(bytes) if bytes.len() <= 4096 => bytes,
        Ok(_) => return Err("專輯封面索引超過大小限制。".to_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("讀取專輯封面索引失敗：{error}")),
    };
    let selection: AlbumArtworkSelection =
        serde_json::from_slice(&json).map_err(|error| format!("專輯封面索引格式無效：{error}"))?;
    if selection.track_ids.is_empty()
        || selection.track_ids.len() > 4
        || selection
            .track_ids
            .iter()
            .any(|id| id.is_empty() || id.len() > 128)
    {
        return Err("專輯封面索引的曲目清單無效。".to_owned());
    }
    Ok(Some(selection))
}

pub(crate) fn put_selection(
    directory: &Path,
    key: &str,
    selection: &AlbumArtworkSelection,
) -> Result<(), String> {
    let path = selection_path(directory, key)?;
    if selection.track_ids.is_empty()
        || selection.track_ids.len() > 4
        || selection
            .track_ids
            .iter()
            .any(|id| id.is_empty() || id.len() > 128)
    {
        return Err("專輯封面索引的曲目清單無效。".to_owned());
    }
    let bytes = serde_json::to_vec(selection)
        .map_err(|error| format!("序列化專輯封面索引失敗：{error}"))?;
    if bytes.len() > 4096 {
        return Err("專輯封面索引超過大小限制。".to_owned());
    }
    fs::create_dir_all(directory).map_err(|error| format!("建立專輯圖片快取目錄失敗：{error}"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_path = directory.join(format!(".album-selection-{nonce}.tmp"));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|error| format!("建立專輯封面索引暫存檔失敗：{error}"))?;
        file.write_all(&bytes)
            .map_err(|error| format!("寫入專輯封面索引失敗：{error}"))?;
        file.sync_all()
            .map_err(|error| format!("同步專輯封面索引失敗：{error}"))?;
        drop(file);
        if path.exists() {
            fs::remove_file(&path).map_err(|error| format!("更新專輯封面索引失敗：{error}"))?;
        }
        fs::rename(&temp_path, &path).map_err(|error| format!("完成專輯封面索引失敗：{error}"))
    })();
    if temp_path.exists() {
        let _ = fs::remove_file(temp_path);
    }
    result
}

fn cache_path(directory: &Path, sha256: &str) -> Result<PathBuf, String> {
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("專輯圖片快取索引無效。".to_owned());
    }
    Ok(directory.join(format!("{}.jpg", sha256.to_ascii_lowercase())))
}

fn track_reference_path(directory: &Path, identity: &str) -> PathBuf {
    let digest = Sha256::digest(identity.as_bytes());
    let key = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    directory.join(format!("track-{key}.ref"))
}

/// Empty means not indexed yet; `[0]` means this track was checked and has no artwork.
pub(crate) fn get_for_track(directory: &Path, identity: &str) -> Result<Vec<u8>, String> {
    let reference = match fs::read_to_string(track_reference_path(directory, identity)) {
        Ok(reference) => reference,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("讀取曲目縮圖索引失敗：{error}")),
    };
    if reference == "none" {
        return Ok(vec![0]);
    }
    get(directory, &reference)
}

pub(crate) fn put_track_reference(
    directory: &Path,
    identity: &str,
    sha256: Option<&str>,
) -> Result<(), String> {
    if let Some(sha256) = sha256 {
        cache_path(directory, sha256)?;
    }
    fs::create_dir_all(directory).map_err(|error| format!("建立專輯圖片快取目錄失敗：{error}"))?;
    let path = track_reference_path(directory, identity);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_path = directory.join(format!(".album-ref-{nonce}.tmp"));
    let value = sha256.unwrap_or("none");
    let result = fs::write(&temp_path, value)
        .map_err(|error| format!("寫入曲目縮圖索引失敗：{error}"))
        .and_then(|()| {
            if path.exists() {
                fs::remove_file(&path).map_err(|error| format!("更新曲目縮圖索引失敗：{error}"))?;
            }
            fs::rename(&temp_path, &path).map_err(|error| format!("完成曲目縮圖索引失敗：{error}"))
        });
    if temp_path.exists() {
        let _ = fs::remove_file(temp_path);
    }
    result
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
        if stem.len() != 64 || !stem.bytes().all(|byte| byte.is_ascii_hexdigit()) {
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
        // Cache pruning is best-effort and amortized; thumbnail writes remain available if
        // the filesystem temporarily prevents cleanup.
        let _ = prune(directory, MAX_CACHE_BYTES);
    }
    Ok(())
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
        fs::remove_dir_all(directory).expect("remove isolated cache");
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
    }

    #[test]
    fn per_track_reference_returns_cached_jpeg_and_remembers_missing_artwork() {
        let directory = temp_dir();
        let identity = "source-a\\0song-1\\01024\\0200";
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

        let missing_identity = "source-a\\0song-2\\01024\\0200";
        put_track_reference(&directory, missing_identity, None).expect("save missing marker");
        assert_eq!(
            get_for_track(&directory, missing_identity).expect("read missing marker"),
            [0]
        );
        fs::remove_dir_all(directory).expect("remove isolated cache");
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
        fs::remove_dir_all(directory).expect("remove isolated cache");
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
        fs::remove_dir_all(directory).expect("remove isolated cache");
    }
}
