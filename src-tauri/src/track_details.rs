//! Single-track details panel and spectrogram window IPC (ADR 0013).
//!
//! Phase 1 is Windows-only: Android tag writing needs SAF write grants (ADR 0010) and is
//! reported as unsupported for now.

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

#[cfg(not(target_os = "windows"))]
fn track_path(_state: &AppState, _track_id: &str) -> Result<std::path::PathBuf, String> {
    Err("此平台尚未支援歌曲詳細資訊編輯".to_owned())
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
    state: State<'_, AppState>,
    track_id: String,
) -> Result<player_tags::TrackDetails, String> {
    let path = track_path(&state, &track_id)?;
    blocking(move || player_tags::read_details(&path)).await
}

/// Raw bytes of the embedded front cover (empty when none).
#[tauri::command]
pub async fn track_details_cover(
    state: State<'_, AppState>,
    track_id: String,
) -> Result<Response, String> {
    let path = track_path(&state, &track_id)?;
    let cover = blocking(move || player_tags::tags::read_cover(&path)).await?;
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
    #[serde(default)]
    pub remove_cover: bool,
    pub cover_options: player_tags::CoverOptions,
}

#[tauri::command]
pub async fn track_details_save(
    state: State<'_, AppState>,
    track_id: String,
    request: TrackTagSaveRequest,
) -> Result<player_tags::TrackDetails, String> {
    let path = track_path(&state, &track_id)?;
    blocking(move || {
        let replace_cover = match &request.replace_cover_path {
            Some(image) => {
                Some(std::fs::read(image).map_err(|error| format!("讀取封面圖片失敗：{error}"))?)
            }
            None => None,
        };
        let edit = player_tags::TagEdit {
            fields: request.fields,
            replace_cover,
            remove_cover: request.remove_cover,
            cover_options: request.cover_options,
        };
        player_tags::write_edit(&path, &edit)
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
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, window);
        Err("此平台尚未支援選擇封面".to_owned())
    }
}

/// Preview bytes of a picked (not yet saved) cover file.
#[tauri::command]
pub async fn track_details_read_image(path: String) -> Result<Response, String> {
    let bytes = blocking(move || {
        let meta = std::fs::metadata(&path).map_err(|error| error.to_string())?;
        if meta.len() > 64 * 1024 * 1024 {
            return Err("圖片過大".to_owned());
        }
        std::fs::read(&path).map_err(|error| error.to_string())
    })
    .await?;
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
    let path = track_path(&state, &track_id)?;
    let cover = blocking(move || player_tags::tags::read_cover(&path)).await?;
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
    #[cfg(not(target_os = "windows"))]
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
    let path = track_path(&state, &track_id)?;
    #[cfg(target_os = "windows")]
    {
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
        let _ = path;
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
    path: &std::path::Path,
    options: Option<player_tags::SpectrogramOptions>,
) -> Result<SpectrogramDto, String> {
    let options = options.unwrap_or_default();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let spectrogram = player_tags::render_spectrogram(path, options, &cancel)?;
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
        file_name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: path.to_string_lossy().into_owned(),
    })
}

/// Render a library track's spectrogram with the window's current settings (FFT size,
/// window function, colormap, channel, dB range, linear/log axis, max frequency).
#[tauri::command]
pub async fn track_spectrogram(
    state: State<'_, AppState>,
    track_id: String,
    options: Option<player_tags::SpectrogramOptions>,
) -> Result<SpectrogramDto, String> {
    let path = track_path(&state, &track_id)?;
    blocking(move || spectrogram_dto(&path, options)).await
}

/// Render any audio file chosen in the spectrogram window (dialog or drag-and-drop);
/// it does not need to be in the library.
#[tauri::command]
pub async fn spectrogram_render_file(
    path: String,
    options: Option<player_tags::SpectrogramOptions>,
) -> Result<SpectrogramDto, String> {
    let path = std::path::PathBuf::from(path);
    if !path.is_file() {
        return Err("找不到檔案".to_owned());
    }
    blocking(move || spectrogram_dto(&path, options)).await
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
    #[cfg(not(target_os = "windows"))]
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
    use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

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
