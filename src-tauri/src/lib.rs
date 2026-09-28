use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "windows")]
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use player_core::{
    LibraryRoot, ListTracksQuery, MediaLocator, MediaSourceKind, Page, PlaybackQueue, PlaylistId,
    PlaylistPage, PlaylistSummary, QueueRepeatMode, SourceId, SourceScanState, SyncEngine,
    SyncProgress, SyncReport, TrackId, TrackSummary,
};
use player_db::{Database, ThemePreferences};
use serde::{Deserialize, Serialize};
use tauri::{ipc::Response, AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_media_index::MediaIndexExt;

mod playlist_exchange;
use playlist_exchange::{
    export_playlist_file, import_playlist_file_with_id, read_playlist_file, PlaylistExportResult,
    PlaylistImportResult,
};
mod settings;
use settings::{
    AppSettings, NowPlayingLayout, RepeatMode, SettingsStore, SourceEntry, SourceEntryKind,
    ThemeSettings, TrackListColumnSettings,
};
mod playlist_source_sync;

#[cfg(target_os = "windows")]
use player_platform_windows::{
    find_artwork, windows_locator_key, ArtworkLookup, WindowsMediaIndex,
};

#[cfg(target_os = "windows")]
use player_audio_windows::{
    system_media::{
        MediaControlMetadata, MediaControlUpdate, SystemMediaController, SystemMediaError,
        SystemMediaEvent,
    },
    AudioError, CommandTicket, PlaybackState as AudioPlaybackState, PlayerHandle,
};

const LIBRARY_SYNC_FINISHED_EVENT: &str = "library-sync-finished";
const LIBRARY_SYNC_PROGRESS_EVENT: &str = "library-sync-progress";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThemePreferencesDto {
    background_hex: String,
    accent_hex: String,
}

impl From<ThemePreferences> for ThemePreferencesDto {
    fn from(preferences: ThemePreferences) -> Self {
        Self {
            background_hex: preferences.background_hex,
            accent_hex: preferences.accent_hex,
        }
    }
}

impl From<ThemePreferencesDto> for ThemePreferences {
    fn from(preferences: ThemePreferencesDto) -> Self {
        Self {
            background_hex: preferences.background_hex,
            accent_hex: preferences.accent_hex,
        }
    }
}

impl From<ThemeSettings> for ThemePreferencesDto {
    fn from(preferences: ThemeSettings) -> Self {
        Self {
            background_hex: preferences.background_hex,
            accent_hex: preferences.accent_hex,
        }
    }
}

struct AppState {
    database: Option<Database>,
    database_path: Option<std::path::PathBuf>,
    database_error: Option<String>,
    settings: SettingsStore,
    #[cfg(target_os = "windows")]
    playback: Option<WindowsPlaybackService>,
    #[cfg(target_os = "windows")]
    playback_error: Option<String>,
    #[cfg(target_os = "windows")]
    system_media: Option<WindowsSystemMediaService>,
    #[cfg(target_os = "windows")]
    system_media_error: Option<String>,
}

#[cfg(target_os = "windows")]
struct WindowsPlaybackService {
    player: PlayerHandle,
    current_track: Mutex<Option<TrackSummary>>,
    command_gate: Mutex<()>,
    queue: Mutex<Option<PlaybackQueue>>,
    repeat_mode: Mutex<RepeatMode>,
    shuffle: Mutex<bool>,
    queue_advance_pending: Mutex<bool>,
}

#[cfg(target_os = "windows")]
struct WindowsSystemMediaService {
    controller: Mutex<Option<SystemMediaController>>,
    status: Mutex<WindowsSystemMediaStatus>,
    pump_gate: Mutex<()>,
    last_update: Mutex<Option<Instant>>,
}

#[cfg(target_os = "windows")]
#[derive(Clone)]
enum WindowsSystemMediaStatus {
    Starting,
    Ready,
    Unavailable(String),
    Closed,
}

#[cfg(target_os = "windows")]
impl WindowsPlaybackService {
    #[cfg(test)]
    fn new(player: PlayerHandle) -> Self {
        Self::with_modes(player, false, RepeatMode::Off)
    }

    fn with_modes(player: PlayerHandle, shuffle: bool, repeat_mode: RepeatMode) -> Self {
        Self {
            player,
            current_track: Mutex::new(None),
            command_gate: Mutex::new(()),
            queue: Mutex::new(None),
            repeat_mode: Mutex::new(repeat_mode),
            shuffle: Mutex::new(shuffle),
            queue_advance_pending: Mutex::new(false),
        }
    }

    fn snapshot(&self) -> PlaybackSnapshot {
        let audio = self.player.snapshot();
        let current_track = self
            .current_track
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let repeat_mode = *self
            .repeat_mode
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let shuffle = *self
            .shuffle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let queue = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let can_next = queue.as_ref().is_some_and(PlaybackQueue::can_next);
        let can_previous = queue.as_ref().is_some_and(PlaybackQueue::can_previous);
        let duration_ms = playback_duration_ms(
            audio.duration,
            current_track.as_ref().and_then(|track| track.duration_ms),
        );
        PlaybackSnapshot {
            current_track,
            state: audio_state_name(audio.state).to_owned(),
            is_playing: audio.state == AudioPlaybackState::Playing,
            position_ms: duration_millis(audio.position),
            duration_ms,
            volume: f64::from(audio.volume),
            last_error: audio.last_error.as_ref().map(playback_audio_error_message),
            repeat_mode,
            shuffle,
            can_next,
            can_previous,
        }
    }
}

#[cfg(target_os = "windows")]
impl WindowsSystemMediaService {
    fn new(controller: SystemMediaController) -> Self {
        Self {
            controller: Mutex::new(Some(controller)),
            status: Mutex::new(WindowsSystemMediaStatus::Starting),
            pump_gate: Mutex::new(()),
            last_update: Mutex::new(None),
        }
    }

    fn capability(&self) -> FeatureCapability {
        match self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            WindowsSystemMediaStatus::Starting => feature(
                FeatureState::NotReady,
                Some("正在連接 Windows 系統媒體控制。".to_owned()),
            ),
            WindowsSystemMediaStatus::Ready => feature(FeatureState::Ready, None),
            WindowsSystemMediaStatus::Unavailable(detail) => {
                feature(FeatureState::Unavailable, Some(detail))
            }
            WindowsSystemMediaStatus::Closed => feature(
                FeatureState::Unavailable,
                Some("主視窗已關閉，系統媒體控制已停止。".to_owned()),
            ),
        }
    }

    fn shutdown(&self) {
        let controller = self
            .controller
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(controller);
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = WindowsSystemMediaStatus::Closed;
    }

    fn pump(&self, playback: &WindowsPlaybackService, database: Option<&Database>) {
        let _pump = self
            .pump_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let controller_guard = self
            .controller
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(controller) = controller_guard.as_ref() else {
            return;
        };

        loop {
            match controller.try_recv_event() {
                Ok(Some(SystemMediaEvent::Ready)) => {
                    *self
                        .status
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        WindowsSystemMediaStatus::Ready;
                }
                Ok(Some(SystemMediaEvent::Error(error))) => {
                    self.mark_unavailable(error);
                    return;
                }
                Ok(Some(event)) => {
                    let _ = apply_system_media_event(&event, database, playback);
                }
                Ok(None) => break,
                Err(error) => {
                    self.mark_unavailable(error);
                    return;
                }
            }
        }

        if matches!(
            &*self
                .status
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            WindowsSystemMediaStatus::Unavailable(_) | WindowsSystemMediaStatus::Closed
        ) {
            return;
        }

        let now = Instant::now();
        let should_update = self
            .last_update
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .map(|last| now.duration_since(last) >= Duration::from_millis(750))
            .unwrap_or(true);
        if !should_update {
            return;
        }

        let current_track = playback
            .current_track
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let metadata = current_track.map(|track| MediaControlMetadata {
            title: track.title,
            artist: track.artist,
            album: track.album,
        });
        let queue = playback
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let capabilities = player_audio_windows::system_media::MediaControlCapabilities {
            can_next: queue.as_ref().is_some_and(PlaybackQueue::can_next),
            can_previous: queue.as_ref().is_some_and(PlaybackQueue::can_previous),
        };
        let update = MediaControlUpdate {
            snapshot: playback.player.snapshot(),
            metadata,
            capabilities,
        };
        match controller.update(update) {
            Ok(()) => {
                *self
                    .last_update
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(now);
            }
            Err(error) => self.handle_update_error(error),
        }
    }

    fn mark_unavailable(&self, error: SystemMediaError) {
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            WindowsSystemMediaStatus::Unavailable(format!("Windows 系統媒體控制無法使用：{error}"));
    }

    fn handle_update_error(&self, error: SystemMediaError) {
        if !matches!(error, SystemMediaError::CommandQueueFull) {
            self.mark_unavailable(error);
        }
    }
}

#[cfg(target_os = "windows")]
fn apply_system_media_event(
    event: &SystemMediaEvent,
    database: Option<&Database>,
    playback: &WindowsPlaybackService,
) -> Result<(), String> {
    match event {
        SystemMediaEvent::PlayRequested => {
            let database = database.ok_or_else(|| "曲庫資料庫尚未開啟。".to_owned())?;
            let track_id = playback
                .current_track
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .map(|track| track.id)
                .ok_or_else(|| "目前沒有可播放的曲目。".to_owned())?;
            let _ = play_track_from_database(database, playback, &track_id.to_string(), None)?;
        }
        SystemMediaEvent::PauseRequested => {
            let _ = pause_playback(playback)?;
        }
        SystemMediaEvent::StopRequested => {
            let _ = stop_playback(playback)?;
        }
        SystemMediaEvent::SeekRequested(position) => {
            let _ = seek_playback(playback, duration_millis(*position))?;
        }
        SystemMediaEvent::NextRequested => {
            if let Some(database) = database {
                if let Some(ticket) = navigate_queue(database, playback, true)? {
                    ticket
                        .wait(Duration::from_secs(2))
                        .map_err(|error| playback_audio_error_message(&error))?;
                }
            }
        }
        SystemMediaEvent::PreviousRequested => {
            if let Some(database) = database {
                if let Some(ticket) = navigate_queue(database, playback, false)? {
                    ticket
                        .wait(Duration::from_secs(2))
                        .map_err(|error| playback_audio_error_message(&error))?;
                }
            }
        }
        SystemMediaEvent::Ready | SystemMediaEvent::Error(_) => {}
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn create_windows_system_media_service(
    app: &AppHandle,
) -> (Option<WindowsSystemMediaService>, Option<String>) {
    let result = app
        .get_webview_window("main")
        .ok_or_else(|| "找不到 Tauri 主視窗，無法連接 Windows 系統媒體控制。".to_owned())
        .and_then(|window| {
            window
                .hwnd()
                .map_err(|error| format!("無法取得 Tauri 主視窗 HWND：{error}"))
        })
        .and_then(|hwnd| {
            SystemMediaController::attach(hwnd.0 as isize)
                .map_err(|error| format!("無法啟動 Windows 系統媒體控制：{error}"))
        });

    windows_system_media_service_from_attach_result(result)
}

#[cfg(target_os = "windows")]
fn windows_system_media_service_from_attach_result(
    result: Result<SystemMediaController, String>,
) -> (Option<WindowsSystemMediaService>, Option<String>) {
    match result {
        Ok(controller) => (Some(WindowsSystemMediaService::new(controller)), None),
        Err(error) => (None, Some(error)),
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

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn playback_duration_ms(
    audio_duration: Option<Duration>,
    track_duration_ms: Option<u64>,
) -> Option<u64> {
    audio_duration
        .map(duration_millis)
        .or(track_duration_ms.filter(|duration| *duration > 0))
}

#[cfg(target_os = "windows")]
fn playback_audio_error_message(error: &AudioError) -> String {
    match error {
        AudioError::OutputDevice(_) => "無法連線到預設音訊輸出裝置。".to_owned(),
        AudioError::WorkerStart(_) => "無法啟動音訊背景工作。".to_owned(),
        AudioError::WorkerStopped => "音訊工作階段已結束。".to_owned(),
        AudioError::CommandQueueFull => "音訊服務忙碌，請稍後再試。".to_owned(),
        AudioError::CommandTimeout => "音訊命令逾時，請查看播放器狀態後重試。".to_owned(),
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
    location: String,
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LibrarySyncProgressEvent {
    run_id: String,
    source_index: usize,
    source_count: usize,
    display_name: String,
    #[serde(flatten)]
    progress: SyncProgress,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LibrarySyncFinishedEvent {
    run_id: String,
    source_count: usize,
    sources: Vec<SourceSyncResult>,
    error: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum PlaybackQueueSource {
    Library {
        query: Option<String>,
    },
    Playlist {
        #[serde(rename = "playlistId")]
        playlist_id: String,
        #[serde(rename = "entryPosition")]
        entry_position: u64,
    },
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
    can_next: bool,
    can_previous: bool,
}

#[cfg(test)]
mod playback_queue_ipc_tests {
    use super::{
        feature, playback_duration_ms, FeatureState, LibrarySource, PlaybackQueueSource,
        PlaybackSnapshot, RepeatMode, RuntimeCapabilities,
    };

    #[test]
    fn playback_snapshot_uses_track_metadata_when_audio_duration_is_unknown() {
        assert_eq!(playback_duration_ms(None, Some(184_000)), Some(184_000));
        assert_eq!(
            playback_duration_ms(
                Some(std::time::Duration::from_millis(183_500)),
                Some(184_000)
            ),
            Some(183_500),
            "a decoder duration remains preferred when it is available",
        );
        assert_eq!(playback_duration_ms(None, Some(0)), None);
        assert_eq!(playback_duration_ms(None, None), None);
    }

    #[test]
    fn queue_source_deserializes_camel_case_ipc_fields() {
        let source: PlaybackQueueSource = serde_json::from_value(serde_json::json!({
            "kind": "playlist",
            "playlistId": "test-playlist-id",
            "entryPosition": 7
        }))
        .expect("deserialize renderer queue source");
        assert!(matches!(
            source,
            PlaybackQueueSource::Playlist {
                playlist_id,
                entry_position: 7
            } if playlist_id == "test-playlist-id"
        ));
    }

    #[test]
    fn queue_source_deserializes_library_context() {
        let source: PlaybackQueueSource = serde_json::from_value(serde_json::json!({
            "kind": "library",
            "query": "ambient"
        }))
        .expect("deserialize library query source");
        assert!(matches!(
            source,
            PlaybackQueueSource::Library { query: Some(query) } if query == "ambient"
        ));
    }

    #[test]
    fn playback_snapshot_and_capabilities_serialize_renderer_camel_case() {
        let snapshot = PlaybackSnapshot {
            current_track: None,
            state: "paused".to_owned(),
            is_playing: false,
            position_ms: 42,
            duration_ms: Some(100),
            volume: 0.5,
            last_error: None,
            repeat_mode: RepeatMode::One,
            shuffle: true,
            can_next: true,
            can_previous: false,
        };
        let value = serde_json::to_value(snapshot).expect("serialize playback snapshot");
        for key in [
            "currentTrack",
            "isPlaying",
            "positionMs",
            "durationMs",
            "repeatMode",
            "shuffle",
            "canNext",
            "canPrevious",
        ] {
            assert!(value.get(key).is_some(), "missing camelCase field {key}");
        }
        assert_eq!(value["repeatMode"], "one");

        let ready = || feature(FeatureState::Ready, None);
        let capabilities = RuntimeCapabilities {
            platform: "windows".to_owned(),
            desktop_runtime: ready(),
            library: ready(),
            source_sync: ready(),
            playback: ready(),
            playback_navigation: ready(),
            playback_modes: ready(),
            playlist_exchange: ready(),
            system_media_controls: ready(),
        };
        let value = serde_json::to_value(capabilities).expect("serialize capabilities");
        for key in ["playbackNavigation", "playbackModes", "systemMediaControls"] {
            assert!(value.get(key).is_some(), "missing camelCase field {key}");
        }
    }

    #[test]
    fn source_summary_serializes_renderer_camel_case_fields() {
        let source = LibrarySource {
            id: "source-id".to_owned(),
            kind: "playlistFile".to_owned(),
            display_name: "測試清單".to_owned(),
            location: r"C:\音樂\清單.m3u8".to_owned(),
            enabled: true,
            sync_state: Some("complete".to_owned()),
            last_attempt_utc_ms: Some(1_800_000_000_001),
            last_success_utc_ms: Some(1_800_000_000_002),
            error_count: 0,
        };
        let value = serde_json::to_value(source).expect("serialize library source summary");
        for key in [
            "displayName",
            "location",
            "syncState",
            "lastAttemptUtcMs",
            "lastSuccessUtcMs",
            "errorCount",
        ] {
            assert!(value.get(key).is_some(), "missing camelCase field {key}");
        }
        assert_eq!(value["location"], r"C:\音樂\清單.m3u8");
    }
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

    let system_media_controls = system_media_controls_capability(&state);

    RuntimeCapabilities {
        platform: platform_name().to_owned(),
        desktop_runtime: feature(FeatureState::Ready, None),
        library,
        source_sync,
        playback: playback.clone(),
        playback_navigation: playback.clone(),
        playback_modes: playback.clone(),
        playlist_exchange,
        system_media_controls,
    }
}

#[cfg(target_os = "windows")]
fn system_media_controls_capability(state: &AppState) -> FeatureCapability {
    state
        .system_media
        .as_ref()
        .map(WindowsSystemMediaService::capability)
        .unwrap_or_else(|| unavailable_system_media_capability(state.system_media_error.clone()))
}

#[cfg(target_os = "windows")]
fn unavailable_system_media_capability(detail: Option<String>) -> FeatureCapability {
    feature(
        FeatureState::Unavailable,
        detail.or_else(|| Some("Windows 系統媒體控制尚未啟動。".to_owned())),
    )
}

#[cfg(not(target_os = "windows"))]
fn system_media_controls_capability(_state: &AppState) -> FeatureCapability {
    feature(
        FeatureState::NotReady,
        Some("目前只有 Windows 提供系統媒體控制。".to_owned()),
    )
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
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<PlaylistImportResult>, String> {
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_dialog::DialogExt;

        let selected = app
            .dialog()
            .file()
            .set_parent(&window)
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
        let settings = state
            .settings
            .snapshot()
            .map_err(|error| error.to_string())?;
        let (source_id, registered_playlist_id, path) =
            playlist_source_sync::source_ids_for_path(&settings.sources, &path)?;
        let parsed = read_playlist_file(&path)?;
        let playlist_id = match registered_playlist_id {
            Some(playlist_id) => playlist_id,
            None => database
                .unique_playlist_id_matching_locators(
                    &parsed.name,
                    &parsed
                        .entries
                        .iter()
                        .map(|entry| entry.locator.clone())
                        .collect::<Vec<_>>(),
                )
                .map_err(|error| error.to_string())?
                .unwrap_or(parsed.id),
        };
        let mut imported = import_playlist_file_with_id(database, &path, playlist_id)?;
        state
            .settings
            .upsert_source(SourceEntry {
                id: source_id,
                display_name: imported.playlist.name.clone(),
                enabled: true,
                kind: SourceEntryKind::PlaylistFile {
                    playlist_id,
                    path: settings::StoredPath::from_path(&path)
                        .map_err(|error| error.to_string())?,
                },
            })
            .map_err(|error| error.to_string())?;
        let root = LibraryRoot {
            id: source_id,
            kind: MediaSourceKind::PlaylistFile,
            display_name: imported.playlist.name.clone(),
            locator: MediaLocator::FileSystem(path),
            enabled: true,
        };
        database
            .save_library_root(&root)
            .map_err(|error| error.to_string())?;
        let _ = library_sync(app.clone()).await;
        if let Some(database) = app.state::<AppState>().database.as_ref() {
            if let Some(saved) = database
                .get_playlist(playlist_id)
                .map_err(|error| error.to_string())?
            {
                imported.matched_entries = saved
                    .entries
                    .iter()
                    .filter(|entry| entry.track_id.is_some())
                    .count() as u64;
            }
        }
        Ok(Some(imported))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, window, state);
        Err("M3U/M3U8 原生檔案匯入目前只支援 Windows。".to_owned())
    }
}

#[tauri::command]
async fn playlist_export_m3u(
    app: AppHandle,
    window: WebviewWindow,
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
                .set_parent(&window)
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
            .set_parent(&window)
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
        let _ = (app, window, state, playlist_id, format, relative_paths);
        Err("M3U/M3U8 原生檔案匯出目前只支援 Windows。".to_owned())
    }
}

#[tauri::command]
fn library_list_sources(state: State<'_, AppState>) -> Result<Vec<LibrarySource>, String> {
    let settings = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?;
    settings
        .sources
        .iter()
        .map(|source| source_entry_summary(state.database.as_ref(), source))
        .collect()
}

#[tauri::command]
fn settings_get_recovery_warning(state: State<'_, AppState>) -> Option<String> {
    state.settings.recovery_warning()
}

#[tauri::command]
fn settings_source_registry_authoritative(state: State<'_, AppState>) -> Result<bool, String> {
    state
        .settings
        .source_registry_authoritative()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_confirm_source_registry(state: State<'_, AppState>) -> Result<bool, String> {
    state
        .settings
        .confirm_source_registry()
        .map(|settings| settings.source_registry_authoritative)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn theme_get_preferences(state: State<'_, AppState>) -> Result<ThemePreferencesDto, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.theme.into())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn theme_set_preferences(
    state: State<'_, AppState>,
    preferences: ThemePreferencesDto,
) -> Result<ThemePreferencesDto, String> {
    let preferences = ThemeSettings {
        background_hex: preferences.background_hex,
        accent_hex: preferences.accent_hex,
    };
    state
        .settings
        .update(|settings| {
            settings.theme = preferences.clone();
            Ok(())
        })
        .map(|settings| settings.theme.into())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_get_track_list_columns(
    state: State<'_, AppState>,
) -> Result<TrackListColumnSettings, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.track_list_columns)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_set_track_list_columns(
    state: State<'_, AppState>,
    preferences: TrackListColumnSettings,
) -> Result<TrackListColumnSettings, String> {
    state
        .settings
        .update(|settings| {
            settings.track_list_columns = preferences.clone();
            Ok(())
        })
        .map(|settings| settings.track_list_columns)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_get_now_playing_layout(state: State<'_, AppState>) -> Result<NowPlayingLayout, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.now_playing_layout)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_set_now_playing_layout(
    state: State<'_, AppState>,
    layout: NowPlayingLayout,
) -> Result<NowPlayingLayout, String> {
    state
        .settings
        .update(|settings| {
            settings.now_playing_layout = layout;
            Ok(())
        })
        .map(|settings| settings.now_playing_layout)
        .map_err(|error| error.to_string())
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
        add_windows_folder_to_settings(database, &state.settings, &path)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, path);
        Err("Windows 資料夾來源只適用於 Windows。".to_owned())
    }
}

#[tauri::command]
async fn library_pick_windows_folder(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<LibrarySource>, String> {
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_dialog::DialogExt;

        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let selected = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("選擇音樂來源資料夾")
            .blocking_pick_folder();
        let Some(selected) = selected else {
            return Ok(None);
        };
        let path = selected
            .into_path()
            .map_err(|error| format!("無法取得選取的資料夾路徑：{error}"))?;
        add_windows_folder_path_to_settings(database, &state.settings, &path).map(Some)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, window, state);
        Err("Windows 資料夾選擇器只適用於 Windows。".to_owned())
    }
}

#[cfg(all(target_os = "windows", test))]
fn add_windows_folder_to_database(
    database: &Database,
    path: &str,
) -> Result<LibrarySource, String> {
    let requested_path = path.trim();
    if requested_path.is_empty() {
        return Err("請選擇音樂資料夾。".to_owned());
    }
    add_windows_folder_path_to_database(database, std::path::Path::new(requested_path))
}

#[cfg(all(target_os = "windows", test))]
fn add_windows_folder_path_to_database(
    database: &Database,
    requested_path: &std::path::Path,
) -> Result<LibrarySource, String> {
    if requested_path.as_os_str().is_empty() {
        return Err("請選擇音樂資料夾。".to_owned());
    }
    let canonical_path = std::fs::canonicalize(requested_path)
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

#[cfg(target_os = "windows")]
fn add_windows_folder_to_settings(
    database: &Database,
    settings: &SettingsStore,
    path: &str,
) -> Result<LibrarySource, String> {
    let requested_path = path.trim();
    if requested_path.is_empty() {
        return Err("請選擇音樂資料夾。".to_owned());
    }
    add_windows_folder_path_to_settings(database, settings, std::path::Path::new(requested_path))
}

#[cfg(target_os = "windows")]
fn add_windows_folder_path_to_settings(
    database: &Database,
    settings: &SettingsStore,
    requested_path: &std::path::Path,
) -> Result<LibrarySource, String> {
    let canonical_path = std::fs::canonicalize(requested_path)
        .map_err(|error| format!("無法開啟指定資料夾：{error}"))?;
    if !canonical_path.is_dir() {
        return Err("指定路徑不是資料夾。".to_owned());
    }
    let canonical_key = windows_locator_key(&canonical_path);
    let current = settings.snapshot().map_err(|error| error.to_string())?;
    let existing = current.sources.iter().find(|source| {
        matches!(source.kind, SourceEntryKind::Folder { .. })
            && source
                .path()
                .to_path_buf()
                .is_ok_and(|path| windows_locator_key(&path) == canonical_key)
    });
    let mut root = if let Some(source) = existing {
        source
            .library_root()
            .map_err(|error| error.to_string())?
            .expect("folder source")
    } else {
        let database_root = database
            .library_roots()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|root| {
                matches!(root.kind, MediaSourceKind::WindowsFilesystem | MediaSourceKind::WindowsSystemIndex)
                    && matches!(&root.locator, MediaLocator::FileSystem(path) if windows_locator_key(path) == canonical_key)
            });
        if let Some(root) = database_root {
            root
        } else {
            let display_name = canonical_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "本機音樂資料夾".to_owned());
            LibraryRoot {
                id: SourceId::new(),
                kind: MediaSourceKind::WindowsSystemIndex,
                display_name,
                locator: MediaLocator::FileSystem(canonical_path.clone()),
                enabled: true,
            }
        }
    };
    root.enabled = true;
    let entry = SourceEntry::from_library_root(&root).map_err(|error| error.to_string())?;
    settings
        .upsert_source(entry.clone())
        .map_err(|error| error.to_string())?;
    database
        .save_library_root(&root)
        .map_err(|error| error.to_string())?;
    source_entry_summary(Some(database), &entry)
}

#[tauri::command]
fn library_set_source_enabled(
    state: State<'_, AppState>,
    source_id: String,
    enabled: bool,
) -> Result<Vec<LibrarySource>, String> {
    let id = SourceId::parse(&source_id).map_err(|error| format!("來源 ID 無效：{error}"))?;
    state
        .settings
        .set_source_enabled(id, enabled)
        .map_err(|error| error.to_string())?;
    let settings = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?;
    if let Some(database) = state.database.as_ref() {
        if let Some(source) = settings.sources.iter().find(|source| source.id == id) {
            let root = source
                .database_projection()
                .map_err(|error| error.to_string())?;
            database
                .save_library_root(&root)
                .map_err(|error| error.to_string())?;
        }
    }
    settings
        .sources
        .iter()
        .map(|source| source_entry_summary(state.database.as_ref(), source))
        .collect()
}

fn source_entry_summary(
    database: Option<&Database>,
    source: &SourceEntry,
) -> Result<LibrarySource, String> {
    let (kind, location) = match &source.kind {
        SourceEntryKind::Folder { media_kind, path } => (
            source_kind_name(*media_kind).to_owned(),
            path.display_lossy(),
        ),
        SourceEntryKind::PlaylistFile { path, .. } => {
            ("playlist_file".to_owned(), path.display_lossy())
        }
    };
    let sync = database
        .map(|database| database.sync_state(source.id))
        .transpose()
        .map_err(|error| error.to_string())?
        .flatten();
    Ok(LibrarySource {
        id: source.id.to_string(),
        kind,
        display_name: source.display_name.clone(),
        location,
        enabled: source.enabled,
        sync_state: sync.as_ref().map(|state| state.state.clone()),
        last_attempt_utc_ms: sync.as_ref().and_then(|state| state.last_attempt_utc_ms),
        last_success_utc_ms: sync.as_ref().and_then(|state| state.last_success_utc_ms),
        error_count: sync.map_or(0, |state| state.error_count),
    })
}

#[tauri::command]
fn library_remove_source(
    state: State<'_, AppState>,
    source_id: String,
) -> Result<Vec<LibrarySource>, String> {
    let id = SourceId::parse(&source_id).map_err(|error| format!("來源 ID 無效：{error}"))?;
    let (settings, removed) = state
        .settings
        .remove_source(id)
        .map_err(|error| error.to_string())?;
    if let Some(database) = state.database.as_ref() {
        if matches!(removed.kind, SourceEntryKind::PlaylistFile { .. }) {
            database
                .remove_playlist_file_source(id)
                .map_err(|error| error.to_string())?;
        }
        let mut root = removed
            .database_projection()
            .map_err(|error| error.to_string())?;
        root.enabled = false;
        database
            .save_library_root(&root)
            .map_err(|error| error.to_string())?;
    }
    settings
        .sources
        .iter()
        .map(|source| source_entry_summary(state.database.as_ref(), source))
        .collect()
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
    let mut root = add_or_get_root(
        database,
        MediaSourceKind::AndroidMediaStore,
        media_volume_display_name(volume_name),
        locator,
    )?;
    root.enabled = true;
    register_folder_source(database, &state.settings, &root)?;
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

    let mut root = add_or_get_root(
        database,
        MediaSourceKind::AndroidSaf,
        "Android 文件資料夾".to_owned(),
        MediaLocator::ContentUri(uri),
    )?;
    root.enabled = true;
    register_folder_source(database, &state.settings, &root)?;
    source_summary(database, &root).map(Some)
}

fn register_folder_source(
    database: &Database,
    settings: &SettingsStore,
    root: &LibraryRoot,
) -> Result<(), String> {
    let source = SourceEntry::from_library_root(root).map_err(|error| error.to_string())?;
    settings
        .upsert_source(source)
        .map_err(|error| error.to_string())?;
    database
        .save_library_root(root)
        .map_err(|error| error.to_string())
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
        location: match &root.locator {
            MediaLocator::FileSystem(path) => path.to_string_lossy().into_owned(),
            MediaLocator::ContentUri(uri) => uri.clone(),
        },
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
        MediaSourceKind::PlaylistFile => "playlistFile",
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
    let source_settings = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?;
    let source_registry_authoritative = source_settings.source_registry_authoritative;
    if !source_registry_authoritative {
        return Err(
            "來源清單尚未確認，已暫停同步以保留既有曲庫資料。請重新登記所有音樂資料夾與播放清單來源，再確認來源清單。"
                .to_owned(),
        );
    }
    let database_path = state.database_path.clone().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    let run_id = SourceId::new().to_string();
    let progress_run_id = run_id.clone();
    let sync_app = app.clone();
    let event_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut database = Database::open(database_path).map_err(|error| error.to_string())?;
        let mut on_progress = |root: &LibraryRoot,
                               source_index: usize,
                               source_count: usize,
                               progress: SyncProgress| {
            emit_library_sync_progress(
                &sync_app,
                &progress_run_id,
                root,
                source_index,
                source_count,
                progress,
            );
        };
        sync_configured_sources(
            &sync_app,
            &mut database,
            &source_settings.sources,
            source_registry_authoritative,
            &mut on_progress,
        )
    })
    .await
    .map_err(|error| format!("曲庫同步工作失敗：{error}"))
    .and_then(|result| result);
    let finished = library_sync_finished_event(run_id, &result);
    let _ = event_app.emit(LIBRARY_SYNC_FINISHED_EVENT, finished);
    result
}

fn sync_configured_sources<F>(
    app: &AppHandle,
    database: &mut Database,
    configured_sources: &[SourceEntry],
    source_registry_authoritative: bool,
    on_progress: &mut F,
) -> Result<LibrarySyncResult, String>
where
    F: FnMut(&LibraryRoot, usize, usize, SyncProgress),
{
    if !source_registry_authoritative {
        return Err("設定來源清單尚未確認；為保護現有曲庫，本次已略過所有來源同步。".to_owned());
    }
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        sync_windows_configured_sources(
            database,
            configured_sources,
            source_registry_authoritative,
            on_progress,
        )
    }

    #[cfg(target_os = "android")]
    {
        let roots = configured_sources
            .iter()
            .filter_map(|source| source.library_root().ok().flatten())
            .collect::<Vec<_>>();
        let synced_at_utc_ms = now_utc_epoch_ms()?;
        let roots: Vec<_> = roots
            .into_iter()
            .filter(|root| {
                root.enabled
                    && matches!(
                        root.kind,
                        MediaSourceKind::AndroidMediaStore | MediaSourceKind::AndroidSaf
                    )
            })
            .collect();
        let source_count = roots.len();
        let mut sources = Vec::new();
        app.media_index()
            .with_adapter(|index| -> Result<(), String> {
                for (source_index, root) in roots.iter().enumerate() {
                    let report = SyncEngine::sync_with_progress(
                        root,
                        index,
                        database,
                        synced_at_utc_ms,
                        |progress| {
                            on_progress(root, source_index, source_count, progress);
                        },
                    )
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
fn sync_windows_configured_sources<F>(
    database: &mut Database,
    configured_sources: &[SourceEntry],
    source_registry_authoritative: bool,
    on_progress: &mut F,
) -> Result<LibrarySyncResult, String>
where
    F: FnMut(&LibraryRoot, usize, usize, SyncProgress),
{
    if !source_registry_authoritative {
        return Err("設定來源清單尚未確認；為保護現有曲庫，本次已略過所有來源同步。".to_owned());
    }
    let synced_at_utc_ms = now_utc_epoch_ms()?;
    let configured_ids = configured_sources
        .iter()
        .map(|source| source.id)
        .collect::<std::collections::HashSet<_>>();
    for mut root in database
        .library_roots()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|root| {
            root.kind == MediaSourceKind::PlaylistFile && !configured_ids.contains(&root.id)
        })
    {
        database
            .remove_playlist_file_source(root.id)
            .map_err(|error| error.to_string())?;
        root.enabled = false;
        database
            .save_library_root(&root)
            .map_err(|error| error.to_string())?;
    }
    let sources: Vec<_> = configured_sources
        .iter()
        .filter(|source| source.enabled)
        .filter(|source| match source.kind {
            SourceEntryKind::Folder { media_kind, .. } => matches!(
                media_kind,
                MediaSourceKind::WindowsFilesystem | MediaSourceKind::WindowsSystemIndex
            ),
            SourceEntryKind::PlaylistFile { .. } => true,
        })
        .cloned()
        .collect();
    let source_count = sources.len();
    let mut results = Vec::new();
    for (source_index, source) in sources.iter().enumerate() {
        match &source.kind {
            SourceEntryKind::Folder { .. } => {
                let root = source
                    .library_root()
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| "folder source has no library root".to_owned())?;
                database
                    .save_library_root(&root)
                    .map_err(|error| error.to_string())?;
                let mut index = WindowsMediaIndex::new();
                let report = SyncEngine::sync_with_progress(
                    &root,
                    &mut index,
                    database,
                    synced_at_utc_ms,
                    |progress| on_progress(&root, source_index, source_count, progress),
                )
                .map_err(|error| error.to_string())?;
                results.push(sync_summary(&root, report));
            }
            SourceEntryKind::PlaylistFile { .. } => {
                let root = LibraryRoot {
                    id: source.id,
                    kind: MediaSourceKind::PlaylistFile,
                    display_name: source.display_name.clone(),
                    locator: source
                        .path()
                        .to_media_locator()
                        .map_err(|error| error.to_string())?,
                    enabled: source.enabled,
                };
                database
                    .save_library_root(&root)
                    .map_err(|error| error.to_string())?;
                if let Some(report) = playlist_source_sync::sync_playlist_file_source(
                    database,
                    source,
                    synced_at_utc_ms,
                )? {
                    on_progress(
                        &root,
                        source_index,
                        source_count,
                        SyncProgress {
                            source_id: source.id,
                            stage: player_core::SyncProgressStage::Finished,
                            processed: report.observed,
                            total: Some(report.observed),
                            unit: player_core::SyncProgressUnit::Tracks,
                            observed: report.observed,
                            metadata_reads: report.metadata_reads,
                            unchanged: report.unchanged,
                            error_count: report.source_errors.len() as u64
                                + report.metadata_errors.len() as u64,
                            outcome: report.state.as_ref().map(|state| match state {
                                player_core::SourceScanState::Complete => {
                                    player_core::SyncProgressOutcome::Complete
                                }
                                player_core::SourceScanState::Incomplete { .. } => {
                                    player_core::SyncProgressOutcome::Incomplete
                                }
                                player_core::SourceScanState::Unavailable { .. } => {
                                    player_core::SyncProgressOutcome::Unavailable
                                }
                                player_core::SourceScanState::PermissionRevoked { .. } => {
                                    player_core::SyncProgressOutcome::PermissionRevoked
                                }
                            }),
                        },
                    );
                    results.push(sync_summary(&root, report));
                }
            }
        }
    }
    // Disabled sources remain registered for re-enabling but cannot keep their mappings active.
    for source in configured_sources.iter().filter(|source| !source.enabled) {
        if let Some(mut root) = source.library_root().map_err(|error| error.to_string())? {
            root.enabled = false;
            database
                .save_library_root(&root)
                .map_err(|error| error.to_string())?;
        } else if let SourceEntryKind::PlaylistFile { .. } = source.kind {
            let root = LibraryRoot {
                id: source.id,
                kind: MediaSourceKind::PlaylistFile,
                display_name: source.display_name.clone(),
                locator: source
                    .path()
                    .to_media_locator()
                    .map_err(|error| error.to_string())?,
                enabled: false,
            };
            database
                .save_library_root(&root)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(LibrarySyncResult { sources: results })
}

#[cfg(all(target_os = "windows", test))]
fn sync_windows_sources(database: &mut Database) -> Result<LibrarySyncResult, String> {
    sync_windows_sources_with_progress(database, &mut |_, _, _, _| {})
}

#[cfg(all(target_os = "windows", test))]
fn sync_windows_sources_with_progress<F>(
    database: &mut Database,
    on_progress: &mut F,
) -> Result<LibrarySyncResult, String>
where
    F: FnMut(&LibraryRoot, usize, usize, SyncProgress),
{
    let roots = database
        .library_roots()
        .map_err(|error| error.to_string())?;
    let synced_at_utc_ms = now_utc_epoch_ms()?;
    let roots: Vec<_> = roots
        .into_iter()
        .filter(|root| {
            root.enabled
                && matches!(
                    root.kind,
                    MediaSourceKind::WindowsFilesystem | MediaSourceKind::WindowsSystemIndex
                )
        })
        .collect();
    let source_count = roots.len();
    let mut sources = Vec::new();
    let mut index = WindowsMediaIndex::new();
    for (source_index, root) in roots.iter().enumerate() {
        let report = SyncEngine::sync_with_progress(
            root,
            &mut index,
            database,
            synced_at_utc_ms,
            |progress| on_progress(root, source_index, source_count, progress),
        )
        .map_err(|error| error.to_string())?;
        sources.push(sync_summary(root, report));
    }
    Ok(LibrarySyncResult { sources })
}

fn emit_library_sync_progress(
    app: &AppHandle,
    run_id: &str,
    root: &LibraryRoot,
    source_index: usize,
    source_count: usize,
    progress: SyncProgress,
) {
    let _ = app.emit(
        LIBRARY_SYNC_PROGRESS_EVENT,
        LibrarySyncProgressEvent {
            run_id: run_id.to_owned(),
            source_index,
            source_count,
            display_name: root.display_name.clone(),
            progress,
        },
    );
}

fn library_sync_finished_event(
    run_id: String,
    result: &Result<LibrarySyncResult, String>,
) -> LibrarySyncFinishedEvent {
    match result {
        Ok(result) => LibrarySyncFinishedEvent {
            run_id,
            source_count: result.sources.len(),
            sources: result.sources.clone(),
            error: None,
        },
        Err(error) => LibrarySyncFinishedEvent {
            run_id,
            source_count: 0,
            sources: Vec::new(),
            error: Some(error.clone()),
        },
    }
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
async fn playback_get_snapshot(state: State<'_, AppState>) -> Result<PlaybackSnapshot, String> {
    #[cfg(target_os = "windows")]
    {
        let playback = windows_playback_service(&state)?;
        if let Some(database) = state.database.as_ref() {
            if let Some(ticket) = advance_ended_playback(database, playback)? {
                let result = await_playback_ack(ticket).await;
                if playback.player.snapshot().state != AudioPlaybackState::Ended {
                    *playback
                        .queue_advance_pending
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
                }
                result?;
            }
        }
        if let Some(system_media) = state.system_media.as_ref() {
            system_media.pump(playback, state.database.as_ref());
        }
        Ok(playback.snapshot())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = state;
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

/// Read the active track's original encoded cover bytes on a blocking worker.
/// An empty binary response means that no supported cover is available.
#[tauri::command]
async fn library_get_track_artwork(
    state: State<'_, AppState>,
    track_id: String,
) -> Result<Response, String> {
    #[cfg(target_os = "windows")]
    {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let id = TrackId::parse(&track_id).map_err(|error| format!("曲目 ID 無效：{error}"))?;
        let locators = database
            .track_locators(id)
            .map_err(|error| error.to_string())?;
        let artwork = tauri::async_runtime::spawn_blocking(move || find_artwork(&locators))
            .await
            .map_err(|error| format!("封面讀取工作失敗：{error}"))?;
        match artwork {
            ArtworkLookup::Found(image) => Ok(Response::new(image.into_bytes())),
            ArtworkLookup::Missing => Ok(Response::new(Vec::new())),
            ArtworkLookup::Oversized => Err("ARTWORK_TOO_LARGE".to_owned()),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, track_id);
        Err("此平台尚未提供本機封面讀取服務。".to_owned())
    }
}

#[tauri::command]
async fn playback_play(
    state: State<'_, AppState>,
    track_id: String,
    queue_source: Option<PlaybackQueueSource>,
) -> Result<PlaybackSnapshot, String> {
    #[cfg(target_os = "windows")]
    {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let service = windows_playback_service(&state)?;
        let (ticket, track) = play_track_from_database(database, service, &track_id, queue_source)?;
        await_playback_ack(ticket).await?;
        *service
            .current_track
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(track);
        Ok(service.snapshot())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, track_id, queue_source);
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
async fn playback_pause(state: State<'_, AppState>) -> Result<PlaybackSnapshot, String> {
    #[cfg(target_os = "windows")]
    {
        let service = windows_playback_service(&state)?;
        await_playback_ack(pause_playback(service)?).await?;
        Ok(service.snapshot())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = state;
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
async fn playback_next(state: State<'_, AppState>) -> Result<PlaybackSnapshot, String> {
    #[cfg(target_os = "windows")]
    {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let service = windows_playback_service(&state)?;
        if let Some(ticket) = navigate_queue(database, service, true)? {
            await_playback_ack(ticket).await?;
        }
        Ok(service.snapshot())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = state;
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
async fn playback_previous(state: State<'_, AppState>) -> Result<PlaybackSnapshot, String> {
    #[cfg(target_os = "windows")]
    {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let service = windows_playback_service(&state)?;
        if let Some(ticket) = navigate_queue(database, service, false)? {
            await_playback_ack(ticket).await?;
        }
        Ok(service.snapshot())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = state;
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
async fn playback_seek(
    state: State<'_, AppState>,
    position_ms: u64,
) -> Result<PlaybackSnapshot, String> {
    #[cfg(target_os = "windows")]
    {
        let service = windows_playback_service(&state)?;
        await_playback_ack(seek_playback(service, position_ms)?).await?;
        Ok(service.snapshot())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, position_ms);
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
async fn playback_set_volume(
    state: State<'_, AppState>,
    volume: f64,
) -> Result<PlaybackSnapshot, String> {
    #[cfg(target_os = "windows")]
    {
        let service = windows_playback_service(&state)?;
        await_playback_ack(set_playback_volume(service, volume)?).await?;
        Ok(service.snapshot())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, volume);
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
fn playback_set_repeat(
    state: State<'_, AppState>,
    mode: RepeatMode,
) -> Result<PlaybackSnapshot, String> {
    state
        .settings
        .update(|settings| {
            settings.repeat_mode = mode;
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "windows")]
    {
        let service = windows_playback_service(&state)?;
        let _gate = service
            .command_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *service
            .repeat_mode
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = mode;
        if let Some(queue) = service
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            queue.set_repeat_mode(queue_repeat_mode(mode));
        }
        Ok(service.snapshot())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, mode);
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
fn playback_set_shuffle(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<PlaybackSnapshot, String> {
    state
        .settings
        .update(|settings| {
            settings.shuffle = enabled;
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "windows")]
    {
        let service = windows_playback_service(&state)?;
        let _gate = service
            .command_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *service
            .shuffle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = enabled;
        if let Some(queue) = service
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            queue.set_shuffle(enabled);
        }
        Ok(service.snapshot())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, enabled);
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
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
fn queue_repeat_mode(mode: RepeatMode) -> QueueRepeatMode {
    match mode {
        RepeatMode::Off => QueueRepeatMode::Off,
        RepeatMode::One => QueueRepeatMode::One,
        RepeatMode::All => QueueRepeatMode::All,
    }
}

#[cfg(target_os = "windows")]
fn queue_for_track(
    database: &Database,
    track_id: TrackId,
    source: Option<PlaybackQueueSource>,
) -> Result<PlaybackQueue, String> {
    let (track_ids, selected_index) = match source {
        Some(PlaybackQueueSource::Library { query }) => {
            let track_ids = database
                .list_track_ids(query.as_deref())
                .map_err(|error| error.to_string())?;
            let selected = track_ids
                .iter()
                .position(|candidate| *candidate == track_id)
                .ok_or_else(|| "所選曲目已不在目前的曲庫查詢結果中。".to_owned())?;
            (track_ids, selected)
        }
        Some(PlaybackQueueSource::Playlist {
            playlist_id,
            entry_position,
        }) => {
            let playlist_id = PlaylistId::parse(&playlist_id)
                .map_err(|error| format!("播放清單 ID 無效：{error}"))?;
            let entries = database
                .playlist_track_ids(playlist_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "播放清單已不存在，請重新整理。".to_owned())?;
            let selected = entries
                .iter()
                .position(|(position, candidate)| {
                    *position == entry_position && *candidate == track_id
                })
                .ok_or_else(|| "所選播放清單項目已無法播放。".to_owned())?;
            (entries.into_iter().map(|(_, id)| id).collect(), selected)
        }
        None => {
            let track_ids = database
                .list_track_ids(None)
                .map_err(|error| error.to_string())?;
            let selected = track_ids
                .iter()
                .position(|candidate| *candidate == track_id)
                .ok_or_else(|| "所選曲目已不在曲庫中。".to_owned())?;
            (track_ids, selected)
        }
    };
    PlaybackQueue::new(track_ids, selected_index)
        .ok_or_else(|| "播放清單沒有可播放的曲目。".to_owned())
}

#[cfg(target_os = "windows")]
fn enqueue_track_locked(
    database: &Database,
    service: &WindowsPlaybackService,
    track_id: TrackId,
) -> Result<CommandTicket, String> {
    let track = database
        .get_track_summary(track_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "曲目已不在曲庫中，請重新整理列表。".to_owned())?;
    let path = database
        .resolve_playable_filesystem_locator(track_id)
        .map_err(|error| error.to_string())?;
    service
        .player
        .load(path)
        .map_err(|error| playback_audio_error_message(&error))?;
    let ticket = service
        .player
        .request_play()
        .map_err(|error| playback_audio_error_message(&error))?;
    *service
        .current_track
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(track);
    Ok(ticket)
}

#[cfg(target_os = "windows")]
fn navigate_queue(
    database: &Database,
    service: &WindowsPlaybackService,
    next: bool,
) -> Result<Option<CommandTicket>, String> {
    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let target = {
        let mut queue = service
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(queue) = queue.as_mut() else {
            return Ok(None);
        };
        if next {
            queue.next(true)
        } else {
            queue.previous()
        }
    };
    target
        .map(|track_id| enqueue_track_locked(database, service, track_id))
        .transpose()
}

#[cfg(target_os = "windows")]
fn advance_ended_playback(
    database: &Database,
    service: &WindowsPlaybackService,
) -> Result<Option<CommandTicket>, String> {
    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if service.player.snapshot().state != AudioPlaybackState::Ended {
        *service
            .queue_advance_pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
        return Ok(None);
    }
    let mut pending = service
        .queue_advance_pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *pending {
        return Ok(None);
    }
    *pending = true;
    drop(pending);

    let target = service
        .queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_mut()
        .and_then(|queue| queue.next(false));
    let Some(track_id) = target else {
        *service
            .queue_advance_pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
        return Ok(None);
    };
    match enqueue_track_locked(database, service, track_id) {
        Ok(ticket) => Ok(Some(ticket)),
        Err(error) => {
            *service
                .queue_advance_pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
            Err(error)
        }
    }
}

#[cfg(target_os = "windows")]
fn pause_playback(service: &WindowsPlaybackService) -> Result<CommandTicket, String> {
    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    service
        .player
        .request_pause()
        .map_err(|error| playback_audio_error_message(&error))
}

#[cfg(target_os = "windows")]
fn stop_playback(service: &WindowsPlaybackService) -> Result<PlaybackSnapshot, String> {
    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    service
        .player
        .stop()
        .map_err(|error| playback_audio_error_message(&error))?;
    Ok(service.snapshot())
}

#[cfg(target_os = "windows")]
fn seek_playback(
    service: &WindowsPlaybackService,
    position_ms: u64,
) -> Result<CommandTicket, String> {
    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    service
        .player
        .request_seek(Duration::from_millis(position_ms))
        .map_err(|error| playback_audio_error_message(&error))
}

#[cfg(target_os = "windows")]
fn set_playback_volume(
    service: &WindowsPlaybackService,
    volume: f64,
) -> Result<CommandTicket, String> {
    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    service
        .player
        .request_set_volume(volume as f32)
        .map_err(|error| playback_audio_error_message(&error))
}

#[cfg(target_os = "windows")]
async fn await_playback_ack(ticket: CommandTicket) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || ticket.wait(Duration::from_secs(2)))
        .await
        .map_err(|error| format!("等待播放命令工作失敗：{error}"))?
        .map(|_| ())
        .map_err(|error| playback_audio_error_message(&error))
}

#[cfg(target_os = "windows")]
fn play_track_from_database(
    database: &Database,
    service: &WindowsPlaybackService,
    track_id: &str,
    source: Option<PlaybackQueueSource>,
) -> Result<(CommandTicket, TrackSummary), String> {
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
    let preserve_queue = source.is_none()
        && service
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|queue| queue.current() == track_id);
    if !preserve_queue {
        let mut queue = queue_for_track(database, track_id, source)?;
        let repeat_mode = *service
            .repeat_mode
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let shuffle = *service
            .shuffle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queue.set_repeat_mode(queue_repeat_mode(repeat_mode));
        queue.set_shuffle(shuffle);
        *service
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(queue);
    }
    let audio_snapshot = service.player.snapshot();
    let ticket = if current_track_id == Some(track_id)
        && matches!(
            audio_snapshot.state,
            AudioPlaybackState::Ready | AudioPlaybackState::Paused
        ) {
        service
            .player
            .request_play()
            .map_err(|error| playback_audio_error_message(&error))?
    } else {
        service
            .player
            .load(path)
            .map_err(|error| playback_audio_error_message(&error))?;
        service
            .player
            .request_play()
            .map_err(|error| playback_audio_error_message(&error))?
    };
    *service
        .current_track
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(track.clone());
    Ok((ticket, track))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_media_index::init());
    #[cfg(target_os = "windows")]
    let builder = builder.plugin(tauri_plugin_dialog::init());

    builder
        .on_window_event(|window, event| {
            #[cfg(target_os = "windows")]
            if window.label() == "main"
                && matches!(event, tauri::WindowEvent::CloseRequested { .. })
            {
                let state = window.app_handle().state::<AppState>();
                if let Some(system_media) = state.system_media.as_ref() {
                    system_media.shutdown();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            settings_get_recovery_warning,
            settings_source_registry_authoritative,
            settings_confirm_source_registry,
            theme_get_preferences,
            theme_set_preferences,
            settings_get_track_list_columns,
            settings_set_track_list_columns,
            settings_get_now_playing_layout,
            settings_set_now_playing_layout,
            get_runtime_capabilities,
            library_get_page,
            playlist_list,
            playlist_get_page,
            playlist_import_m3u,
            playlist_export_m3u,
            library_list_sources,
            library_add_windows_folder,
            library_pick_windows_folder,
            library_set_source_enabled,
            library_remove_source,
            android_media_request_permission,
            android_media_list_volumes,
            android_media_add_volume,
            android_saf_pick_source,
            library_sync,
            playback_get_snapshot,
            library_get_track_artwork,
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
            let settings_path = database_path.with_file_name("settings.json");
            let database_result = Database::open(&database_path).map_err(|error| error.to_string());
            let (database, stored_path, database_error) = match database_result {
                Ok(database) => (Some(database), Some(database_path), None),
                Err(error) => (None, None, Some(error)),
            };
            let legacy_settings = if SettingsStore::needs_legacy_import(&settings_path) {
                if let Some(database) = database.as_ref() {
                    let theme = database
                        .get_theme_preferences()
                        .map_err(|error| format!("讀取舊版外觀設定失敗：{error}"))?;
                    let roots = database
                        .library_roots()
                        .map_err(|error| format!("讀取舊版音樂來源失敗：{error}"))?;
                    AppSettings::from_legacy(theme, roots)
                        .map_err(|error| format!("轉換舊版設定失敗：{error}"))?
                } else {
                    AppSettings::default()
                }
            } else {
                AppSettings::default()
            };
            let settings = SettingsStore::open(&settings_path, legacy_settings)
                .map_err(|error| format!("開啟使用者設定失敗：{error}"))?;
            if let Some(database) = database.as_ref() {
                if let Err(error) = database.clear_legacy_theme_preferences() {
                    eprintln!("無法清除已遷移的 SQLite 外觀偏好：{error}");
                }
            }
            #[cfg(target_os = "windows")]
            let initial_settings = settings
                .snapshot()
                .map_err(|error| format!("讀取使用者設定失敗：{error}"))?;
            #[cfg(target_os = "windows")]
            let (playback, playback_error) = match PlayerHandle::new() {
                Ok(player) => (
                    Some(WindowsPlaybackService::with_modes(
                        player,
                        initial_settings.shuffle,
                        initial_settings.repeat_mode,
                    )),
                    None,
                ),
                Err(error) => (None, Some(playback_audio_error_message(&error))),
            };
            #[cfg(target_os = "windows")]
            let (system_media, system_media_error) =
                create_windows_system_media_service(app.handle());
            app.manage(AppState {
                database,
                database_path: stored_path.clone(),
                database_error,
                settings,
                #[cfg(target_os = "windows")]
                playback,
                #[cfg(target_os = "windows")]
                playback_error,
                #[cfg(target_os = "windows")]
                system_media,
                #[cfg(target_os = "windows")]
                system_media_error,
            });

            if let Some(window) = app.get_webview_window("main") {
                window.set_title("MoeMusicPlayer")?;
                window.set_always_on_top(false)?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("MoeMusicPlayer could not start");
}

#[cfg(all(test, target_os = "windows"))]
mod windows_library_integration_tests {
    use super::{
        add_windows_folder_path_to_database, add_windows_folder_to_database, sync_windows_sources,
    };
    use super::{
        apply_system_media_event, play_track_from_database, playback_audio_error_message,
        queue_for_track, set_playback_volume, PlaybackQueueSource, WindowsPlaybackService,
    };
    use player_audio_windows::{
        system_media::{SystemMediaController, SystemMediaEvent},
        AudioBackend, AudioError, PlayerHandle,
    };
    use player_core::{
        ListTracksQuery, MediaLocator, MediaSourceKind, Playlist, PlaylistEntry, SourceId,
    };
    use player_db::Database;
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
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

    #[test]
    fn selected_windows_folder_keeps_unicode_path_and_rejects_invalid_roots() {
        let temporary = TestDirectory::new();
        let selected_path = temporary.0.join("東京🌸 音樂");
        fs::create_dir_all(&selected_path).expect("create Unicode source folder");
        let database = Database::open(temporary.0.join("picker-test.sqlite3"))
            .expect("open isolated source database");

        let source = add_windows_folder_path_to_database(&database, &selected_path)
            .expect("add native Unicode PathBuf");
        assert_eq!(source.display_name, "東京🌸 音樂");
        let canonical_path = fs::canonicalize(&selected_path).expect("canonicalize test folder");
        let roots = database.library_roots().expect("read persisted source");
        assert_eq!(roots.len(), 1);
        assert!(matches!(
            &roots[0].locator,
            MediaLocator::FileSystem(saved_path) if saved_path == &canonical_path
        ));

        let missing_path = temporary.0.join("does-not-exist");
        let missing_error = match add_windows_folder_path_to_database(&database, &missing_path) {
            Ok(_) => panic!("missing folders must not become sources"),
            Err(error) => error,
        };
        assert!(missing_error.contains("無法開啟指定資料夾"));

        let file_path = temporary.0.join("not-a-folder.txt");
        fs::write(&file_path, "test").expect("write a regular file for invalid-root test");
        let file_error = match add_windows_folder_path_to_database(&database, &file_path) {
            Ok(_) => panic!("files must not become folder sources"),
            Err(error) => error,
        };
        assert!(file_error.contains("指定路徑不是資料夾"));
        assert_eq!(
            database
                .library_roots()
                .expect("read sources after errors")
                .len(),
            1
        );
    }

    #[test]
    fn non_authoritative_settings_skip_orphan_cleanup_and_keep_playlist_mapping() {
        let temporary = TestDirectory::new();
        let media_folder = temporary.0.join("曲目");
        fs::create_dir_all(&media_folder).expect("create media folder");
        let media_path = media_folder.join("保持資料.mp3");
        write_tagged_mp3(&media_path);
        let playlist_path = temporary.0.join("保留清單.m3u8");
        fs::write(
            &playlist_path,
            format!("#EXTM3U\n{}\n", media_path.display()),
        )
        .expect("write playlist source");
        let source = super::SourceEntry {
            id: SourceId::new(),
            display_name: "保留清單".to_owned(),
            enabled: true,
            kind: super::SourceEntryKind::PlaylistFile {
                playlist_id: super::PlaylistId::new(),
                path: super::settings::StoredPath::from_path(&playlist_path)
                    .expect("store native playlist path"),
            },
        };
        let mut database = Database::open_in_memory().expect("database");
        super::playlist_source_sync::sync_playlist_file_source(
            &mut database,
            &source,
            1_800_000_000_000,
        )
        .expect("create an existing playlist projection")
        .expect("playlist source is readable");
        assert_eq!(
            database.count_tracks(None).expect("initial track mapping"),
            1
        );

        let result =
            super::sync_windows_configured_sources(&mut database, &[], false, &mut |_, _, _, _| {});
        let error = match result {
            Ok(_) => panic!("non-authoritative registry must stop cleanup"),
            Err(error) => error,
        };
        assert!(error.contains("設定來源清單尚未確認"));
        assert_eq!(
            database.count_tracks(None).expect("mapping is preserved"),
            1
        );
        let root = database
            .library_roots()
            .expect("read preserved source projection")
            .into_iter()
            .find(|root| root.id == source.id)
            .expect("source root remains");
        assert!(root.enabled);
    }

    #[test]
    fn inaccessible_windows_folder_returns_error_without_adding_a_source() {
        let temporary = TestDirectory::new();
        let protected_path = temporary.0.join("inaccessible");
        fs::create_dir_all(&protected_path).expect("create isolated permission-test folder");
        let database = Database::open(temporary.0.join("permission-test.sqlite3"))
            .expect("open isolated source database");
        let denied = Command::new("icacls.exe")
            .arg(&protected_path)
            .args(["/deny", "*S-1-1-0:(RX)"])
            .output()
            .expect("run Windows ACL tool against disposable test folder");
        assert!(
            denied.status.success(),
            "icacls deny failed: {}",
            String::from_utf8_lossy(&denied.stderr)
        );

        let result = add_windows_folder_path_to_database(&database, &protected_path);
        let restored = Command::new("icacls.exe")
            .arg(&protected_path)
            .args(["/remove:d", "*S-1-1-0"])
            .output()
            .expect("restore inherited ACL on disposable test folder");
        assert!(
            restored.status.success(),
            "icacls ACL restoration failed: {}",
            String::from_utf8_lossy(&restored.stderr)
        );

        let error = match result {
            Ok(_) => panic!("an inaccessible folder must not become a source"),
            Err(error) => error,
        };
        assert!(error.contains("無法開啟指定資料夾"));
        assert!(database
            .library_roots()
            .expect("read sources after permission error")
            .is_empty());
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
        let (ticket, track) =
            play_track_from_database(&database, &service, &track_id.to_string(), None)
                .expect("resolve internal ID and enqueue playback");
        assert_eq!(track.id, track_id);
        ticket
            .wait(Duration::from_secs(2))
            .expect("worker should acknowledge playback");
        *service
            .current_track
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(track);

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
            player_platform_windows::windows_locator_key(&fs::canonicalize(&track_path).unwrap())
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
        let active_track_id = service
            .snapshot()
            .current_track
            .as_ref()
            .map(|track| track.id);
        for event in [
            SystemMediaEvent::NextRequested,
            SystemMediaEvent::PreviousRequested,
        ] {
            apply_system_media_event(&event, Some(&database), &service)
                .expect("ignore disabled queue navigation request");
        }
        assert!(service.snapshot().is_playing);
        assert_eq!(
            service
                .snapshot()
                .current_track
                .as_ref()
                .map(|track| track.id),
            active_track_id
        );

        let mut playlist = Playlist::new("duplicate queue positions");
        let locator = MediaLocator::FileSystem(track_path.clone());
        playlist.entries = vec![
            PlaylistEntry {
                track_id: Some(track_id),
                locator: locator.clone(),
                title: None,
                duration_ms: None,
            },
            PlaylistEntry {
                track_id: Some(track_id),
                locator,
                title: None,
                duration_ms: None,
            },
        ];
        database
            .save_playlist(&playlist)
            .expect("save duplicate playlist entries");
        let first_entry_queue = queue_for_track(
            &database,
            track_id,
            Some(PlaybackQueueSource::Playlist {
                playlist_id: playlist.id.to_string(),
                entry_position: 0,
            }),
        )
        .expect("resolve first duplicate entry by position");
        assert!(first_entry_queue.can_next());
        let last_entry_queue = queue_for_track(
            &database,
            track_id,
            Some(PlaybackQueueSource::Playlist {
                playlist_id: playlist.id.to_string(),
                entry_position: 1,
            }),
        )
        .expect("resolve second duplicate entry by position");
        assert!(!last_entry_queue.can_next());
        assert!(last_entry_queue.can_previous());

        apply_system_media_event(&SystemMediaEvent::PauseRequested, Some(&database), &service)
            .expect("apply SMTC pause request");
        let paused_deadline = Instant::now() + Duration::from_secs(2);
        while service.snapshot().state != "paused" {
            assert!(
                Instant::now() < paused_deadline,
                "player worker did not enter Paused state"
            );
            thread::sleep(Duration::from_millis(5));
        }

        apply_system_media_event(
            &SystemMediaEvent::SeekRequested(Duration::from_millis(1_250)),
            Some(&database),
            &service,
        )
        .expect("apply SMTC seek request");
        let seek_deadline = Instant::now() + Duration::from_secs(2);
        while service.snapshot().position_ms != 1_250 {
            assert!(
                Instant::now() < seek_deadline,
                "player worker did not apply seek position"
            );
            thread::sleep(Duration::from_millis(5));
        }

        set_playback_volume(&service, 0.35)
            .expect("queue volume command")
            .wait(Duration::from_secs(2))
            .expect("worker should acknowledge volume");
        let volume_deadline = Instant::now() + Duration::from_secs(2);
        while (service.snapshot().volume - 0.35).abs() > 0.000_01 {
            assert!(
                Instant::now() < volume_deadline,
                "player worker did not apply volume"
            );
            thread::sleep(Duration::from_millis(5));
        }

        apply_system_media_event(&SystemMediaEvent::StopRequested, Some(&database), &service)
            .expect("apply SMTC stop request");
        let stopped_deadline = Instant::now() + Duration::from_secs(2);
        while service.snapshot().state != "stopped" {
            assert!(
                Instant::now() < stopped_deadline,
                "player worker did not enter Stopped state"
            );
            thread::sleep(Duration::from_millis(5));
        }

        apply_system_media_event(&SystemMediaEvent::PlayRequested, Some(&database), &service)
            .expect("apply SMTC play request");
        let resumed_deadline = Instant::now() + Duration::from_secs(2);
        while !service.snapshot().is_playing {
            assert!(
                Instant::now() < resumed_deadline,
                "player worker did not resume after an SMTC play request"
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

    #[test]
    fn system_media_attach_and_update_failures_only_degrade_system_capability() {
        let attach_error = match SystemMediaController::attach(0) {
            Ok(_) => panic!("null HWND must not attach"),
            Err(error) => error.to_string(),
        };
        let (attached, attach_error) =
            super::windows_system_media_service_from_attach_result(Err(attach_error));
        assert!(attached.is_none());
        let attach_capability = super::unavailable_system_media_capability(attach_error.clone());
        assert!(matches!(
            attach_capability.state,
            super::FeatureState::Unavailable
        ));

        let playback_error_service = super::WindowsSystemMediaService {
            controller: Mutex::new(None),
            status: Mutex::new(super::WindowsSystemMediaStatus::Starting),
            pump_gate: Mutex::new(()),
            last_update: Mutex::new(None),
        };
        playback_error_service.handle_update_error(super::SystemMediaError::WorkerStopped);
        assert!(matches!(
            playback_error_service.capability().state,
            super::FeatureState::Unavailable
        ));

        let player = PlayerHandle::with_backend_factory({
            let loaded_path = Arc::new(Mutex::new(None));
            move || {
                Ok(Box::new(CapturingAudioBackend {
                    loaded_path: Arc::clone(&loaded_path),
                    position: Duration::ZERO,
                }) as Box<dyn AudioBackend>)
            }
        })
        .expect("start independent test audio worker");
        player
            .set_volume(0.4)
            .expect("local audio commands remain available");
        let volume_deadline = Instant::now() + Duration::from_secs(2);
        while (player.snapshot().volume - 0.4).abs() > 0.000_01 {
            assert!(
                Instant::now() < volume_deadline,
                "SMTC failures affected or blocked the local audio worker"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}
