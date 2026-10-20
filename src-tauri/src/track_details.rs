//! Single-track details panel and spectrogram window IPC (ADR 0013).
//!
//! Windows edits files in place (atomic temp + rename). Android tracks are real paths under All
//! files access (ADR 0014) and are edited like files; a remaining legacy `content://` track is
//! copied to a private work file, edited there, then written back in place through its real path
//! (see `android_io`), with byte verification and restore of the original on failure.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tauri::ipc::Response;
use tauri::{AppHandle, State};

use super::AppState;

/// Library locators can carry the `\\?\` verbatim prefix (from `std::fs::canonicalize`).
/// Explorer / the shell do not accept it and users should not see it: `\\?\C:\x` -> `C:\x`,
/// `\\?\UNC\server\share\x` -> `\\server\share\x`. Other verbatim forms are kept.
fn strip_verbatim(path: std::path::PathBuf) -> std::path::PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return std::path::PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        let bytes = rest.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return std::path::PathBuf::from(rest);
        }
    }
    path
}

#[cfg(target_os = "windows")]
fn track_path(state: &AppState, track_id: &str) -> Result<std::path::PathBuf, String> {
    track_path_raw(state, track_id).map(strip_verbatim)
}

#[cfg(target_os = "windows")]
fn track_path_raw(state: &AppState, track_id: &str) -> Result<std::path::PathBuf, String> {
    use player_core::{MediaLocator, TrackId};

    let id = TrackId::parse(track_id).map_err(|error| format!("曲目 ID 無效：{error}"))?;
    let external = state.playback.as_ref().and_then(|service| {
        service
            .external_tracks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id)
            .map(|track| track.path.clone())
    });
    if let Some(path) = external {
        return Ok(path);
    }
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫無法使用".to_owned())
    })?;
    let locators = database
        .track_locators(id)
        .map_err(|error| error.to_string())?;
    let has_remote = locators
        .iter()
        .any(|locator| matches!(locator, MediaLocator::Remote { .. }));
    let mut fallback = None;
    for locator in locators {
        if let MediaLocator::FileSystem(path) = locator {
            if path.is_file() {
                return Ok(path);
            }
            fallback.get_or_insert(path);
        }
    }
    match fallback {
        Some(path) => Ok(path),
        // ADR 0026: remote tracks are read-only in the first release, and the details panel works on a local file.
        None if has_remote => Err("遠端曲目沒有本機檔案位置。".to_owned()),
        None => Err("找不到此曲目的檔案路徑".to_owned()),
    }
}

/// Where a track's bytes live.
enum TrackSource {
    File(std::path::PathBuf),
    /// A file on a remote server (SMB, WebDAV): viewed, never written.
    Remote {
        source_id: player_core::SourceId,
        path: String,
    },
    #[cfg(target_os = "android")]
    Content(String),
}

/// The file of a track on a remote server, when it has one.
fn remote_source(state: &AppState, track_id: &str) -> Option<TrackSource> {
    use player_core::{MediaLocator, TrackId};

    let id = TrackId::parse(track_id).ok()?;
    let locators = state.database.as_ref()?.track_locators(id).ok()?;
    locators.into_iter().find_map(|locator| match locator {
        MediaLocator::Remote { source_id, path } => Some(TrackSource::Remote { source_id, path }),
        _ => None,
    })
}

#[cfg(target_os = "windows")]
fn track_source(state: &AppState, track_id: &str) -> Result<TrackSource, String> {
    match track_path(state, track_id) {
        Ok(path) => Ok(TrackSource::File(path)),
        Err(error) => remote_source(state, track_id).ok_or(error),
    }
}

#[cfg(target_os = "android")]
fn track_source(state: &AppState, track_id: &str) -> Result<TrackSource, String> {
    use player_core::{MediaLocator, TrackId};

    let id = TrackId::parse(track_id).map_err(|error| format!("曲目 ID 無效：{error}"))?;
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫無法使用".to_owned())
    })?;
    let locators = database
        .track_locators(id)
        .map_err(|error| error.to_string())?;
    let mut content = None;
    for locator in locators {
        match locator {
            MediaLocator::FileSystem(path) if path.is_file() => return Ok(TrackSource::File(path)),
            MediaLocator::ContentUri(uri) => {
                content.get_or_insert(uri);
            }
            _ => {}
        }
    }
    content
        .map(TrackSource::Content)
        .or_else(|| remote_source(state, track_id))
        .ok_or_else(|| "找不到此曲目的檔案位置".to_owned())
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
fn track_source(_state: &AppState, _track_id: &str) -> Result<TrackSource, String> {
    Err("此平台尚未支援歌曲詳細資訊".to_owned())
}

/// A user-supplied path string (dialog result / drag-drop / Android document URI).
fn source_from_string(value: &str) -> Result<TrackSource, String> {
    #[cfg(target_os = "android")]
    if value.starts_with("content://") {
        return Ok(TrackSource::Content(value.to_owned()));
    }
    let path = strip_verbatim(std::path::PathBuf::from(value));
    if !path.is_file() {
        return Err("找不到檔案".to_owned());
    }
    Ok(TrackSource::File(path))
}

/// Run a read-only operation on a local path for the source (Android: a private work copy).
/// Also returns the user-facing file name and location string.
/// How much of each end of a remote file is fetched to read its tags (they sit at the start of most formats, at the end of
/// some: ID3v1, APEv2, DSF). A file up to twice this is fetched whole.
const REMOTE_END_BYTES: u64 = 4 * 1024 * 1024;
/// The largest remote file fetched whole when its two ends were not enough.
const REMOTE_WHOLE_LIMIT: u64 = 1024 * 1024 * 1024;

/// A local scratch copy of a remote file (the same name, so the format is recognised), removed when dropped.
struct RemoteScratch {
    directory: std::path::PathBuf,
    file: std::path::PathBuf,
}

impl Drop for RemoteScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn remote_file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_owned()
}

/// Fetch a remote file into a scratch file: both ends (`whole` false; the middle stays zeros and the length is right) or all
/// of it. Returns the scratch and the real length.
fn fetch_remote(
    source_id: player_core::SourceId,
    path: &str,
    whole: bool,
) -> Result<(RemoteScratch, u64), String> {
    use std::io::{Read, Seek, SeekFrom, Write};

    let content = player_core::remote_content().ok_or_else(|| "遠端來源尚未就緒。".to_owned())?;
    let mut handle = content
        .open(source_id, path)
        .map_err(|error| format!("無法讀取遠端檔案：{error}"))?;
    let length = handle.len;
    if whole && length > REMOTE_WHOLE_LIMIT {
        return Err("遠端檔案太大（超過 1 GiB），無法整首讀取。".to_owned());
    }
    static CLEANED: std::sync::Once = std::sync::Once::new();
    let root = std::env::temp_dir().join("moemusicplayer-remote-details");
    CLEANED.call_once(|| {
        let _ = std::fs::remove_dir_all(&root);
    });
    let directory = root.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&directory).map_err(|error| format!("建立暫存資料夾失敗：{error}"))?;
    let name: String = remote_file_name(path)
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let scratch = RemoteScratch {
        file: directory.join(if name.is_empty() { "track".to_owned() } else { name }),
        directory,
    };
    let io_error = |error: std::io::Error| format!("讀取遠端檔案失敗：{error}");
    let mut out = std::fs::File::create(&scratch.file).map_err(io_error)?;
    if whole || length <= REMOTE_END_BYTES * 2 {
        std::io::copy(&mut (&mut handle.reader).take(length), &mut out).map_err(io_error)?;
    } else {
        out.set_len(length).map_err(io_error)?;
        handle.reader.seek(SeekFrom::Start(0)).map_err(io_error)?;
        std::io::copy(&mut (&mut handle.reader).take(REMOTE_END_BYTES), &mut out)
            .map_err(io_error)?;
        let tail_start = length - REMOTE_END_BYTES;
        handle
            .reader
            .seek(SeekFrom::Start(tail_start))
            .map_err(io_error)?;
        out.seek(SeekFrom::Start(tail_start)).map_err(io_error)?;
        std::io::copy(&mut (&mut handle.reader).take(REMOTE_END_BYTES), &mut out)
            .map_err(io_error)?;
    }
    out.flush().map_err(io_error)?;
    Ok((scratch, length))
}

/// The last scratch copy, kept for a short while: the panel asks for the details, the cover (and later the export, the
/// spectrogram) of the same song one after another, and each must not fetch the file from the server again.
struct KeptScratch {
    source_id: player_core::SourceId,
    path: String,
    /// The scratch holds the whole file (not only its two ends).
    whole: bool,
    length: u64,
    at: std::time::Instant,
    scratch: std::sync::Arc<RemoteScratch>,
}

static KEPT_SCRATCH: std::sync::Mutex<Option<KeptScratch>> = std::sync::Mutex::new(None);
const KEPT_SCRATCH_FOR: std::time::Duration = std::time::Duration::from_secs(120);

/// A scratch copy of the remote file: the kept one when it is of the same file, recent and complete enough, otherwise a new
/// one. The lock is held while fetching, so requests that arrive together wait for the first fetch instead of repeating it.
fn scratch_for(
    source_id: player_core::SourceId,
    path: &str,
    whole: bool,
) -> Result<(std::sync::Arc<RemoteScratch>, u64), String> {
    let mut slot = KEPT_SCRATCH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(kept) = slot.as_ref() {
        if kept.source_id == source_id
            && kept.path == path
            && (kept.whole || !whole)
            && kept.at.elapsed() < KEPT_SCRATCH_FOR
        {
            return Ok((std::sync::Arc::clone(&kept.scratch), kept.length));
        }
    }
    *slot = None;
    let (scratch, length) = fetch_remote(source_id, path, whole)?;
    let scratch = std::sync::Arc::new(scratch);
    *slot = Some(KeptScratch {
        source_id,
        path: path.to_owned(),
        whole: whole || length <= REMOTE_END_BYTES * 2,
        length,
        at: std::time::Instant::now(),
        scratch: std::sync::Arc::clone(&scratch),
    });
    Ok((scratch, length))
}

/// Run `work` on a scratch copy of a remote file. The two ends are tried first; when the format cannot be read from them (a
/// cover larger than an end, a layout that keeps its metadata in the middle) the whole file is fetched and `work` runs again.
fn with_remote_file<T>(
    source_id: player_core::SourceId,
    path: &str,
    mut work: impl FnMut(&std::path::Path) -> Result<T, String>,
) -> Result<(T, u64), String> {
    let (partial, length) = scratch_for(source_id, path, false)?;
    match work(&partial.file) {
        Ok(value) => Ok((value, length)),
        Err(first) if length > REMOTE_END_BYTES * 2 && length <= REMOTE_WHOLE_LIMIT => {
            drop(partial);
            let (whole, length) = scratch_for(source_id, path, true).map_err(|_| first)?;
            Ok((work(&whole.file)?, length))
        }
        Err(error) => Err(error),
    }
}

fn read_remote_details(
    source_id: player_core::SourceId,
    path: &str,
) -> Result<player_tags::TrackDetails, String> {
    let (mut details, length) = with_remote_file(source_id, path, player_tags::read_details)?;
    // The facts of the scratch file are not the file's: the real size is known, the times are not.
    details.file_size = length;
    details.modified_unix_ms = None;
    details.created_unix_ms = None;
    details.file_name = remote_file_name(path);
    details.path = path.to_owned();
    details.read_only = true;
    details.read_only_reason = Some("遠端來源為唯讀，無法儲存修改。".to_owned());
    details.remote = true;
    Ok(details)
}

fn with_readable<T>(
    app: &AppHandle,
    source: &TrackSource,
    mut work: impl FnMut(&std::path::Path) -> Result<T, String>,
) -> Result<(T, String, String), String> {
    let _ = app;
    match source {
        TrackSource::Remote { source_id, path } => {
            let (value, _) = with_remote_file(*source_id, path, work)?;
            Ok((value, remote_file_name(path), path.clone()))
        }
        TrackSource::File(path) => {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok((work(path)?, name, path.to_string_lossy().into_owned()))
        }
        #[cfg(target_os = "android")]
        TrackSource::Content(uri) => {
            let file = android_io::materialize(app, uri)?;
            let name = file.display_name();
            Ok((work(&file.path)?, name, uri.clone()))
        }
    }
}

fn present_details(
    app: &AppHandle,
    mut details: player_tags::TrackDetails,
    name: String,
    location: String,
) -> player_tags::TrackDetails {
    let _ = app;
    #[cfg(target_os = "android")]
    if location.starts_with("content://") {
        details.modified_unix_ms = android_io::document_modified_ms(app, &location);
        details.created_unix_ms = None;
    }
    details.file_name = name;
    details.path = location;
    details
}

/// Read a picked image (path, or Android document URI), bounded to 64 MiB.
fn read_image_source(app: &AppHandle, value: &str) -> Result<Vec<u8>, String> {
    let _ = app;
    const MAX_IMAGE: u64 = 64 * 1024 * 1024;
    #[cfg(target_os = "android")]
    if value.starts_with("content://") {
        return android_io::read_small_document(app, value, MAX_IMAGE);
    }
    let meta = std::fs::metadata(value).map_err(|error| error.to_string())?;
    if meta.len() > MAX_IMAGE {
        return Err("圖片過大".to_owned());
    }
    std::fs::read(value).map_err(|error| format!("讀取封面圖片失敗：{error}"))
}

#[cfg(target_os = "android")]
mod android_io {
    use std::io::Read;
    use std::path::{Path, PathBuf};

    use tauri::{AppHandle, Manager};
    use tauri_plugin_media_index::MediaIndexExt;

    /// Private copy of a document; deleted on drop.
    pub struct WorkFile {
        pub path: PathBuf,
        pub display_name: Option<String>,
    }

    /// Document modification time reported by the provider (the work copy's own times are
    /// meaningless). Android exposes no creation time.
    pub fn document_modified_ms(app: &AppHandle, uri: &str) -> Option<u64> {
        app.media_index()
            .probe_content_uri(uri)
            .ok()
            .flatten()
            .and_then(|meta| meta.modified_at_utc_ms)
            .and_then(|ms| u64::try_from(ms).ok())
    }

    impl WorkFile {
        pub fn display_name(&self) -> String {
            self.display_name.clone().unwrap_or_else(|| {
                self.path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
        }
    }

    impl Drop for WorkFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn work_dir(app: &AppHandle) -> Result<PathBuf, String> {
        let dir = app
            .path()
            .app_cache_dir()
            .map_err(|error| error.to_string())?
            .join("track-details");
        std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        // Sweep work files left by a crash (older than an hour).
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
            for entry in entries.flatten() {
                let stale = entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .map(|modified| modified < cutoff)
                    .unwrap_or(false);
                if stale {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        Ok(dir)
    }

    fn unique_name(extension: &str) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        format!("{nanos:x}-{}.{extension}", COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// File extension so lofty / symphonia detect the format: display name first, MIME second.
    fn extension_for(name: Option<&str>, mime: Option<&str>) -> String {
        if let Some(extension) = name
            .and_then(|name| Path::new(name).extension())
            .and_then(|extension| extension.to_str())
            .filter(|extension| {
                !extension.is_empty()
                    && extension.len() <= 5
                    && extension.chars().all(|c| c.is_ascii_alphanumeric())
            })
        {
            return extension.to_ascii_lowercase();
        }
        player_formats::extension_for_mime(mime.unwrap_or_default())
            .unwrap_or("bin")
            .to_owned()
    }

    /// Copy the document into a private work file with a format-revealing extension.
    pub fn materialize(app: &AppHandle, uri: &str) -> Result<WorkFile, String> {
        let (lease, display_name) = app
            .media_index()
            .cache_audio_for_edit(uri)
            .map_err(|error| format!("讀取檔案失敗：{error}"))?;
        let extension = extension_for(display_name.as_deref(), lease.mime_type());
        let path = work_dir(app)?.join(unique_name(&extension));
        std::fs::copy(lease.path(), &path).map_err(|error| format!("複製工作檔失敗：{error}"))?;
        drop(lease);
        Ok(WorkFile { path, display_name })
    }

    /// Byte copy of a work file (kept as the restore point before editing).
    pub fn duplicate(app: &AppHandle, file: &WorkFile) -> Result<WorkFile, String> {
        let extension = file
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("bin")
            .to_owned();
        let path = work_dir(app)?.join(unique_name(&extension));
        std::fs::copy(&file.path, &path).map_err(|error| format!("建立還原點失敗：{error}"))?;
        Ok(WorkFile {
            path,
            display_name: file.display_name.clone(),
        })
    }

    fn files_equal(left: &Path, right: &Path) -> Result<bool, String> {
        let open = |path: &Path| std::fs::File::open(path).map_err(|error| error.to_string());
        let (mut a, mut b) = (open(left)?, open(right)?);
        if a.metadata().map_err(|e| e.to_string())?.len() != b.metadata().map_err(|e| e.to_string())?.len() {
            return Ok(false);
        }
        let (mut buf_a, mut buf_b) = (vec![0u8; 256 * 1024], vec![0u8; 256 * 1024]);
        loop {
            let read = a.read(&mut buf_a).map_err(|error| error.to_string())?;
            if read == 0 {
                return Ok(true);
            }
            b.read_exact(&mut buf_b[..read]).map_err(|error| error.to_string())?;
            if buf_a[..read] != buf_b[..read] {
                return Ok(false);
            }
        }
    }

    /// Real file behind the document when All files access is granted (ADR 0014).
    fn direct_path(app: &AppHandle, uri: &str) -> Option<PathBuf> {
        app.media_index()
            .resolve_direct_path(uri)
            .ok()
            .flatten()
            .map(PathBuf::from)
            .filter(|path| path.is_file())
    }

    /// Overwrite the file in place (same path and MediaStore row; no temp + rename, which would
    /// make MediaStore assign a new id and break library locators).
    fn write_in_place(source: &Path, destination: &Path) -> Result<(), String> {
        let mut input = std::fs::File::open(source).map_err(|error| error.to_string())?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(destination)
            .map_err(|error| error.to_string())?;
        std::io::copy(&mut input, &mut output).map_err(|error| error.to_string())?;
        output.sync_all().map_err(|error| error.to_string())
    }

    fn push(app: &AppHandle, source: &Path, uri: &str) -> Result<(), String> {
        // No per-file MediaStore/SAF write prompts (ADR 0014): writes go to the real path,
        // which needs All files access.
        match direct_path(app, uri) {
            Some(path) => write_in_place(source, &path),
            None => Err("NO_DIRECT_PATH".to_owned()),
        }
    }

    /// Write the edited work file back through the ContentResolver, then re-read and compare
    /// bytes. On a write/verify failure the original bytes are pushed back.
    pub fn write_back(app: &AppHandle, edited: &Path, original: &Path, uri: &str) -> Result<(), String> {
        if let Err(error) = push(app, edited, uri) {
            if error.contains("NO_DIRECT_PATH") {
                return Err(
                    "無法取得此檔案的實際路徑；請在 Android 設定開啟 MoeMusicPlayer 的「所有檔案存取權」。".to_owned(),
                );
            }
            let restored = push(app, original, uri).is_ok();
            return Err(format!(
                "寫回檔案失敗：{error}{}",
                if restored { "（已還原原檔）" } else { "（還原原檔也失敗，請檢查檔案）" }
            ));
        }
        let check = materialize(app, uri)?;
        if !files_equal(&check.path, edited)? {
            drop(check);
            let restored = push(app, original, uri).is_ok();
            rescan(app, uri);
            return Err(format!(
                "寫回後驗證不一致{}",
                if restored { "，已還原原檔" } else { "，還原原檔也失敗，請檢查檔案" }
            ));
        }
        rescan(app, uri);
        Ok(())
    }

    /// After a direct-path write, MediaStore must re-read size/tags (ContentResolver writes do this
    /// themselves).
    fn rescan(app: &AppHandle, uri: &str) {
        if let Some(path) = direct_path(app, uri) {
            let _ = app.media_index().scan_file(&path.to_string_lossy());
        }
    }

    /// Small document (picked cover image) into memory.
    pub fn read_small_document(app: &AppHandle, uri: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
        let lease = app
            .media_index()
            .cache_content_uri(uri, max_bytes)
            .map_err(|error| format!("讀取檔案失敗：{error}"))?;
        std::fs::read(lease.path()).map_err(|error| error.to_string())
    }

    /// Write small bytes (exported cover) into a document created by the save picker.
    pub fn write_small_document(app: &AppHandle, uri: &str, bytes: &[u8]) -> Result<(), String> {
        let index = app.media_index();
        let lease = index.create_cache_lease().map_err(|error| error.to_string())?;
        std::fs::write(lease.path(), bytes).map_err(|error| error.to_string())?;
        index
            .write_content_cache_lease(&lease, uri)
            .map_err(|error| format!("寫入失敗：{error}"))
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| format!("背景工作失敗：{error}"))?
}

#[tauri::command]
pub async fn track_details_get(
    app: AppHandle,
    state: State<'_, AppState>,
    track_id: String,
) -> Result<player_tags::TrackDetails, String> {
    let source = track_source(&state, &track_id)?;
    blocking(move || {
        if let TrackSource::Remote { source_id, path } = &source {
            return read_remote_details(*source_id, path);
        }
        let (details, name, location) = with_readable(&app, &source, player_tags::read_details)?;
        Ok(present_details(&app, details, name, location))
    })
    .await
}

/// Raw bytes of the embedded front cover (empty when none).
#[tauri::command]
pub async fn track_details_cover(
    app: AppHandle,
    state: State<'_, AppState>,
    track_id: String,
) -> Result<Response, String> {
    let source = track_source(&state, &track_id)?;
    let cover = blocking(move || {
        with_readable(&app, &source, player_tags::tags::read_cover).map(|(cover, _, _)| cover)
    })
    .await?;
    Ok(Response::new(
        cover.map(|(bytes, _)| bytes).unwrap_or_default(),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackTagSaveRequest {
    pub fields: player_tags::TagFields,
    #[serde(default)]
    pub replace_cover_path: Option<String>,
    /// Clipboard image (base64 of the original bytes), staged like a picked file.
    #[serde(default)]
    pub replace_cover_base64: Option<String>,
    #[serde(default)]
    pub remove_cover: bool,
    pub cover_options: player_tags::CoverOptions,
}

#[tauri::command]
pub async fn track_details_save(
    app: AppHandle,
    state: State<'_, AppState>,
    track_id: String,
    request: TrackTagSaveRequest,
) -> Result<player_tags::TrackDetails, String> {
    let source = track_source(&state, &track_id)?;
    blocking(move || {
        let replace_cover = match &request.replace_cover_path {
            Some(image) => Some(read_image_source(&app, image)?),
            None => match &request.replace_cover_base64 {
                Some(data) => Some(
                    base64::engine::general_purpose::STANDARD
                        .decode(data.as_bytes())
                        .map_err(|error| format!("剪貼簿圖片格式錯誤：{error}"))?,
                ),
                None => None,
            },
        };
        let edit = player_tags::TagEdit {
            fields: request.fields,
            replace_cover,
            remove_cover: request.remove_cover,
            cover_options: request.cover_options,
        };
        match &source {
            TrackSource::Remote { .. } => Err("遠端來源為唯讀，無法儲存歌曲資訊。".to_owned()),
            TrackSource::File(path) => player_tags::write_edit(path, &edit),
            #[cfg(target_os = "android")]
            TrackSource::Content(uri) => {
                // Same semantics as desktop on a private copy: write_edit does temp + verify +
                // rename inside the work dir; then the bytes go back via the ContentResolver.
                let work = android_io::materialize(&app, uri)?;
                let original = android_io::duplicate(&app, &work)?;
                let details = player_tags::write_edit(&work.path, &edit)?;
                android_io::write_back(&app, &work.path, &original.path, uri)?;
                Ok(present_details(&app, details, work.display_name(), uri.clone()))
            }
        }
    })
    .await
}

/// Pick a replacement cover image; returns its path (staged until Save).
#[tauri::command]
pub async fn track_details_pick_cover(
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Option<String>, String> {
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_dialog::DialogExt;
        let selected = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("選擇封面圖片")
            .add_filter("圖片", &["jpg", "jpeg", "png", "webp", "gif", "bmp"])
            .blocking_pick_file();
        let Some(selected) = selected else {
            return Ok(None);
        };
        let path = selected
            .into_path()
            .map_err(|error| format!("無法取得檔案路徑：{error}"))?;
        Ok(Some(path.to_string_lossy().into_owned()))
    }
    #[cfg(target_os = "android")]
    {
        let _ = window;
        use tauri_plugin_media_index::MediaIndexExt;
        return blocking(move || {
            app.media_index()
                .pick_open_document(&["image/*"])
                .map(|picked| picked.map(|(uri, _)| uri))
                .map_err(|error| error.to_string())
        })
        .await;
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (app, window);
        Err("此平台尚未支援選擇封面".to_owned())
    }
}

/// Preview bytes of a picked (not yet saved) cover file.
#[tauri::command]
pub async fn track_details_read_image(app: AppHandle, path: String) -> Result<Response, String> {
    let bytes = blocking(move || read_image_source(&app, &path)).await?;
    Ok(Response::new(bytes))
}

/// Export the embedded front cover through a save dialog. Returns the saved path.
#[tauri::command]
pub async fn track_details_export_cover(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    track_id: String,
) -> Result<Option<String>, String> {
    let source = track_source(&state, &track_id)?;
    let reader = app.clone();
    let cover = blocking(move || {
        with_readable(&reader, &source, player_tags::tags::read_cover).map(|(cover, _, _)| cover)
    })
    .await?;
    let Some((bytes, mime)) = cover else {
        return Err("此曲目沒有內嵌封面".to_owned());
    };
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_dialog::DialogExt;
        let extension = match mime.as_str() {
            "image/png" => "png",
            "image/webp" => "webp",
            "image/gif" => "gif",
            "image/bmp" => "bmp",
            _ => "jpg",
        };
        let selected = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("匯出封面")
            .set_file_name(format!("cover.{extension}"))
            .add_filter("圖片", &[extension])
            .blocking_save_file();
        let Some(selected) = selected else {
            return Ok(None);
        };
        let target = selected
            .into_path()
            .map_err(|error| format!("無法取得檔案路徑：{error}"))?;
        std::fs::write(&target, bytes).map_err(|error| format!("寫入失敗：{error}"))?;
        Ok(Some(target.to_string_lossy().into_owned()))
    }
    #[cfg(target_os = "android")]
    {
        let _ = window;
        use tauri_plugin_media_index::MediaIndexExt;
        let extension = match mime.as_str() {
            "image/png" => "png",
            "image/webp" => "webp",
            "image/gif" => "gif",
            "image/bmp" => "bmp",
            _ => "jpg",
        };
        let mime = if extension == "jpg" { "image/jpeg".to_owned() } else { mime };
        return blocking(move || {
            let Some(uri) = app
                .media_index()
                .pick_create_document(&format!("cover.{extension}"), &mime)
                .map_err(|error| error.to_string())?
            else {
                return Ok(None);
            };
            android_io::write_small_document(&app, &uri, &bytes)?;
            Ok(Some(format!("cover.{extension}")))
        })
        .await;
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (app, window, bytes, mime);
        Err("此平台尚未支援匯出封面".to_owned())
    }
}

/// Reveal the file in Explorer with it selected. Android: 「在資料夾中開啟」 shows the
/// containing folder in a file manager (best effort; no app selects the file itself).
#[tauri::command]
pub async fn track_details_reveal(
    app: AppHandle,
    state: State<'_, AppState>,
    track_id: String,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        let path = track_path(&state, &track_id)?;
        if !path.exists() {
            return Err(format!("找不到檔案：{}", path.display()));
        }
        // SHOpenFolderAndSelectItems takes the path as data, so spaces / commas / Unicode
        // need no command-line quoting (explorer.exe "/select," parsing breaks on those).
        blocking(move || reveal_in_explorer(&path)).await
    }
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_media_index::MediaIndexExt;
        let TrackSource::File(path) = track_source(&state, &track_id)? else {
            return Err("此曲目沒有實體路徑，請開啟「所有檔案存取權」後重新同步。".to_owned());
        };
        let folder = path
            .parent()
            .ok_or_else(|| "找不到曲目所在資料夾。".to_owned())?
            .to_string_lossy()
            .into_owned();
        let opened = blocking(move || {
            app.media_index()
                .open_folder_in_file_manager(&folder)
                .map_err(|error| error.to_string())
        })
        .await?;
        if opened {
            Ok(())
        } else {
            Err("找不到可開啟資料夾的檔案管理員；可用「複製路徑」後在檔案管理員中前往。".to_owned())
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (app, state, track_id);
        Err("此平台不支援".to_owned())
    }
}

/// Android: whether the clipboard holds an image. Android WebView rejects
/// `navigator.clipboard.read()` for images, so the cover paste uses ClipboardManager natively.
#[tauri::command]
pub async fn track_details_clipboard_has_image(app: AppHandle) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_media_index::MediaIndexExt;
        blocking(move || app.media_index().clipboard_has_image().map_err(|error| error.to_string()))
            .await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(false)
    }
}

/// Android: the clipboard image bytes (copied to the app cache by the native side first); an
/// error message when the clipboard holds no readable image.
#[tauri::command]
pub async fn track_details_read_clipboard_image(app: AppHandle) -> Result<Response, String> {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_media_index::MediaIndexExt;
        let bytes = blocking(move || {
            let path = match app
                .media_index()
                .read_clipboard_image()
                .map_err(|error| error.to_string())?
            {
                Ok(path) => path,
                Err(reason) if reason == "noClip" => {
                    return Err(
                        "無法讀取剪貼簿：請確認 MoeMusicPlayer 在前景後再按「貼上」。".to_owned(),
                    )
                }
                Err(_) => {
                    return Err(
                        "剪貼簿中沒有可讀取的圖片（支援 JPEG、PNG、WebP、GIF、BMP）。".to_owned(),
                    )
                }
            };
            let result = read_image_source(&app, &path);
            let _ = std::fs::remove_file(&path);
            result
        })
        .await?;
        Ok(Response::new(bytes))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err("此平台使用 WebView 剪貼簿".to_owned())
    }
}

/// Android: put the track's embedded cover on the system clipboard natively. Android WebView's
/// navigator.clipboard.write() resolves for images without placing anything on ClipboardManager,
/// so the cover is written to a private cache file and shared as a FileProvider content URI.
#[tauri::command]
pub async fn track_details_copy_cover(
    app: AppHandle,
    state: State<'_, AppState>,
    track_id: String,
) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_media_index::MediaIndexExt;
        let source = track_source(&state, &track_id)?;
        blocking(move || {
            let (cover, _, _) = with_readable(&app, &source, player_tags::tags::read_cover)?;
            let Some((bytes, _)) = cover.filter(|(bytes, _)| !bytes.is_empty()) else {
                return Err("此曲目沒有內嵌封面".to_owned());
            };
            let extension =
                image_extension(&bytes).ok_or_else(|| "封面不是可複製的圖片格式".to_owned())?;
            let directory = app
                .media_index()
                .clipboard_share_directory()
                .map_err(|error| error.to_string())?;
            std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
            let path = directory.join(format!("cover-{}.{extension}", uuid::Uuid::new_v4()));
            std::fs::write(&path, &bytes).map_err(|error| format!("寫入暫存封面失敗：{error}"))?;
            app.media_index()
                .write_clipboard_image(&path.to_string_lossy())
                .map_err(|error| format!("複製封面失敗：{error}"))
        })
        .await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, state, track_id);
        Err("此平台使用 WebView 剪貼簿".to_owned())
    }
}

#[cfg(any(target_os = "android", test))]
fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.starts_with(b"BM") {
        Some("bmp")
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn reveal_in_explorer(path: &std::path::Path) -> Result<(), String> {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems};
    use windows::core::HSTRING;

    let wide = HSTRING::from(path.as_os_str());
    // SAFETY: COM is initialized for this blocking worker thread and balanced below; the
    // PIDL is owned here and freed exactly once.
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let pidl = ILCreateFromPathW(&wide);
        let result = if pidl.is_null() {
            Err(format!("無法解析路徑：{}", path.display()))
        } else {
            let opened = SHOpenFolderAndSelectItems(pidl, None, 0)
                .map_err(|error| format!("無法開啟檔案總管：{error}"));
            ILFree(Some(pidl));
            opened
        };
        if initialized {
            CoUninitialize();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::strip_verbatim;
    use std::path::PathBuf;

    #[test]
    fn clipboard_cover_extension_follows_the_image_signature() {
        use super::image_extension;
        assert_eq!(image_extension(&[0xff, 0xd8, 0xff, 0xe0]), Some("jpg"));
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(image_extension(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(image_extension(b"RIFF\0\0\0\0WAVEfmt "), None);
        assert_eq!(image_extension(b"GIF89a"), Some("gif"));
        assert_eq!(image_extension(b"text"), None);
    }

    #[test]
    fn strips_verbatim_drive_and_unc_prefixes() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\音樂\a, b (1).flac")),
            PathBuf::from(r"C:\音樂\a, b (1).flac")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\nas\share\x y.mp3")),
            PathBuf::from(r"\\nas\share\x y.mp3")
        );
        assert_eq!(strip_verbatim(PathBuf::from(r"C:\x.mp3")), PathBuf::from(r"C:\x.mp3"));
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\Volume{abc}\x")),
            PathBuf::from(r"\\?\Volume{abc}\x")
        );
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectrogramDto {
    pub png_base64: String,
    pub levels_base64: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub channel_labels: Vec<String>,
    pub panels: u32,
    pub duration_ms: u64,
    pub columns: u32,
    pub height: u32,
    pub max_frequency_hz: u32,
    pub cutoff_hz: Option<u32>,
    pub file_name: String,
    pub path: String,
    /// Diagnostics: local copy (Android content://), decode+FFT+PNG, base64.
    pub prepare_ms: u64,
    pub render_ms: u64,
    pub encode_ms: u64,
}

fn spectrogram_dto(
    app: &AppHandle,
    source: &TrackSource,
    options: Option<player_tags::SpectrogramOptions>,
) -> Result<SpectrogramDto, String> {
    let options = options.unwrap_or_default();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let started = std::time::Instant::now();
    let mut render_ms = 0u64;
    let mut render = |path: &std::path::Path| {
        let render_started = std::time::Instant::now();
        let result = player_tags::render_spectrogram(path, options, &cancel);
        render_ms = render_started.elapsed().as_millis() as u64;
        result
    };
    let (spectrogram, file_name, location) = match source {
        // The spectrogram decodes the whole song: a remote file is fetched whole (not only its two ends), and removed after.
        TrackSource::Remote { source_id, path } => {
            let (scratch, _) = scratch_for(*source_id, path, true)?;
            let value = render(&scratch.file)?;
            (value, remote_file_name(path), path.clone())
        }
        _ => with_readable(app, source, render)?,
    };
    let prepare_ms = (started.elapsed().as_millis() as u64).saturating_sub(render_ms);
    let encode_started = std::time::Instant::now();
    let engine = base64::engine::general_purpose::STANDARD;
    let png_base64 = engine.encode(&spectrogram.png);
    let levels_base64 = engine.encode(&spectrogram.levels);
    let encode_ms = encode_started.elapsed().as_millis() as u64;
    Ok(SpectrogramDto {
        prepare_ms,
        render_ms,
        encode_ms,
        png_base64,
        levels_base64,
        sample_rate: spectrogram.sample_rate,
        channels: spectrogram.channels,
        channel_labels: spectrogram.channel_labels,
        panels: spectrogram.panels,
        duration_ms: spectrogram.duration_ms,
        columns: spectrogram.columns,
        height: spectrogram.height,
        max_frequency_hz: spectrogram.max_frequency_hz,
        cutoff_hz: spectrogram.cutoff_hz,
        file_name,
        path: location,
    })
}

/// Render a library track's spectrogram with the window's current settings (FFT size,
/// window function, colormap, channel, dB range, linear/log axis, max frequency).
#[tauri::command]
pub async fn track_spectrogram(
    app: AppHandle,
    state: State<'_, AppState>,
    track_id: String,
    options: Option<player_tags::SpectrogramOptions>,
) -> Result<SpectrogramDto, String> {
    let source = track_source(&state, &track_id)?;
    blocking(move || spectrogram_dto(&app, &source, options)).await
}

/// Render any audio file chosen in the spectrogram window (dialog or drag-and-drop);
/// it does not need to be in the library.
#[tauri::command]
pub async fn spectrogram_render_file(
    app: AppHandle,
    path: String,
    options: Option<player_tags::SpectrogramOptions>,
) -> Result<SpectrogramDto, String> {
    let source = source_from_string(&path)?;
    blocking(move || spectrogram_dto(&app, &source, options)).await
}

/// File dialog for the spectrogram window's 「開啟 A／B」.
#[tauri::command]
pub async fn spectrogram_pick_file(
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Option<String>, String> {
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_dialog::DialogExt;
        let selected = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("開啟音訊檔")
            .add_filter(
                "音訊",
                &player_formats::open_dialog_extensions(),
            )
            .add_filter("所有檔案", &["*"])
            .blocking_pick_file();
        let Some(selected) = selected else {
            return Ok(None);
        };
        let path = selected
            .into_path()
            .map_err(|error| format!("無法取得檔案路徑：{error}"))?;
        Ok(Some(path.to_string_lossy().into_owned()))
    }
    #[cfg(target_os = "android")]
    {
        let _ = window;
        use tauri_plugin_media_index::MediaIndexExt;
        return blocking(move || {
            app.media_index()
                .pick_open_document(&["audio/*", "application/ogg", "video/mp4", "video/x-matroska", "video/webm"])
                .map(|picked| picked.map(|(uri, _)| uri))
                .map_err(|error| error.to_string())
        })
        .await;
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (app, window);
        Err("此平台尚未支援開啟檔案".to_owned())
    }
}

pub const SPECTROGRAM_WINDOW_LABEL: &str = "spectrogram";
pub const SPECTROGRAM_TRACK_EVENT: &str = "spectrogram-track";

/// Desktop: open (or focus and retarget) the separate spectrogram window.
#[tauri::command]
pub async fn spectrogram_window_open(app: AppHandle, track_id: String) -> Result<(), String> {
    use tauri::{Emitter, Manager};
    #[cfg(desktop)]
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    player_core::TrackId::parse(&track_id).map_err(|error| format!("曲目 ID 無效：{error}"))?;
    if let Some(window) = app.get_webview_window(SPECTROGRAM_WINDOW_LABEL) {
        window
            .emit(SPECTROGRAM_TRACK_EVENT, &track_id)
            .map_err(|error| error.to_string())?;
        #[cfg(desktop)]
        let _ = window.unminimize();
        let _ = window.set_focus();
        return Ok(());
    }
    #[cfg(mobile)]
    return Err("spectrogram window is desktop-only".into());
    #[cfg(desktop)]
    {
    let url = format!("index.html?view=spectrogram&track={track_id}");
    WebviewWindowBuilder::new(&app, SPECTROGRAM_WINDOW_LABEL, WebviewUrl::App(url.into()))
        .title("頻譜圖")
        .inner_size(1100.0, 600.0)
        .min_inner_size(480.0, 300.0)
        .build()
        .map_err(|error| format!("無法開啟頻譜圖視窗：{error}"))?;
    Ok(())
    }
}
