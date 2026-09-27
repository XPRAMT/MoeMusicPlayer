use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "windows")]
use std::{sync::Mutex, time::Duration};

use player_core::{
    LibraryRoot, ListTracksQuery, MediaLocator, MediaSourceKind, Page, PlaylistId, PlaylistPage,
    PlaylistSummary, SourceScanState, SyncEngine, SyncReport, TrackSummary,
};
use player_db::Database;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_media_index::MediaIndexExt;

mod playlist_exchange;
use playlist_exchange::{
    export_playlist_file, import_playlist_file, PlaylistExportResult, PlaylistImportResult,
};

#[cfg(target_os = "windows")]
use player_platform_windows::{windows_locator_key, WindowsMediaIndex};

#[cfg(target_os = "windows")]
use player_audio_windows::{AudioError, PlaybackState as AudioPlaybackState, PlayerHandle};

#[cfg(target_os = "windows")]
use player_core::TrackId;

const LIBRARY_SYNC_FINISHED_EVENT: &str = "library-sync-finished";

struct AppState {
    database: Option<Database>,
    database_path: Option<std::path::PathBuf>,
    database_error: Option<String>,
    #[cfg(target_os = "windows")]
    playback: Option<WindowsPlaybackService>,
    #[cfg(target_os = "windows")]
    playback_error: Option<String>,
}

#[cfg(target_os = "windows")]
struct WindowsPlaybackService {
    player: PlayerHandle,
    current_track: Mutex<Option<TrackSummary>>,
    command_gate: Mutex<()>,
}

#[cfg(target_os = "windows")]
impl WindowsPlaybackService {
    fn new(player: PlayerHandle) -> Self {
        Self {
            player,
            current_track: Mutex::new(None),
            command_gate: Mutex::new(()),
        }
    }

    fn snapshot(&self) -> PlaybackSnapshot {
        let audio = self.player.snapshot();
        let current_track = self
            .current_track
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        PlaybackSnapshot {
            current_track,
            state: audio_state_name(audio.state).to_owned(),
            is_playing: audio.state == AudioPlaybackState::Playing,
            position_ms: duration_millis(audio.position),
            duration_ms: audio.duration.map(duration_millis),
            volume: f64::from(audio.volume),
            last_error: audio.last_error.as_ref().map(playback_audio_error_message),
            repeat_mode: RepeatMode::Off,
            shuffle: false,
        }
    }
}

#[cfg(target_os = "windows")]
fn audio_state_name(state: AudioPlaybackState) -> &'static str {
    match state {
        AudioPlaybackState::Initializing => "initializing",
        AudioPlaybackState::Empty => "empty",
        AudioPlaybackState::Loading => "loading",
        AudioPlaybackState::Ready => "ready",
        AudioPlaybackState::Playing => "playing",
        AudioPlaybackState::Paused => "paused",
        AudioPlaybackState::Stopped => "stopped",
        AudioPlaybackState::Ended => "ended",
        AudioPlaybackState::Error => "error",
    }
}

#[cfg(target_os = "windows")]
fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(target_os = "windows")]
fn playback_audio_error_message(error: &AudioError) -> String {
    match error {
        AudioError::OutputDevice(_) => "無法連線到預設音訊輸出裝置。".to_owned(),
        AudioError::WorkerStart(_) => "無法啟動音訊背景工作。".to_owned(),
        AudioError::WorkerStopped => "音訊工作階段已結束。".to_owned(),
        AudioError::CommandQueueFull => "音訊服務忙碌，請稍後再試。".to_owned(),
        AudioError::UnsupportedPlatform => "此平台尚未提供本機音訊播放服務。".to_owned(),
        AudioError::FileOpen { .. } => "無法開啟曲目檔案，請確認檔案仍可讀取。".to_owned(),
        AudioError::UnsupportedFormat { .. } => "此音訊格式目前尚未支援。".to_owned(),
        AudioError::Decode { .. } => "曲目解碼失敗。".to_owned(),
        AudioError::Seek(_) => "無法移動到指定播放位置。".to_owned(),
        AudioError::Backend(_) => "音訊輸出發生錯誤。".to_owned(),
        AudioError::BackendUnavailable => "音訊輸出裝置目前無法使用。".to_owned(),
        AudioError::NoTrackLoaded => "尚未載入曲目。".to_owned(),
        AudioError::InvalidVolume => "音量設定值無效。".to_owned(),
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FeatureCapability {
    state: FeatureState,
    detail: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
enum FeatureState {
    Ready,
    NotReady,
    Unavailable,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeCapabilities {
    platform: String,
    desktop_runtime: FeatureCapability,
    library: FeatureCapability,
    source_sync: FeatureCapability,
    playback: FeatureCapability,
    playback_navigation: FeatureCapability,
    playback_modes: FeatureCapability,
    playlist_exchange: FeatureCapability,
    system_media_controls: FeatureCapability,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LibrarySource {
    id: String,
    kind: String,
    display_name: String,
    enabled: bool,
    sync_state: Option<String>,
    last_attempt_utc_ms: Option<i64>,
    last_success_utc_ms: Option<i64>,
    error_count: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaStoreVolumeOption {
    volume_name: String,
    display_name: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceSyncResult {
    source_id: String,
    display_name: String,
    state: String,
    observed: u64,
    metadata_reads: u64,
    unchanged: u64,
    added_or_updated: u64,
    removed_mappings: u64,
    error_count: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LibrarySyncResult {
    sources: Vec<SourceSyncResult>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum RepeatMode {
    Off,
    One,
    All,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaybackSnapshot {
    current_track: Option<TrackSummary>,
    state: String,
    is_playing: bool,
    position_ms: u64,
    duration_ms: Option<u64>,
    volume: f64,
    last_error: Option<String>,
    repeat_mode: RepeatMode,
    shuffle: bool,
}

fn feature(state: FeatureState, detail: Option<String>) -> FeatureCapability {
    FeatureCapability { state, detail }
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "android") {
        "android"
    } else {
        "unsupported"
    }
}

#[tauri::command]
fn get_runtime_capabilities(state: State<'_, AppState>) -> RuntimeCapabilities {
    let (library, source_sync) = if state.database.is_some() {
        let source_capability = if cfg!(any(target_os = "windows", target_os = "android")) {
            feature(FeatureState::Ready, None)
        } else {
            feature(
                FeatureState::NotReady,
                Some("目前平台尚未提供媒體來源同步。".to_owned()),
            )
        };
        (feature(FeatureState::Ready, None), source_capability)
    } else {
        let detail = state
            .database_error
            .clone()
            .or_else(|| Some("曲庫資料庫尚未開啟。".to_owned()));
        (
            feature(FeatureState::Unavailable, detail.clone()),
            feature(FeatureState::Unavailable, detail),
        )
    };

    let playback = {
        #[cfg(target_os = "windows")]
        {
            if state.database.is_some() && state.playback.is_some() {
                feature(FeatureState::Ready, None)
            } else {
                feature(
                    FeatureState::NotReady,
                    state.database_error.clone().or_else(|| {
                        state
                            .playback_error
                            .clone()
                            .or_else(|| Some("Windows 音訊服務尚未啟動。".to_owned()))
                    }),
                )
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            feature(
                FeatureState::NotReady,
                Some("此平台尚未提供本機音訊播放服務。".to_owned()),
            )
        }
    };

    let playlist_exchange = if cfg!(target_os = "windows") {
        if state.database.is_some() {
            feature(FeatureState::Ready, None)
        } else {
            feature(
                FeatureState::Unavailable,
                state
                    .database_error
                    .clone()
                    .or_else(|| Some("曲庫資料庫尚未開啟。".to_owned())),
            )
        }
    } else {
        feature(
            FeatureState::NotReady,
            Some("M3U/M3U8 原生檔案匯入與匯出目前只支援 Windows。".to_owned()),
        )
    };

    let system_media_controls = feature(
        FeatureState::NotReady,
        Some("Windows 系統媒體控制尚未接入 Tauri 外殼。".to_owned()),
    );

    RuntimeCapabilities {
        platform: platform_name().to_owned(),
        desktop_runtime: feature(FeatureState::Ready, None),
        library,
        source_sync,
        playback,
        playback_navigation: feature(
            FeatureState::NotReady,
            Some("播放佇列與前後首尚未實作。".to_owned()),
        ),
        playback_modes: feature(
            FeatureState::NotReady,
            Some("隨機與循環播放尚未實作。".to_owned()),
        ),
        playlist_exchange,
        system_media_controls,
    }
}

#[tauri::command]
fn library_get_page(
    state: State<'_, AppState>,
    query: Option<String>,
    offset: u64,
    limit: u32,
) -> Result<Page<TrackSummary>, String> {
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;

    database
        .list_tracks_page(ListTracksQuery {
            query,
            offset,
            limit,
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn playlist_list(state: State<'_, AppState>) -> Result<Vec<PlaylistSummary>, String> {
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    database.list_playlists().map_err(|error| error.to_string())
}

#[tauri::command]
fn playlist_get_page(
    state: State<'_, AppState>,
    playlist_id: String,
    offset: u64,
    limit: u32,
) -> Result<PlaylistPage, String> {
    let playlist_id = PlaylistId::parse(&playlist_id)
        .map_err(|_| "播放清單識別碼無效，請重新載入清單。".to_owned())?;
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    database
        .get_playlist_page(playlist_id, offset, limit)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "找不到這份播放清單，請重新載入清單。".to_owned())
}

#[tauri::command]
async fn playlist_import_m3u(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<PlaylistImportResult>, String> {
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_dialog::DialogExt;

        let selected = app
            .dialog()
            .file()
            .set_title("匯入播放清單")
            .add_filter("M3U / M3U8 播放清單", &["m3u", "m3u8"])
            .blocking_pick_file();
        let Some(selected) = selected else {
            return Ok(None);
        };
        let path = selected
            .into_path()
            .map_err(|error| format!("無法取得選取的本機播放清單路徑：{error}"))?;
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        Ok(Some(import_playlist_file(database, &path)?))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, state);
        Err("M3U/M3U8 原生檔案匯入目前只支援 Windows。".to_owned())
    }
}

#[tauri::command]
async fn playlist_export_m3u(
    app: AppHandle,
    state: State<'_, AppState>,
    playlist_id: String,
    format: String,
    relative_paths: bool,
) -> Result<Option<PlaylistExportResult>, String> {
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_dialog::DialogExt;

        let default_file_name = match format.as_str() {
            "m3u" => "playlist.m3u",
            "m3u8" => "playlist.m3u8",
            _ => return Err("匯出格式必須是 M3U 或 M3U8。".to_owned()),
        };

        let relative_root = if relative_paths {
            let selected = app
                .dialog()
                .file()
                .set_title("選擇播放清單與音樂共用的資料夾")
                .blocking_pick_folder();
            let Some(selected) = selected else {
                return Ok(None);
            };
            Some(
                selected
                    .into_path()
                    .map_err(|error| format!("無法取得選取的共同資料夾：{error}"))?,
            )
        } else {
            None
        };

        let mut dialog = app
            .dialog()
            .file()
            .set_title("匯出播放清單")
            .set_file_name(default_file_name)
            .add_filter("M3U / M3U8 播放清單", &["m3u", "m3u8"]);
        if let Some(root) = relative_root.as_deref() {
            dialog = dialog.set_directory(root);
        }
        let selected = dialog.blocking_save_file();
        let Some(selected) = selected else {
            return Ok(None);
        };
        let path = selected
            .into_path()
            .map_err(|error| format!("無法取得匯出檔案路徑：{error}"))?;
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        Ok(Some(export_playlist_file(
            database,
            &playlist_id,
            &path,
            &format,
            relative_root.as_deref(),
        )?))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, state, playlist_id, format, relative_paths);
        Err("M3U/M3U8 原生檔案匯出目前只支援 Windows。".to_owned())
    }
}

#[tauri::command]
fn library_list_sources(state: State<'_, AppState>) -> Result<Vec<LibrarySource>, String> {
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    database
        .library_roots()
        .map_err(|error| error.to_string())?
        .iter()
        .map(|root| source_summary(database, root))
        .collect()
}

#[tauri::command]
fn library_add_windows_folder(
    state: State<'_, AppState>,
    path: String,
) -> Result<LibrarySource, String> {
    #[cfg(target_os = "windows")]
    {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        add_windows_folder_to_database(database, &path)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, path);
        Err("Windows 資料夾來源只適用於 Windows。".to_owned())
    }
}

#[cfg(target_os = "windows")]
fn add_windows_folder_to_database(
    database: &Database,
    path: &str,
) -> Result<LibrarySource, String> {
    let requested_path = std::path::PathBuf::from(path.trim());
    if requested_path.as_os_str().is_empty() {
        return Err("請輸入音樂資料夾路徑。".to_owned());
    }
    let canonical_path = std::fs::canonicalize(&requested_path)
        .map_err(|error| format!("無法開啟指定資料夾：{error}"))?;
    if !canonical_path.is_dir() {
        return Err("指定路徑不是資料夾。".to_owned());
    }

    let target_key = windows_locator_key(&canonical_path);
    let existing = database
        .library_roots()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|root| {
            matches!(
                root.kind,
                MediaSourceKind::WindowsFilesystem | MediaSourceKind::WindowsSystemIndex
            ) && matches!(&root.locator, MediaLocator::FileSystem(existing) if windows_locator_key(existing) == target_key)
        });
    let root = match existing {
        Some(root) => root,
        None => {
            let display_name = canonical_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "本機音樂資料夾".to_owned());
            database
                .add_library_root(
                    MediaSourceKind::WindowsSystemIndex,
                    display_name,
                    MediaLocator::FileSystem(canonical_path),
                )
                .map_err(|error| error.to_string())?
        }
    };
    source_summary(database, &root)
}

#[tauri::command]
fn android_media_request_permission(app: AppHandle) -> Result<bool, String> {
    app.media_index()
        .request_media_store_permission()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn android_media_list_volumes(app: AppHandle) -> Result<Vec<MediaStoreVolumeOption>, String> {
    app.media_index()
        .list_media_store_volumes()
        .map(|volumes| {
            volumes
                .volumes
                .into_iter()
                .map(|volume| MediaStoreVolumeOption {
                    display_name: media_volume_display_name(&volume.volume_name),
                    volume_name: volume.volume_name,
                })
                .collect()
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn android_media_add_volume(
    state: State<'_, AppState>,
    app: AppHandle,
    volume_name: String,
) -> Result<LibrarySource, String> {
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    let volume_name = volume_name.trim();
    if volume_name.is_empty() {
        return Err("MediaStore 儲存空間名稱不可為空。".to_owned());
    }

    let volume_exists = app
        .media_index()
        .list_media_store_volumes()
        .map_err(|error| error.to_string())?
        .volumes
        .iter()
        .any(|volume| volume.volume_name == volume_name);
    if !volume_exists {
        return Err("這個 MediaStore 儲存空間已不存在，請重新載入清單。".to_owned());
    }

    let locator = MediaLocator::ContentUri(format!("content://media/{volume_name}/audio/media"));
    let root = add_or_get_root(
        database,
        MediaSourceKind::AndroidMediaStore,
        media_volume_display_name(volume_name),
        locator,
    )?;
    source_summary(database, &root)
}

#[tauri::command]
fn android_saf_pick_source(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Option<LibrarySource>, String> {
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    let Some(uri) = app
        .media_index()
        .pick_saf_tree()
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    if !app
        .media_index()
        .has_saf_permission(&uri)
        .map_err(|error| error.to_string())?
    {
        return Err("Android 未保留這個資料夾的讀取授權。".to_owned());
    }

    let root = add_or_get_root(
        database,
        MediaSourceKind::AndroidSaf,
        "Android 文件資料夾".to_owned(),
        MediaLocator::ContentUri(uri),
    )?;
    source_summary(database, &root).map(Some)
}

fn add_or_get_root(
    database: &Database,
    kind: MediaSourceKind,
    display_name: String,
    locator: MediaLocator,
) -> Result<LibraryRoot, String> {
    if let Some(existing) = database
        .library_roots()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|root| root.kind == kind && root.locator == locator)
    {
        return Ok(existing);
    }
    database
        .add_library_root(kind, display_name, locator)
        .map_err(|error| error.to_string())
}

fn source_summary(database: &Database, root: &LibraryRoot) -> Result<LibrarySource, String> {
    let sync = database
        .sync_state(root.id)
        .map_err(|error| error.to_string())?;
    Ok(LibrarySource {
        id: root.id.to_string(),
        kind: source_kind_name(root.kind).to_owned(),
        display_name: root.display_name.clone(),
        enabled: root.enabled,
        sync_state: sync.as_ref().map(|state| state.state.clone()),
        last_attempt_utc_ms: sync.as_ref().and_then(|state| state.last_attempt_utc_ms),
        last_success_utc_ms: sync.as_ref().and_then(|state| state.last_success_utc_ms),
        error_count: sync.map_or(0, |state| state.error_count),
    })
}

fn source_kind_name(kind: MediaSourceKind) -> &'static str {
    match kind {
        MediaSourceKind::WindowsSystemIndex => "windowsSystemIndex",
        MediaSourceKind::WindowsFilesystem => "windowsFilesystem",
        MediaSourceKind::AndroidMediaStore => "androidMediaStore",
        MediaSourceKind::AndroidSaf => "androidSaf",
        MediaSourceKind::Other => "other",
    }
}

fn media_volume_display_name(volume_name: &str) -> String {
    match volume_name {
        "external_primary" => "共用儲存空間".to_owned(),
        "external" => "外部儲存空間".to_owned(),
        _ => format!("儲存空間 {volume_name}"),
    }
}

#[tauri::command]
async fn library_sync(app: AppHandle) -> Result<LibrarySyncResult, String> {
    let state = app.state::<AppState>();
    let database_path = state.database_path.clone().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    let sync_app = app.clone();
    let event_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut database = Database::open(database_path).map_err(|error| error.to_string())?;
        sync_configured_sources(&sync_app, &mut database)
    })
    .await
    .map_err(|error| format!("曲庫同步工作失敗：{error}"))??;
    let _ = event_app.emit(LIBRARY_SYNC_FINISHED_EVENT, ());
    Ok(result)
}

fn sync_configured_sources(
    app: &AppHandle,
    database: &mut Database,
) -> Result<LibrarySyncResult, String> {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        sync_windows_sources(database)
    }

    #[cfg(target_os = "android")]
    {
        let roots = database
            .library_roots()
            .map_err(|error| error.to_string())?;
        let synced_at_utc_ms = now_utc_epoch_ms()?;
        let mut sources = Vec::new();
        app.media_index()
            .with_adapter(|index| -> Result<(), String> {
                for root in roots.iter().filter(|root| {
                    root.enabled
                        && matches!(
                            root.kind,
                            MediaSourceKind::AndroidMediaStore | MediaSourceKind::AndroidSaf
                        )
                }) {
                    let report = SyncEngine::sync(root, index, database, synced_at_utc_ms)
                        .map_err(|error| error.to_string())?;
                    sources.push(sync_summary(root, report));
                }
                Ok(())
            })?;
        return Ok(LibrarySyncResult { sources });
    }

    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (app, database);
        return Err("目前平台尚未提供媒體來源同步。".to_owned());
    }
}

#[cfg(target_os = "windows")]
fn sync_windows_sources(database: &mut Database) -> Result<LibrarySyncResult, String> {
    let roots = database
        .library_roots()
        .map_err(|error| error.to_string())?;
    let synced_at_utc_ms = now_utc_epoch_ms()?;
    let mut sources = Vec::new();
    let mut index = WindowsMediaIndex::new();
    for root in roots.iter().filter(|root| {
        root.enabled
            && matches!(
                root.kind,
                MediaSourceKind::WindowsFilesystem | MediaSourceKind::WindowsSystemIndex
            )
    }) {
        let report = SyncEngine::sync(root, &mut index, database, synced_at_utc_ms)
            .map_err(|error| error.to_string())?;
        sources.push(sync_summary(root, report));
    }
    Ok(LibrarySyncResult { sources })
}

fn sync_summary(root: &LibraryRoot, report: SyncReport) -> SourceSyncResult {
    let state = report
        .state
        .as_ref()
        .map(scan_state_name)
        .unwrap_or("unknown")
        .to_owned();
    SourceSyncResult {
        source_id: root.id.to_string(),
        display_name: root.display_name.clone(),
        state,
        observed: report.observed,
        metadata_reads: report.metadata_reads,
        unchanged: report.unchanged,
        added_or_updated: report.applied.inserted_or_updated,
        removed_mappings: report.applied.removed_source_mappings,
        error_count: (report.metadata_errors.len() + report.source_errors.len()) as u64,
    }
}

fn scan_state_name(state: &SourceScanState) -> &'static str {
    match state {
        SourceScanState::Complete => "complete",
        SourceScanState::Incomplete { .. } => "incomplete",
        SourceScanState::Unavailable { .. } => "unavailable",
        SourceScanState::PermissionRevoked { .. } => "permissionRevoked",
    }
}

fn now_utc_epoch_ms() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("系統時間無法轉換為 UTC epoch：{error}"))?;
    i64::try_from(duration.as_millis()).map_err(|_| "系統時間超出支援範圍。".to_owned())
}

#[tauri::command]
fn playback_get_snapshot(state: State<'_, AppState>) -> Result<PlaybackSnapshot, String> {
    #[cfg(target_os = "windows")]
    {
        Ok(windows_playback_service(&state)?.snapshot())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = state;
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
fn playback_play(state: State<'_, AppState>, track_id: String) -> Result<PlaybackSnapshot, String> {
    #[cfg(target_os = "windows")]
    {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let service = windows_playback_service(&state)?;
        play_track_from_database(database, service, &track_id)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, track_id);
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
fn playback_pause(state: State<'_, AppState>) -> Result<PlaybackSnapshot, String> {
    #[cfg(target_os = "windows")]
    {
        let service = windows_playback_service(&state)?;
        pause_playback(service)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = state;
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
fn playback_next() -> Result<PlaybackSnapshot, String> {
    Err("曲目佇列與下一首播放尚未實作。".to_owned())
}

#[tauri::command]
fn playback_previous() -> Result<PlaybackSnapshot, String> {
    Err("曲目佇列與上一首播放尚未實作。".to_owned())
}

#[tauri::command]
fn playback_seek(state: State<'_, AppState>, position_ms: u64) -> Result<PlaybackSnapshot, String> {
    #[cfg(target_os = "windows")]
    {
        let service = windows_playback_service(&state)?;
        seek_playback(service, position_ms)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, position_ms);
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
fn playback_set_volume(
    state: State<'_, AppState>,
    volume: f64,
) -> Result<PlaybackSnapshot, String> {
    #[cfg(target_os = "windows")]
    {
        let service = windows_playback_service(&state)?;
        set_playback_volume(service, volume)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, volume);
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
fn playback_set_repeat(_mode: RepeatMode) -> Result<PlaybackSnapshot, String> {
    Err("循環播放尚未實作。".to_owned())
}

#[tauri::command]
fn playback_set_shuffle(_enabled: bool) -> Result<PlaybackSnapshot, String> {
    Err("隨機播放尚未實作。".to_owned())
}

#[cfg(target_os = "windows")]
fn windows_playback_service(state: &AppState) -> Result<&WindowsPlaybackService, String> {
    state.playback.as_ref().ok_or_else(|| {
        state
            .playback_error
            .clone()
            .unwrap_or_else(|| "Windows 音訊服務尚未啟動。".to_owned())
    })
}

#[cfg(target_os = "windows")]
fn pause_playback(service: &WindowsPlaybackService) -> Result<PlaybackSnapshot, String> {
    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    service
        .player
        .pause()
        .map_err(|error| playback_audio_error_message(&error))?;
    Ok(service.snapshot())
}

#[cfg(target_os = "windows")]
fn seek_playback(
    service: &WindowsPlaybackService,
    position_ms: u64,
) -> Result<PlaybackSnapshot, String> {
    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    service
        .player
        .seek(Duration::from_millis(position_ms))
        .map_err(|error| playback_audio_error_message(&error))?;
    Ok(service.snapshot())
}

#[cfg(target_os = "windows")]
fn set_playback_volume(
    service: &WindowsPlaybackService,
    volume: f64,
) -> Result<PlaybackSnapshot, String> {
    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    service
        .player
        .set_volume(volume as f32)
        .map_err(|error| playback_audio_error_message(&error))?;
    Ok(service.snapshot())
}

#[cfg(target_os = "windows")]
fn play_track_from_database(
    database: &Database,
    service: &WindowsPlaybackService,
    track_id: &str,
) -> Result<PlaybackSnapshot, String> {
    let track_id = TrackId::parse(track_id).map_err(|error| format!("曲目 ID 無效：{error}"))?;
    let track = database
        .get_track_summary(track_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "曲目已不在曲庫中，請重新整理列表。".to_owned())?;
    let path = database
        .resolve_playable_filesystem_locator(track_id)
        .map_err(|error| error.to_string())?;

    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let current_track_id = service
        .current_track
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .map(|current| current.id);
    let audio_snapshot = service.player.snapshot();
    if current_track_id == Some(track_id)
        && matches!(
            audio_snapshot.state,
            AudioPlaybackState::Ready | AudioPlaybackState::Paused
        )
    {
        service
            .player
            .play()
            .map_err(|error| playback_audio_error_message(&error))?;
    } else {
        service
            .player
            .load(path)
            .map_err(|error| playback_audio_error_message(&error))?;
        service
            .player
            .play()
            .map_err(|error| playback_audio_error_message(&error))?;
    }
    *service
        .current_track
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(track);
    Ok(service.snapshot())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_media_index::init());
    #[cfg(target_os = "windows")]
    let builder = builder.plugin(tauri_plugin_dialog::init());

    builder
        .invoke_handler(tauri::generate_handler![
            get_runtime_capabilities,
            library_get_page,
            playlist_list,
            playlist_get_page,
            playlist_import_m3u,
            playlist_export_m3u,
            library_list_sources,
            library_add_windows_folder,
            android_media_request_permission,
            android_media_list_volumes,
            android_media_add_volume,
            android_saf_pick_source,
            library_sync,
            playback_get_snapshot,
            playback_play,
            playback_pause,
            playback_next,
            playback_previous,
            playback_seek,
            playback_set_volume,
            playback_set_repeat,
            playback_set_shuffle,
        ])
        .setup(|app| {
            let database_path = app
                .path()
                .app_data_dir()
                .map_err(|error| error.to_string())?
                .join("moemusicplayer.sqlite3");
            let database_result = Database::open(&database_path).map_err(|error| error.to_string());
            let (database, stored_path, database_error) = match database_result {
                Ok(database) => (Some(database), Some(database_path), None),
                Err(error) => (None, None, Some(error)),
            };
            #[cfg(target_os = "windows")]
            let (playback, playback_error) = match PlayerHandle::new() {
                Ok(player) => (Some(WindowsPlaybackService::new(player)), None),
                Err(error) => (None, Some(playback_audio_error_message(&error))),
            };
            app.manage(AppState {
                database,
                database_path: stored_path.clone(),
                database_error,
                #[cfg(target_os = "windows")]
                playback,
                #[cfg(target_os = "windows")]
                playback_error,
            });

            if let Some(database_path) = stored_path {
                let app_handle = app.handle().clone();
                let event_handle = app_handle.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let result = Database::open(database_path)
                        .map_err(|error| error.to_string())
                        .and_then(|mut database| {
                            sync_configured_sources(&app_handle, &mut database)
                        });
                    if let Err(error) = result {
                        eprintln!("startup library sync failed: {error}");
                    }
                    let _ = event_handle.emit(LIBRARY_SYNC_FINISHED_EVENT, ());
                });
            }

            if let Some(window) = app.get_webview_window("main") {
                window.set_title("MoeMusicPlayer")?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("MoeMusicPlayer could not start");
}

#[cfg(all(test, target_os = "windows"))]
mod windows_library_integration_tests {
    use super::{add_windows_folder_to_database, sync_windows_sources};
    use super::{
        pause_playback, play_track_from_database, playback_audio_error_message, seek_playback,
        set_playback_volume, WindowsPlaybackService,
    };
    use player_audio_windows::{AudioBackend, AudioError, PlayerHandle};
    use player_core::{ListTracksQuery, MediaLocator, MediaSourceKind, SourceId};
    use player_db::Database;
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("moemusicplayer-e2e-{}", SourceId::new())))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct CapturingAudioBackend {
        loaded_path: Arc<Mutex<Option<PathBuf>>>,
        position: Duration,
    }

    impl AudioBackend for CapturingAudioBackend {
        fn load(&mut self, path: &Path) -> Result<Option<Duration>, AudioError> {
            *self
                .loaded_path
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(path.to_path_buf());
            self.position = Duration::ZERO;
            Ok(Some(Duration::from_secs(3)))
        }

        fn play(&mut self) -> Result<(), AudioError> {
            Ok(())
        }

        fn pause(&mut self) -> Result<(), AudioError> {
            Ok(())
        }

        fn stop(&mut self) -> Result<(), AudioError> {
            self.position = Duration::ZERO;
            Ok(())
        }

        fn seek(&mut self, position: Duration) -> Result<Duration, AudioError> {
            self.position = position.min(Duration::from_secs(3));
            Ok(self.position)
        }

        fn set_volume(&mut self, _volume: f32) -> Result<(), AudioError> {
            Ok(())
        }

        fn position(&self) -> Duration {
            self.position
        }

        fn is_empty(&self) -> bool {
            false
        }
    }

    fn write_tagged_mp3(path: &std::path::Path) {
        fn add_text_frame(body: &mut Vec<u8>, name: &[u8; 4], value: &str) {
            let mut contents = vec![3]; // UTF-8 text encoding in ID3v2.4.
            contents.extend(value.as_bytes());
            contents.push(0);
            body.extend(name);
            let size = contents.len() as u32;
            body.extend([
                ((size >> 21) & 0x7f) as u8,
                ((size >> 14) & 0x7f) as u8,
                ((size >> 7) & 0x7f) as u8,
                (size & 0x7f) as u8,
                0,
                0,
            ]);
            body.extend(contents);
        }

        let mut body = Vec::new();
        add_text_frame(&mut body, b"TIT2", "夜色 – Sample Track");
        add_text_frame(&mut body, b"TPE1", "Acceptance Artist");
        add_text_frame(&mut body, b"TALB", "Acceptance Album");
        let size = body.len() as u32;
        let mut bytes = vec![
            b'I',
            b'D',
            b'3',
            4,
            0,
            0,
            ((size >> 21) & 0x7f) as u8,
            ((size >> 14) & 0x7f) as u8,
            ((size >> 7) & 0x7f) as u8,
            (size & 0x7f) as u8,
        ];
        bytes.extend(body);
        // Three locally generated MPEG-1 Layer III frames keep this a valid MP3 without
        // embedding audio copied from a third party.
        for _ in 0..3 {
            bytes.extend([0xff, 0xfb, 0x90, 0x64]);
            bytes.resize(bytes.len() + 413, 0);
        }
        fs::write(path, bytes).expect("write synthetic MP3 fixture");
    }

    #[test]
    fn adding_syncing_paging_and_reopening_windows_library_keeps_unicode_data() {
        let temporary = TestDirectory::new();
        let source_path = temporary.0.join("東京🌸 音樂");
        fs::create_dir_all(&source_path).expect("create Unicode source folder");
        write_tagged_mp3(&source_path.join("夜色 - Sample Track.mp3"));
        let database_path = temporary.0.join("app-data").join("library.sqlite3");

        let first_track_id;
        {
            let mut database = Database::open(&database_path).expect("open app database");
            let source = add_windows_folder_to_database(
                &database,
                source_path.to_str().expect("Unicode Windows path"),
            )
            .expect("add Windows folder using the Tauri command helper");
            assert_eq!(source.kind, "windowsSystemIndex");

            let duplicate = add_windows_folder_to_database(
                &database,
                source_path.to_str().expect("Unicode Windows path"),
            )
            .expect("adding the same folder is idempotent");
            assert_eq!(duplicate.id, source.id);
            assert_eq!(database.library_roots().expect("list roots").len(), 1);

            let sync = sync_windows_sources(&mut database).expect("run Windows sync command path");
            assert_eq!(sync.sources.len(), 1);
            let report = &sync.sources[0];
            assert_eq!(report.source_id, source.id);
            assert_eq!(report.state, "complete");
            assert_eq!(report.observed, 1);
            assert_eq!(report.metadata_reads, 1);
            assert_eq!(report.added_or_updated, 1);

            let page = database
                .list_tracks_page(ListTracksQuery {
                    query: None,
                    offset: 0,
                    limit: 25,
                })
                .expect("load first library page");
            assert_eq!(page.total_count, 1);
            assert_eq!(page.items.len(), 1);
            assert_eq!(page.items[0].title.as_deref(), Some("夜色 – Sample Track"));
            assert_eq!(page.items[0].artist.as_deref(), Some("Acceptance Artist"));
            assert_eq!(page.items[0].album.as_deref(), Some("Acceptance Album"));
            first_track_id = page.items[0].id;

            let filtered = database
                .list_tracks_page(ListTracksQuery {
                    query: Some("夜色".to_owned()),
                    offset: 0,
                    limit: 25,
                })
                .expect("search only the requested page");
            assert_eq!(filtered.total_count, 1);
            assert_eq!(filtered.items[0].id, first_track_id);

            let sync_state = database
                .sync_state(SourceId::parse(&source.id).expect("parse source id"))
                .expect("read source sync state")
                .expect("sync state persisted");
            assert_eq!(sync_state.state, "complete");
        }

        let reopened = Database::open(&database_path).expect("reopen app database");
        let roots = reopened.library_roots().expect("read persisted roots");
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].kind, MediaSourceKind::WindowsSystemIndex);
        let MediaLocator::FileSystem(stored_path) = &roots[0].locator else {
            panic!("Windows source locator must remain a filesystem path");
        };
        assert_eq!(
            player_platform_windows::windows_locator_key(stored_path),
            player_platform_windows::windows_locator_key(&fs::canonicalize(source_path).unwrap())
        );

        let reopened_page = reopened
            .list_tracks_page(ListTracksQuery {
                query: None,
                offset: 0,
                limit: 25,
            })
            .expect("read persisted library page after reopen");
        assert_eq!(reopened_page.total_count, 1);
        assert_eq!(reopened_page.items[0].id, first_track_id);
        assert_eq!(
            reopened_page.items[0].title.as_deref(),
            Some("夜色 – Sample Track")
        );
        assert_eq!(
            reopened_page.items[0].artist.as_deref(),
            Some("Acceptance Artist")
        );
        assert_eq!(
            reopened_page.items[0].album.as_deref(),
            Some("Acceptance Album")
        );
    }

    #[test]
    fn playback_resolves_internal_track_id_to_saved_path_and_loads_player_handle() {
        let temporary = TestDirectory::new();
        let source_path = temporary.0.join("東京🌸 音樂");
        fs::create_dir_all(&source_path).expect("create Unicode source folder");
        let track_path = source_path.join("夜色 - Sample Track.mp3");
        write_tagged_mp3(&track_path);
        let database_path = temporary.0.join("app-data").join("library.sqlite3");

        let mut database = Database::open(&database_path).expect("open app database");
        let source = add_windows_folder_to_database(
            &database,
            source_path.to_str().expect("Unicode Windows path"),
        )
        .expect("add Windows folder through command helper");
        assert_eq!(source.kind, "windowsSystemIndex");
        let sync = sync_windows_sources(&mut database).expect("sync source before playback");
        assert_eq!(sync.sources[0].state, "complete");
        let page = database
            .list_tracks_page(ListTracksQuery {
                query: None,
                offset: 0,
                limit: 10,
            })
            .expect("read track page");
        assert_eq!(page.total_count, 1);
        let track_id = page.items[0].id;

        let loaded_path = Arc::new(Mutex::new(None));
        let backend_path = Arc::clone(&loaded_path);
        let player = PlayerHandle::with_backend_factory(move || {
            Ok(Box::new(CapturingAudioBackend {
                loaded_path: backend_path,
                position: Duration::ZERO,
            }) as Box<dyn AudioBackend>)
        })
        .expect("create test audio worker");
        let service = WindowsPlaybackService::new(player);
        let response = play_track_from_database(&database, &service, &track_id.to_string())
            .expect("resolve internal ID and enqueue playback");
        assert_eq!(
            response.current_track.as_ref().map(|track| track.id),
            Some(track_id)
        );

        let path_deadline = Instant::now() + Duration::from_secs(2);
        let actual_path = loop {
            if let Some(path) = loaded_path
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
            {
                break path;
            }
            assert!(
                Instant::now() < path_deadline,
                "player worker did not receive a file path"
            );
            thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(
            player_platform_windows::windows_locator_key(&actual_path),
            player_platform_windows::windows_locator_key(&fs::canonicalize(track_path).unwrap())
        );

        let playing_deadline = Instant::now() + Duration::from_secs(2);
        while !service.snapshot().is_playing {
            assert!(
                Instant::now() < playing_deadline,
                "player worker did not enter Playing state"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(service.snapshot().state, "playing");
        assert!(service.snapshot().last_error.is_none());

        pause_playback(&service).expect("queue pause command");
        let paused_deadline = Instant::now() + Duration::from_secs(2);
        while service.snapshot().state != "paused" {
            assert!(
                Instant::now() < paused_deadline,
                "player worker did not enter Paused state"
            );
            thread::sleep(Duration::from_millis(5));
        }

        seek_playback(&service, 1_250).expect("queue seek command");
        let seek_deadline = Instant::now() + Duration::from_secs(2);
        while service.snapshot().position_ms != 1_250 {
            assert!(
                Instant::now() < seek_deadline,
                "player worker did not apply seek position"
            );
            thread::sleep(Duration::from_millis(5));
        }

        set_playback_volume(&service, 0.35).expect("queue volume command");
        let volume_deadline = Instant::now() + Duration::from_secs(2);
        while (service.snapshot().volume - 0.35).abs() > 0.000_01 {
            assert!(
                Instant::now() < volume_deadline,
                "player worker did not apply volume"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn playback_error_text_does_not_include_native_file_path() {
        let private_path = PathBuf::from(r"C:\Users\Private\Music\secret-track.mp3");
        let error = AudioError::FileOpen {
            path: private_path.clone(),
            message: "access denied".to_owned(),
        };
        let message = playback_audio_error_message(&error);
        assert!(!message.contains(private_path.to_string_lossy().as_ref()));
    }
}
