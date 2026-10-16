//! Single-track details panel and spectrogram window IPC (ADR 0013).
//!
//! Windows edits files in place (atomic temp + rename). Android tracks are `content://`
//! documents (MediaStore or SAF): they are copied to a private work file, edited/rendered
//! there with the same code, then written back through the ContentResolver (see
//! `android_io`), with byte verification and restore of the original on failure.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tauri::ipc::Response;
use tauri::{AppHandle, State};

use super::AppState;

#[cfg(target_os = "windows")]
fn track_path(state: &AppState, track_id: &str) -> Result<std::path::PathBuf, String> {
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
    let mut fallback = None;
    for locator in locators {
        if let MediaLocator::FileSystem(path) = locator {
            if path.is_file() {
                return Ok(path);
            }
            fallback.get_or_insert(path);
        }
    }
    fallback.ok_or_else(|| "找不到此曲目的檔案路徑".to_owned())
}

/// Where a track's bytes live.
enum TrackSource {
    File(std::path::PathBuf),
    #[cfg(target_os = "android")]
    Content(String),
}

#[cfg(target_os = "windows")]
fn track_source(state: &AppState, track_id: &str) -> Result<TrackSource, String> {
    track_path(state, track_id).map(TrackSource::File)
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
    let path = std::path::PathBuf::from(value);
    if !path.is_file() {
        return Err("找不到檔案".to_owned());
    }
    Ok(TrackSource::File(path))
}

/// Run a read-only operation on a local path for the source (Android: a private work copy).
/// Also returns the user-facing file name and location string.
fn with_readable<T>(
    app: &AppHandle,
    source: &TrackSource,
    work: impl FnOnce(&std::path::Path) -> Result<T, String>,
) -> Result<(T, String, String), String> {
    let _ = app;
    match source {
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
        match mime.unwrap_or_default().to_ascii_lowercase().as_str() {
            "audio/mpeg" | "audio/mp3" => "mp3",
            "audio/flac" | "audio/x-flac" => "flac",
            "audio/mp4" | "audio/x-m4a" | "audio/m4a" | "audio/aac" | "audio/mp4a-latm" => "m4a",
            "audio/ogg" | "application/ogg" | "audio/vorbis" => "ogg",
            "audio/opus" => "opus",
            "audio/wav" | "audio/x-wav" | "audio/vnd.wave" | "audio/wave" => "wav",
            "audio/aiff" | "audio/x-aiff" => "aiff",
            "audio/x-dsf" | "audio/dsf" => "dsf",
            _ => "bin",
        }
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

    fn push(app: &AppHandle, source: &Path, uri: &str) -> Result<(), String> {
        let index = app.media_index();
        let lease = index.create_cache_lease().map_err(|error| error.to_string())?;
        std::fs::copy(source, lease.path()).map_err(|error| error.to_string())?;
        index
            .write_audio_cache_lease(&lease, uri)
            .map_err(|error| error.to_string())
    }

    /// Write the edited work file back through the ContentResolver, then re-read and compare
    /// bytes. On a write/verify failure the original bytes are pushed back.
    pub fn write_back(app: &AppHandle, edited: &Path, original: &Path, uri: &str) -> Result<(), String> {
        if let Err(error) = push(app, edited, uri) {
            if error.contains("WRITE_ACCESS_DENIED") {
                return Err(
                    "沒有寫入此檔案的權限（已拒絕授權，或 SAF 資料夾只授權讀取；請重新授權可寫入的資料夾）".to_owned(),
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
            return Err(format!(
                "寫回後驗證不一致{}",
                if restored { "，已還原原檔" } else { "，還原原檔也失敗，請檢查檔案" }
            ));
        }
        Ok(())
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

/// Reveal the file in Explorer with it selected.
#[tauri::command]
pub async fn track_details_reveal(
    state: State<'_, AppState>,
    track_id: String,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let path = track_path(&state, &track_id)?;
        let mut argument = std::ffi::OsString::from("/select,");
        argument.push(path.as_os_str());
        std::process::Command::new("explorer.exe")
            .arg(argument)
            .spawn()
            .map_err(|error| format!("無法開啟檔案總管：{error}"))?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, track_id);
        Err("此平台不支援".to_owned())
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
}

fn spectrogram_dto(
    app: &AppHandle,
    source: &TrackSource,
    options: Option<player_tags::SpectrogramOptions>,
) -> Result<SpectrogramDto, String> {
    let options = options.unwrap_or_default();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let (spectrogram, file_name, location) = with_readable(app, source, |path| {
        player_tags::render_spectrogram(path, options, &cancel)
    })?;
    let engine = base64::engine::general_purpose::STANDARD;
    Ok(SpectrogramDto {
        png_base64: engine.encode(&spectrogram.png),
        levels_base64: engine.encode(&spectrogram.levels),
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
                &[
                    "mp3", "flac", "m4a", "mp4", "aac", "alac", "ogg", "oga", "wav", "wave", "aif",
                    "aiff", "mka", "mkv", "webm", "caf",
                ],
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
