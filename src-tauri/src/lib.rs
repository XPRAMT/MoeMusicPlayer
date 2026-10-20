use std::collections::HashMap;
#[cfg(any(target_os = "windows", test))]
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use std::sync::Arc;
#[cfg(target_os = "windows")]
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Mutex,
    },
    time::Instant,
};

#[cfg(target_os = "windows")]
use player_core::display_track_title;
#[cfg(target_os = "windows")]
use player_core::PlaybackCheckpoint;
use player_core::{
    FileFingerprint, LibraryRepository, LibraryRoot, ListTracksQuery, MediaLocator,
    MediaSourceError, MediaSourceKind, MediaTrackRecord, Page, PlaybackQueue, PlaybackQueueContext,
    PlaybackQueueEntry, PlaybackQueueSnapshot, PlaylistId, PlaylistListSort, PlaylistPage,
    PlaylistSummary, QueueRepeatMode, QueueTrackListeningStats, SourceId, SourceScanState,
    SyncCancellation, SyncEngine, SyncProgress, SyncReport, TrackFieldFilter, TrackId,
    TrackIdentity, TrackListSort, TrackMetadata, TrackMetadataError, TrackSortDirection,
    TrackSortField, TrackSummary,
};
use player_db::natural_title_cmp;
#[cfg(target_os = "windows")]
use player_db::PlaybackSessionCheckpoint;
use player_db::{Database, ThemePreferences};
use serde::{Deserialize, Serialize};
use tauri::{
    ipc::Response, AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Position, Size,
    State, WebviewWindow,
};
#[cfg_attr(not(target_os = "android"), allow(unused_imports))]
use tauri_plugin_media_index::MediaIndexExt;
#[cfg(any(target_os = "windows", target_os = "android"))]
mod database_lifecycle;
#[cfg(target_os = "windows")]
mod diagnostics;
#[cfg(target_os = "windows")]
mod maximized_work_area;
mod playlist_exchange;
#[cfg(target_os = "windows")]
mod window_icon;
#[cfg(target_os = "windows")]
mod windows_file_associations;
use playlist_exchange::{export_playlist_file, PlaylistExportResult, PlaylistImportResult};
#[cfg(any(target_os = "windows", target_os = "android"))]
use playlist_exchange::{import_playlist_file_with_id, read_playlist_file};
mod backup;
mod settings;
mod sleep_timer;
#[cfg(target_os = "windows")]
mod taskbar_lyrics;
mod track_details;
pub mod updater;
mod volume_sync;
use settings::{
    clamp_window_geometry_to_monitors, AlbumDetailSort, AlbumIndexSort, AppSettings,
    ArtistListSort, BackgroundOpacityTextColor, ExcludedFolderEntry, LyricsPreferences,
    MainBackgroundMode, NowPlayingAppearancePreferences, NowPlayingLayout, RepeatMode,
    ResamplingMode, SettingsStore, SlideshowOrder, SourceEntry, SourceEntryKind, ThemeColorMode,
    ThemeSettings, TrackListColumnSettings, WindowGeometry,
};
mod album_artwork_analysis;
mod album_artwork_cache;
#[cfg(any(target_os = "android", test))]
mod android_artwork;
#[cfg(any(target_os = "android", test))]
mod android_dsd;
mod android_folder_browser;
#[cfg(target_os = "android")]
mod android_path_migration;
#[cfg(any(target_os = "android", test))]
#[cfg_attr(test, allow(dead_code))]
mod android_playback;
mod favorites;
mod lyrics_provider;
mod lyrics_service;
mod main_background;
#[cfg(any(target_os = "windows", target_os = "android", test))]
mod playlist_source_sync;
mod remote_accounts;
mod remote_connections;
#[cfg(any(target_os = "android", test))]
#[cfg_attr(test, allow(dead_code))]
mod remote_media;
mod remote_playlist_sync;
mod remote_sources;
#[cfg(target_os = "windows")]
mod windows_data_directory;

#[cfg(target_os = "windows")]
use player_platform_windows::{
    find_artwork, is_supported_audio_file, read_standalone_audio_metadata, windows_locator_key,
    ArtworkLookup, WindowsMediaIndex,
};

#[cfg(target_os = "windows")]
use player_audio_windows::{
    system_media::{
        MediaControlMetadata, MediaControlUpdate, SystemMediaController, SystemMediaError,
        SystemMediaEvent,
    },
    AudioError, CommandTicket, PlaybackAccountingCheckpoint, PlaybackAccountingId,
    PlaybackState as AudioPlaybackState, PlayerHandle, ResamplingConversion, ResamplingInfo,
    ResamplingMode as AudioResamplingMode,
};

const LIBRARY_SYNC_FINISHED_EVENT: &str = "library-sync-finished";
const LIBRARY_SYNC_PROGRESS_EVENT: &str = "library-sync-progress";
const PLAYBACK_QUEUE_PAGE_MAX_LIMIT: u32 = 1000;
const SOURCE_SYNC_ERROR_DETAILS_LIMIT: usize = 100;
const ALBUM_ARTWORK_CACHE_DIRECTORY_NAME: &str = "album-cover";

fn album_artwork_cache_namespace(root: &std::path::Path) -> std::path::PathBuf {
    root.join(ALBUM_ARTWORK_CACHE_DIRECTORY_NAME)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThemePreferencesDto {
    background_hex: String,
    accent_hex: String,
    #[serde(default = "default_border_opacity_percent")]
    border_opacity_percent: u8,
    #[serde(default = "default_divider_opacity_percent")]
    divider_opacity_percent: u8,
    #[serde(default)]
    color_mode: ThemeColorMode,
    #[serde(default)]
    background_opacity_text_color: BackgroundOpacityTextColor,
    #[serde(default)]
    main_background_clarity_percent: u8,
    #[serde(default)]
    cover_background_clarity_percent: u8,
    #[serde(default = "default_quick_settings_opacity_percent")]
    quick_settings_opacity_percent: u8,
    #[serde(default = "default_quick_settings_blur_px")]
    quick_settings_blur_px: u8,
    #[serde(default = "default_main_background_blur_px")]
    main_background_blur_px: u8,
    #[serde(
        default = "default_main_background_opacity_percent",
        alias = "mainBackgroundBrightnessPercent"
    )]
    main_background_opacity_percent: u8,
    #[serde(default)]
    main_background_mode: MainBackgroundMode,
    #[serde(default)]
    slideshow_folder: Option<String>,
    #[serde(default = "default_slideshow_interval_minutes")]
    slideshow_interval_minutes: u8,
    #[serde(default)]
    slideshow_order: SlideshowOrder,
    #[serde(default = "default_auto_text_color_threshold_percent")]
    auto_text_color_threshold_percent: u8,
}

#[cfg(test)]
mod album_artwork_responsiveness_tests {
    use super::*;
    use base64::Engine as _;
    use std::{
        fs,
        path::PathBuf,
        sync::Arc,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    #[tokio::test(flavor = "current_thread")]
    async fn cold_and_warm_disk_thumbnail_lookup_keeps_async_caller_responsive() {
        let mut database = Database::open_in_memory().expect("isolated synthetic database");
        let root = database
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                "isolated-thumbnail-test",
                MediaLocator::FileSystem(PathBuf::from(r"C:\synthetic\music")),
            )
            .expect("synthetic library root");
        let record = MediaTrackRecord {
            identity: TrackIdentity {
                source_id: root.id,
                source_item_id: "thumbnail-track".to_owned(),
                locator_key: Some("synthetic-thumbnail-track".to_owned()),
            },
            locator: MediaLocator::FileSystem(PathBuf::from(r"C:\synthetic\music\track.flac")),
            fingerprint: FileFingerprint {
                size_bytes: 128,
                modified_at_utc_ms: Some(1_000),
                created_at_utc_ms: None,
            },
            metadata: Some(TrackMetadata {
                embedded_lyrics: None,
                title: Some("Synthetic Track".to_owned()),
                artist: Some("Synthetic Artist".to_owned()),
                album: Some("Synthetic Album".to_owned()),
                album_artist: None,
                track_number: Some(1),
                disc_number: Some(1),
                duration_ms: Some(180_000),
                codec: Some("FLAC".to_owned()),
                bitrate_bps: Some(900_000),
                sample_rate_hz: Some(48_000),
                release_date: None,
                year: None,
                bit_depth: Some(24),
            }),
        };
        database
            .apply_source_scan(
                &root,
                &SourceScanState::Complete,
                std::slice::from_ref(&record),
                std::slice::from_ref(&record),
                &[],
                2_000,
            )
            .expect("insert synthetic track");
        let track_id = database
            .resolve_track_id_for_locator(&record.locator)
            .expect("resolve synthetic track")
            .expect("track exists");
        let database = Arc::new(database);
        let identity = database
            .album_artwork_cache_identity(track_id)
            .expect("initial cache identity")
            .expect("enabled synthetic source mapping");
        let directory = std::env::temp_dir().join(format!(
            "moemusicplayer-album-artwork-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let gate = database_lifecycle::DatabaseWorkGate::default();

        async fn check_lookup(
            database: Arc<Database>,
            gate: database_lifecycle::DatabaseWorkGate,
            track_id: TrackId,
            directory: PathBuf,
            expected: Vec<u8>,
        ) {
            let started_work = Arc::new(AtomicBool::new(false));
            let worker_started = Arc::clone(&started_work);
            let worker_gate = gate.clone();
            let work = run_blocking_database_work(move || {
                run_guarded_database_work(&worker_gate, || {
                    worker_started.store(true, Ordering::Release);
                    let identity = database
                        .album_artwork_cache_identity(track_id)
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| "synthetic cache identity missing".to_owned())?;
                    // Hold an admitted native album job open after doing the real database lookup,
                    // so executor responsiveness and shutdown draining are deterministic.
                    std::thread::sleep(Duration::from_millis(120));
                    let bytes = album_artwork_cache::get_for_track(&directory, &identity)?;
                    if bytes != expected {
                        return Err(format!("unexpected thumbnail bytes: {}", bytes.len()));
                    }
                    Ok(())
                })
            });
            tokio::pin!(work);
            let first_poll = tokio::time::timeout(Duration::from_millis(10), &mut work).await;
            assert!(
                first_poll.is_err(),
                "slow album DB work should remain pending after its first poll"
            );
            while !started_work.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            let close_gate = gate.clone();
            let (close_tx, close_rx) = mpsc::channel();
            std::thread::spawn(move || {
                let closed = close_gate.close_and_wait(Duration::from_secs(2));
                let _ = close_tx.send(closed);
            });
            tokio::time::sleep(Duration::from_millis(25)).await;
            assert!(
                close_rx.try_recv().is_err(),
                "shutdown drain returned while the database and disk operation was still held open"
            );
            let mut heartbeats = 0;
            loop {
                tokio::select! {
                    result = &mut work => {
                        result.expect("album database work");
                        break;
                    }
                    _ = tokio::time::sleep(Duration::from_millis(10)) => heartbeats += 1,
                }
            }
            assert!(
                heartbeats >= 3,
                "the calling executor stalled during album artwork DB and disk I/O"
            );
            assert!(close_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("shutdown drain result"));
            assert!(
                !gate.is_accepting_work(),
                "shutdown gate should close after the database job drains"
            );
        }

        check_lookup(
            Arc::clone(&database),
            gate.clone(),
            track_id,
            directory.clone(),
            Vec::new(),
        )
        .await;

        let jpeg = vec![0xff, 0xd8, 0xff, 0x00];
        let encoded = base64::engine::general_purpose::STANDARD.encode(&jpeg);
        let digest = "a".repeat(64);
        album_artwork_cache::put(&directory, &digest, &encoded).expect("seed warm disk thumbnail");
        album_artwork_cache::put_track_reference(&directory, &identity, Some(&digest))
            .expect("seed warm track reference");
        check_lookup(
            database,
            database_lifecycle::DatabaseWorkGate::default(),
            track_id,
            directory.clone(),
            jpeg,
        )
        .await;
        album_artwork_cache::close_for_test(&directory);
        fs::remove_dir_all(&directory).expect("remove isolated thumbnail fixture");
    }

    #[test]
    fn album_cover_namespace_does_not_hit_or_remove_legacy_artwork_cache() {
        let root = std::env::temp_dir().join(format!(
            "moemusicplayer-album-cache-namespace-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let old_directory = root.join("album-artwork");
        let current_directory = album_artwork_cache_namespace(&root);
        let identity = "source-a\\0song-1\\01024\\0200";
        let digest = "d".repeat(64);
        let jpeg = vec![0xff, 0xd8, 0xff, 0x00];
        let encoded = base64::engine::general_purpose::STANDARD.encode(&jpeg);

        album_artwork_cache::put(&old_directory, &digest, &encoded).expect("seed old cache");
        album_artwork_cache::put_track_reference(&old_directory, identity, Some(&digest))
            .expect("seed old track reference");

        assert_eq!(current_directory, root.join("album-cover"));
        assert!(
            album_artwork_cache::get_for_track(&current_directory, identity)
                .expect("new namespace cache lookup")
                .is_empty()
        );
        assert_eq!(
            album_artwork_cache::get_for_track(&old_directory, identity)
                .expect("legacy cache remains readable in place"),
            jpeg
        );
        album_artwork_cache::close_for_test(&old_directory);
        album_artwork_cache::close_for_test(&current_directory);
        fs::remove_dir_all(root).expect("remove isolated namespace fixture");
    }
}

fn default_quick_settings_opacity_percent() -> u8 {
    70
}

fn default_quick_settings_blur_px() -> u8 {
    12
}

fn default_main_background_blur_px() -> u8 {
    20
}

fn default_main_background_opacity_percent() -> u8 {
    40
}

fn default_slideshow_interval_minutes() -> u8 {
    10
}

fn default_auto_text_color_threshold_percent() -> u8 {
    50
}

fn default_border_opacity_percent() -> u8 {
    100
}

fn default_divider_opacity_percent() -> u8 {
    100
}

impl From<ThemePreferences> for ThemePreferencesDto {
    fn from(preferences: ThemePreferences) -> Self {
        Self {
            background_hex: preferences.background_hex,
            accent_hex: preferences.accent_hex,
            border_opacity_percent: default_border_opacity_percent(),
            divider_opacity_percent: default_divider_opacity_percent(),
            color_mode: ThemeColorMode::default(),
            background_opacity_text_color: BackgroundOpacityTextColor::default(),
            main_background_clarity_percent: 0,
            cover_background_clarity_percent: 0,
            quick_settings_opacity_percent: default_quick_settings_opacity_percent(),
            quick_settings_blur_px: default_quick_settings_blur_px(),
            main_background_blur_px: default_main_background_blur_px(),
            main_background_opacity_percent: default_main_background_opacity_percent(),
            main_background_mode: MainBackgroundMode::Solid,
            slideshow_folder: None,
            slideshow_interval_minutes: default_slideshow_interval_minutes(),
            slideshow_order: SlideshowOrder::default(),
            auto_text_color_threshold_percent: default_auto_text_color_threshold_percent(),
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
            border_opacity_percent: preferences.border_opacity_percent,
            divider_opacity_percent: preferences.divider_opacity_percent,
            color_mode: preferences.color_mode,
            background_opacity_text_color: preferences.background_opacity_text_color,
            main_background_clarity_percent: preferences.main_background_clarity_percent,
            cover_background_clarity_percent: preferences.cover_background_clarity_percent,
            quick_settings_opacity_percent: preferences.quick_settings_opacity_percent,
            quick_settings_blur_px: preferences.quick_settings_blur_px,
            main_background_blur_px: preferences.main_background_blur_px,
            main_background_opacity_percent: preferences.main_background_opacity_percent,
            main_background_mode: preferences.main_background_mode,
            slideshow_folder: preferences
                .slideshow_folder
                .as_ref()
                .map(settings::StoredPath::display_lossy),
            slideshow_interval_minutes: preferences.slideshow_interval_minutes,
            slideshow_order: preferences.slideshow_order,
            auto_text_color_threshold_percent: preferences.auto_text_color_threshold_percent,
        }
    }
}

#[cfg(target_os = "windows")]
#[derive(Debug, Default)]
struct WindowFrameMemory {
    /// Last non-maximized outer position + inner size.
    normal: Option<WindowGeometry>,
}

struct AppState {
    database: Option<Database>,
    database_path: Option<std::path::PathBuf>,
    database_error: Option<String>,
    database_work: database_lifecycle::DatabaseWorkGate,
    queue_display_sort_cache: std::sync::Mutex<Option<QueueDisplaySortCache>>,
    /// The library sync in progress, so closing the program can stop its reading and save what it has read.
    active_sync: std::sync::Mutex<Option<SyncCancellation>>,
    #[cfg(target_os = "windows")]
    shutdown: database_lifecycle::ShutdownCoordinator,
    settings: Arc<SettingsStore>,
    main_background: main_background::MainBackgroundRuntime,
    #[cfg(target_os = "windows")]
    window_frame: std::sync::Mutex<WindowFrameMemory>,
    lyrics_service: lyrics_service::LyricsService,
    /// Open connections to remote servers (ADR 0026), shared by scanning, tag reads and playback.
    remote_connections: Arc<remote_connections::RemoteConnections>,
    /// The disk cache of played remote files (first in, first out).
    remote_cache: Option<Arc<player_remote::cache::DiskCache>>,
    #[cfg(target_os = "windows")]
    playback: Option<WindowsPlaybackService>,
    #[cfg(target_os = "windows")]
    playback_error: Option<String>,
    #[cfg(target_os = "windows")]
    system_media: Option<WindowsSystemMediaService>,
    #[cfg(target_os = "windows")]
    system_media_error: Option<String>,
    #[cfg(target_os = "android")]
    android_playback: Option<android_playback::AndroidPlaybackService>,
    #[cfg(target_os = "android")]
    android_playback_error: Option<String>,
}

struct QueueDisplaySortCache {
    revision: u64,
    sort: TrackListSort,
    order: Vec<usize>,
}

enum LyricsDatabase<'a> {
    Library(&'a Database),
    Temporary(Arc<Database>),
}

impl LyricsDatabase<'_> {
    fn as_database(&self) -> &Database {
        match self {
            Self::Library(database) => database,
            Self::Temporary(database) => database,
        }
    }
}

fn lyrics_database_for_track(
    state: &AppState,
    track_id: TrackId,
) -> Result<LyricsDatabase<'_>, String> {
    #[cfg(target_os = "windows")]
    let temporary_database = state
        .playback
        .as_ref()
        .and_then(|playback| playback.temporary_database_for_track(track_id));
    #[cfg(not(target_os = "windows"))]
    let temporary_database = None;
    lyrics_database_from_databases(
        state.database.as_ref(),
        temporary_database,
        track_id,
        state.database_error.as_deref(),
    )
}

fn lyrics_database_from_databases<'a>(
    library: Option<&'a Database>,
    temporary: Option<Arc<Database>>,
    track_id: TrackId,
    database_error: Option<&str>,
) -> Result<LyricsDatabase<'a>, String> {
    if let Some(database) = temporary {
        if database
            .get_track_summary(track_id)
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Ok(LyricsDatabase::Temporary(database));
        }
    }
    library
        .map(LyricsDatabase::Library)
        .ok_or_else(|| database_error.unwrap_or("曲庫資料庫尚未開啟。").to_owned())
}

fn enter_database_work(state: &AppState) -> Result<database_lifecycle::DatabaseWorkGuard, String> {
    state
        .database_work
        .try_enter()
        .ok_or_else(|| "應用程式正在關閉，暫時無法開始新的資料庫工作。".to_owned())
}

#[cfg(target_os = "android")]
fn android_playback_service(
    state: &AppState,
) -> Result<&android_playback::AndroidPlaybackService, String> {
    state.android_playback.as_ref().ok_or_else(|| {
        state
            .android_playback_error
            .clone()
            .unwrap_or_else(|| "Android MediaSession 播放服務尚未啟動。".to_owned())
    })
}

#[cfg(target_os = "windows")]
struct WindowsPlaybackService {
    player: PlayerHandle,
    accounting_track_ids: Arc<Mutex<HashMap<PlaybackAccountingId, AccountingTrack>>>,
    statistics_runtime_id: Mutex<Option<uuid::Uuid>>,
    statistics_status: Arc<Mutex<Option<String>>>,
    statistics_collector: Mutex<Option<PlaybackStatisticsCollector>>,
    current_track: Mutex<Option<TrackSummary>>,
    command_gate: Mutex<()>,
    queue: Mutex<Option<PlaybackQueue>>,
    external_tracks: Mutex<HashMap<TrackId, ExternalPlaybackTrack>>,
    temporary_catalog: Mutex<Option<Arc<Database>>>,
    external_queue_active: AtomicBool,
    queue_revision: AtomicU64,
    repeat_mode: Mutex<RepeatMode>,
    shuffle: Mutex<bool>,
    queue_advance_pending: Mutex<bool>,
    last_position_checkpoint: Mutex<Instant>,
    restore_warning: Mutex<Option<String>>,
}

#[cfg(target_os = "windows")]
#[derive(Clone)]
struct AccountingTrack {
    track_id: TrackId,
    temporary_database: Option<Arc<Database>>,
}

#[cfg(target_os = "windows")]
#[derive(Clone)]
struct ExternalPlaybackTrack {
    path: std::path::PathBuf,
    summary: TrackSummary,
    transient: bool,
}

#[cfg(target_os = "windows")]
struct PlaybackStatisticsCollector {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[cfg(target_os = "windows")]
struct PreparedTrackChange {
    ticket: CommandTicket,
    track: TrackSummary,
    replacement_queue: Option<PlaybackQueue>,
    target_cursor: Option<usize>,
}

#[cfg(target_os = "windows")]
struct WindowsSystemMediaService {
    controller: Mutex<Option<SystemMediaController>>,
    status: Mutex<WindowsSystemMediaStatus>,
    pump_gate: Mutex<()>,
    last_update: Mutex<Option<Instant>>,
    artwork: Mutex<ArtworkPump>,
    published_metadata: Mutex<Option<MediaControlMetadata>>,
}

#[cfg(target_os = "windows")]
struct ArtworkRequest {
    track_id: TrackId,
    locators: Vec<MediaLocator>,
}

#[cfg(target_os = "windows")]
struct ArtworkResult {
    track_id: TrackId,
    bytes: Option<Arc<[u8]>>,
}

#[cfg(target_os = "windows")]
#[derive(Clone, Debug, PartialEq, Eq)]
enum ArtworkAvailability {
    Pending,
    Ready(Option<Arc<[u8]>>),
}

#[cfg(target_os = "windows")]
#[derive(Clone, Debug, PartialEq, Eq)]
enum MetadataPublication {
    Hold,
    Set(Option<MediaControlMetadata>),
}

#[cfg(target_os = "windows")]
struct ArtworkReader {
    requests: Option<SyncSender<ArtworkRequest>>,
    results: Arc<Mutex<Option<ArtworkResult>>>,
    worker: Option<std::thread::JoinHandle<()>>,
    stopped: Arc<std::sync::atomic::AtomicBool>,
    worker_done: Receiver<()>,
}

#[cfg(target_os = "windows")]
struct ArtworkPump {
    reader: ArtworkReader,
    active_track: Option<TrackId>,
    pending: Option<ArtworkRequest>,
    image: Option<Arc<[u8]>>,
    lookup_complete: bool,
    stopped: bool,
}

#[cfg(target_os = "windows")]
impl ArtworkReader {
    fn new() -> Self {
        let (request_tx, request_rx) = mpsc::sync_channel::<ArtworkRequest>(1);
        let results = Arc::new(Mutex::new(None::<ArtworkResult>));
        let worker_results = Arc::clone(&results);
        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_stopped = Arc::clone(&stopped);
        let (done_tx, worker_done) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("moemusicplayer-smtc-artwork".to_owned())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    let bytes = match find_artwork(&request.locators) {
                        ArtworkLookup::Found(image) => Some(Arc::<[u8]>::from(image.into_bytes())),
                        ArtworkLookup::Missing | ArtworkLookup::Oversized => None,
                    };
                    if !worker_stopped.load(Ordering::Acquire) {
                        *worker_results
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            Some(ArtworkResult {
                                track_id: request.track_id,
                                bytes,
                            })
                    }
                }
                let _ = done_tx.send(());
            })
            .ok();
        Self {
            requests: worker.as_ref().map(|_| request_tx),
            results,
            worker,
            stopped,
            worker_done,
        }
    }

    fn shutdown(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            if self
                .worker_done
                .recv_timeout(Duration::from_millis(250))
                .is_ok()
            {
                let _ = worker.join();
            }
            // A filesystem call can stall on an unavailable network volume.
            // Keep shutdown bounded; the single worker exits and drops its
            // handles once that OS call returns.
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for ArtworkReader {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(target_os = "windows")]
fn metadata_publication(
    mut current: Option<MediaControlMetadata>,
    artwork: ArtworkAvailability,
) -> MetadataPublication {
    let Some(metadata) = current.as_mut() else {
        return MetadataPublication::Set(None);
    };
    match artwork {
        ArtworkAvailability::Pending => MetadataPublication::Hold,
        ArtworkAvailability::Ready(thumbnail) => {
            metadata.thumbnail = thumbnail;
            MetadataPublication::Set(current)
        }
    }
}

#[cfg(target_os = "windows")]
fn metadata_for_update(
    publication: &MetadataPublication,
    published: &Option<MediaControlMetadata>,
) -> Option<MediaControlMetadata> {
    match publication {
        MetadataPublication::Hold => published.clone(),
        MetadataPublication::Set(metadata) => metadata.clone(),
    }
}

#[cfg(target_os = "windows")]
fn commit_metadata_publication(
    publication: MetadataPublication,
    published: &mut Option<MediaControlMetadata>,
) {
    if let MetadataPublication::Set(metadata) = publication {
        *published = metadata;
    }
}

#[cfg(target_os = "windows")]
fn publish_metadata_update<E>(
    publication: MetadataPublication,
    published: &mut Option<MediaControlMetadata>,
    update: impl FnOnce(Option<MediaControlMetadata>) -> Result<(), E>,
) -> Result<(), E> {
    update(metadata_for_update(&publication, published))?;
    commit_metadata_publication(publication, published);
    Ok(())
}

#[cfg(target_os = "windows")]
impl ArtworkPump {
    fn new() -> Self {
        Self {
            reader: ArtworkReader::new(),
            active_track: None,
            pending: None,
            image: None,
            lookup_complete: false,
            stopped: false,
        }
    }

    /// Forget the cover of the active track so the next update reads it again (its file's tags were edited).
    fn invalidate(&mut self) {
        self.active_track = None;
        self.pending = None;
        self.image = None;
        self.lookup_complete = false;
    }

    #[cfg(test)]
    fn update(
        &mut self,
        track_id: Option<TrackId>,
        database: Option<&Database>,
    ) -> ArtworkAvailability {
        self.update_with_external_locators(track_id, database, None)
    }

    fn update_with_external_locators(
        &mut self,
        track_id: Option<TrackId>,
        database: Option<&Database>,
        external_locators: Option<Vec<MediaLocator>>,
    ) -> ArtworkAvailability {
        if self.stopped {
            return ArtworkAvailability::Ready(None);
        }
        let result = self
            .reader
            .results
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(result) = result {
            if Some(result.track_id) == self.active_track {
                self.image = result.bytes;
                self.lookup_complete = true;
            }
        }

        if track_id != self.active_track {
            self.active_track = track_id;
            self.pending = None;
            self.image = None;
            self.lookup_complete = track_id.is_none() || database.is_none();
            if let Some(track_id) = track_id {
                let locators = external_locators.or_else(|| {
                    database.and_then(|database| database.track_locators(track_id).ok())
                });
                match locators {
                    Some(locators) => {
                        self.pending = Some(ArtworkRequest { track_id, locators });
                        self.lookup_complete = false;
                    }
                    None => self.lookup_complete = true,
                }
            }
        }

        if let Some(request) = self.pending.take() {
            if let Some(sender) = self.reader.requests.as_ref() {
                match sender.try_send(request) {
                    Ok(()) => {}
                    Err(TrySendError::Full(request)) => self.pending = Some(request),
                    Err(TrySendError::Disconnected(_)) => self.lookup_complete = true,
                }
            }
        }
        if self.lookup_complete {
            ArtworkAvailability::Ready(self.image.clone())
        } else {
            ArtworkAvailability::Pending
        }
    }

    fn shutdown(&mut self) {
        self.stopped = true;
        self.pending = None;
        self.image = None;
        self.lookup_complete = true;
        self.reader.shutdown();
    }
}

#[cfg(all(test, target_os = "windows"))]
mod system_media_artwork_tests {
    use super::{
        metadata_publication, publish_metadata_update, ArtworkAvailability, ArtworkPump,
        ArtworkRequest, ArtworkResult,
    };
    use player_audio_windows::system_media::MediaControlMetadata;
    use player_core::{
        FileFingerprint, LibraryRepository, ListTracksQuery, MediaLocator, MediaSourceKind,
        MediaTrackRecord, SourceScanState, TrackId, TrackIdentity, TrackMetadata,
    };
    use player_db::Database;
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };

    #[derive(Default)]
    struct FakeMetadataSink {
        fail_next: bool,
        updates: Vec<(u64, Option<MediaControlMetadata>)>,
    }

    impl FakeMetadataSink {
        fn update(
            &mut self,
            timeline_revision: u64,
            metadata: Option<MediaControlMetadata>,
        ) -> Result<(), ()> {
            if std::mem::take(&mut self.fail_next) {
                return Err(());
            }
            self.updates.push((timeline_revision, metadata));
            Ok(())
        }
    }

    fn metadata(title: &str, thumbnail: Option<&'static [u8]>) -> MediaControlMetadata {
        MediaControlMetadata {
            title: Some(title.to_owned()),
            artist: Some(format!("{title} artist")),
            album: Some(format!("{title} album")),
            thumbnail: thumbnail.map(Arc::from),
        }
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "moe-smtc-artwork-{}-{}",
                std::process::id(),
                TrackId::new()
            ));
            fs::create_dir_all(&path).expect("create isolated artwork directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_png(rgb: [u8; 3]) -> Vec<u8> {
        fn chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            output.extend_from_slice(&(data.len() as u32).to_be_bytes());
            output.extend_from_slice(kind);
            output.extend_from_slice(data);
            let crc = kind.iter().chain(data).fold(!0_u32, |crc, byte| {
                let mut value = crc ^ u32::from(*byte);
                for _ in 0..8 {
                    value = if value & 1 == 1 {
                        (value >> 1) ^ 0xedb8_8320
                    } else {
                        value >> 1
                    };
                }
                value
            });
            output.extend_from_slice(&(!crc).to_be_bytes());
        }

        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        chunk(&mut png, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
        let scanline = [0, rgb[0], rgb[1], rgb[2]];
        let mut zlib = vec![0x78, 0x01, 0x01, 4, 0, 0xfb, 0xff];
        zlib.extend_from_slice(&scanline);
        let (mut a, mut b) = (1_u32, 0_u32);
        for byte in scanline {
            a = (a + u32::from(byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());
        chunk(&mut png, b"IDAT", &zlib);
        chunk(&mut png, b"IEND", &[]);
        png
    }

    fn record(root: &player_core::LibraryRoot, path: &Path, title: &str) -> MediaTrackRecord {
        MediaTrackRecord {
            identity: TrackIdentity {
                source_id: root.id,
                source_item_id: title.to_owned(),
                locator_key: None,
            },
            locator: MediaLocator::FileSystem(path.to_path_buf()),
            fingerprint: FileFingerprint {
                size_bytes: 1,
                modified_at_utc_ms: Some(1),
                created_at_utc_ms: None,
            },
            metadata: Some(TrackMetadata {
                title: Some(title.to_owned()),
                artist: Some("test artist".to_owned()),
                album: Some("test album".to_owned()),
                ..TrackMetadata::default()
            }),
        }
    }

    fn wait_for_result(pump: &ArtworkPump, track_id: TrackId) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let ready = pump
                .reader
                .results
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .is_some_and(|result| result.track_id == track_id);
            if ready {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for artwork result"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn metadata_publication_holds_pending_then_atomically_sets_or_clears_artwork() {
        let previous = Some(metadata("Track A", Some(b"cover-a")));
        let mut published = previous.clone();
        let mut sink = FakeMetadataSink::default();

        let pending_b = metadata_publication(
            Some(metadata("Track B", None)),
            ArtworkAvailability::Pending,
        );
        publish_metadata_update(pending_b, &mut published, |outgoing| {
            sink.update(2, outgoing)
        })
        .expect("pending artwork still updates timeline");
        assert_eq!(sink.updates[0].0, 2);
        assert_eq!(sink.updates[0].1, published);
        assert_eq!(
            sink.updates[0].1.as_ref().unwrap().title.as_deref(),
            Some("Track A")
        );

        let ready_b = metadata_publication(
            Some(metadata("Track B", None)),
            ArtworkAvailability::Ready(Some(Arc::from(&b"cover-b"[..]))),
        );
        publish_metadata_update(ready_b, &mut published, |outgoing| sink.update(3, outgoing))
            .expect("ready metadata and artwork publish together");
        assert_eq!(sink.updates[1].0, 3);
        assert_eq!(sink.updates[1].1, published);
        assert_eq!(
            published.as_ref().unwrap().title.as_deref(),
            Some("Track B")
        );
        assert_eq!(
            published.as_ref().unwrap().thumbnail.as_deref(),
            Some(&b"cover-b"[..])
        );

        let missing_c = metadata_publication(
            Some(metadata("Track C", None)),
            ArtworkAvailability::Ready(None),
        );
        publish_metadata_update(missing_c, &mut published, |outgoing| {
            sink.update(4, outgoing)
        })
        .expect("missing artwork publishes new metadata and clears previous cover");
        assert_eq!(sink.updates[2].0, 4);
        assert_eq!(published, Some(metadata("Track C", None)));
        assert_eq!(sink.updates[2].1, published);
    }

    #[test]
    fn metadata_sink_error_does_not_commit_and_stale_artwork_keeps_previous_metadata() {
        let track_a = TrackId::new();
        let track_b = TrackId::new();
        let mut pump = ArtworkPump::new();
        pump.active_track = Some(track_b);
        pump.lookup_complete = false;
        pump.reader
            .results
            .lock()
            .expect("result slot")
            .replace(ArtworkResult {
                track_id: track_a,
                bytes: Some(Arc::from(&b"stale-cover"[..])),
            });
        assert_eq!(
            pump.update(Some(track_b), None),
            ArtworkAvailability::Pending,
            "a late result from the previous track must not complete the current lookup"
        );
        assert!(pump.image.is_none());

        let previous = Some(metadata("Track A", Some(b"cover-a")));
        let mut published = previous.clone();
        let mut sink = FakeMetadataSink {
            fail_next: true,
            ..FakeMetadataSink::default()
        };
        let ready_b = metadata_publication(
            Some(metadata("Track B", None)),
            ArtworkAvailability::Ready(Some(Arc::from(&b"cover-b"[..]))),
        );
        assert!(
            publish_metadata_update(ready_b.clone(), &mut published, |outgoing| {
                sink.update(2, outgoing)
            })
            .is_err()
        );
        assert_eq!(published, previous);

        let stale_pending = metadata_publication(
            Some(metadata("Track B", None)),
            ArtworkAvailability::Pending,
        );
        publish_metadata_update(stale_pending, &mut published, |outgoing| {
            sink.update(3, outgoing)
        })
        .expect("pending or stale artwork preserves the last complete metadata");
        assert_eq!(sink.updates[0].0, 3);
        assert_eq!(sink.updates[0].1, published);

        publish_metadata_update(ready_b, &mut published, |outgoing| sink.update(4, outgoing))
            .expect("retry can commit after a transient sink error");
        assert_eq!(
            published.as_ref().unwrap().title.as_deref(),
            Some("Track B")
        );
        assert_eq!(
            published.as_ref().unwrap().thumbnail.as_deref(),
            Some(&b"cover-b"[..])
        );
        pump.shutdown();
    }

    #[test]
    fn stale_track_result_is_discarded_and_latest_track_request_is_serviced() {
        let mut pump = ArtworkPump::new();
        let track_a = TrackId::new();
        let track_b = TrackId::new();
        pump.active_track = Some(track_a);
        *pump.reader.results.lock().expect("result slot") = Some(ArtworkResult {
            track_id: track_a,
            bytes: Some(Arc::from(&b"old"[..])),
        });

        assert_eq!(
            pump.update(Some(track_b), None),
            ArtworkAvailability::Ready(None)
        );
        assert!(pump.image.is_none());
        pump.pending = Some(ArtworkRequest {
            track_id: track_b,
            locators: Vec::new(),
        });
        pump.lookup_complete = false;
        assert_eq!(
            pump.update(Some(track_b), None),
            ArtworkAvailability::Pending
        );
        wait_for_result(&pump, track_b);
        assert_eq!(
            pump.update(Some(track_b), None),
            ArtworkAvailability::Ready(None)
        );
        assert!(pump.image.is_none());
        pump.shutdown();
    }

    #[test]
    fn missing_art_clears_previous_image_and_same_track_does_not_requeue() {
        let mut pump = ArtworkPump::new();
        let track = TrackId::new();
        pump.active_track = Some(track);
        pump.image = Some(Arc::from(&b"previous cover"[..]));
        pump.lookup_complete = false;
        pump.reader
            .results
            .lock()
            .expect("result slot")
            .replace(ArtworkResult {
                track_id: track,
                bytes: None,
            });

        assert_eq!(
            pump.update(Some(track), None),
            ArtworkAvailability::Ready(None)
        );
        assert!(pump.image.is_none());
        for _ in 0..3 {
            assert_eq!(
                pump.update(Some(track), None),
                ArtworkAvailability::Ready(None)
            );
        }
        assert!(pump.pending.is_none());
        pump.shutdown();
    }

    #[test]
    fn shutdown_with_a_full_result_slot_joins_idle_reader() {
        let mut pump = ArtworkPump::new();
        *pump.reader.results.lock().expect("result slot") = Some(ArtworkResult {
            track_id: TrackId::new(),
            bytes: None,
        });
        let started = Instant::now();
        pump.shutdown();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(pump.reader.worker.is_none());
    }

    #[test]
    fn current_track_reader_uses_enabled_database_locators_for_two_covers_then_missing() {
        let directory = TestDirectory::new();
        let mut database = Database::open_in_memory().expect("isolated in-memory library");
        let root = database
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                "isolated test source",
                MediaLocator::FileSystem(directory.0.clone()),
            )
            .expect("register isolated source");
        let path_a = directory.0.join("A").join("track-a.mp3");
        let path_b = directory.0.join("B").join("track-b.mp3");
        let path_c = directory.0.join("C").join("track-c.mp3");
        for path in [&path_a, &path_b, &path_c] {
            fs::create_dir_all(path.parent().expect("track parent"))
                .expect("create isolated album directory");
        }
        let bytes_a = test_png([255, 0, 0]);
        let bytes_b = test_png([0, 0, 255]);
        fs::write(path_a.parent().unwrap().join("cover.jpg"), &bytes_a)
            .expect("write A artwork sidecar");
        fs::write(path_b.parent().unwrap().join("cover.jpg"), &bytes_b)
            .expect("write B artwork sidecar");
        let records = [
            record(&root, &path_a, "Track A"),
            record(&root, &path_b, "Track B"),
            record(&root, &path_c, "Track C"),
        ];
        database
            .apply_source_scan(
                &root,
                &SourceScanState::Complete,
                &records,
                &records,
                &[],
                1_800_000_000_000,
            )
            .expect("persist fixture tracks and locators");
        let tracks = database
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
                field_filter: None,
                sort: Default::default(),
            })
            .expect("list fixture tracks")
            .items;
        let id_for = |title: &str| {
            tracks
                .iter()
                .find(|track| track.title.as_deref() == Some(title))
                .expect("find fixture track")
                .id
        };
        let (track_a, track_b, track_c) = (id_for("Track A"), id_for("Track B"), id_for("Track C"));
        let mut pump = ArtworkPump::new();
        assert_eq!(
            pump.update(Some(track_a), Some(&database)),
            ArtworkAvailability::Pending
        );
        wait_for_result(&pump, track_a);
        let ArtworkAvailability::Ready(Some(image_a)) = pump.update(Some(track_a), Some(&database))
        else {
            panic!("track A artwork should be ready");
        };
        assert_eq!(image_a.as_ref(), bytes_a.as_slice());

        assert_eq!(
            pump.update(Some(track_b), Some(&database)),
            ArtworkAvailability::Pending
        );
        wait_for_result(&pump, track_b);
        let ArtworkAvailability::Ready(Some(image_b)) = pump.update(Some(track_b), Some(&database))
        else {
            panic!("track B artwork should be ready");
        };
        assert_eq!(image_b.as_ref(), bytes_b.as_slice());

        assert_eq!(
            pump.update(Some(track_c), Some(&database)),
            ArtworkAvailability::Pending
        );
        wait_for_result(&pump, track_c);
        assert_eq!(
            pump.update(Some(track_c), Some(&database)),
            ArtworkAvailability::Ready(None)
        );
        assert!(pump.image.is_none(), "track C without artwork clears B");
        pump.shutdown();
    }
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
            accounting_track_ids: Arc::new(Mutex::new(HashMap::new())),
            statistics_runtime_id: Mutex::new(None),
            statistics_status: Arc::new(Mutex::new(None)),
            statistics_collector: Mutex::new(None),
            current_track: Mutex::new(None),
            command_gate: Mutex::new(()),
            queue: Mutex::new(None),
            external_tracks: Mutex::new(HashMap::new()),
            temporary_catalog: Mutex::new(None),
            external_queue_active: AtomicBool::new(false),
            queue_revision: AtomicU64::new(0),
            repeat_mode: Mutex::new(repeat_mode),
            shuffle: Mutex::new(shuffle),
            queue_advance_pending: Mutex::new(false),
            last_position_checkpoint: Mutex::new(Instant::now()),
            restore_warning: Mutex::new(None),
        }
    }

    fn accounting_id_for(&self, track_id: TrackId) -> PlaybackAccountingId {
        let id = PlaybackAccountingId::new(track_id.as_uuid().as_u128());
        self.accounting_track_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                id,
                AccountingTrack {
                    track_id,
                    temporary_database: None,
                },
            );
        id
    }

    fn accounting_id_for_temporary(
        &self,
        track_id: TrackId,
        database: Arc<Database>,
    ) -> PlaybackAccountingId {
        let id = PlaybackAccountingId::new(track_id.as_uuid().as_u128());
        self.accounting_track_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                id,
                AccountingTrack {
                    track_id,
                    temporary_database: Some(database),
                },
            );
        id
    }

    fn temporary_database_for_track(&self, track_id: TrackId) -> Option<Arc<Database>> {
        let is_transient = self
            .external_tracks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&track_id)
            .is_some_and(|track| track.transient);
        is_transient.then(|| {
            self.temporary_catalog
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .cloned()
        })?
    }

    fn start_statistics_collector(&self, database_path: &std::path::Path) -> Result<(), String> {
        let database = Database::open(database_path)
            .map_err(|error| format!("開啟播放時長統計連線失敗：{error}"))?;
        cleanup_stale_statistics_runtimes(&database);

        let runtime_marker = TrackId::new();
        let runtime_id = runtime_marker.as_uuid();
        let owner_pid = std::process::id();
        let owner_process_started_utc_ms = player_audio_windows::process_started_utc_ms(owner_pid)
            .ok()
            .flatten();
        database
            .register_playback_statistics_runtime(
                runtime_id,
                Some(owner_pid),
                owner_process_started_utc_ms,
            )
            .map_err(|error| format!("登記播放時長統計工作階段失敗：{error}"))?;
        *self
            .statistics_runtime_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(runtime_id);

        let player = self.player.clone();
        let track_ids = Arc::clone(&self.accounting_track_ids);
        let status = Arc::clone(&self.statistics_status);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = std::thread::Builder::new()
            .name("moe-playback-statistics".to_owned())
            .spawn(move || {
                playback_statistics_worker(
                    database,
                    player,
                    track_ids,
                    status,
                    worker_stop,
                    runtime_marker,
                );
            })
            .map_err(|error| format!("啟動播放時長統計工作失敗：{error}"))?;
        *self
            .statistics_collector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(PlaybackStatisticsCollector {
                stop,
                worker: Some(worker),
            });
        Ok(())
    }

    fn shutdown_statistics_collector(&self) {
        let collector = self
            .statistics_collector
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(mut collector) = collector {
            collector.stop.store(true, Ordering::Release);
            self.player.request_playback_accounting_flush();
            if let Some(worker) = collector.worker.take() {
                if worker.join().is_err() {
                    *self
                        .statistics_status
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(
                        "播放時長統計工作意外停止；已提交統計保留，未保存增量可能遺失。".to_owned(),
                    );
                }
            }
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
        let queue_sort = queue.as_ref().and_then(PlaybackQueue::queue_sort);
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
            last_error: audio
                .last_error
                .as_ref()
                .map(playback_audio_error_message)
                .or_else(|| {
                    self.restore_warning
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone()
                })
                .or_else(|| {
                    self.statistics_status
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone()
                }),
            repeat_mode,
            shuffle,
            queue_sort,
            can_next,
            can_previous,
            output_format: audio.resampling.as_ref().map(playback_output_format),
        }
    }
}

#[cfg(target_os = "windows")]
fn playback_output_format(info: &ResamplingInfo) -> PlaybackOutputFormat {
    PlaybackOutputFormat {
        mode: match info.mode {
            AudioResamplingMode::WindowsBuiltin => ResamplingMode::WindowsBuiltin,
            AudioResamplingMode::HighQuality => ResamplingMode::HighQuality,
        },
        source_rate_hz: info.source_rate,
        output_rate_hz: info.output_rate,
        device_rate_hz: info.device_rate,
        conversion: match info.conversion {
            ResamplingConversion::None => OutputConversion::None,
            ResamplingConversion::HighQuality => OutputConversion::HighQuality,
            ResamplingConversion::Windows => OutputConversion::Windows,
            ResamplingConversion::Basic => OutputConversion::Basic,
        },
        fallback_reason: info.fallback_reason.clone(),
        dsee_hx: match info.dsee_hx {
            player_audio_windows::DseeHxState::Off => DseeHxActivity::Off,
            player_audio_windows::DseeHxState::Active => DseeHxActivity::Active,
            player_audio_windows::DseeHxState::Bypassed => DseeHxActivity::Bypassed,
            player_audio_windows::DseeHxState::Unavailable => DseeHxActivity::Unavailable,
        },
        dsee_notice: info.dsee_notice.clone(),
        dsee_output_rate_hz: info.dsee_output_rate,
        exclusive: info.exclusive.as_ref().map(|exclusive| ExclusiveOutput {
            bits: exclusive.bits,
            channels: exclusive.channels,
            source_channels: exclusive.source_channels,
            bit_perfect: exclusive.bit_perfect,
            volume_applied: exclusive.volume_applied,
            underruns: exclusive.underruns,
            dop_dsd_rate_hz: exclusive.dop_dsd_rate,
            dop_fallback: exclusive.dop_fallback.clone(),
        }),
        exclusive_fallback: info.exclusive_fallback.clone(),
    }
}

#[cfg(target_os = "windows")]
fn cleanup_stale_statistics_runtimes(database: &Database) {
    let runtimes = match database.list_playback_statistics_runtimes() {
        Ok(runtimes) => runtimes,
        Err(error) => {
            eprintln!("無法檢查過期播放統計工作階段：{error}");
            return;
        }
    };
    for runtime in runtimes {
        let Some(pid) = runtime.owner_pid else {
            continue;
        };
        let stale = match player_audio_windows::process_started_utc_ms(pid) {
            Ok(None) => true,
            Ok(Some(actual_started)) => runtime
                .owner_process_started_utc_ms
                .is_some_and(|saved_started| saved_started != actual_started),
            Err(error) => {
                eprintln!("無法確認播放統計工作階段 PID {pid}：{error}");
                false
            }
        };
        if stale {
            if let Err(error) = database.finish_playback_statistics_runtime(runtime.runtime_id, &[])
            {
                eprintln!("無法清理已結束的播放統計工作階段：{error}");
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn playback_statistics_worker(
    database: Database,
    player: PlayerHandle,
    accounting_track_ids: Arc<Mutex<HashMap<PlaybackAccountingId, AccountingTrack>>>,
    status: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    runtime_marker: TrackId,
) {
    let runtime_id = runtime_marker.as_uuid();
    let mut shutdown_failures = 0u8;
    loop {
        let shutting_down = stop.load(Ordering::Acquire);
        if !shutting_down {
            player.wait_for_playback_accounting_flush(Duration::from_secs(5));
        }
        let shutting_down = stop.load(Ordering::Acquire);
        if shutting_down {
            match player
                .request_pause()
                .and_then(|ticket| ticket.wait(Duration::from_secs(2)))
            {
                Ok(_) => {}
                Err(error) => {
                    set_statistics_status(
                        &status,
                        Some(format!(
                            "關閉時無法確認音訊已暫停；記憶體中的未保存時長可能遺失：{error}"
                        )),
                    );
                    return;
                }
            }
        }
        let audio_checkpoints = player.playback_accounting_checkpoints();
        let track_map = accounting_track_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut library_group = (Vec::new(), Vec::new());
        let mut temporary_groups: HashMap<usize, (Arc<Database>, Vec<PlaybackCheckpoint>, Vec<_>)> =
            HashMap::new();
        let mut unmapped = false;
        for checkpoint in &audio_checkpoints {
            if let Some(target) = track_map.get(&checkpoint.id) {
                let database_checkpoint = PlaybackCheckpoint {
                    track_id: target.track_id,
                    played_ms: checkpoint.played_ms,
                    duration_ms: checkpoint.duration_ms,
                };
                if let Some(temporary_database) = target.temporary_database.as_ref() {
                    let key = Arc::as_ptr(temporary_database) as usize;
                    let group = temporary_groups.entry(key).or_insert_with(|| {
                        (Arc::clone(temporary_database), Vec::new(), Vec::new())
                    });
                    group.1.push(database_checkpoint);
                    group.2.push(*checkpoint);
                } else {
                    library_group.0.push(database_checkpoint);
                    library_group.1.push(*checkpoint);
                }
            } else {
                unmapped = true;
            }
        }
        drop(track_map);

        if unmapped {
            set_statistics_status(
                &status,
                Some("部分播放時長仍待保存：找不到對應的曲目識別。".to_owned()),
            );
        }
        if audio_checkpoints.is_empty() && !unmapped && !shutting_down {
            continue;
        }

        let mut retry = false;
        if !library_group.0.is_empty() {
            match database.record_playback_checkpoints(runtime_id, &library_group.0) {
                Ok(()) => acknowledge_statistics_batch(&player, &library_group.1),
                Err(error) => {
                    retry = true;
                    set_statistics_status(
                        &status,
                        Some(format!("播放時長檢查點仍待保存，背景工作會重試：{error}")),
                    );
                    eprintln!("正式曲庫播放時長仍待保存：{error}");
                }
            }
        }
        for (_, (temporary_database, checkpoints, audio)) in temporary_groups {
            if checkpoints.is_empty() {
                continue;
            }
            match temporary_database.record_playback_checkpoints(runtime_id, &checkpoints) {
                Ok(()) => acknowledge_statistics_batch(&player, &audio),
                Err(error) => {
                    retry = true;
                    set_statistics_status(
                        &status,
                        Some(format!("暫存曲庫播放時長仍待保存，背景工作會重試：{error}")),
                    );
                    eprintln!("暫存曲庫播放時長仍待保存：{error}");
                }
            }
        }

        if shutting_down {
            if unmapped {
                set_statistics_status(
                    &status,
                    Some("部分播放時長無法對應曲目；未保存增量可能遺失。".to_owned()),
                );
                return;
            }
            if retry {
                shutdown_failures = shutdown_failures.saturating_add(1);
                if shutdown_failures >= 10 {
                    eprintln!("關閉時播放時長仍有未保存檢查點；已提交統計保留。");
                    return;
                }
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
            if let Err(error) = database.finish_playback_statistics_runtime(runtime_id, &[]) {
                eprintln!("無法結束正式曲庫播放統計工作階段：{error}");
            }
            let map = accounting_track_ids
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let temporary_databases = map
                .values()
                .filter_map(|target| target.temporary_database.clone())
                .fold(
                    HashMap::<usize, Arc<Database>>::new(),
                    |mut databases, db| {
                        databases.entry(Arc::as_ptr(&db) as usize).or_insert(db);
                        databases
                    },
                );
            drop(map);
            for temporary_database in temporary_databases.into_values() {
                if let Err(error) =
                    temporary_database.finish_playback_statistics_runtime(runtime_id, &[])
                {
                    eprintln!("無法結束暫存曲庫播放統計工作階段：{error}");
                }
            }
            set_statistics_status(&status, None);
            return;
        }

        if !retry && !unmapped {
            set_statistics_status(&status, None);
        }
        if retry {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

#[cfg(target_os = "windows")]
fn acknowledge_statistics_batch(
    player: &PlayerHandle,
    checkpoints: &[PlaybackAccountingCheckpoint],
) {
    for checkpoint in checkpoints {
        player.acknowledge_playback_accounting(
            checkpoint.id,
            checkpoint.played_ms,
            checkpoint.duration_ms,
        );
    }
}

#[cfg(target_os = "windows")]
fn set_statistics_status(status: &Mutex<Option<String>>, message: Option<String>) {
    *status
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = message;
}

#[cfg(target_os = "windows")]
impl WindowsSystemMediaService {
    /// The current track's metadata and cover changed: publish them again on the next update.
    fn refresh_current_track(&self) {
        self.artwork
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .invalidate();
        *self
            .last_update
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    fn new(controller: SystemMediaController) -> Self {
        Self {
            controller: Mutex::new(Some(controller)),
            status: Mutex::new(WindowsSystemMediaStatus::Starting),
            pump_gate: Mutex::new(()),
            last_update: Mutex::new(None),
            artwork: Mutex::new(ArtworkPump::new()),
            published_metadata: Mutex::new(None),
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
        self.artwork
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .shutdown();
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
        // A pump already in progress may be waiting (bounded) on audio
        // acknowledgements; skip instead of parking more IPC threads behind it.
        let _pump = match self.pump_gate.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return,
        };
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
        let track_id = current_track.as_ref().map(|track| track.id);
        let external_locators = track_id.and_then(|track_id| {
            playback
                .external_tracks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&track_id)
                .map(|track| vec![MediaLocator::FileSystem(track.path.clone())])
        });
        let artwork = self
            .artwork
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .update_with_external_locators(track_id, database, external_locators);
        let current_metadata = current_track.map(|track| MediaControlMetadata {
            title: display_track_title(track.title.as_deref(), track.file_name.as_deref()),
            artist: track.artist,
            album: track.album,
            thumbnail: None,
        });
        let publication = metadata_publication(current_metadata, artwork);
        let queue = playback
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let capabilities = player_audio_windows::system_media::MediaControlCapabilities {
            can_next: queue.as_ref().is_some_and(PlaybackQueue::can_next),
            can_previous: queue.as_ref().is_some_and(PlaybackQueue::can_previous),
        };
        let mut published_metadata = self
            .published_metadata
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match publish_metadata_update(publication, &mut published_metadata, |metadata| {
            controller.update(MediaControlUpdate {
                snapshot: playback.player.snapshot(),
                metadata,
                capabilities,
            })
        }) {
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
            let change = play_track_from_database(database, playback, &track_id.to_string(), None)?;
            let PreparedTrackChange {
                ticket,
                track,
                replacement_queue,
                target_cursor,
            } = change;
            ticket
                .wait(Duration::from_secs(2))
                .map_err(|error| playback_audio_error_message(&error))?;
            commit_track_change(database, playback, track, replacement_queue, target_cursor)?;
        }
        SystemMediaEvent::PauseRequested => {
            pause_playback(playback)?
                .wait(Duration::from_secs(2))
                .map_err(|error| playback_audio_error_message(&error))?;
            if let Some(database) = database {
                checkpoint_playback_position(database, playback, true)?;
            }
        }
        SystemMediaEvent::StopRequested => {
            let _ = stop_playback(playback)?;
            if let Some(database) = database {
                checkpoint_playback_position(database, playback, true)?;
            }
        }
        SystemMediaEvent::SeekRequested(position) => {
            seek_playback(playback, duration_millis(*position))?
                .wait(Duration::from_secs(2))
                .map_err(|error| playback_audio_error_message(&error))?;
            if let Some(database) = database {
                checkpoint_playback_position(database, playback, true)?;
            }
        }
        SystemMediaEvent::NextRequested => {
            if let Some(database) = database {
                if let Some(change) = navigate_queue(database, playback, true)? {
                    let PreparedTrackChange {
                        ticket,
                        track,
                        replacement_queue,
                        target_cursor,
                    } = change;
                    ticket
                        .wait(Duration::from_secs(2))
                        .map_err(|error| playback_audio_error_message(&error))?;
                    commit_track_change(
                        database,
                        playback,
                        track,
                        replacement_queue,
                        target_cursor,
                    )?;
                }
            }
        }
        SystemMediaEvent::PreviousRequested => {
            if let Some(database) = database {
                if let Some(change) = navigate_queue(database, playback, false)? {
                    let PreparedTrackChange {
                        ticket,
                        track,
                        replacement_queue,
                        target_cursor,
                    } = change;
                    ticket
                        .wait(Duration::from_secs(2))
                        .map_err(|error| playback_audio_error_message(&error))?;
                    commit_track_change(
                        database,
                        playback,
                        track,
                        replacement_queue,
                        target_cursor,
                    )?;
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
            maximized_work_area::install(hwnd);
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

#[cfg(any(target_os = "windows", test))]
fn duration_millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(any(target_os = "windows", test))]
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
        AudioError::OutputDevice(_) => {
            "找不到可用的音訊輸出裝置；裝置恢復後會自動重新連線。".to_owned()
        }
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
    /// For a remote folder: the server (`remoteAccounts[].id`) it is on.
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_account_id: Option<String>,
    kind: String,
    display_name: String,
    location: String,
    enabled: bool,
    sync_state: Option<String>,
    last_attempt_utc_ms: Option<i64>,
    last_success_utc_ms: Option<i64>,
    error_count: u64,
    last_error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExcludedFolder {
    id: String,
    display_name: String,
    location: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceSyncError {
    item: Option<String>,
    message: String,
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
    errors: Vec<SourceSyncError>,
    errors_truncated: bool,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum PlaybackQueueSource {
    Library {
        query: Option<String>,
        #[serde(default, rename = "queryAlternates")]
        query_alternates: Vec<String>,
        #[serde(default, rename = "fieldFilter")]
        field_filter: Option<TrackFieldFilter>,
        #[serde(default)]
        sort: TrackListSort,
    },
    Playlist {
        #[serde(rename = "playlistId")]
        playlist_id: String,
        #[serde(rename = "entryPosition")]
        entry_position: u64,
    },
    /// Jump within the active queue. `traversal_position` is the cursor index
    /// shown by the queue page, not the source entry index.
    Queue {
        #[serde(rename = "traversalPosition")]
        traversal_position: u64,
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
    queue_sort: Option<TrackListSort>,
    can_next: bool,
    can_previous: bool,
    /// Windows only: the output's sample-rate path for the playback settings.
    output_format: Option<PlaybackOutputFormat>,
}

/// Who converts the current track's sample rate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
enum OutputConversion {
    /// Native rate, or nothing loaded.
    None,
    HighQuality,
    Windows,
    /// Basic converter; only if the high-quality resampler is unavailable.
    Basic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
enum DseeHxActivity {
    Off,
    Active,
    Bypassed,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
struct PlaybackOutputFormat {
    mode: ResamplingMode,
    source_rate_hz: Option<u32>,
    output_rate_hz: u32,
    device_rate_hz: u32,
    conversion: OutputConversion,
    /// Windows built-in mode could not open the track's rate; the track is
    /// converted in-app at the mix rate instead.
    fallback_reason: Option<String>,
    dsee_hx: DseeHxActivity,
    dsee_notice: Option<String>,
    dsee_output_rate_hz: Option<u32>,
    /// Set while the output is WASAPI exclusive mode.
    exclusive: Option<ExclusiveOutput>,
    /// Exclusive mode is on but could not play this track; it plays in shared mode, and this says why.
    exclusive_fallback: Option<String>,
}

/// The exclusive-mode stream as opened for the current track.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
struct ExclusiveOutput {
    bits: u16,
    channels: u16,
    source_channels: u16,
    bit_perfect: bool,
    volume_applied: bool,
    underruns: u64,
    /// The 1-bit rate of the DSD file when the stream carries it as DoP.
    dop_dsd_rate_hz: Option<u32>,
    /// DoP is on but this track plays as converted PCM, and why.
    dop_fallback: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaybackQueuePage {
    revision: u64,
    total: u64,
    offset: u64,
    cursor: Option<u64>,
    current_entry_position: Option<u64>,
    items: Vec<PlaybackQueuePageItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaybackQueuePageItem {
    traversal_position: u64,
    entry_position: u64,
    source_position: Option<u64>,
    track_id: TrackId,
    track: Option<TrackSummary>,
    is_current: bool,
}

#[cfg(test)]
mod playback_queue_ipc_tests {
    use super::{
        compare_track_summaries, feature, playback_duration_ms, sync_summary, DseeHxActivity,
        FeatureState, LibraryRoot, LibrarySource, MediaLocator, MediaSourceError, MediaSourceKind,
        OutputConversion, PlaybackOutputFormat, PlaybackQueuePage, PlaybackQueuePageItem,
        PlaybackQueueSource, PlaybackSnapshot, RepeatMode, ResamplingMode, RuntimeCapabilities,
        SourceId, SourceScanState, SyncReport, TrackId, TrackListSort, TrackMetadataError,
        PLAYBACK_QUEUE_PAGE_MAX_LIMIT,
    };

    #[test]
    fn playback_queue_pages_allow_one_thousand_items() {
        assert_eq!(PLAYBACK_QUEUE_PAGE_MAX_LIMIT, 1000);
        assert_eq!(1001_u32.clamp(1, PLAYBACK_QUEUE_PAGE_MAX_LIMIT), 1000);
    }

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
            PlaybackQueueSource::Library {
                query: Some(query),
                query_alternates: _,
                field_filter: None,
                sort: _,
            } if query == "ambient"
        ));
    }

    #[test]
    fn queue_row_source_deserializes_traversal_position() {
        let source: PlaybackQueueSource = serde_json::from_value(serde_json::json!({
            "kind": "queue",
            "traversalPosition": 4
        }))
        .expect("deserialize queue row source");
        assert!(matches!(
            source,
            PlaybackQueueSource::Queue {
                traversal_position: 4
            }
        ));
    }

    #[test]
    fn library_field_filter_json_matches_renderer_contract() {
        let source: PlaybackQueueSource = serde_json::from_value(serde_json::json!({
            "kind": "library",
            "query": "ambient",
            "fieldFilter": { "field": "artist", "value": "演出者 🎧" }
        }))
        .expect("deserialize exact field filter");
        assert_eq!(
            source,
            PlaybackQueueSource::Library {
                query: Some("ambient".to_owned()),
                query_alternates: Vec::new(),
                field_filter: Some(player_core::TrackFieldFilter {
                    field: player_core::TrackField::Artist,
                    value: "演出者 🎧".to_owned(),
                }),
                sort: TrackListSort::default(),
            }
        );
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
            queue_sort: Some(TrackListSort {
                field: player_core::TrackSortField::Year,
                direction: player_core::TrackSortDirection::Desc,
            }),
            can_next: true,
            can_previous: false,
            output_format: Some(PlaybackOutputFormat {
                mode: ResamplingMode::WindowsBuiltin,
                source_rate_hz: Some(44_100),
                output_rate_hz: 96_000,
                device_rate_hz: 96_000,
                conversion: OutputConversion::HighQuality,
                fallback_reason: Some("unsupported".to_owned()),
                dsee_hx: DseeHxActivity::Off,
                dsee_notice: None,
                dsee_output_rate_hz: None,
                exclusive: None,
                exclusive_fallback: None,
            }),
        };
        let value = serde_json::to_value(snapshot).expect("serialize playback snapshot");
        assert_eq!(
            value["outputFormat"],
            serde_json::json!({
                "mode": "windowsBuiltin",
                "sourceRateHz": 44_100,
                "outputRateHz": 96_000,
                "deviceRateHz": 96_000,
                "conversion": "highQuality",
                "fallbackReason": "unsupported",
                "dseeHx": "off",
                "dseeNotice": null,
                "dseeOutputRateHz": null,
                "exclusive": null,
                "exclusiveFallback": null,
            })
        );
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
        assert_eq!(
            value["queueSort"],
            serde_json::json!({ "field": "year", "direction": "desc" })
        );
        assert!(
            value.get("items").is_none(),
            "queue items stay out of snapshots"
        );

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
            remote_account_id: None,
            id: "source-id".to_owned(),
            kind: "playlistFile".to_owned(),
            display_name: "測試清單".to_owned(),
            location: r"C:\音樂\清單.m3u8".to_owned(),
            enabled: true,
            sync_state: Some("complete".to_owned()),
            last_attempt_utc_ms: Some(1_800_000_000_001),
            last_success_utc_ms: Some(1_800_000_000_002),
            error_count: 1,
            last_error: Some("playlist permission revoked".to_owned()),
        };
        let value = serde_json::to_value(source).expect("serialize library source summary");
        for key in [
            "displayName",
            "location",
            "syncState",
            "lastAttemptUtcMs",
            "lastSuccessUtcMs",
            "errorCount",
            "lastError",
        ] {
            assert!(value.get(key).is_some(), "missing camelCase field {key}");
        }
        assert_eq!(value["location"], r"C:\音樂\清單.m3u8");
        assert_eq!(value["lastError"], "playlist permission revoked");
    }

    #[test]
    fn sync_summary_serializes_nullable_source_item_and_metadata_error_details() {
        let root = LibraryRoot {
            id: SourceId::new(),
            kind: MediaSourceKind::WindowsFilesystem,
            display_name: "音樂資料夾".to_owned(),
            locator: MediaLocator::FileSystem(std::path::PathBuf::from(r"C:\Music")),
            enabled: true,
        };
        let mut report = SyncReport {
            state: Some(SourceScanState::Complete),
            ..SyncReport::default()
        };
        report.source_errors.push(MediaSourceError {
            source_item_id: None,
            message: "無法列舉來源資料夾".to_owned(),
        });
        report.metadata_errors.push((
            r"C:\Music\歌手\歌曲.flac".to_owned(),
            TrackMetadataError {
                message: "音訊標籤無法解析".to_owned(),
            },
        ));

        let value = serde_json::to_value(sync_summary(&root, report))
            .expect("serialize source sync summary");

        assert_eq!(value["state"], "complete");
        assert_eq!(value["errorCount"], 2);
        assert_eq!(value["errorsTruncated"], false);
        assert_eq!(value["errors"].as_array().unwrap().len(), 2);
        assert!(value["errors"][0]["item"].is_null());
        assert_eq!(value["errors"][0]["message"], "無法列舉來源資料夾");
        assert_eq!(value["errors"][1]["item"], r"C:\Music\歌手\歌曲.flac");
        assert_eq!(value["errors"][1]["message"], "音訊標籤無法解析");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn sync_summary_decodes_only_valid_windows_locator_keys_for_display() {
        let path = r"D:\Music\Loss\[hanser]\泠鳶yousa,hanser - 1 2 FanClub.m4a";
        let key = super::windows_locator_key(std::path::Path::new(path));
        let root = LibraryRoot {
            id: SourceId::new(),
            kind: MediaSourceKind::WindowsFilesystem,
            display_name: "Music".to_owned(),
            locator: MediaLocator::FileSystem(std::path::PathBuf::from(r"D:\Music")),
            enabled: true,
        };
        let mut report = SyncReport {
            state: Some(SourceScanState::Complete),
            ..SyncReport::default()
        };
        report.source_errors.push(MediaSourceError {
            source_item_id: Some("content://provider/document/track".to_owned()),
            message: "URI source error".to_owned(),
        });
        report.source_errors.push(MediaSourceError {
            source_item_id: Some("windows-u16-v1:d800".to_owned()),
            message: "invalid UTF-16 key".to_owned(),
        });
        report.metadata_errors.push((
            key,
            TrackMetadataError {
                message: "MP4 metadata parse failed".to_owned(),
            },
        ));

        let value = serde_json::to_value(sync_summary(&root, report))
            .expect("serialize source sync summary");

        assert_eq!(
            value["errors"][0]["item"],
            "content://provider/document/track"
        );
        assert_eq!(value["errors"][1]["item"], "windows-u16-v1:d800");
        assert_eq!(value["errors"][2]["item"], path);
    }

    #[test]
    fn sync_summary_caps_error_details_without_changing_total_count() {
        let root = LibraryRoot {
            id: SourceId::new(),
            kind: MediaSourceKind::WindowsFilesystem,
            display_name: "Music".to_owned(),
            locator: MediaLocator::FileSystem(std::path::PathBuf::from(r"C:\Music")),
            enabled: true,
        };
        let mut report = SyncReport {
            state: Some(SourceScanState::Complete),
            ..SyncReport::default()
        };
        for index in 0..101 {
            report.source_errors.push(MediaSourceError {
                source_item_id: Some(format!("item-{index}")),
                message: "source item error".to_owned(),
            });
        }
        report.metadata_errors.push((
            "metadata-item".to_owned(),
            TrackMetadataError {
                message: "metadata error".to_owned(),
            },
        ));

        let value = serde_json::to_value(sync_summary(&root, report))
            .expect("serialize capped source sync summary");

        assert_eq!(value["errorCount"], 102);
        assert_eq!(value["errors"].as_array().unwrap().len(), 100);
        assert_eq!(value["errorsTruncated"], true);
        assert_eq!(value["errors"][99]["item"], "item-99");
    }

    #[test]
    fn playback_queue_page_serializes_traversal_and_stable_entry_positions() {
        let page = PlaybackQueuePage {
            revision: 12,
            total: 3,
            offset: 1,
            cursor: Some(1),
            current_entry_position: Some(2),
            items: vec![PlaybackQueuePageItem {
                traversal_position: 1,
                entry_position: 2,
                source_position: Some(7),
                track_id: TrackId::new(),
                track: None,
                is_current: true,
            }],
        };

        let value = serde_json::to_value(page).expect("serialize playback queue page");
        assert_eq!(value["revision"], 12);
        assert_eq!(value["total"], 3);
        assert_eq!(value["offset"], 1);
        assert_eq!(value["cursor"], 1);
        assert_eq!(value["currentEntryPosition"], 2);
        assert_eq!(value["items"][0]["traversalPosition"], 1);
        assert_eq!(value["items"][0]["entryPosition"], 2);
        assert_eq!(value["items"][0]["sourcePosition"], 7);
        assert!(value["items"][0]["isCurrent"].as_bool().unwrap());
        assert!(value["items"][0]["track"].is_null());
    }

    #[test]
    fn merged_title_sort_matches_the_library_icu_collation_for_symbols_zhuyin_and_numbers() {
        let database = player_db::Database::open_in_memory().expect("in-memory database");
        let titles = [
            "不", "比", "蘋", "迷", "-", "\"", "[", "「", "八", "ㄅ", "1", "2", "10",
        ];
        let canonical_indices = database
            .sort_text_indices(&titles.map(str::to_owned))
            .expect("sort names through SQLite")
            .into_iter();
        let expected = canonical_indices
            .map(|index| titles[index])
            .collect::<Vec<_>>();
        for field in [
            player_core::TrackSortField::Title,
            player_core::TrackSortField::Artist,
            player_core::TrackSortField::Album,
        ] {
            let mut summaries = titles
                .iter()
                .map(|title| player_core::TrackSummary {
                    id: TrackId::new(),
                    title: Some((*title).to_owned()),
                    artist: Some((*title).to_owned()),
                    album: Some((*title).to_owned()),
                    album_artist: None,
                    track_number: None,
                    disc_number: None,
                    duration_ms: None,
                    codec: None,
                    bitrate_bps: None,
                    sample_rate_hz: None,
                    release_date: None,
                    year: None,
                    created_at_utc_ms: None,
                    bit_depth: None,
                    played_ms: 0,
                    file_name: None,
                    source_name: None,
                })
                .collect::<Vec<_>>();
            summaries.sort_by(|left, right| {
                compare_track_summaries(
                    left,
                    right,
                    TrackListSort {
                        field,
                        direction: player_core::TrackSortDirection::Asc,
                    },
                    Some(&database),
                )
            });

            let actual = summaries
                .iter()
                .map(|summary| match field {
                    player_core::TrackSortField::Title => summary.title.as_deref().expect("title"),
                    player_core::TrackSortField::Artist => {
                        summary.artist.as_deref().expect("artist")
                    }
                    player_core::TrackSortField::Album => summary.album.as_deref().expect("album"),
                    _ => unreachable!("only text fields are covered"),
                })
                .collect::<Vec<_>>();
            assert_eq!(
                actual, expected,
                "text field {field:?} uses the library collation"
            );
        }
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
        #[cfg(target_os = "android")]
        {
            if state.database.is_some() && state.android_playback.is_some() {
                feature(FeatureState::Ready, None)
            } else {
                feature(
                    FeatureState::NotReady,
                    state
                        .database_error
                        .clone()
                        .or_else(|| state.android_playback_error.clone()),
                )
            }
        }
        #[cfg(not(any(target_os = "windows", target_os = "android")))]
        {
            feature(
                FeatureState::NotReady,
                Some("此平台尚未提供本機音訊播放服務。".to_owned()),
            )
        }
    };

    let playlist_exchange = if cfg!(any(target_os = "windows", target_os = "android")) {
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
            Some("M3U/M3U8 原生檔案匯入與匯出目前不可用。".to_owned()),
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

#[cfg(target_os = "android")]
fn system_media_controls_capability(state: &AppState) -> FeatureCapability {
    if state.android_playback.is_some() {
        feature(FeatureState::Ready, None)
    } else {
        feature(
            FeatureState::NotReady,
            state
                .android_playback_error
                .clone()
                .or_else(|| Some("Android MediaSession 尚未啟動。".to_owned())),
        )
    }
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
fn system_media_controls_capability(_state: &AppState) -> FeatureCapability {
    feature(
        FeatureState::NotReady,
        Some("目前只有 Windows 提供系統媒體控制。".to_owned()),
    )
}

#[tauri::command]
async fn library_get_page(
    app: AppHandle,
    query: Option<String>,
    query_alternates: Option<Vec<String>>,
    field_filter: Option<TrackFieldFilter>,
    sort: Option<TrackListSort>,
    offset: u64,
    limit: u32,
) -> Result<Page<TrackSummary>, String> {
    let request = ListTracksQuery {
        query,
        field_filter,
        offset,
        limit,
        sort: sort.unwrap_or_default(),
    };
    let query_alternates = query_alternates.unwrap_or_default();
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        #[cfg(target_os = "windows")]
        if let Some(temporary_database) = state
            .playback
            .as_ref()
            .filter(|playback| playback.external_queue_active.load(Ordering::Acquire))
            .and_then(|playback| {
                playback
                    .temporary_catalog
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone()
            })
        {
            return merged_library_page(database, &temporary_database, request, &query_alternates)
                .map_err(|error| error.to_string());
        }
        database
            .list_tracks_page_with_alternates(request, &query_alternates)
            .map_err(|error| error.to_string())
    })
    .await
}

/// Where `track_id` is in the library list as the page query orders it (0-based), or `None` when the list does not
/// contain it (a filter hides it, or it is not in the library). The library page's "locate the playing track" button
/// scrolls there.
#[tauri::command]
async fn library_get_track_position(
    app: AppHandle,
    query: Option<String>,
    query_alternates: Option<Vec<String>>,
    field_filter: Option<TrackFieldFilter>,
    sort: Option<TrackListSort>,
    track_id: String,
) -> Result<Option<u64>, String> {
    let track_id = TrackId::parse(&track_id).map_err(|_| "曲目識別碼無效。".to_owned())?;
    let request = ListTracksQuery {
        query,
        field_filter,
        offset: 0,
        limit: u32::MAX,
        sort: sort.unwrap_or_default(),
    };
    let query_alternates = query_alternates.unwrap_or_default();
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        #[cfg(target_os = "windows")]
        if let Some(temporary_database) = state
            .playback
            .as_ref()
            .filter(|playback| playback.external_queue_active.load(Ordering::Acquire))
            .and_then(|playback| {
                playback
                    .temporary_catalog
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone()
            })
        {
            // The page query merges the Explorer-opened tracks into the library list: find the track in that merge.
            let page =
                merged_library_page(database, &temporary_database, request, &query_alternates)
                    .map_err(|error| error.to_string())?;
            return Ok(page
                .items
                .iter()
                .position(|track| track.id == track_id)
                .map(|index| index as u64));
        }
        database
            .track_position(&request, &query_alternates, track_id)
            .map_err(|error| error.to_string())
    })
    .await
}

/// What kind of quick-index groups a sort makes, and the text key of `track` that the group label is made from; `None`
/// for sorts that are not grouped (length, play count).
fn quick_index_key(
    field: TrackSortField,
    track: Option<&TrackSummary>,
) -> Option<(player_db::QuickIndexKind, String)> {
    use player_db::QuickIndexKind;
    Some(match field {
        TrackSortField::Title => (
            QuickIndexKind::Text,
            track
                .and_then(|track| {
                    player_core::display_track_title(
                        track.title.as_deref(),
                        track.file_name.as_deref(),
                    )
                })
                .unwrap_or_default(),
        ),
        TrackSortField::Artist => (
            QuickIndexKind::Text,
            track
                .and_then(|track| track.artist.clone())
                .unwrap_or_default(),
        ),
        TrackSortField::Album => (
            QuickIndexKind::Text,
            track
                .and_then(|track| track.album.clone())
                .unwrap_or_default(),
        ),
        TrackSortField::Year => (
            QuickIndexKind::Year,
            track
                .and_then(|track| {
                    track
                        .release_date
                        .clone()
                        .filter(|date| !date.trim().is_empty())
                        .or_else(|| track.year.map(|year| year.to_string()))
                })
                .unwrap_or_default(),
        ),
        TrackSortField::CreatedAt => (
            QuickIndexKind::AddedYear,
            track
                .and_then(|track| track.created_at_utc_ms)
                .map(|milliseconds| milliseconds.to_string())
                .unwrap_or_default(),
        ),
        TrackSortField::AudioFormat => (
            QuickIndexKind::Format,
            track
                .and_then(|track| track.codec.clone())
                .unwrap_or_default(),
        ),
        TrackSortField::Source => (
            QuickIndexKind::Exact,
            track
                .and_then(|track| track.source_name.clone())
                .unwrap_or_default(),
        ),
        TrackSortField::Duration | TrackSortField::PlayCount => return None,
    })
}

#[tauri::command]
async fn library_get_quick_index_for_tracks(
    app: AppHandle,
    query: Option<String>,
    query_alternates: Option<Vec<String>>,
    field_filter: Option<TrackFieldFilter>,
    sort: Option<TrackListSort>,
) -> Result<player_db::QuickIndexSnapshot, String> {
    let request = ListTracksQuery {
        query,
        field_filter,
        offset: 0,
        limit: 1,
        sort: sort.unwrap_or_default(),
    };
    let alternates = query_alternates.unwrap_or_default();
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        #[cfg(target_os = "windows")]
        if let Some(temporary_database) = state
            .playback
            .as_ref()
            .filter(|playback| playback.external_queue_active.load(Ordering::Acquire))
            .and_then(|playback| {
                playback
                    .temporary_catalog
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone()
            })
        {
            // Quick-index construction belongs to the backend and needs the complete sorted
            // result. Ask SQLite for that result once instead of scanning it through 1,000-row
            // IPC-shaped pages.
            let page = merged_library_page(
                database,
                &temporary_database,
                ListTracksQuery {
                    limit: u32::MAX,
                    ..request.clone()
                },
                &alternates,
            )?;
            let Some((kind, _)) = quick_index_key(request.sort.field, None) else {
                return Ok(player_db::QuickIndexSnapshot::default());
            };
            let keys = page
                .items
                .iter()
                .map(|track| {
                    quick_index_key(request.sort.field, Some(track))
                        .map(|(_, key)| key)
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>();
            return database
                .quick_index_for_ordered_keys(kind, &keys)
                .map_err(|error| error.to_string());
        }
        database
            .quick_index_for_tracks(&request, &alternates)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn library_get_quick_index_for_artists(
    app: AppHandle,
    artist_names: Vec<String>,
    sort: TrackListSort,
) -> Result<player_db::QuickIndexSnapshot, String> {
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        database
            .quick_index_for_artists(None, &[], &artist_names, sort)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn library_get_quick_index_for_album_tracks(
    app: AppHandle,
    album_names: Vec<String>,
    sort: TrackListSort,
) -> Result<player_db::QuickIndexSnapshot, String> {
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        database
            .quick_index_for_album_tracks(&album_names, sort)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn playlist_get_quick_index_groupings(
    app: AppHandle,
    playlist_id: String,
    sort: Option<PlaylistListSort>,
) -> Result<player_db::QuickIndexSnapshot, String> {
    let playlist_id = PlaylistId::parse(&playlist_id)
        .map_err(|_| "播放清單識別碼無效，請重新載入清單。".to_owned())?;
    let sort = sort.unwrap_or_default();
    run_database_work(app, move |state| {
        let settings = state
            .settings
            .snapshot()
            .map_err(|error| error.to_string())?;
        require_enabled_playlist_source(&settings.sources, playlist_id)?;
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        database
            .quick_index_for_playlist(playlist_id, sort)
            .map_err(|error| error.to_string())
            .and_then(|snapshot| {
                snapshot.ok_or_else(|| "找不到這份播放清單，請重新載入清單。".to_owned())
            })
    })
    .await
}

async fn run_blocking_database_work<R, F>(work: F) -> Result<R, String>
where
    R: Send + 'static,
    F: FnOnce() -> Result<R, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| format!("資料庫背景工作失敗：{error}"))?
}

fn run_guarded_database_work<R, F>(
    gate: &database_lifecycle::DatabaseWorkGate,
    operation: F,
) -> Result<R, String>
where
    F: FnOnce() -> Result<R, String>,
{
    let _database_work = gate
        .try_enter()
        .ok_or_else(|| "應用程式正在關閉，暫時無法開始新的資料庫工作。".to_owned())?;
    operation()
}

async fn run_database_work<R, F>(app: AppHandle, operation: F) -> Result<R, String>
where
    R: Send + 'static,
    F: FnOnce(&AppState) -> Result<R, String> + Send + 'static,
{
    run_blocking_database_work(move || {
        let state = app.state::<AppState>();
        run_guarded_database_work(&state.database_work, || operation(&state))
    })
    .await
}

#[tauri::command]
async fn library_create_album_index_snapshot(
    app: AppHandle,
    sort_field: String,
    direction: String,
    grouping_enabled: bool,
) -> Result<player_db::IndexSnapshotInfo, String> {
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        database
            .create_album_index_snapshot(&sort_field, &direction, grouping_enabled)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn library_get_album_index_snapshot_page(
    app: AppHandle,
    snapshot_id: String,
    offset: u64,
    limit: u32,
) -> Result<player_core::Page<player_db::AlbumIndexEntry>, String> {
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        database
            .get_index_snapshot_page(&snapshot_id, offset, limit)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn library_resolve_album_index_entry(
    app: AppHandle,
    identity_key: String,
) -> Result<Option<player_db::AlbumIndexEntry>, String> {
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let snapshot = database
            .create_album_navigation_snapshot()
            .map_err(|error| error.to_string())?;
        database
            .get_index_snapshot_entry(&snapshot.snapshot_id, &identity_key)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn library_resolve_album_index_entry_by_name(
    app: AppHandle,
    name: String,
) -> Result<Option<player_db::AlbumIndexEntry>, String> {
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let key = if name.trim().is_empty() {
            "album:unmarked".to_owned()
        } else {
            format!(
                "album:{}",
                database
                    .music_identity_key(&name)
                    .map_err(|error| error.to_string())?
            )
        };
        let snapshot = database
            .create_album_navigation_snapshot()
            .map_err(|error| error.to_string())?;
        database
            .get_index_snapshot_entry(&snapshot.snapshot_id, &key)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn library_get_album_page(
    app: AppHandle,
    album_names: Vec<String>,
    sort: Option<TrackListSort>,
    index_sort: Option<TrackSortDirection>,
    offset: u64,
    limit: u32,
) -> Result<Page<TrackSummary>, String> {
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        database
            .list_album_tracks_page_with_index_sort(&album_names, sort, index_sort, offset, limit)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn library_get_album_detail_facets(
    app: AppHandle,
    album_names: Vec<String>,
) -> Result<player_db::AlbumDetailFacets, String> {
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        database
            .album_detail_facets(&album_names)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn library_get_album_detail_artists(
    app: AppHandle,
    album_names: Vec<String>,
) -> Result<Vec<String>, String> {
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        database
            .album_detail_artists(&album_names)
            .map_err(|error| error.to_string())
    })
    .await
}

fn album_artwork_cache_directory(
    app: &AppHandle,
    state: &AppState,
) -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        Ok(album_artwork_cache_namespace(&user_data_directory(state)?))
    }
    #[cfg(target_os = "android")]
    {
        let _ = state;
        app.path()
            .app_cache_dir()
            .map(|path| album_artwork_cache_namespace(&path))
            .map_err(|error| format!("無法取得 Android 專輯縮圖快取路徑：{error}"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = state;
        app.path()
            .app_cache_dir()
            .map(|path| album_artwork_cache_namespace(&path))
            .map_err(|error| format!("無法取得專輯縮圖快取路徑：{error}"))
    }
}

#[tauri::command]
async fn album_artwork_cache_get(app: AppHandle, sha256: String) -> Result<Response, String> {
    let app_for_directory = app.clone();
    run_database_work(app, move |state| {
        let directory = album_artwork_cache_directory(&app_for_directory, state)?;
        album_artwork_cache::get(&directory, &sha256).map(Response::new)
    })
    .await
}

#[tauri::command]
async fn album_artwork_cache_put(
    app: AppHandle,
    sha256: String,
    data_base64: String,
) -> Result<(), String> {
    let app_for_directory = app.clone();
    run_database_work(app, move |state| {
        let directory = album_artwork_cache_directory(&app_for_directory, state)?;
        album_artwork_cache::put(&directory, &sha256, &data_base64)
    })
    .await
}

#[tauri::command]
async fn album_artwork_selection_ensure(
    app: AppHandle,
    key: String,
    album_names: Vec<String>,
    track_count: u64,
    representative: bool,
    force_rebuild: bool,
) -> Result<Option<album_artwork_cache::AlbumArtworkSelection>, String> {
    let app_for_directory = app.clone();
    let selection_key = key.clone();
    let (directory, cached, candidates, tree_uris) = run_database_work(app.clone(), move |state| {
        let directory = album_artwork_cache_directory(&app_for_directory, state)?;
        let cached = album_artwork_cache::get_selection(&directory, &selection_key)?;
        // An album with no cover remembered from before remote tracks were told apart from missing ones may only have failed
        // on the network: for an album that has tracks on a server the empty answer is not trusted and the covers are
        // looked for again.
        let empty_but_remote = cached
            .as_ref()
            .is_some_and(|selection| selection.track_ids.is_empty())
            && state.database.as_ref().is_some_and(|database| {
                database
                    .album_artwork_candidate_locators(&album_names)
                    .map(|candidates| {
                        candidates.iter().any(|(_, locators)| {
                            locators
                                .iter()
                                .any(|locator| matches!(locator, MediaLocator::Remote { .. }))
                        })
                    })
                    .unwrap_or(false)
            });
        if !force_rebuild
            && !empty_but_remote
            && cached.as_ref().is_some_and(|selection| {
                selection.track_count == track_count
                    && (selection.track_ids.is_empty()
                        || !representative
                        || selection.representative)
            })
        {
            return Ok((directory, cached, None, Vec::new()));
        }
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let candidates = database
            .album_artwork_candidate_locators(&album_names)
            .map_err(|error| error.to_string())?;
        // Android folder artwork is read through real paths (ADR 0014); no SAF trees.
        let tree_uris: Vec<String> = Vec::new();
        Ok((directory, None, Some(candidates), tree_uris))
    })
    .await?;
    if let Some(selection) = cached {
        return Ok(Some(selection));
    }

    let mut candidate_list = candidates.unwrap_or_default();
    // An album card takes the first cover that can be read and stops; the page of an album (representative) looks through every
    // song to choose from. A card of an album none of whose songs has a cover would still go through all of them, which for
    // songs on a server is a network read each: a card stops after the first few.
    if !representative
        && candidate_list.iter().any(|(_, locators)| {
            locators
                .iter()
                .any(|locator| matches!(locator, MediaLocator::Remote { .. }))
        })
    {
        candidate_list.truncate(8);
    }
    let app_for_scan = app.clone();
    let (selected_ids, may_have_failed) = tauri::async_runtime::spawn_blocking(move || {
        run_as_bulk_cover_work(|| {
            let failed = std::cell::Cell::new(false);
            let artwork = candidate_list
                .into_iter()
                .filter_map(|(track_id, locators)| {
                    let bytes = read_album_candidate_artwork(&app_for_scan, &locators, &tree_uris);
                    if bytes.is_none()
                        && !failed.get()
                        && remote_cover_lookup_may_have_failed(&locators)
                    {
                        failed.set(true);
                    }
                    bytes.map(|bytes| (track_id, bytes))
                });
            let selected = if representative {
                album_artwork_analysis::select_representative_artwork(artwork)
            } else {
                album_artwork_analysis::first_artwork(artwork)
                    .into_iter()
                    .collect()
            };
            (selected, failed.get())
        })
    })
    .await
    .map_err(|error| format!("專輯封面候選分析工作失敗：{error}"))?;
    if selected_ids.is_empty() {
        let selection = album_artwork_cache::AlbumArtworkSelection {
            track_count,
            track_ids: Vec::new(),
            representative,
        };
        // No cover was found. Remember that only when it is certain: a track on a server that could not be read (the server was
        // busy, the link was down) says nothing about the album, and the next look must try again.
        if !may_have_failed {
            album_artwork_cache::put_selection(&directory, &key, &selection)?;
        }
        return Ok(Some(selection));
    }
    let selection = album_artwork_cache::AlbumArtworkSelection {
        track_count,
        track_ids: selected_ids,
        representative,
    };
    eprintln!(
        "[art] ensure {} representative={} -> {} covers (scanned)",
        &key[..key.len().min(8)],
        representative,
        selection.track_ids.len()
    );
    album_artwork_cache::put_selection(&directory, &key, &selection)?;
    Ok(Some(selection))
}

/// Run album-cover picking as bulk work (its remote lookups wait behind those a person is waiting for). Only Android reads
/// covers of remote tracks through the shared lookup this applies to.
#[cfg(target_os = "android")]
fn run_as_bulk_cover_work<T>(work: impl FnOnce() -> T) -> T {
    remote_media::as_bulk_cover_work(work)
}

#[cfg(not(target_os = "android"))]
fn run_as_bulk_cover_work<T>(work: impl FnOnce() -> T) -> T {
    work()
}

/// Whether a lookup that found no cover for these locators may only have failed on the network: one of them is a track on a
/// remote server whose file cannot be opened right now (for a reason other than "it is not there"). A cover that is not
/// found then is not a cover that does not exist, and must not be remembered as missing.
fn remote_cover_lookup_may_have_failed(locators: &[MediaLocator]) -> bool {
    let Some(content) = player_core::remote_content() else {
        // Remote tracks exist but nothing can read them yet (the app is still starting).
        return locators
            .iter()
            .any(|locator| matches!(locator, MediaLocator::Remote { .. }));
    };
    locators.iter().any(|locator| match locator {
        MediaLocator::Remote { source_id, path } => content
            .open(*source_id, path)
            .is_err_and(|error| error.kind() != std::io::ErrorKind::NotFound),
        _ => false,
    })
}

fn read_album_candidate_artwork(
    _app: &AppHandle,
    locators: &[MediaLocator],
    _tree_uris: &[String],
) -> Option<Vec<u8>> {
    #[cfg(target_os = "windows")]
    {
        match find_artwork(locators) {
            ArtworkLookup::Found(image) => Some(image.into_bytes()),
            ArtworkLookup::Missing | ArtworkLookup::Oversized => None,
        }
    }
    #[cfg(target_os = "android")]
    {
        find_android_artwork(_app, locators, _tree_uris)
            .ok()
            .map(player_core::ArtworkImage::into_bytes)
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (_app, locators, _tree_uris);
        None
    }
}

fn album_artwork_track_identity(
    state: &AppState,
    track_id: &str,
) -> Result<Option<String>, String> {
    let track_id = TrackId::parse(track_id).map_err(|error| format!("曲目 ID 無效：{error}"))?;
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    database
        .album_artwork_cache_identity(track_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn album_artwork_cache_get_for_track(
    app: AppHandle,
    track_id: String,
) -> Result<Response, String> {
    let app_for_directory = app.clone();
    run_database_work(app, move |state| {
        let directory = album_artwork_cache_directory(&app_for_directory, state)?;
        let Some(identity) = album_artwork_track_identity(state, &track_id)? else {
            return Ok(Response::new(Vec::new()));
        };
        let cached = album_artwork_cache::get_for_track(&directory, &identity)?;
        eprintln!(
            "[art] get_for_track {} -> {} bytes",
            &track_id[..track_id.len().min(8)],
            cached.len()
        );
        // "Missing" remembered for a track on a server may be left from before such tracks were told apart from covers the
        // server failed to deliver: it is not trusted, the cover is read again.
        if cached.as_slice() == [0] && track_may_be_unreachable_on_a_server(state, &track_id) {
            return Ok(Response::new(Vec::new()));
        }
        Ok(Response::new(cached))
    })
    .await
}

/// A track on a server whose file cannot be opened right now: an empty cover answer for it may only be a server that did
/// not respond.
fn track_may_be_unreachable_on_a_server(state: &AppState, track_id: &str) -> bool {
    TrackId::parse(track_id)
        .ok()
        .and_then(|id| state.database.as_ref()?.track_locators(id).ok())
        .is_some_and(|locators| remote_cover_lookup_may_have_failed(&locators))
}

#[tauri::command]
async fn album_artwork_cache_put_for_track(
    app: AppHandle,
    track_id: String,
    sha256: String,
    data_base64: String,
) -> Result<(), String> {
    let app_for_directory = app.clone();
    run_database_work(app, move |state| {
        let directory = album_artwork_cache_directory(&app_for_directory, state)?;
        let Some(identity) = album_artwork_track_identity(state, &track_id)? else {
            eprintln!(
                "[art] put_for_track {} has no identity: nothing stored",
                &track_id[..track_id.len().min(8)]
            );
            return Ok(());
        };
        eprintln!(
            "[art] put_for_track {} stored",
            &track_id[..track_id.len().min(8)]
        );
        album_artwork_cache::put(&directory, &sha256, &data_base64)?;
        album_artwork_cache::put_track_reference(&directory, &identity, Some(&sha256))
    })
    .await
}

/// The slowest recent uses of the library connection (waited or held longer than 250 ms): diagnoses a stall of the pages.
#[tauri::command]
fn db_slow_locks() -> Vec<player_db::SlowLockEvent> {
    player_db::slow_lock_events()
}

/// The cover thumbnail cache as the page sees it (Settings → Music sources → Cache).
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CoverCacheView {
    limit_mb: u32,
    used_bytes: u64,
}

fn cover_cache_view(app: &AppHandle, state: &AppState) -> Result<CoverCacheView, String> {
    let settings = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?;
    let directory = album_artwork_cache_directory(app, state)?;
    Ok(CoverCacheView {
        limit_mb: settings.cover_cache_limit_mb,
        used_bytes: album_artwork_cache::used_bytes(&directory),
    })
}

#[tauri::command]
async fn album_artwork_cache_stats(app: AppHandle) -> Result<CoverCacheView, String> {
    let app_for_view = app.clone();
    run_database_work(app, move |state| cover_cache_view(&app_for_view, state)).await
}

/// Change the size limit: stored in the settings, applied at once (a smaller limit deletes the oldest thumbnails).
#[tauri::command]
async fn album_artwork_cache_set_limit(
    app: AppHandle,
    limit_mb: u32,
) -> Result<CoverCacheView, String> {
    let limit_mb = settings::clamp_cover_cache_limit_mb(limit_mb);
    let app_for_view = app.clone();
    run_database_work(app, move |state| {
        state
            .settings
            .update(|settings| {
                settings.cover_cache_limit_mb = limit_mb;
                Ok(())
            })
            .map_err(|error| error.to_string())?;
        let directory = album_artwork_cache_directory(&app_for_view, state)?;
        album_artwork_cache::set_limit_bytes(Some(&directory), u64::from(limit_mb) * 1_000_000);
        cover_cache_view(&app_for_view, state)
    })
    .await
}

/// Forget every cached album cover. The pages make them again from the files when they are shown.
#[tauri::command]
async fn album_artwork_cache_clear(app: AppHandle) -> Result<CoverCacheView, String> {
    let app_for_view = app.clone();
    run_database_work(app, move |state| {
        let directory = album_artwork_cache_directory(&app_for_view, state)?;
        album_artwork_cache::clear(&directory)?;
        cover_cache_view(&app_for_view, state)
    })
    .await
}

#[tauri::command]
async fn album_artwork_cache_mark_missing_for_track(
    app: AppHandle,
    track_id: String,
    definitive: Option<bool>,
) -> Result<(), String> {
    let app_for_directory = app.clone();
    run_database_work(app, move |state| {
        let directory = album_artwork_cache_directory(&app_for_directory, state)?;
        let Some(identity) = album_artwork_track_identity(state, &track_id)? else {
            return Ok(());
        };
        // A track on a server is never remembered as without a cover from here: the page cannot tell "no cover" from "the
        // server did not answer", and a wrong "missing" would hide a cover for good.
        if !definitive.unwrap_or(false) && track_may_be_unreachable_on_a_server(state, &track_id) {
            eprintln!(
                "[art] mark_missing {} skipped (server unreachable)",
                &track_id[..track_id.len().min(8)]
            );
            return Ok(());
        }
        eprintln!(
            "[art] mark_missing {} definitive={:?}",
            &track_id[..track_id.len().min(8)],
            definitive
        );
        album_artwork_cache::put_track_reference(&directory, &identity, None)
    })
    .await
}

#[tauri::command]
async fn library_create_artist_index_snapshot(
    app: AppHandle,
    sort_field: String,
    direction: String,
    grouping_enabled: bool,
) -> Result<player_db::IndexSnapshotInfo, String> {
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        database
            .create_artist_index_snapshot(&sort_field, &direction, grouping_enabled)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn library_get_artist_index_snapshot_page(
    app: AppHandle,
    snapshot_id: String,
    offset: u64,
    limit: u32,
) -> Result<player_core::Page<player_db::ArtistIndexEntry>, String> {
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        database
            .get_index_snapshot_page(&snapshot_id, offset, limit)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn library_resolve_artist_index_entry(
    app: AppHandle,
    identity_key: String,
) -> Result<Option<player_db::ArtistIndexEntry>, String> {
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let snapshot = database
            .create_artist_navigation_snapshot()
            .map_err(|error| error.to_string())?;
        database
            .get_index_snapshot_entry(&snapshot.snapshot_id, &identity_key)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn library_resolve_artist_index_entry_by_name(
    app: AppHandle,
    name: String,
) -> Result<Option<player_db::ArtistIndexEntry>, String> {
    run_database_work(app, move |state| {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let identity_key = database
            .music_identity_key(&name)
            .map_err(|error| error.to_string())?;
        let snapshot = database
            .create_artist_navigation_snapshot()
            .map_err(|error| error.to_string())?;
        database
            .get_index_snapshot_entry(&snapshot.snapshot_id, &identity_key)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
fn library_get_artist_page(
    state: State<'_, AppState>,
    query: Option<String>,
    query_alternates: Option<Vec<String>>,
    sort: Option<TrackListSort>,
    offset: u64,
    limit: u32,
    artist_names: Vec<String>,
) -> Result<Page<TrackSummary>, String> {
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    database
        .list_tracks_page_for_artists(
            ListTracksQuery {
                query,
                field_filter: None,
                offset,
                limit,
                sort: sort.unwrap_or_default(),
            },
            &query_alternates.unwrap_or_default(),
            &artist_names,
        )
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
fn merged_library_page(
    database: &Database,
    temporary_database: &Database,
    request: ListTracksQuery,
    query_alternates: &[String],
) -> Result<Page<TrackSummary>, String> {
    let sort = request.sort;
    let temporary_query = ListTracksQuery {
        query: request.query.clone(),
        field_filter: request.field_filter.clone(),
        sort,
        ..ListTracksQuery::default()
    };
    // The transient catalog is already a SQLite database. Read its complete filtered result in
    // one SQLite statement instead of repeatedly issuing small internal pages.
    let temporary_items = temporary_database
        .list_tracks_all_with_alternates(&temporary_query, query_alternates)
        .map_err(|error| error.to_string())?;

    let mut temporary_paths = HashSet::new();
    let mut duplicate_library_ids = HashSet::new();
    let temporary_ids = temporary_items
        .iter()
        .map(|track| track.id)
        .collect::<Vec<_>>();
    let temporary_locators = temporary_database
        .filesystem_locators_for_tracks(&temporary_ids)
        .map_err(|error| error.to_string())?;
    let paths = temporary_locators
        .iter()
        .map(|(_, path)| path.clone())
        .collect::<Vec<_>>();
    for path in &paths {
        temporary_paths.insert(normalized_filesystem_path_key(path));
    }
    for track_id in database
        .resolve_track_ids_for_windows_paths(&paths)
        .map_err(|error| error.to_string())?
        .into_iter()
        .flatten()
    {
        duplicate_library_ids.insert(track_id);
    }

    let persistent_query = ListTracksQuery {
        query: request.query.clone(),
        field_filter: request.field_filter.clone(),
        sort,
        ..ListTracksQuery::default()
    };
    let persistent_total = database
        .list_tracks_page_with_alternates(
            ListTracksQuery {
                limit: 1,
                ..persistent_query.clone()
            },
            query_alternates,
        )
        .map_err(|error| error.to_string())?;
    let temporary_count = temporary_items.len() as u64;
    // A transient item can displace at most one persistent result. Fetch the first K+T
    // persistent rows in one SQLite query, where K is the requested page end and T is the
    // transient result count. This guarantees enough candidates even after duplicate removal,
    // without reading the full persistent library for every visible page.
    let persistent_candidate_limit = request
        .offset
        .saturating_add(u64::from(request.limit))
        .saturating_add(temporary_count)
        .min(persistent_total.total_count);
    let mut persistent_items = database
        .list_tracks_window_with_alternates(
            &persistent_query,
            query_alternates,
            0,
            persistent_candidate_limit,
        )
        .map_err(|error| error.to_string())?;
    let mut duplicate_path_ids = database
        .filesystem_locators_for_tracks(
            &persistent_items
                .iter()
                .map(|track| track.id)
                .collect::<Vec<_>>(),
        )
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter_map(|(track_id, path)| {
            temporary_paths
                .contains(&normalized_filesystem_path_key(&path))
                .then_some(track_id)
        })
        .collect::<HashSet<_>>();
    duplicate_path_ids.extend(duplicate_library_ids);
    persistent_items.retain(|track| !duplicate_path_ids.contains(&track.id));
    let total_count = persistent_total
        .total_count
        .saturating_add(temporary_count)
        .saturating_sub(duplicate_path_ids.len() as u64);
    let mut merged_items = persistent_items;
    merged_items.extend(temporary_items);
    // Sorting the union is still performed by SQLite using the same registered ICU4X collation
    // and composite sort expressions as the persistent library.
    merged_items = sort_track_summaries_sql(database, merged_items, sort)?;
    let items = merged_items
        .into_iter()
        .skip(usize::try_from(request.offset).unwrap_or(usize::MAX))
        .take(request.limit as usize)
        .collect();
    Ok(Page {
        items,
        offset: request.offset,
        limit: request.limit,
        total_count,
    })
}

#[cfg(target_os = "windows")]
fn normalized_filesystem_path_key(path: &std::path::Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
}

fn compare_track_summaries(
    left: &TrackSummary,
    right: &TrackSummary,
    sort: TrackListSort,
    database: Option<&Database>,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let reverse = sort.direction == TrackSortDirection::Desc;
    let compare_direction = |order: Ordering| if reverse { order.reverse() } else { order };
    let compare_text = |left: &str, right: &str| match database {
        Some(database) => database.compare_music_text(left, right),
        None => natural_title_cmp(left, right),
    };
    let compare_directional_text =
        |left: &str, right: &str| match (left.trim().is_empty(), right.trim().is_empty()) {
            (false, true) => {
                if reverse {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (true, false) => {
                if reverse {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            _ => compare_direction(compare_text(left, right)),
        };
    let order = match sort.field {
        TrackSortField::Title => compare_directional_text(
            left.title
                .as_deref()
                .or(left.file_name.as_deref())
                .unwrap_or_default(),
            right
                .title
                .as_deref()
                .or(right.file_name.as_deref())
                .unwrap_or_default(),
        ),
        TrackSortField::Source => compare_directional_text(
            left.source_name.as_deref().unwrap_or_default(),
            right.source_name.as_deref().unwrap_or_default(),
        ),
        TrackSortField::Artist => compare_directional_text(
            left.artist.as_deref().unwrap_or_default(),
            right.artist.as_deref().unwrap_or_default(),
        ),
        TrackSortField::Album => compare_directional_text(
            left.album.as_deref().unwrap_or_default(),
            right.album.as_deref().unwrap_or_default(),
        ),
        TrackSortField::Year => {
            let left_date = left.release_date.as_deref().unwrap_or_default();
            let right_date = right.release_date.as_deref().unwrap_or_default();
            // No usable year is the unknown group: last in ascending order, first in descending order.
            let unknown = |track: &TrackSummary| {
                player_db::year_group_label(
                    track
                        .release_date
                        .as_deref()
                        .filter(|date| !date.trim().is_empty())
                        .or(None)
                        .map(str::to_owned)
                        .or_else(|| track.year.map(|year| year.to_string()))
                        .as_deref(),
                ) == "未知"
            };
            // The unknown group is the smallest value: first ascending, last descending.
            match (unknown(left), unknown(right)) {
                (false, true) => compare_direction(Ordering::Greater),
                (true, false) => compare_direction(Ordering::Less),
                _ => match (left_date.is_empty(), right_date.is_empty()) {
                    (false, true) => Ordering::Less,
                    (true, false) => Ordering::Greater,
                    _ => compare_direction(left_date.cmp(right_date))
                        .then_with(|| compare_direction(left.year.cmp(&right.year))),
                },
            }
        }
        TrackSortField::CreatedAt => match (left.created_at_utc_ms, right.created_at_utc_ms) {
            (Some(a), Some(b)) => compare_direction(a.cmp(&b)),
            (Some(_), None) => compare_direction(Ordering::Greater),
            (None, Some(_)) => compare_direction(Ordering::Less),
            (None, None) => Ordering::Equal,
        },
        TrackSortField::AudioFormat => {
            let codec_rank = |codec: Option<&str>| match codec
                .unwrap_or_default()
                .trim()
                .to_ascii_uppercase()
                .as_str()
            {
                "WAV" | "WAVE" => 6,
                "FLAC" => 5,
                "VORBIS" | "OGG VORBIS" => 4,
                "OPUS" => 3,
                "MP4" | "M4A" | "AAC" | "MP4A" => 2,
                "MP3" => 1,
                _ => 0,
            };
            let lossless = |codec: Option<&str>| {
                matches!(
                    codec
                        .unwrap_or_default()
                        .trim()
                        .to_ascii_uppercase()
                        .as_str(),
                    "WAV"
                        | "WAVE"
                        | "FLAC"
                        | "AIFF"
                        | "AIF"
                        | "ALAC"
                        | "APE"
                        | "WV"
                        | "WAVPACK"
                        | "PCM"
                        | "TAK"
                        | "TTA"
                )
            };
            compare_direction(
                codec_rank(left.codec.as_deref()).cmp(&codec_rank(right.codec.as_deref())),
            )
            .then_with(|| {
                // One format name together inside a quality rank, as the SQL ordering does (the groups need it).
                compare_direction(
                    player_db::format_group_label(left.codec.as_deref())
                        .cmp(&player_db::format_group_label(right.codec.as_deref())),
                )
            })
            .then_with(|| {
                compare_direction(
                    left.sample_rate_hz
                        .unwrap_or_default()
                        .cmp(&right.sample_rate_hz.unwrap_or_default()),
                )
            })
            .then_with(|| {
                let left_metric = if lossless(left.codec.as_deref()) {
                    left.bit_depth.unwrap_or_default().into()
                } else {
                    left.bitrate_bps.unwrap_or_default()
                };
                let right_metric = if lossless(right.codec.as_deref()) {
                    right.bit_depth.unwrap_or_default().into()
                } else {
                    right.bitrate_bps.unwrap_or_default()
                };
                compare_direction(left_metric.cmp(&right_metric))
            })
        }
        TrackSortField::Duration => match (left.duration_ms, right.duration_ms) {
            (Some(a), Some(b)) => compare_direction(a.cmp(&b)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        },
        TrackSortField::PlayCount => {
            let ratio = |track: &TrackSummary| {
                track
                    .duration_ms
                    .filter(|duration| *duration > 0)
                    .map(|duration| track.played_ms as f64 / duration as f64)
            };
            match (ratio(left), ratio(right)) {
                (Some(a), Some(b)) => compare_direction(a.total_cmp(&b)),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        }
    };
    if order == Ordering::Equal {
        if sort.field == TrackSortField::AudioFormat || !reverse {
            left.id.cmp(&right.id)
        } else {
            right.id.cmp(&left.id)
        }
    } else {
        order
    }
}

fn sort_track_summaries_sql(
    database: &Database,
    summaries: Vec<TrackSummary>,
    sort: TrackListSort,
) -> Result<Vec<TrackSummary>, String> {
    let candidates = summaries
        .iter()
        .enumerate()
        .map(|(entry_index, summary)| player_db::QueueSortCandidate {
            entry_index,
            summary: Some(summary.clone()),
        })
        .collect::<Vec<_>>();
    let order = database
        .sort_queue_candidates(&candidates, sort)
        .map_err(|error| error.to_string())?;
    Ok(order
        .into_iter()
        .filter_map(|index| summaries.get(index).cloned())
        .collect())
}

#[cfg(target_os = "windows")]
struct LibraryQueueSearch {
    query: Option<String>,
    query_alternates: Vec<String>,
    field_filter: Option<TrackFieldFilter>,
    sort: TrackListSort,
}

#[cfg(target_os = "windows")]
fn queue_for_merged_library_search(
    database: &Database,
    temporary_database: Option<&Database>,
    selected_track_id: TrackId,
    search: LibraryQueueSearch,
    shuffle: bool,
) -> Result<PlaybackQueue, String> {
    let LibraryQueueSearch {
        query,
        query_alternates,
        field_filter,
        sort,
    } = search;
    let query_request = ListTracksQuery {
        query: query.clone(),
        field_filter: field_filter.clone(),
        sort,
        ..ListTracksQuery::default()
    };
    let mut summaries = database
        .list_tracks_all_with_alternates(&query_request, &query_alternates)
        .map_err(|error| error.to_string())?;
    let temporary_tracks = if let Some(temporary_database) = temporary_database {
        temporary_database
            .list_tracks_all_with_alternates(&query_request, &query_alternates)
            .map_err(|error| error.to_string())?
    } else {
        Vec::new()
    };
    let mut temporary_paths = HashSet::new();
    if let Some(temporary_database) = temporary_database {
        for (_, path) in temporary_database
            .filesystem_locators_for_tracks(
                &temporary_tracks
                    .iter()
                    .map(|track| track.id)
                    .collect::<Vec<_>>(),
            )
            .map_err(|error| error.to_string())?
        {
            temporary_paths.insert(normalized_filesystem_path_key(&path));
        }
    }
    let duplicate_ids = database
        .filesystem_locators_for_tracks(&summaries.iter().map(|track| track.id).collect::<Vec<_>>())
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter_map(|(track_id, path)| {
            temporary_paths
                .contains(&normalized_filesystem_path_key(&path))
                .then_some(track_id)
        })
        .collect::<HashSet<_>>();
    summaries.retain(|track| !duplicate_ids.contains(&track.id));
    summaries.extend(temporary_tracks);
    summaries = sort_track_summaries_sql(database, summaries, sort)?;
    let tracks = summaries.iter().map(|track| track.id).collect::<Vec<_>>();
    let selected_index = tracks
        .iter()
        .position(|track_id| *track_id == selected_track_id)
        .ok_or_else(|| "所選曲目已不在目前搜尋結果中。".to_owned())?;
    let mut queue = PlaybackQueue::with_entries(
        tracks
            .into_iter()
            .map(|track_id| PlaybackQueueEntry {
                track_id,
                source_position: None,
            })
            .collect(),
        PlaybackQueueContext::Library {
            query,
            field_filter,
            sort,
        },
        selected_index,
    )
    .ok_or_else(|| "目前搜尋結果沒有可播放的曲目。".to_owned())?;
    set_queue_shuffle_with_mixed_stats(database, temporary_database, &mut queue, shuffle)?;
    Ok(queue)
}

#[tauri::command]
fn playlist_list(state: State<'_, AppState>) -> Result<Vec<PlaylistSummary>, String> {
    let settings = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?;
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    let playlists = database
        .list_playlists()
        .map_err(|error| error.to_string())?;
    Ok(playlists_for_enabled_sources(playlists, &settings.sources))
}

fn playlists_for_enabled_sources(
    playlists: Vec<PlaylistSummary>,
    sources: &[SourceEntry],
) -> Vec<PlaylistSummary> {
    let enabled_playlist_ids = sources
        .iter()
        .filter(|source| source.enabled)
        .filter_map(|source| match &source.kind {
            SourceEntryKind::PlaylistFile { playlist_id, .. } => Some(*playlist_id),
            SourceEntryKind::Folder { .. } => None,
        })
        .collect::<Vec<_>>();
    playlists
        .into_iter()
        .filter(|playlist| enabled_playlist_ids.contains(&playlist.id))
        .collect()
}

fn require_enabled_playlist_source(
    sources: &[SourceEntry],
    playlist_id: PlaylistId,
) -> Result<(), String> {
    if sources.iter().any(|source| {
        source.enabled
            && matches!(
                &source.kind,
                SourceEntryKind::PlaylistFile { playlist_id: registered, .. }
                    if *registered == playlist_id
            )
    }) {
        Ok(())
    } else {
        Err("此播放清單來源已停用或不存在。請到「設定 → 音樂來源」重新啟用。".to_owned())
    }
}

#[cfg(test)]
mod playlist_source_visibility_tests {
    use super::{
        playlists_for_enabled_sources, require_enabled_playlist_source, settings, PlaylistId,
        PlaylistSummary, SourceEntry, SourceEntryKind, SourceId,
    };

    fn playlist_source(playlist_id: PlaylistId, enabled: bool) -> SourceEntry {
        SourceEntry {
            id: SourceId::new(),
            display_name: "fixture.m3u8".to_owned(),
            enabled,
            kind: SourceEntryKind::PlaylistFile {
                playlist_id,
                path: settings::StoredPath::Uri("content://fixture/playlist".to_owned()),
                tree_uri: None,
                remote_account: None,
            },
        }
    }

    #[test]
    fn playlist_projection_includes_only_enabled_registered_playlist_sources() {
        let enabled = PlaylistId::new();
        let disabled = PlaylistId::new();
        let sources = [
            playlist_source(enabled, true),
            playlist_source(disabled, false),
        ];

        let visible = playlists_for_enabled_sources(
            vec![
                PlaylistSummary {
                    id: enabled,
                    name: "enabled".to_owned(),
                    entry_count: 1,
                },
                PlaylistSummary {
                    id: disabled,
                    name: "disabled".to_owned(),
                    entry_count: 2,
                },
            ],
            &sources,
        );

        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, enabled);
        assert_eq!(visible[0].name, "enabled");
    }

    #[test]
    fn disabled_playlist_sources_cannot_be_opened_or_used_as_queue_sources() {
        let enabled = PlaylistId::new();
        let disabled = PlaylistId::new();
        let sources = [
            playlist_source(enabled, true),
            playlist_source(disabled, false),
        ];

        assert!(require_enabled_playlist_source(&sources, enabled).is_ok());
        assert!(require_enabled_playlist_source(&sources, disabled).is_err());
        assert!(require_enabled_playlist_source(&sources, PlaylistId::new()).is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn normalizing_my_love_source_name_does_not_enable_it() {
        let mut source = playlist_source(PlaylistId::new(), false);
        source.display_name = "MyLove.m3u8".to_owned();

        assert!(super::normalize_windows_my_love_source(&mut source));
        assert_eq!(source.display_name, "我的最愛");
        assert!(!source.enabled);
    }
}

#[tauri::command]
fn playlist_get_page(
    state: State<'_, AppState>,
    playlist_id: String,
    sort: Option<PlaylistListSort>,
    offset: u64,
    limit: u32,
) -> Result<PlaylistPage, String> {
    let _database_work = enter_database_work(&state)?;
    let playlist_id = PlaylistId::parse(&playlist_id)
        .map_err(|_| "播放清單識別碼無效，請重新載入清單。".to_owned())?;
    let settings = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?;
    require_enabled_playlist_source(&settings.sources, playlist_id)?;
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    database
        .get_playlist_page_sorted(playlist_id, offset, limit, sort.unwrap_or_default())
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "找不到這份播放清單，請重新載入清單。".to_owned())
}

/// Where `track_id` is in the playlist as the sorted page lists it (0-based), or `None` when the playlist does not
/// contain it. The playlist page's "locate the playing track" button shows only when this is not `None`.
#[tauri::command]
fn playlist_get_track_position(
    state: State<'_, AppState>,
    playlist_id: String,
    sort: Option<PlaylistListSort>,
    track_id: String,
) -> Result<Option<u64>, String> {
    let _database_work = enter_database_work(&state)?;
    let playlist_id = PlaylistId::parse(&playlist_id)
        .map_err(|_| "播放清單識別碼無效，請重新載入清單。".to_owned())?;
    let track_id = TrackId::parse(&track_id).map_err(|_| "曲目識別碼無效。".to_owned())?;
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    database
        .playlist_track_position(playlist_id, sort.unwrap_or_default(), track_id)
        .map_err(|error| error.to_string())
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FavoritePlaylistDto {
    source_id: String,
    display_name: String,
    selected: bool,
    is_my_love: bool,
    writable: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FavoritePlaylistsDto {
    playlists: Vec<FavoritePlaylistDto>,
    my_love_setup_required: bool,
}

#[tauri::command]
fn favorites_get_playlists(
    _app: AppHandle,
    state: State<'_, AppState>,
    track_id: String,
) -> Result<FavoritePlaylistsDto, String> {
    let track_id = TrackId::parse(&track_id).map_err(|_| "曲目識別碼無效。".to_owned())?;
    let _database_work = enter_database_work(&state)?;
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    let locators = favorite_track_locators(&state, database, track_id)?;

    #[cfg(target_os = "windows")]
    {
        ensure_windows_my_love_source(&state, database)?;
    }
    let settings = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?;
    let mut playlists = Vec::new();
    let has_my_love_source = settings.sources.iter().any(is_my_love_source);
    for source in settings.sources.iter().filter(|source| source.enabled) {
        let SourceEntryKind::PlaylistFile {
            playlist_id,
            path,
            tree_uri,
            remote_account,
        } = &source.kind
        else {
            continue;
        };
        if remote_account.is_some() {
            // A playlist on a remote server is read-only here: nothing is written back to it.
            continue;
        }
        let _ = (path, tree_uri);
        let is_my_love = source.display_name == "我的最愛" || is_my_love_path(path);
        let favorite = database
            .get_playlist(*playlist_id)
            .map_err(|error| error.to_string())?;
        let selected = favorite.as_ref().is_some_and(|playlist| {
            playlist.entries.iter().any(|entry| {
                entry.track_id == Some(track_id)
                    || locators
                        .iter()
                        .any(|locator| same_favorite_locator(locator, &entry.locator))
            })
        });
        // Android writes favorites through the real path under All files access (ADR 0014).
        #[cfg(any(target_os = "windows", target_os = "android"))]
        let writable = path.to_path_buf().is_ok();
        #[cfg(not(any(target_os = "windows", target_os = "android")))]
        let writable = false;
        playlists.push(FavoritePlaylistDto {
            source_id: source.id.to_string(),
            display_name: if is_my_love {
                "我的最愛".to_owned()
            } else {
                source.display_name.clone()
            },
            selected,
            is_my_love,
            writable,
        });
    }
    Ok(FavoritePlaylistsDto {
        playlists,
        my_love_setup_required: !has_my_love_source,
    })
}

#[tauri::command]
async fn favorites_set_membership(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
    track_id: String,
    selected: bool,
) -> Result<(), String> {
    let source_id =
        SourceId::parse(&source_id).map_err(|_| "播放清單來源識別碼無效。".to_owned())?;
    let track_id = TrackId::parse(&track_id).map_err(|_| "曲目識別碼無效。".to_owned())?;
    {
        let _database_work = enter_database_work(&state)?;
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
        let source = settings
            .sources
            .iter()
            .find(|source| source.id == source_id && source.enabled)
            .ok_or_else(|| "找不到已啟用的播放清單來源。".to_owned())?;
        let SourceEntryKind::PlaylistFile {
            playlist_id,
            path,
            tree_uri,
            remote_account,
        } = &source.kind
        else {
            return Err("只能收藏到播放清單檔案來源。".to_owned());
        };
        if remote_account.is_some() {
            return Err("遠端伺服器上的播放清單是唯讀的，無法收藏到其中。".to_owned());
        }
        let _ = (path, tree_uri);
        let target_locators = favorite_track_locators(&state, database, track_id)?;
        let target_locator = target_locators
            .first()
            .cloned()
            .ok_or_else(|| "此曲目沒有可用的原生路徑。".to_owned())?;
        let (file_path, bytes) = read_favorite_source(&app, source)?;
        let parse_path = file_path
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new(&source.display_name));
        let mut playlist = playlist_exchange::read_playlist_bytes(&bytes, parse_path)?;
        playlist.id = *playlist_id;
        resolve_favorite_entry_ids(&app, database, source, &mut playlist)?;
        for entry in &mut playlist.entries {
            if target_locators
                .iter()
                .any(|locator| same_favorite_locator(locator, &entry.locator))
            {
                entry.track_id = Some(track_id);
            }
        }
        let changed =
            favorites::set_playlist_membership(&mut playlist, track_id, target_locator, selected);
        if changed {
            // Temporary file-open tracks are not persisted in the main library database. Their
            // locator stays in M3U while the ephemeral TrackId must not leak into the main DB.
            for entry in &mut playlist.entries {
                if !target_locators
                    .iter()
                    .any(|locator| same_favorite_locator(locator, &entry.locator))
                {
                    continue;
                }
                if database
                    .get_track_summary(track_id)
                    .map_err(|error| error.to_string())?
                    .is_none()
                {
                    entry.track_id = None;
                }
            }
            let output = if source.display_name.to_ascii_lowercase().ends_with(".m3u") {
                player_core::write_m3u(
                    &playlist,
                    parse_path,
                    &player_core::M3uExportOptions::default(),
                )
            } else {
                player_core::write_m3u8(
                    &playlist,
                    parse_path,
                    &player_core::M3uExportOptions::default(),
                )
            }
            .map_err(|error| error.to_string())?;
            write_favorite_source(&app, source, file_path.as_deref(), &output)?;
            database
                .save_playlist(&playlist)
                .map_err(|error| format!("播放清單檔已更新，但資料庫投影保存失敗：{error}"))?;
        }
    }
    Ok(())
}

#[tauri::command]
async fn favorites_create_my_love(
    _app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _database_work = enter_database_work(&state)?;
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        ensure_windows_my_love_source(&state, database)?;
        Ok(())
    }
    #[cfg(target_os = "android")]
    {
        // The folder picker only locates a directory; MyLove.m3u8 is created (never
        // overwritten) and synced through its real path under All files access (ADR 0014).
        let picker_app = _app.clone();
        let picked = tauri::async_runtime::spawn_blocking(move || {
            android_folder_browser::pick_directory(&picker_app)
        })
        .await
        .map_err(|error| format!("資料夾選擇工作失敗：{error}"))?
        .map_err(|error| error.to_string())?;
        let Some(directory) = picked else {
            return Ok(());
        };
        let directory = std::fs::canonicalize(&directory)
            .map_err(|error| format!("無法確認選取的資料夾：{error}"))?;
        let path = directory.join("MyLove.m3u8");
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all("#EXTM3U\n#PLAYLIST:我的最愛\n".as_bytes())
                    .map_err(|error| format!("無法建立 MyLove.m3u8：{error}"))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("無法建立 MyLove.m3u8：{error}")),
        }
        {
            let _database_work = enter_database_work(&state)?;
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
            let stored =
                settings::StoredPath::from_path(&path).map_err(|error| error.to_string())?;
            let source = match settings
                .sources
                .iter()
                .find(|source| is_my_love_source(source))
            {
                Some(existing) => {
                    let mut source = existing.clone();
                    source.enabled = true;
                    match &mut source.kind {
                        SourceEntryKind::PlaylistFile {
                            path: source_path,
                            tree_uri,
                            ..
                        } => {
                            *source_path = stored;
                            *tree_uri = None;
                        }
                        _ => return Err("我的最愛來源型態無效。".to_owned()),
                    }
                    source
                }
                None => SourceEntry {
                    id: SourceId::new(),
                    display_name: "我的最愛".to_owned(),
                    enabled: true,
                    kind: SourceEntryKind::PlaylistFile {
                        playlist_id: PlaylistId::new(),
                        path: stored,
                        tree_uri: None,
                        remote_account: None,
                    },
                },
            };
            let SourceEntryKind::PlaylistFile { playlist_id, .. } = &source.kind else {
                unreachable!();
            };
            let playlist_id = *playlist_id;
            state
                .settings
                .upsert_source(source.clone())
                .map_err(|error| error.to_string())?;
            database
                .save_library_root(&LibraryRoot {
                    id: source.id,
                    kind: MediaSourceKind::PlaylistFile,
                    display_name: "我的最愛".to_owned(),
                    locator: MediaLocator::FileSystem(path.clone()),
                    enabled: true,
                })
                .map_err(|error| error.to_string())?;
            if database
                .get_playlist(playlist_id)
                .map_err(|error| error.to_string())?
                .is_none()
            {
                database
                    .save_playlist(&player_core::Playlist {
                        id: playlist_id,
                        name: "我的最愛".to_owned(),
                        entries: Vec::new(),
                    })
                    .map_err(|error| error.to_string())?;
            }
        }
        library_sync(_app.clone()).await?;
        Ok(())
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (_app, state);
        Err("目前平台尚未提供收藏播放清單。".to_owned())
    }
}

#[tauri::command]
async fn favorites_create_playlist(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    #[cfg(any(target_os = "windows", target_os = "android"))]
    {
        #[cfg(target_os = "windows")]
        let mut path = {
            use tauri_plugin_dialog::DialogExt;

            let selected = app
                .dialog()
                .file()
                .set_parent(&window)
                .set_title("建立播放清單")
                .set_file_name("新播放清單.m3u8")
                .add_filter("M3U8 播放清單", &["m3u8"])
                .blocking_save_file();
            let Some(selected) = selected else {
                return Ok(None);
            };
            selected
                .into_path()
                .map_err(|error| format!("無法取得播放清單檔案路徑：{error}"))?
        };
        // Android: the "save as" picker only names the file (it creates it empty); the playlist
        // is then written and synced through its real path (All files access, ADR 0014).
        #[cfg(target_os = "android")]
        let mut path = {
            let _ = &window;
            let picker_app = app.clone();
            let picked = tauri::async_runtime::spawn_blocking(move || {
                picker_app
                    .media_index()
                    .pick_create_document_path("新播放清單.m3u8", "audio/x-mpegurl")
            })
            .await
            .map_err(|error| format!("播放清單建立工作失敗：{error}"))?
            .map_err(|error| error.to_string())?;
            let Some(path) = picked else {
                return Ok(None);
            };
            let path = std::path::PathBuf::from(path);
            if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() == 0) {
                // The picker's empty placeholder: replace it so the header below is written.
                let _ = std::fs::remove_file(&path);
            }
            path
        };
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("m3u8"))
        {
            path.set_extension("m3u8");
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("無法建立播放清單資料夾：{error}"))?;
        }
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                let name = path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("新播放清單")
                    .replace(['\r', '\n'], " ");
                file.write_all(format!("#EXTM3U\n#PLAYLIST:{name}\n").as_bytes())
                    .map_err(|error| format!("無法初始化 M3U8 播放清單：{error}"))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                // Save dialogs may return an existing document after the user confirms the
                // native overwrite prompt. Preserve its contents; this action registers it.
            }
            Err(error) => return Err(format!("無法建立 M3U8 播放清單：{error}")),
        }

        let _ = &window;
        let _database_work = enter_database_work(&state)?;
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
        let imported = import_playlist_file_with_id(database, &path, playlist_id)?;
        let display_name = imported.playlist.name;
        state
            .settings
            .upsert_source(SourceEntry {
                id: source_id,
                display_name: display_name.clone(),
                enabled: true,
                kind: SourceEntryKind::PlaylistFile {
                    playlist_id,
                    path: settings::StoredPath::from_path(&path)
                        .map_err(|error| error.to_string())?,
                    tree_uri: None,
                    remote_account: None,
                },
            })
            .map_err(|error| error.to_string())?;
        database
            .save_library_root(&LibraryRoot {
                id: source_id,
                kind: MediaSourceKind::PlaylistFile,
                display_name: display_name.clone(),
                locator: MediaLocator::FileSystem(path),
                enabled: true,
            })
            .map_err(|error| error.to_string())?;
        drop(_database_work);
        library_sync(app.clone()).await?;
        Ok(Some(display_name))
    }

    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (app, window, state);
        Err("建立 M3U8 播放清單目前只支援 Windows 與 Android。".to_owned())
    }
}

#[cfg(target_os = "windows")]
fn ensure_windows_my_love_source(
    state: &AppState,
    database: &Database,
) -> Result<SourceEntry, String> {
    let directory = user_data_directory(state)?;
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("無法建立 UserData 目錄：{error}"))?;
    let path = directory.join("MyLove.m3u8");
    if !path.exists() {
        std::fs::write(&path, "#EXTM3U\n#PLAYLIST:我的最愛\n")
            .map_err(|error| format!("無法建立 UserData/MyLove.m3u8：{error}"))?;
    }
    let settings = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?;
    let expected_path =
        settings::StoredPath::from_path(&path).map_err(|error| error.to_string())?;
    if let Some(mut source) = settings
        .sources
        .iter()
        .find(|source| {
            is_my_love_source(source)
                && matches!(source.kind, SourceEntryKind::PlaylistFile { .. })
                && source.path().to_path_buf().ok().as_deref() == Some(path.as_path())
        })
        .cloned()
    {
        let source_needs_save = normalize_windows_my_love_source(&mut source);
        if source_needs_save {
            state
                .settings
                .upsert_source(source.clone())
                .map_err(|error| error.to_string())?;
        }
        let root = source
            .database_projection()
            .map_err(|error| error.to_string())?;
        if !database
            .library_roots()
            .map_err(|error| error.to_string())?
            .iter()
            .any(|existing| existing.id == source.id && existing.enabled == source.enabled)
        {
            database
                .save_library_root(&root)
                .map_err(|error| error.to_string())?;
        }
        let playlist_id = match &source.kind {
            SourceEntryKind::PlaylistFile { playlist_id, .. } => *playlist_id,
            SourceEntryKind::Folder { .. } => return Err("我的最愛來源型態無效。".to_owned()),
        };
        if database
            .get_playlist(playlist_id)
            .map_err(|error| error.to_string())?
            .is_none()
        {
            let mut playlist = playlist_exchange::read_playlist_file(&path)?;
            playlist.id = playlist_id;
            for entry in &mut playlist.entries {
                entry.track_id = database
                    .resolve_track_id_for_locator(&entry.locator)
                    .map_err(|error| error.to_string())?;
            }
            database
                .save_playlist(&playlist)
                .map_err(|error| error.to_string())?;
        }
        return Ok(source);
    }
    // An entry for this file that is not a playlist source (settings rebuilt by an older version) is replaced.
    for stale in settings.sources.iter().filter(|source| {
        is_my_love_source(source) && matches!(source.kind, SourceEntryKind::Folder { .. })
    }) {
        let _ = state.settings.remove_source(stale.id);
    }
    let source_id = SourceId::new();
    let playlist_id = PlaylistId::new();
    let stored = expected_path;
    let source = SourceEntry {
        id: source_id,
        display_name: "我的最愛".to_owned(),
        enabled: true,
        kind: SourceEntryKind::PlaylistFile {
            playlist_id,
            path: stored,
            tree_uri: None,
            remote_account: None,
        },
    };
    state
        .settings
        .upsert_source(source.clone())
        .map_err(|error| error.to_string())?;
    let root = LibraryRoot {
        id: source_id,
        kind: MediaSourceKind::PlaylistFile,
        display_name: "我的最愛".to_owned(),
        locator: MediaLocator::FileSystem(path.clone()),
        enabled: true,
    };
    database
        .save_library_root(&root)
        .map_err(|error| error.to_string())?;
    let mut playlist = playlist_exchange::read_playlist_file(&path)?;
    playlist.id = playlist_id;
    for entry in &mut playlist.entries {
        entry.track_id = database
            .resolve_track_id_for_locator(&entry.locator)
            .map_err(|error| error.to_string())?;
    }
    database
        .save_playlist(&playlist)
        .map_err(|error| error.to_string())?;
    Ok(source)
}

#[cfg(target_os = "windows")]
fn normalize_windows_my_love_source(source: &mut SourceEntry) -> bool {
    if source.display_name == "我的最愛" {
        return false;
    }
    source.display_name = "我的最愛".to_owned();
    true
}

fn is_my_love_source(source: &SourceEntry) -> bool {
    source.display_name == "我的最愛" || is_my_love_path(source.path())
}

fn is_my_love_path(path: &settings::StoredPath) -> bool {
    path.to_path_buf()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().eq_ignore_ascii_case("MyLove.m3u8"))
        })
        .unwrap_or(false)
        || matches!(path, settings::StoredPath::Uri(uri) if uri.to_ascii_lowercase().ends_with("/mylove.m3u8"))
}

fn favorite_track_locators(
    state: &AppState,
    database: &Database,
    track_id: TrackId,
) -> Result<Vec<MediaLocator>, String> {
    #[cfg(target_os = "windows")]
    if let Some(temporary) = state
        .playback
        .as_ref()
        .and_then(|playback| playback.temporary_database_for_track(track_id))
    {
        if temporary
            .get_track_summary(track_id)
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return temporary
                .track_locators(track_id)
                .map_err(|error| error.to_string());
        }
    }
    let _ = state;
    database
        .track_locators(track_id)
        .map_err(|error| error.to_string())
}

fn same_favorite_locator(left: &MediaLocator, right: &MediaLocator) -> bool {
    match (left, right) {
        (MediaLocator::ContentUri(left), MediaLocator::ContentUri(right)) => left == right,
        (MediaLocator::FileSystem(left), MediaLocator::FileSystem(right)) => {
            #[cfg(target_os = "windows")]
            {
                player_core::windows_locator_key(left) == player_core::windows_locator_key(right)
            }
            #[cfg(target_os = "android")]
            {
                left == right
                    || std::fs::canonicalize(left).ok().is_some_and(|left| {
                        std::fs::canonicalize(right).ok().as_deref() == Some(left.as_path())
                    })
            }
            #[cfg(not(any(target_os = "windows", target_os = "android")))]
            {
                left == right
            }
        }
        _ => false,
    }
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn read_favorite_source(
    _app: &AppHandle,
    source: &SourceEntry,
) -> Result<(Option<std::path::PathBuf>, Vec<u8>), String> {
    let path = source
        .path()
        .to_path_buf()
        .map_err(|error| error.to_string())?;
    let bytes = std::fs::read(&path).map_err(|error| format!("無法重新讀取播放清單：{error}"))?;
    Ok((Some(path), bytes))
}

fn resolve_favorite_entry_ids(
    app: &AppHandle,
    database: &Database,
    source: &SourceEntry,
    playlist: &mut player_core::Playlist,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = (app, source);
        for entry in &mut playlist.entries {
            entry.track_id = database
                .resolve_track_id_for_locator(&entry.locator)
                .map_err(|error| error.to_string())?;
        }
    }
    #[cfg(target_os = "android")]
    {
        // Real paths (ADR 0014): entries resolve by their canonical path, like library tracks.
        let _ = (app, source);
        for entry in &mut playlist.entries {
            let locator = match &entry.locator {
                MediaLocator::FileSystem(path) => MediaLocator::FileSystem(
                    std::fs::canonicalize(path).unwrap_or_else(|_| path.clone()),
                ),
                other => other.clone(),
            };
            entry.track_id = database
                .resolve_track_id_for_locator(&locator)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn write_favorite_source(
    _app: &AppHandle,
    source: &SourceEntry,
    path: Option<&std::path::Path>,
    bytes: &[u8],
) -> Result<(), String> {
    let path = path.ok_or_else(|| "播放清單檔案路徑遺失。".to_owned())?;
    let parent = path
        .parent()
        .ok_or_else(|| "播放清單沒有父目錄。".to_owned())?;
    let mut temp = path.as_os_str().to_os_string();
    temp.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let temp = std::path::PathBuf::from(temp);
    std::fs::write(&temp, bytes).map_err(|error| format!("無法寫入播放清單暫存檔：{error}"))?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("無法原子更新播放清單：{error}"));
    }
    let _ = (source, parent);
    Ok(())
}

#[tauri::command]
async fn playlist_import_m3u(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<PlaylistImportResult>, String> {
    #[cfg(any(target_os = "windows", target_os = "android"))]
    {
        #[cfg(target_os = "windows")]
        let path = {
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
            selected
                .into_path()
                .map_err(|error| format!("無法取得選取的本機播放清單路徑：{error}"))?
        };
        // Android: the picker only locates the file; it is read through its real path under
        // All files access, with no persisted SAF grant (ADR 0014).
        #[cfg(target_os = "android")]
        let path = {
            let _ = &window;
            let picker_app = app.clone();
            let picked = tauri::async_runtime::spawn_blocking(move || {
                picker_app.media_index().pick_document_path(&["*/*"])
            })
            .await
            .map_err(|error| format!("播放清單選擇工作失敗：{error}"))?
            .map_err(|error| error.to_string())?;
            let Some(path) = picked else {
                return Ok(None);
            };
            let path = std::path::PathBuf::from(path);
            if !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("m3u") || extension.eq_ignore_ascii_case("m3u8")
                })
            {
                return Err("選取的檔案不是 .m3u 或 .m3u8 播放清單。".to_owned());
            }
            path
        };
        let _database_work = enter_database_work(&state)?;
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
                    tree_uri: None,
                    remote_account: None,
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
        drop(_database_work);
        let _ = library_sync(app.clone()).await;
        let post_sync_state = app.state::<AppState>();
        let _post_sync_database_work = enter_database_work(&post_sync_state)?;
        if let Some(database) = post_sync_state.database.as_ref() {
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

    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (app, window, state);
        Err("M3U/M3U8 原生檔案匯入目前只支援 Windows 與 Android。".to_owned())
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
        let _database_work = enter_database_work(&state)?;
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
        #[cfg(target_os = "android")]
        {
            let _ = window;
            let database_path = state.database_path.clone().ok_or_else(|| {
                state
                    .database_error
                    .clone()
                    .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
            })?;
            let playlist_id = PlaylistId::parse(&playlist_id)
                .map_err(|_| "播放清單識別碼無效，請重新載入清單。".to_owned())?;
            let playlist = {
                let _database_work = enter_database_work(&state)?;
                let database = state.database.as_ref().ok_or_else(|| {
                    state
                        .database_error
                        .clone()
                        .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
                })?;
                database
                    .get_playlist(playlist_id)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| "找不到這份播放清單，請重新載入清單。".to_owned())?
            };
            let extension = match format.as_str() {
                "m3u" | "m3u8" => format,
                _ => return Err("匯出格式必須是 M3U 或 M3U8。".to_owned()),
            };
            let suggested_name = format!("{}.{}", playlist.name, extension);
            let app_for_export = app.clone();
            let exported = tauri::async_runtime::spawn_blocking(
                move || -> Result<Option<PlaylistExportResult>, String> {
                    // The picker only names the file; it is written through its real path
                    // under All files access (ADR 0014). Relative entries use its folder.
                    let Some(path) = app_for_export
                        .media_index()
                        .pick_create_document_path(&suggested_name, "audio/x-mpegurl")
                        .map_err(|error| error.to_string())?
                    else {
                        return Ok(None);
                    };
                    let path = std::path::PathBuf::from(path);
                    let relative_root = relative_paths
                        .then(|| path.parent().map(std::path::Path::to_path_buf))
                        .flatten();
                    let database =
                        Database::open(database_path).map_err(|error| error.to_string())?;
                    let result = export_playlist_file(
                        &database,
                        &playlist_id.to_string(),
                        &path,
                        &extension,
                        relative_root.as_deref(),
                    )?;
                    Ok(Some(result))
                },
            )
            .await
            .map_err(|error| format!("匯出 Android 播放清單工作失敗：{error}"))??;
            Ok(exported)
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (app, window, state, playlist_id, format, relative_paths);
            Err("M3U/M3U8 原生檔案匯出目前只支援 Windows 與 Android。".to_owned())
        }
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
        .map(|source| {
            source_entry_summary_with(state.database.as_ref(), source, &settings.remote_accounts)
        })
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
    let saved_folder = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?
        .theme
        .slideshow_folder;
    let preferences = ThemeSettings {
        background_hex: preferences.background_hex,
        accent_hex: preferences.accent_hex,
        border_opacity_percent: preferences.border_opacity_percent,
        divider_opacity_percent: preferences.divider_opacity_percent,
        color_mode: preferences.color_mode,
        background_opacity_text_color: preferences.background_opacity_text_color,
        main_background_clarity_percent: preferences.main_background_clarity_percent,
        cover_background_clarity_percent: preferences.cover_background_clarity_percent,
        quick_settings_opacity_percent: preferences.quick_settings_opacity_percent,
        quick_settings_blur_px: preferences.quick_settings_blur_px,
        main_background_blur_px: preferences.main_background_blur_px,
        main_background_opacity_percent: preferences.main_background_opacity_percent,
        main_background_mode: preferences.main_background_mode,
        slideshow_folder: saved_folder,
        slideshow_interval_minutes: preferences.slideshow_interval_minutes,
        slideshow_order: preferences.slideshow_order,
        auto_text_color_threshold_percent: preferences.auto_text_color_threshold_percent,
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

const MAIN_BACKGROUND_FILE_NAME: &str = "background";
const MAIN_BACKGROUND_MAX_BYTES: u64 = 32 * 1024 * 1024;

fn user_data_directory(state: &AppState) -> Result<std::path::PathBuf, String> {
    state
        .settings
        .data_directory()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| "無法定位 UserData 資料夾。".to_owned())
}

fn supported_background_image(bytes: &[u8]) -> bool {
    let jpeg = bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff;
    let png = bytes.len() >= 8 && bytes.starts_with(b"\x89PNG\r\n\x1a\n");
    let gif = bytes.len() >= 6 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"));
    let webp = bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP";
    let bmp = bytes.len() >= 2 && bytes.starts_with(b"BM");
    jpeg || png || gif || webp || bmp
}

#[cfg(test)]
mod main_background_tests {
    use super::supported_background_image;

    #[test]
    fn accepts_png_jpeg_and_rejects_plain_text() {
        assert!(supported_background_image(b"\x89PNG\r\n\x1a\nrest"));
        assert!(supported_background_image(&[0xff, 0xd8, 0xff, 0x00]));
        assert!(!supported_background_image(b"not an image"));
    }
}

fn install_main_background(state: &AppState, source: &std::path::Path) -> Result<(), String> {
    let metadata =
        std::fs::metadata(source).map_err(|error| format!("無法讀取背景圖片：{error}"))?;
    if !metadata.is_file() {
        return Err("請選擇一個圖片檔。".to_owned());
    }
    if metadata.len() == 0 || metadata.len() > MAIN_BACKGROUND_MAX_BYTES {
        return Err("背景圖片必須小於 32 MiB。".to_owned());
    }
    let bytes = std::fs::read(source).map_err(|error| format!("無法讀取背景圖片：{error}"))?;
    if !supported_background_image(&bytes) {
        return Err("只接受 PNG、JPEG、WebP、GIF 或 BMP 圖片。".to_owned());
    }
    let directory = user_data_directory(state)?;
    std::fs::create_dir_all(&directory).map_err(|error| format!("無法建立 UserData：{error}"))?;
    let destination = directory.join(MAIN_BACKGROUND_FILE_NAME);
    if let (Ok(source_canonical), Ok(destination_canonical)) = (
        std::fs::canonicalize(source),
        std::fs::canonicalize(&destination),
    ) {
        if source_canonical == destination_canonical {
            return Ok(());
        }
    }
    let temporary = destination.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&temporary, &bytes).map_err(|error| format!("無法寫入背景圖片：{error}"))?;
    if destination.exists() {
        let backup = destination.with_extension("replacing");
        if std::fs::rename(&destination, &backup).is_err() {
            let _ = std::fs::remove_file(&temporary);
            return Err("無法替換現有的背景圖片。".to_owned());
        }
        if let Err(error) = std::fs::rename(&temporary, &destination) {
            let _ = std::fs::rename(&backup, &destination);
            return Err(format!("無法保存背景圖片：{error}"));
        }
        let _ = std::fs::remove_file(backup);
    } else if let Err(error) = std::fs::rename(&temporary, &destination) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("無法保存背景圖片：{error}"));
    }
    Ok(())
}

#[tauri::command]
async fn appearance_pick_main_background(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<bool>, String> {
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_dialog::DialogExt;

        let selected = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("選擇主介面背景")
            .add_filter("圖片", &["png", "jpg", "jpeg", "webp", "gif", "bmp"])
            .blocking_pick_file();
        let Some(selected) = selected else {
            return Ok(None);
        };
        let path = selected
            .into_path()
            .map_err(|error| format!("無法取得選取的圖片路徑：{error}"))?;
        install_main_background(&state, &path)?;
        state
            .settings
            .update(|settings| {
                settings.theme.main_background_mode = MainBackgroundMode::SingleImage;
                Ok(())
            })
            .map_err(|error| error.to_string())?;
        Ok(Some(true))
    }
    #[cfg(target_os = "android")]
    {
        let _ = (window, state);
        return tauri::async_runtime::spawn_blocking(move || -> Result<Option<bool>, String> {
            let index = app.media_index();
            // Photo Picker first (no storage permission, image-only); document picker fallback.
            let Some(uri) = index
                .pick_visual_image(&ANDROID_BACKGROUND_IMAGE_MIME_TYPES)
                .map_err(|error| error.to_string())?
            else {
                return Ok(None);
            };
            // Copy the picked document into UserData so the background survives a revoked grant.
            let lease = index
                .cache_content_uri(&uri, MAIN_BACKGROUND_MAX_BYTES)
                .map_err(|error| format!("無法讀取背景圖片：{error}"))?;
            let state = app.state::<AppState>();
            install_main_background(&state, lease.path())?;
            drop(lease);
            state
                .settings
                .update(|settings| {
                    settings.theme.main_background_mode = MainBackgroundMode::SingleImage;
                    Ok(())
                })
                .map_err(|error| error.to_string())?;
            Ok(Some(true))
        })
        .await
        .map_err(|error| error.to_string())?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (app, window, state);
        Err("此平台尚未提供背景圖片選擇。".to_owned())
    }
}

#[tauri::command]
async fn appearance_pick_slideshow_folder(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<bool>, String> {
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_dialog::DialogExt;

        let selected = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("選擇幻燈片圖片資料夾")
            .blocking_pick_folder();
        let Some(selected) = selected else {
            return Ok(None);
        };
        let folder = selected
            .into_path()
            .map_err(|error| format!("無法取得選取的資料夾路徑：{error}"))?;
        let order = state
            .settings
            .snapshot()
            .map_err(|error| error.to_string())?
            .theme
            .slideshow_order;
        state.main_background.configure_slideshow(&folder, order)?;
        let stored_path =
            settings::StoredPath::from_path(&folder).map_err(|error| error.to_string())?;
        state
            .settings
            .update(|settings| {
                settings.theme.slideshow_folder = Some(stored_path);
                settings.theme.main_background_mode = MainBackgroundMode::Slideshow;
                Ok(())
            })
            .map_err(|error| error.to_string())?;
        Ok(Some(true))
    }
    // Android: the folder picker only locates the directory; images are listed and read
    // through its real path under All files access (ADR 0014).
    #[cfg(target_os = "android")]
    {
        let _ = (window, state);
        return tauri::async_runtime::spawn_blocking(move || -> Result<Option<bool>, String> {
            let Some(folder) = android_folder_browser::pick_directory(&app)? else {
                return Ok(None);
            };
            let folder = std::path::PathBuf::from(folder);
            let state = app.state::<AppState>();
            let order = state
                .settings
                .snapshot()
                .map_err(|error| error.to_string())?
                .theme
                .slideshow_order;
            state.main_background.configure_slideshow(&folder, order)?;
            let stored_path =
                settings::StoredPath::from_path(&folder).map_err(|error| error.to_string())?;
            state
                .settings
                .update(|settings| {
                    settings.theme.slideshow_folder = Some(stored_path);
                    settings.theme.main_background_mode = MainBackgroundMode::Slideshow;
                    Ok(())
                })
                .map_err(|error| error.to_string())?;
            Ok(Some(true))
        })
        .await
        .map_err(|error| error.to_string())?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (app, window, state);
        Err("此平台尚未提供幻燈片資料夾選擇。".to_owned())
    }
}

#[tauri::command]
fn appearance_clear_main_background(state: State<'_, AppState>) -> Result<(), String> {
    let path = user_data_directory(&state)?.join(MAIN_BACKGROUND_FILE_NAME);
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("無法移除背景圖片：{error}")),
    }
}

#[tauri::command]
async fn appearance_get_main_background(app: AppHandle) -> Result<Response, String> {
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        main_background_bytes(&app, &state)
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(Response::new(bytes))
}

fn main_background_bytes(app: &AppHandle, state: &AppState) -> Result<Vec<u8>, String> {
    let theme = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?
        .theme;
    let image = match theme.main_background_mode {
        MainBackgroundMode::Solid | MainBackgroundMode::AlbumCover => None,
        MainBackgroundMode::SingleImage => {
            Some(user_data_directory(&state)?.join(MAIN_BACKGROUND_FILE_NAME))
        }
        MainBackgroundMode::Slideshow => return next_slideshow_bytes(app, state, &theme),
        MainBackgroundMode::SystemWallpaper => {
            #[cfg(target_os = "windows")]
            {
                match windows_system_wallpaper_path() {
                    Ok(path) => Some(path),
                    Err(_) => return Ok(Vec::new()),
                }
            }
            #[cfg(target_os = "android")]
            {
                // With All files access (ADR 0014) the static wallpaper is copied like the
                // Windows one. Otherwise (no permission / live wallpaper) return nothing, and the
                // frontend falls back to FLAG_SHOW_WALLPAPER compositing (ADR 0008).
                match app.media_index().read_system_wallpaper() {
                    Ok(read) => match read.path {
                        Some(path) => Some(std::path::PathBuf::from(path)),
                        None => return Ok(Vec::new()),
                    },
                    Err(_) => return Ok(Vec::new()),
                }
            }
            #[cfg(not(any(target_os = "windows", target_os = "android")))]
            {
                return Err("系統桌布背景目前只支援 Windows。".to_owned());
            }
        }
    };
    let Some(path) = image else {
        return Ok(Vec::new());
    };
    match read_background_image(&path) {
        Ok(bytes) => Ok(bytes),
        Err(_) if theme.main_background_mode == MainBackgroundMode::SystemWallpaper => {
            Ok(Vec::new())
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
async fn appearance_next_slideshow_background(app: AppHandle) -> Result<Response, String> {
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let theme = state
            .settings
            .snapshot()
            .map_err(|error| error.to_string())?
            .theme;
        if theme.main_background_mode != MainBackgroundMode::Slideshow {
            return Err("目前沒有啟用幻燈片背景。".to_owned());
        }
        next_slideshow_bytes(&app, &state, &theme)
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(Response::new(bytes))
}

#[cfg(target_os = "android")]
const ANDROID_BACKGROUND_IMAGE_MIME_TYPES: [&str; 5] = [
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/bmp",
];

/// Advance the slideshow and read the next image from the folder's real path.
fn next_slideshow_bytes(
    app: &AppHandle,
    state: &AppState,
    theme: &settings::ThemeSettings,
) -> Result<Vec<u8>, String> {
    let folder = theme
        .slideshow_folder
        .as_ref()
        .ok_or_else(|| "請先選擇幻燈片背景資料夾。".to_owned())?;
    if matches!(folder, settings::StoredPath::Uri(_)) {
        return Err("幻燈片資料夾使用舊版 SAF 授權；請重新選擇資料夾。".to_owned());
    }
    let _ = app;
    let folder = folder.to_path_buf().map_err(|error| error.to_string())?;
    let path = state
        .main_background
        .next_slideshow_image(&folder, theme.slideshow_order)?;
    read_background_image(&path)
}

/// Android All files access (MANAGE_EXTERNAL_STORAGE) state; None on other platforms.
#[tauri::command]
async fn android_all_files_access_status(
    app: AppHandle,
) -> Result<Option<tauri_plugin_media_index::AllFilesAccessStatus>, String> {
    #[cfg(target_os = "android")]
    {
        return tauri::async_runtime::spawn_blocking(move || {
            app.media_index()
                .all_files_access_status()
                .map(Some)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(None)
    }
}

/// Open the Android All files access settings page for this app.
#[tauri::command]
async fn android_request_all_files_access(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        return tauri::async_runtime::spawn_blocking(move || {
            app.media_index()
                .request_all_files_access()
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err("Only Android has All files access".to_owned())
    }
}

/// Android: show the system wallpaper behind the transparent WebView. No-op elsewhere.
#[tauri::command]
async fn appearance_set_android_wallpaper_backdrop(
    app: AppHandle,
    enabled: bool,
    blur_px: u32,
) -> Result<Option<tauri_plugin_media_index::WallpaperBackdropStatus>, String> {
    #[cfg(target_os = "android")]
    {
        return tauri::async_runtime::spawn_blocking(move || {
            app.media_index()
                .set_wallpaper_backdrop(enabled, blur_px)
                .map(Some)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, enabled, blur_px);
        Ok(None)
    }
}

fn read_background_image(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path).map_err(|error| format!("無法讀取背景圖片：{error}"))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAIN_BACKGROUND_MAX_BYTES {
        return Err("背景圖片必須是小於 32 MiB 的圖片檔。".to_owned());
    }
    let bytes = std::fs::read(path).map_err(|error| format!("無法讀取背景圖片：{error}"))?;
    if !supported_background_image(&bytes) {
        return Err("背景檔案不是支援的 PNG、JPEG、WebP、GIF 或 BMP 圖片。".to_owned());
    }
    Ok(bytes)
}

#[cfg(target_os = "windows")]
fn windows_system_wallpaper_path() -> Result<std::path::PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    let mut buffer = vec![0u16; 32_768];
    unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buffer.len() as u32,
            Some(buffer.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .map_err(|error| format!("無法讀取 Windows 系統桌布：{error}"))?;
    }
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    if length == 0 || length == buffer.len() {
        return Err("Windows 沒有回報目前的系統桌布圖片。".to_owned());
    }
    Ok(std::path::PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..length],
    )))
}

#[tauri::command]
fn settings_get_lyrics_preferences(
    state: State<'_, AppState>,
) -> Result<LyricsPreferences, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.lyrics_preferences)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_set_lyrics_preferences(
    state: State<'_, AppState>,
    preferences: LyricsPreferences,
) -> Result<LyricsPreferences, String> {
    state
        .settings
        .update(|settings| {
            settings.lyrics_preferences = preferences;
            Ok(())
        })
        .map(|settings| settings.lyrics_preferences)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_get_now_playing_appearance_preferences(
    state: State<'_, AppState>,
) -> Result<NowPlayingAppearancePreferences, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.now_playing_appearance_preferences)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_set_now_playing_appearance_preferences(
    state: State<'_, AppState>,
    preferences: NowPlayingAppearancePreferences,
) -> Result<NowPlayingAppearancePreferences, String> {
    state
        .settings
        .update(|settings| {
            settings.now_playing_appearance_preferences = preferences;
            Ok(())
        })
        .map(|settings| settings.now_playing_appearance_preferences)
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
            let mut next = preferences.clone();
            next.widths = next.widths.clamp();
            next.row_height_px = next.row_height_px.clamp(30, 60);
            next.album_card_max_width_px =
                settings::clamp_album_card_max_width_px(next.album_card_max_width_px);
            settings.track_list_columns = next;
            Ok(())
        })
        .map(|settings| settings.track_list_columns)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_get_artist_list_sort(state: State<'_, AppState>) -> Result<ArtistListSort, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.artist_list_sort)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_set_artist_list_sort(
    state: State<'_, AppState>,
    preference: ArtistListSort,
) -> Result<ArtistListSort, String> {
    state
        .settings
        .update(|settings| {
            settings.artist_list_sort = preference;
            Ok(())
        })
        .map(|settings| settings.artist_list_sort)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_get_album_index_sort(state: State<'_, AppState>) -> Result<AlbumIndexSort, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.album_index_sort)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_set_album_index_sort(
    state: State<'_, AppState>,
    preference: AlbumIndexSort,
) -> Result<AlbumIndexSort, String> {
    state
        .settings
        .update(|settings| {
            settings.album_index_sort = preference;
            Ok(())
        })
        .map(|settings| settings.album_index_sort)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_get_album_detail_sort(state: State<'_, AppState>) -> Result<AlbumDetailSort, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.album_detail_sort)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_set_album_detail_sort(
    state: State<'_, AppState>,
    preference: AlbumDetailSort,
) -> Result<AlbumDetailSort, String> {
    state
        .settings
        .update(|settings| {
            settings.album_detail_sort = preference;
            Ok(())
        })
        .map(|settings| settings.album_detail_sort)
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
fn settings_get_shortcuts(
    state: State<'_, AppState>,
) -> Result<crate::settings::ShortcutSettings, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.shortcuts)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_set_shortcuts(
    state: State<'_, AppState>,
    shortcuts: crate::settings::ShortcutSettings,
) -> Result<crate::settings::ShortcutSettings, String> {
    state
        .settings
        .update(|settings| {
            settings.shortcuts = shortcuts;
            Ok(())
        })
        .map(|settings| settings.shortcuts)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_get_dsee_hx(state: State<'_, AppState>) -> Result<bool, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.dsee_hx)
        .map_err(|error| error.to_string())
}

/// Persist the DSEE HX switch, then apply it to the running player.
#[tauri::command]
async fn settings_set_dsee_hx(state: State<'_, AppState>, enabled: bool) -> Result<bool, String> {
    let saved = state
        .settings
        .update(|settings| {
            settings.dsee_hx = enabled;
            Ok(())
        })
        .map(|settings| settings.dsee_hx)
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "windows")]
    {
        if let Some(service) = state.playback.as_ref() {
            let applied = match set_playback_dsee_hx(service, saved) {
                Ok(ticket) => await_playback_ack_within(ticket, Duration::from_secs(10)).await,
                Err(error) => Err(error),
            };
            applied.map_err(|error| format!("設定已保存，但套用到播放器失敗：{error}"))?;
        }
    }
    Ok(saved)
}

#[tauri::command]
fn settings_get_cover_zoom(
    state: State<'_, AppState>,
) -> Result<settings::CoverZoomSettings, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.cover_zoom)
        .map_err(|error| error.to_string())
}

/// Persist the Now Playing cover size. Only a shrunk cover (0.4 to 1) is remembered; an enlarged one is saved as 1.
#[tauri::command]
fn settings_set_cover_zoom(
    state: State<'_, AppState>,
    scale: f32,
) -> Result<settings::CoverZoomSettings, String> {
    let requested = settings::CoverZoomSettings { scale }.normalized();
    state
        .settings
        .update(|settings| {
            settings.cover_zoom = requested;
            Ok(())
        })
        .map(|settings| settings.cover_zoom)
        .map_err(|error| error.to_string())
}

/// One playback device as the output settings list it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioOutputDevice {
    id: String,
    name: String,
    is_default: bool,
    channels: u16,
    sample_rate: u32,
}

/// What a chosen device is doing right now (`id` is `None` for the system default).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioOutputDeviceStatus {
    id: Option<String>,
    /// `playing`, `opening` or `lost`.
    state: &'static str,
    message: Option<String>,
    device_rate_hz: Option<u32>,
    channels: Option<u16>,
    /// A device whose rate differs from the track's gets a converted copy.
    converted: bool,
}

/// The playback devices Windows offers right now, the system default first. Android has no device choice.
#[tauri::command]
async fn audio_list_output_devices() -> Result<Vec<AudioOutputDevice>, String> {
    #[cfg(target_os = "windows")]
    {
        tauri::async_runtime::spawn_blocking(|| {
            player_audio_windows::output_devices::list_output_devices()
                .into_iter()
                .map(|device| AudioOutputDevice {
                    id: device.id,
                    name: device.name,
                    is_default: device.is_default,
                    channels: device.channels,
                    sample_rate: device.sample_rate,
                })
                .collect()
        })
        .await
        .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "windows"))]
    Ok(Vec::new())
}

/// A playback device's real Windows volume (the one Windows' volume mixer shows).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioDeviceVolume {
    id: String,
    /// 0 to 100.
    volume_percent: f32,
    muted: bool,
}

/// The real volume of every playback device Windows lists right now.
#[tauri::command]
async fn audio_get_device_volumes() -> Result<Vec<AudioDeviceVolume>, String> {
    #[cfg(target_os = "windows")]
    {
        tauri::async_runtime::spawn_blocking(|| {
            player_audio_windows::output_devices::list_output_devices()
                .into_iter()
                .filter_map(|device| {
                    let volume = player_platform_windows::endpoint_volume::read_endpoint_volume(
                        Some(&device.id),
                    )
                    .ok()?;
                    Some(AudioDeviceVolume {
                        id: device.id,
                        volume_percent: volume.volume * 100.0,
                        muted: volume.muted,
                    })
                })
                .collect()
        })
        .await
        .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "windows"))]
    Ok(Vec::new())
}

/// Set a playback device's real volume. Like the reference mapper, volume 0 also mutes and anything above unmutes.
#[tauri::command]
async fn audio_set_device_volume(id: String, volume_percent: f32) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        tauri::async_runtime::spawn_blocking(move || {
            use player_platform_windows::endpoint_volume::{
                set_endpoint_muted, set_endpoint_volume,
            };
            let volume = (volume_percent / 100.0).clamp(0.0, 1.0);
            set_endpoint_volume(Some(&id), volume)?;
            set_endpoint_muted(Some(&id), volume <= 0.0)?;
            // With the volume link on, the other chosen devices follow (the backend keeps their proportions).
            volume_sync::service::local_change(id, (volume * 100.0).round() as u8);
            Ok(())
        })
        .await
        .map_err(|error| error.to_string())?
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (id, volume_percent);
        Err("只有 Windows 能調整裝置音量。".to_owned())
    }
}

#[tauri::command]
fn audio_output_device_status(state: State<'_, AppState>) -> Vec<AudioOutputDeviceStatus> {
    #[cfg(target_os = "windows")]
    if let Some(service) = state.playback.as_ref() {
        use player_audio_windows::output_devices::DeviceState;
        return service
            .player
            .output_device_status()
            .into_iter()
            .map(|status| AudioOutputDeviceStatus {
                id: status.id,
                state: match status.state {
                    DeviceState::Playing => "playing",
                    DeviceState::Opening => "opening",
                    DeviceState::Lost => "lost",
                },
                message: status.message,
                device_rate_hz: status.device_rate,
                channels: status.channels,
                converted: status.converted,
            })
            .collect();
    }
    #[cfg(not(target_os = "windows"))]
    let _ = &state;
    Vec::new()
}

#[tauri::command]
fn settings_get_output_devices(
    state: State<'_, AppState>,
) -> Result<Vec<settings::OutputDeviceEntry>, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.output_devices)
        .map_err(|error| error.to_string())
}

/// Persist the chosen output devices, then apply them: mirrors open in the background, and the main output is rebuilt
/// only when its device or channel map changed.
#[tauri::command]
async fn settings_set_output_devices(
    state: State<'_, AppState>,
    devices: Vec<settings::OutputDeviceEntry>,
) -> Result<Vec<settings::OutputDeviceEntry>, String> {
    store_and_apply_output_devices(&state, devices).await
}

async fn store_and_apply_output_devices(
    state: &AppState,
    devices: Vec<settings::OutputDeviceEntry>,
) -> Result<Vec<settings::OutputDeviceEntry>, String> {
    let requested = settings::normalize_output_devices(devices);
    let saved = state
        .settings
        .update(|settings| {
            settings.output_devices = requested.clone();
            Ok(())
        })
        .map(|settings| settings.output_devices)
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "windows")]
    {
        if let Some(service) = state.playback.as_ref() {
            let applied = match set_playback_output_devices(service, &saved) {
                Ok(ticket) => await_playback_ack_within(ticket, Duration::from_secs(15)).await,
                Err(error) => Err(error),
            };
            applied.map_err(|error| format!("設定已保存，但套用到播放器失敗：{error}"))?;
        }
    }
    Ok(saved)
}

/// The three output-device presets (`null`: an empty slot).
#[tauri::command]
fn settings_get_output_presets(
    state: State<'_, AppState>,
) -> Result<Vec<Option<Vec<settings::OutputDeviceEntry>>>, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.output_presets)
        .map_err(|error| error.to_string())
}

fn output_preset_slot(slot: usize) -> Result<usize, String> {
    if slot < settings::OUTPUT_PRESET_SLOTS {
        Ok(slot)
    } else {
        Err("方案編號無效。".to_owned())
    }
}

/// Save the devices that are chosen right now into a preset slot.
#[tauri::command]
fn settings_save_output_preset(
    state: State<'_, AppState>,
    slot: usize,
) -> Result<Vec<Option<Vec<settings::OutputDeviceEntry>>>, String> {
    let slot = output_preset_slot(slot)?;
    state
        .settings
        .update(|settings| {
            let current = settings.output_devices.clone();
            settings.output_presets[slot] = Some(current).filter(|devices| !devices.is_empty());
            Ok(())
        })
        .map(|settings| settings.output_presets)
        .map_err(|error| error.to_string())
}

/// Empty a preset slot.
#[tauri::command]
fn settings_clear_output_preset(
    state: State<'_, AppState>,
    slot: usize,
) -> Result<Vec<Option<Vec<settings::OutputDeviceEntry>>>, String> {
    let slot = output_preset_slot(slot)?;
    state
        .settings
        .update(|settings| {
            settings.output_presets[slot] = None;
            Ok(())
        })
        .map(|settings| settings.output_presets)
        .map_err(|error| error.to_string())
}

/// Switch to a preset: its devices become the chosen devices and are applied to the player.
#[tauri::command]
async fn settings_apply_output_preset(
    state: State<'_, AppState>,
    slot: usize,
) -> Result<Vec<settings::OutputDeviceEntry>, String> {
    let slot = output_preset_slot(slot)?;
    let preset = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?
        .output_presets
        .get(slot)
        .cloned()
        .flatten()
        .ok_or_else(|| "這個方案是空的。".to_owned())?;
    store_and_apply_output_devices(&state, preset).await
}

#[tauri::command]
fn settings_get_output_volume_sync(state: State<'_, AppState>) -> Result<bool, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.output_volume_sync)
        .map_err(|error| error.to_string())
}

/// Switch "音量同步" on or off. It takes effect at once and keeps working with no settings UI open.
#[tauri::command]
fn settings_set_output_volume_sync(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<bool, String> {
    let saved = state
        .settings
        .update(|settings| {
            settings.output_volume_sync = enabled;
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "windows")]
    volume_sync::service::set_enabled(saved.output_volume_sync);
    Ok(saved.output_volume_sync)
}

#[tauri::command]
fn settings_get_output_exclusive(state: State<'_, AppState>) -> Result<bool, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.output_exclusive)
        .map_err(|error| error.to_string())
}

/// Switch WASAPI exclusive output on or off. Exclusive mode plays on one device, so turning it on keeps only the
/// first chosen device; the player rebuilds its output and the current track carries on from where it was.
#[tauri::command]
async fn settings_set_output_exclusive(
    state: State<'_, AppState>,
    exclusive: bool,
) -> Result<bool, String> {
    let saved = state
        .settings
        .update(|settings| {
            settings.output_exclusive = exclusive;
            if exclusive {
                let mut seen_enabled = false;
                for entry in settings.output_devices.iter_mut() {
                    if entry.enabled {
                        if seen_enabled {
                            entry.enabled = false;
                        }
                        seen_enabled = true;
                    }
                }
            }
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "windows")]
    {
        if let Some(service) = state.playback.as_ref() {
            let applied = match set_playback_output_devices(service, &saved.output_devices) {
                Ok(ticket) => await_playback_ack_within(ticket, Duration::from_secs(15)).await,
                Err(error) => Err(error),
            };
            let applied = match applied {
                Ok(_) => match set_playback_exclusive(service, exclusive) {
                    Ok(ticket) => await_playback_ack_within(ticket, Duration::from_secs(15)).await,
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            };
            applied.map_err(|error| format!("設定已保存，但套用到播放器失敗：{error}"))?;
        }
    }
    Ok(saved.output_exclusive)
}

#[tauri::command]
fn settings_get_dsd(state: State<'_, AppState>) -> Result<settings::DsdPlaybackSettings, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.dsd)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_get_language(state: State<'_, AppState>) -> Result<settings::UiLanguage, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.language)
        .map_err(|error| error.to_string())
}

/// Persist the interface language and tell every window (each reloads itself to apply it).
#[tauri::command]
fn settings_set_language(
    app: AppHandle,
    state: State<'_, AppState>,
    language: settings::UiLanguage,
) -> Result<settings::UiLanguage, String> {
    let saved = state
        .settings
        .update(|stored| {
            stored.language = language;
            Ok(())
        })
        .map(|stored| stored.language)
        .map_err(|error| error.to_string())?;
    let _ = app.emit("app-language-changed", saved);
    Ok(saved)
}

#[tauri::command]
fn settings_get_taskbar_lyrics(
    state: State<'_, AppState>,
) -> Result<settings::TaskbarLyricsSettings, String> {
    state
        .settings
        .snapshot()
        .map(|settings| settings.taskbar_lyrics)
        .map_err(|error| error.to_string())
}

/// Persist the taskbar-lyrics settings and apply them: the window opens or closes, and every window gets the new
/// values through the `taskbar-lyrics-settings` event.
#[tauri::command]
async fn settings_set_taskbar_lyrics(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: settings::TaskbarLyricsSettings,
) -> Result<settings::TaskbarLyricsSettings, String> {
    let requested = settings.normalized();
    let saved = state
        .settings
        .update(|stored| {
            stored.taskbar_lyrics = requested.clone();
            Ok(())
        })
        .map(|stored| stored.taskbar_lyrics)
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "windows")]
    taskbar_lyrics::apply(&app, &saved).await?;
    #[cfg(not(target_os = "windows"))]
    let _ = &app;
    Ok(saved)
}

/// Whether the Windows taskbar is in its light theme (the taskbar lyrics colour themselves from this).
#[tauri::command]
fn taskbar_lyrics_theme() -> bool {
    #[cfg(target_os = "windows")]
    return taskbar_lyrics::system_uses_light_theme();
    #[cfg(not(target_os = "windows"))]
    false
}

/// Persist the DSD gain and output rate, then apply them to the running player: the gain eases in smoothly while a
/// track plays; a new output rate restarts a playing DSD track from its position.
#[tauri::command]
async fn settings_set_dsd(
    state: State<'_, AppState>,
    gain_db: f32,
    output_rate_hz: u32,
    dop: Option<bool>,
) -> Result<settings::DsdPlaybackSettings, String> {
    let current = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?
        .dsd;
    let requested = settings::DsdPlaybackSettings {
        gain_db,
        output_rate_hz,
        dop: dop.unwrap_or(current.dop),
    }
    .normalized();
    let saved = state
        .settings
        .update(|settings| {
            settings.dsd = requested;
            Ok(())
        })
        .map(|settings| settings.dsd)
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "android")]
    android_dsd::set_gain_db(saved.gain_db);
    #[cfg(target_os = "windows")]
    {
        if let Some(service) = state.playback.as_ref() {
            let applied = match set_playback_dsd(service, saved) {
                Ok(ticket) => await_playback_ack_within(ticket, Duration::from_secs(10)).await,
                Err(error) => Err(error),
            };
            applied.map_err(|error| format!("設定已保存，但套用到播放器失敗：{error}"))?;
        }
    }
    Ok(saved)
}

#[tauri::command]
fn library_add_windows_folder(
    state: State<'_, AppState>,
    path: String,
) -> Result<LibrarySource, String> {
    #[cfg(target_os = "windows")]
    {
        let _database_work = enter_database_work(&state)?;
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
        let _database_work = enter_database_work(&state)?;
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
    validate_source_folder_not_excluded(&canonical_path, &current.excluded_folders)?;
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
fn library_list_excluded_folders(
    state: State<'_, AppState>,
) -> Result<Vec<ExcludedFolder>, String> {
    #[cfg(any(target_os = "windows", target_os = "android"))]
    {
        let settings = state
            .settings
            .snapshot()
            .map_err(|error| error.to_string())?;
        Ok(settings
            .excluded_folders
            .iter()
            .map(excluded_folder_summary)
            .collect())
    }

    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = state;
        Ok(Vec::new())
    }
}

#[tauri::command]
async fn library_pick_excluded_folder(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<ExcludedFolder>, String> {
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_dialog::DialogExt;

        let selected = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("選擇排除資料夾")
            .blocking_pick_folder();
        let Some(selected) = selected else {
            return Ok(None);
        };
        let path = selected
            .into_path()
            .map_err(|error| format!("無法取得選取的資料夾路徑：{error}"))?;
        let _database_work = enter_database_work(&state)?;
        add_excluded_folder_path_to_settings(&state.settings, &path).map(Some)
    }

    // Android: the system folder picker only points at a location; the excluded folder is stored
    // as a real path and applied by the real-path scanner (ADR 0014).
    #[cfg(target_os = "android")]
    {
        let _ = window;
        let picker_app = app.clone();
        let picked = tauri::async_runtime::spawn_blocking(move || {
            android_folder_browser::pick_directory(&picker_app)
        })
        .await
        .map_err(|error| format!("資料夾選擇工作失敗：{error}"))?
        .map_err(|error| error.to_string())?;
        let Some(path) = picked else {
            return Ok(None);
        };
        let _database_work = enter_database_work(&state)?;
        add_excluded_folder_path_to_settings(&state.settings, std::path::Path::new(&path)).map(Some)
    }

    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (app, window, state);
        Err("排除資料夾只適用於 Windows 與 Android。".to_owned())
    }
}

/// Comparison key for folder identity: case-folded Windows locator key, exact path on Android.
#[cfg(target_os = "windows")]
fn folder_identity_key(path: &std::path::Path) -> String {
    windows_locator_key(path)
}

#[cfg(target_os = "android")]
fn folder_identity_key(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn add_excluded_folder_path_to_settings(
    settings: &SettingsStore,
    requested_path: &std::path::Path,
) -> Result<ExcludedFolder, String> {
    let canonical_path = std::fs::canonicalize(requested_path)
        .map_err(|error| format!("無法開啟排除資料夾：{error}"))?;
    if !canonical_path.is_dir() {
        return Err("指定路徑不是資料夾。".to_owned());
    }
    let current = settings.snapshot().map_err(|error| error.to_string())?;
    validate_excluded_folder_path(&canonical_path, &current.sources)?;
    let canonical_key = folder_identity_key(&canonical_path);
    if let Some(existing) = current.excluded_folders.iter().find(|entry| {
        entry
            .path
            .to_path_buf()
            .is_ok_and(|path| folder_identity_key(&path) == canonical_key)
    }) {
        return Ok(excluded_folder_summary(existing));
    }
    let display_name = canonical_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "排除資料夾".to_owned());
    let entry = ExcludedFolderEntry {
        id: SourceId::new(),
        display_name,
        path: settings::StoredPath::from_path(&canonical_path)
            .map_err(|error| error.to_string())?,
    };
    settings
        .upsert_excluded_folder(entry.clone())
        .map_err(|error| error.to_string())?;
    Ok(excluded_folder_summary(&entry))
}

#[tauri::command]
fn library_remove_excluded_folder(
    state: State<'_, AppState>,
    excluded_id: String,
) -> Result<Vec<ExcludedFolder>, String> {
    #[cfg(any(target_os = "windows", target_os = "android"))]
    {
        let _database_work = enter_database_work(&state)?;
        let id = SourceId::parse(&excluded_id)
            .map_err(|error| format!("排除資料夾 ID 無效：{error}"))?;
        state
            .settings
            .remove_excluded_folder(id)
            .map_err(|error| error.to_string())?;
        let settings = state
            .settings
            .snapshot()
            .map_err(|error| error.to_string())?;
        Ok(settings
            .excluded_folders
            .iter()
            .map(excluded_folder_summary)
            .collect())
    }

    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (state, excluded_id);
        Err("排除資料夾只適用於 Windows 與 Android。".to_owned())
    }
}

#[tauri::command]
fn library_set_source_enabled(
    state: State<'_, AppState>,
    source_id: String,
    enabled: bool,
) -> Result<Vec<LibrarySource>, String> {
    let _database_work = enter_database_work(&state)?;
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
        .map(|source| {
            source_entry_summary_with(state.database.as_ref(), source, &settings.remote_accounts)
        })
        .collect()
}

fn source_entry_summary(
    database: Option<&Database>,
    source: &SourceEntry,
) -> Result<LibrarySource, String> {
    source_entry_summary_with(database, source, &[])
}

/// The place a remote source points at, for the list of sources: the account's name and the folder under it.
fn remote_source_location(
    path: &settings::StoredPath,
    account: Option<&String>,
    accounts: &[player_remote::account::RemoteAccount],
) -> String {
    let folder = match path.to_media_locator() {
        Ok(MediaLocator::Remote { path, .. }) => path,
        _ => String::new(),
    };
    let name = account
        .and_then(|id| accounts.iter().find(|candidate| &candidate.id == id))
        .map(|candidate| candidate.name.as_str());
    match (name, folder.is_empty()) {
        (Some(name), true) => name.to_owned(),
        (Some(name), false) => format!("{name} / {folder}"),
        (None, _) => folder,
    }
}

fn source_entry_summary_with(
    database: Option<&Database>,
    source: &SourceEntry,
    accounts: &[player_remote::account::RemoteAccount],
) -> Result<LibrarySource, String> {
    let remote_account_id = match &source.kind {
        SourceEntryKind::Folder { remote_account, .. }
        | SourceEntryKind::PlaylistFile { remote_account, .. } => remote_account.clone(),
    };
    let (kind, location) = match &source.kind {
        SourceEntryKind::Folder {
            media_kind,
            path,
            remote_account,
        } if *media_kind == MediaSourceKind::Remote => (
            source_kind_name(*media_kind).to_owned(),
            remote_source_location(path, remote_account.as_ref(), accounts),
        ),
        SourceEntryKind::Folder {
            media_kind, path, ..
        } => (
            source_kind_name(*media_kind).to_owned(),
            path.display_lossy(),
        ),
        SourceEntryKind::PlaylistFile {
            path,
            remote_account: Some(_),
            ..
        } => (
            "playlist_file".to_owned(),
            remote_source_location(path, remote_account_id.as_ref(), accounts),
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
        remote_account_id,
        id: source.id.to_string(),
        kind,
        display_name: source.display_name.clone(),
        location,
        enabled: source.enabled,
        sync_state: sync.as_ref().map(|state| state.state.clone()),
        last_attempt_utc_ms: sync.as_ref().and_then(|state| state.last_attempt_utc_ms),
        last_success_utc_ms: sync.as_ref().and_then(|state| state.last_success_utc_ms),
        error_count: sync.as_ref().map_or(0, |state| state.error_count),
        last_error: sync.and_then(|state| state.last_error),
    })
}

#[tauri::command]
fn library_remove_source(
    state: State<'_, AppState>,
    app: AppHandle,
    source_id: String,
) -> Result<Vec<LibrarySource>, String> {
    let _database_work = enter_database_work(&state)?;
    let id = SourceId::parse(&source_id).map_err(|error| format!("來源 ID 無效：{error}"))?;
    let (settings, removed) = state
        .settings
        .remove_source(id)
        .map_err(|error| error.to_string())?;
    if let Some(database) = state.database.as_ref() {
        if let SourceEntryKind::PlaylistFile { playlist_id, .. } = removed.kind {
            database
                .remove_playlist_file_source(id)
                .map_err(|error| error.to_string())?;
            // Drop the sidebar playlist projection for this source. Shared Track rows stay.
            database
                .delete_playlist(playlist_id)
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
    let _ = app;
    settings
        .sources
        .iter()
        .map(|source| {
            source_entry_summary_with(state.database.as_ref(), source, &settings.remote_accounts)
        })
        .collect()
}

/// Android: choose a folder and register it as a real-path library source (ADR 0014). The system
/// picker is used only to point at a location; no SAF grant is kept and scanning reads the folder
/// directly under All files access.
#[tauri::command]
async fn android_pick_folder_source(app: AppHandle) -> Result<Option<LibrarySource>, String> {
    #[cfg(target_os = "android")]
    {
        let picker_app = app.clone();
        let picked = tauri::async_runtime::spawn_blocking(move || {
            android_folder_browser::pick_directory(&picker_app)
        })
        .await
        .map_err(|error| format!("資料夾選擇工作失敗：{error}"))?
        .map_err(|error| error.to_string())?;
        let Some(path) = picked else {
            return Ok(None);
        };
        let path =
            std::fs::canonicalize(&path).map_err(|error| format!("無法開啟指定資料夾：{error}"))?;
        if !path.is_dir() {
            return Err("指定路徑不是資料夾。".to_owned());
        }
        let state = app.state::<AppState>();
        let current = state
            .settings
            .snapshot()
            .map_err(|error| error.to_string())?;
        validate_source_folder_not_excluded(&path, &current.excluded_folders)?;
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let _database_work = enter_database_work(&state)?;
        let mut root = add_or_get_root(
            database,
            MediaSourceKind::AndroidFilesystem,
            android_folder_display_name(&path),
            MediaLocator::FileSystem(path),
        )?;
        root.enabled = true;
        register_folder_source(database, &state.settings, &root)?;
        source_summary(database, &root).map(Some)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err("Android 資料夾選擇器只適用於 Android。".to_owned())
    }
}

#[cfg(target_os = "android")]
fn android_folder_display_name(path: &std::path::Path) -> String {
    if path == std::path::Path::new("/storage/emulated/0") {
        return "內部儲存空間".to_owned();
    }
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "音樂資料夾".to_owned())
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
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

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
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
        remote_account_id: None,
        id: root.id.to_string(),
        kind: source_kind_name(root.kind).to_owned(),
        display_name: root.display_name.clone(),
        location: match &root.locator {
            MediaLocator::FileSystem(path) => path.to_string_lossy().into_owned(),
            MediaLocator::ContentUri(uri) => uri.clone(),
            MediaLocator::Remote { path, .. } => path.clone(),
        },
        enabled: root.enabled,
        sync_state: sync.as_ref().map(|state| state.state.clone()),
        last_attempt_utc_ms: sync.as_ref().and_then(|state| state.last_attempt_utc_ms),
        last_success_utc_ms: sync.as_ref().and_then(|state| state.last_success_utc_ms),
        error_count: sync.as_ref().map_or(0, |state| state.error_count),
        last_error: sync.and_then(|state| state.last_error),
    })
}

fn source_kind_name(kind: MediaSourceKind) -> &'static str {
    match kind {
        MediaSourceKind::WindowsSystemIndex => "windowsSystemIndex",
        MediaSourceKind::WindowsFilesystem => "windowsFilesystem",
        MediaSourceKind::PlaylistFile => "playlistFile",
        MediaSourceKind::AndroidMediaStore => "androidMediaStore",
        MediaSourceKind::AndroidSaf => "androidSaf",
        MediaSourceKind::AndroidFilesystem => "androidFilesystem",
        MediaSourceKind::Remote => "remote",
        MediaSourceKind::Other => "other",
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
    let database_work = enter_database_work(&state)?;
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
    let cancellation = SyncCancellation::default();
    *state
        .active_sync
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cancellation.clone());
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _database_work = database_work;
        let mut database = Database::open(database_path).map_err(|error| error.to_string())?;
        let _ = remote_sources::refresh_source_labels(&database, &source_settings);
        // Legacy MediaStore/SAF sources become real-path sources before syncing (ADR 0014).
        #[cfg(target_os = "android")]
        let source_settings = {
            let report = android_path_migration::migrate_legacy_sources(&sync_app, &database)?;
            if report.backup_directory.is_some() {
                sync_app
                    .state::<AppState>()
                    .settings
                    .snapshot()
                    .map_err(|error| error.to_string())?
            } else {
                source_settings
            }
        };
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
        #[cfg(any(target_os = "windows", target_os = "android"))]
        let excluded_folders = excluded_folder_paths(&source_settings);
        #[cfg(not(any(target_os = "windows", target_os = "android")))]
        let excluded_folders = Vec::new();
        let remote_resolver = {
            let state = sync_app.state::<AppState>();
            let data_dir = user_data_directory(&state)?;
            state
                .remote_connections
                .resolver(data_dir, &source_settings)
        };
        sync_configured_sources(
            &sync_app,
            &mut database,
            &source_settings.sources,
            &excluded_folders,
            source_registry_authoritative,
            &cancellation,
            remote_resolver,
            &mut on_progress,
        )
    })
    .await
    .map_err(|error| format!("曲庫同步工作失敗：{error}"))
    .and_then(|result| result);
    *event_app
        .state::<AppState>()
        .active_sync
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    let finished = library_sync_finished_event(run_id, &result);
    let _ = event_app.emit(LIBRARY_SYNC_FINISHED_EVENT, finished);
    result
}

/// Stops the library sync in progress, if any: its reading stops and what it has read so far is saved (the next sync
/// continues from there). Closing the program calls this, then waits for the saving to finish.
#[cfg(target_os = "windows")]
fn cancel_library_sync(state: &AppState) {
    if let Some(cancellation) = state
        .active_sync
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
    {
        cancellation.cancel();
    }
}

fn sync_configured_sources<F>(
    app: &AppHandle,
    database: &mut Database,
    configured_sources: &[SourceEntry],
    excluded_folders: &[std::path::PathBuf],
    source_registry_authoritative: bool,
    cancellation: &SyncCancellation,
    remote_resolver: Arc<dyn player_remote::index::RemoteResolver>,
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
        sync_windows_configured_sources_cancellable(
            database,
            configured_sources,
            excluded_folders,
            source_registry_authoritative,
            cancellation,
            remote_resolver,
            on_progress,
        )
    }

    #[cfg(target_os = "android")]
    {
        let synced_at_utc_ms = now_utc_epoch_ms()?;
        let roots = configured_sources
            .iter()
            .filter_map(|source| source.library_root().ok().flatten())
            .filter(|root| root.enabled)
            .collect::<Vec<_>>();
        let playlist_sources = configured_sources
            .iter()
            .filter(|source| source.enabled)
            .filter(|source| matches!(source.kind, SourceEntryKind::PlaylistFile { .. }))
            .cloned()
            .collect::<Vec<_>>();
        let source_count = roots.len() + playlist_sources.len();
        let mut sources = Vec::new();
        let mut folder_roots = Vec::new();
        let mut remote_roots = Vec::new();
        for root in roots {
            if root.kind == MediaSourceKind::AndroidFilesystem {
                folder_roots.push(root);
            } else if root.kind == MediaSourceKind::Remote {
                remote_roots.push(root);
            } else {
                // Legacy MediaStore/SAF: never scanned again; its mappings stay untouched until
                // the path migration (needs All files access) converts it (ADR 0014).
                sources.push(unavailable_source_summary(
                    &root,
                    "舊版 MediaStore/SAF 來源：請開啟「所有檔案存取權」，MoeMusicPlayer 會自動改為資料夾路徑來源。",
                ));
            }
        }
        app.media_index().with_adapter_excluding(
            excluded_folders,
            |index| -> Result<(), String> {
                for (source_index, root) in folder_roots.iter().enumerate() {
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
            },
        )?;
        let mut remote_index =
            player_remote::index::RemoteMediaIndex::new(Arc::clone(&remote_resolver));
        for (remote_index_number, root) in remote_roots.iter().enumerate() {
            let report = SyncEngine::sync_cancellable_with_progress(
                root,
                &mut remote_index,
                database,
                synced_at_utc_ms,
                cancellation,
                |progress| {
                    on_progress(
                        root,
                        folder_roots.len() + remote_index_number,
                        source_count,
                        progress,
                    );
                },
            )
            .map_err(|error| error.to_string())?;
            sources.push(sync_summary(root, report));
        }
        let mut reader = AndroidPlaylistReader { app };
        for (playlist_index, source) in playlist_sources.into_iter().enumerate() {
            let root = source
                .database_projection()
                .map_err(|error| error.to_string())?;
            if matches!(
                &source.kind,
                SourceEntryKind::PlaylistFile {
                    remote_account: Some(_),
                    ..
                }
            ) {
                match remote_playlist_sync::sync_remote_playlist_source(
                    database,
                    &source,
                    configured_sources,
                    Arc::clone(&remote_resolver),
                    synced_at_utc_ms,
                    &mut |progress| {
                        on_progress(
                            &root,
                            folder_roots.len() + remote_roots.len() + playlist_index,
                            source_count,
                            progress,
                        )
                    },
                ) {
                    Ok(Some(report)) => sources.push(sync_summary(&root, report)),
                    Ok(None) => sources.push(unavailable_source_summary(
                        &root,
                        "無法讀取伺服器上的播放清單檔案；請確認連線與檔案存在。",
                    )),
                    Err(error) => sources.push(unavailable_source_summary(
                        &root,
                        format!("播放清單同步失敗：{error}"),
                    )),
                }
                continue;
            }
            if matches!(source.path(), settings::StoredPath::Uri(_)) {
                sources.push(unavailable_source_summary(
                    &root,
                    "舊版 SAF 播放清單：請開啟「所有檔案存取權」，MoeMusicPlayer 會自動改為檔案路徑。",
                ));
                continue;
            }
            match playlist_source_sync::sync_playlist_file_source_with(
                database,
                &source,
                synced_at_utc_ms,
                &mut reader,
            ) {
                Ok(Some(report)) => sources.push(sync_summary(&root, report)),
                Ok(None) => sources.push(unavailable_source_summary(
                    &root,
                    "無法讀取播放清單檔案；請確認檔案存在且已開啟「所有檔案存取權」。",
                )),
                Err(error) => sources.push(unavailable_source_summary(
                    &root,
                    format!("播放清單同步失敗：{error}"),
                )),
            }
        }
        Ok(LibrarySyncResult { sources })
    }

    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (app, database, configured_sources, excluded_folders);
        return Err("目前平台尚未提供媒體來源同步。".to_owned());
    }
}

/// Android playlist media read through real paths under All files access (ADR 0014).
#[cfg(target_os = "android")]
struct AndroidPlaylistReader<'a> {
    app: &'a AppHandle,
}

#[cfg(target_os = "android")]
impl playlist_source_sync::PlaylistMediaReader for AndroidPlaylistReader<'_> {
    fn scan_paths(
        &mut self,
        source_id: SourceId,
        paths: &[std::path::PathBuf],
    ) -> player_core::SourceScan {
        tauri_plugin_media_index::fs_scan::scan_paths(source_id, paths)
    }

    fn read_metadata(
        &mut self,
        track: &player_core::MediaTrackRecord,
    ) -> Result<player_core::TrackMetadata, player_core::TrackMetadataError> {
        self.app.media_index().read_metadata(track)
    }
}

#[cfg(target_os = "windows")]
fn excluded_folder_paths(settings: &AppSettings) -> Vec<std::path::PathBuf> {
    settings
        .excluded_folders
        .iter()
        .filter_map(|entry| entry.path.to_path_buf().ok())
        .collect()
}

/// Android scanner compares canonical real paths component-wise, so stored paths are
/// canonicalized here (a missing folder simply stays as stored).
#[cfg(target_os = "android")]
fn excluded_folder_paths(settings: &AppSettings) -> Vec<std::path::PathBuf> {
    settings
        .excluded_folders
        .iter()
        .filter_map(|entry| entry.path.to_path_buf().ok())
        .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
        .collect()
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn windows_path_is_same_or_under(path: &std::path::Path, ancestor: &std::path::Path) -> bool {
    path == ancestor || path.starts_with(ancestor)
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn excluded_folder_summary(entry: &ExcludedFolderEntry) -> ExcludedFolder {
    ExcludedFolder {
        id: entry.id.to_string(),
        display_name: entry.display_name.clone(),
        location: entry.path.display_lossy(),
    }
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn validate_excluded_folder_path(
    excluded: &std::path::Path,
    sources: &[SourceEntry],
) -> Result<(), String> {
    let excluded_canonical =
        std::fs::canonicalize(excluded).map_err(|error| format!("無法開啟排除資料夾：{error}"))?;
    if !excluded_canonical.is_dir() {
        return Err("指定路徑不是資料夾。".to_owned());
    }
    let excluded_key = folder_identity_key(&excluded_canonical);
    for source in sources {
        let SourceEntryKind::Folder { path, .. } = &source.kind else {
            continue;
        };
        let Ok(source_path) = path.to_path_buf() else {
            continue;
        };
        let source_canonical = std::fs::canonicalize(&source_path).unwrap_or(source_path);
        let source_key = folder_identity_key(&source_canonical);
        if source_key == excluded_key {
            return Err("不能排除已加入的來源根目錄。".to_owned());
        }
        if windows_path_is_same_or_under(&source_canonical, &excluded_canonical) {
            return Err("排除資料夾不能包含已加入的來源根目錄。".to_owned());
        }
    }
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn validate_source_folder_not_excluded(
    source: &std::path::Path,
    excluded_folders: &[ExcludedFolderEntry],
) -> Result<(), String> {
    let source_canonical =
        std::fs::canonicalize(source).map_err(|error| format!("無法開啟指定資料夾：{error}"))?;
    for excluded in excluded_folders {
        let Ok(excluded_path) = excluded.path.to_path_buf() else {
            continue;
        };
        let excluded_canonical = std::fs::canonicalize(&excluded_path).unwrap_or(excluded_path);
        if windows_path_is_same_or_under(&source_canonical, &excluded_canonical) {
            return Err("此資料夾位於已排除路徑內，請先移除排除。".to_owned());
        }
    }
    Ok(())
}

/// Resolve the sidebar playlist projection for a playlist-file root that is no longer configured.
/// Older builds cleared `playlist_file_sync_state` without deleting the playlist; recover those by
/// exact display-name match when exactly one non-retained playlist remains.
#[cfg(target_os = "windows")]
fn resolve_orphan_playlist_file_id(
    database: &Database,
    source_id: SourceId,
    display_name: &str,
    retained_playlist_ids: &std::collections::HashSet<PlaylistId>,
) -> Result<Option<PlaylistId>, String> {
    if let Some(state) = database
        .playlist_file_sync_state(source_id)
        .map_err(|error| error.to_string())?
    {
        return Ok(Some(state.playlist_id));
    }
    let matches = database
        .list_playlists()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|playlist| {
            playlist.name == display_name && !retained_playlist_ids.contains(&playlist.id)
        })
        .map(|playlist| playlist.id)
        .collect::<Vec<_>>();
    Ok(if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    })
}

#[cfg(all(target_os = "windows", test))]
fn sync_windows_configured_sources<F>(
    database: &mut Database,
    configured_sources: &[SourceEntry],
    excluded_folders: &[std::path::PathBuf],
    source_registry_authoritative: bool,
    on_progress: &mut F,
) -> Result<LibrarySyncResult, String>
where
    F: FnMut(&LibraryRoot, usize, usize, SyncProgress),
{
    sync_windows_configured_sources_cancellable(
        database,
        configured_sources,
        excluded_folders,
        source_registry_authoritative,
        &SyncCancellation::default(),
        remote_connections::none(),
        on_progress,
    )
}

#[cfg(target_os = "windows")]
fn sync_windows_configured_sources_cancellable<F>(
    database: &mut Database,
    configured_sources: &[SourceEntry],
    excluded_folders: &[std::path::PathBuf],
    source_registry_authoritative: bool,
    cancellation: &SyncCancellation,
    remote_resolver: Arc<dyn player_remote::index::RemoteResolver>,
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
    let retained_playlist_ids = configured_sources
        .iter()
        .filter_map(|source| match source.kind {
            SourceEntryKind::PlaylistFile { playlist_id, .. } => Some(playlist_id),
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>();
    for mut root in database
        .library_roots()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|root| {
            root.kind == MediaSourceKind::PlaylistFile && !configured_ids.contains(&root.id)
        })
    {
        // Prefer sync_state; if an older build already cleared it, fall back to the unique
        // sidebar playlist that still matches this orphan root's display name.
        let orphan_playlist_id = resolve_orphan_playlist_file_id(
            database,
            root.id,
            &root.display_name,
            &retained_playlist_ids,
        )?;
        database
            .remove_playlist_file_source(root.id)
            .map_err(|error| error.to_string())?;
        if let Some(playlist_id) = orphan_playlist_id {
            database
                .delete_playlist(playlist_id)
                .map_err(|error| error.to_string())?;
        }
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
                MediaSourceKind::WindowsFilesystem
                    | MediaSourceKind::WindowsSystemIndex
                    | MediaSourceKind::Remote
            ),
            SourceEntryKind::PlaylistFile { .. } => true,
        })
        .cloned()
        .collect();
    let source_count = sources.len();
    let mut results = Vec::new();
    let mut index = WindowsMediaIndex::with_excluded_folders(excluded_folders.iter().cloned());
    let mut remote_index =
        player_remote::index::RemoteMediaIndex::new(Arc::clone(&remote_resolver));
    for (source_index, source) in sources.iter().enumerate() {
        // Closing the program stops the sync: the source being read saves what it has, the others are left for next time.
        if cancellation.is_cancelled() {
            break;
        }
        match &source.kind {
            SourceEntryKind::Folder { .. } => {
                let root = source
                    .library_root()
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| "folder source has no library root".to_owned())?;
                database
                    .save_library_root(&root)
                    .map_err(|error| error.to_string())?;
                let report = if root.kind == MediaSourceKind::Remote {
                    SyncEngine::sync_cancellable_with_progress(
                        &root,
                        &mut remote_index,
                        database,
                        synced_at_utc_ms,
                        cancellation,
                        |progress| on_progress(&root, source_index, source_count, progress),
                    )
                } else {
                    SyncEngine::sync_cancellable_with_progress(
                        &root,
                        &mut index,
                        database,
                        synced_at_utc_ms,
                        cancellation,
                        |progress| on_progress(&root, source_index, source_count, progress),
                    )
                }
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
                let report = if matches!(
                    &source.kind,
                    SourceEntryKind::PlaylistFile {
                        remote_account: Some(_),
                        ..
                    }
                ) {
                    remote_playlist_sync::sync_remote_playlist_source(
                        database,
                        source,
                        configured_sources,
                        Arc::clone(&remote_resolver),
                        synced_at_utc_ms,
                        &mut |progress| on_progress(&root, source_index, source_count, progress),
                    )?
                } else {
                    playlist_source_sync::sync_playlist_file_source(
                        database,
                        source,
                        excluded_folders,
                        synced_at_utc_ms,
                    )?
                };
                if let Some(report) = report {
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
    let error_count = report
        .metadata_errors
        .len()
        .saturating_add(report.source_errors.len());
    let errors = source_sync_error_details(&report.source_errors, &report.metadata_errors);
    SourceSyncResult {
        source_id: root.id.to_string(),
        display_name: root.display_name.clone(),
        state,
        observed: report.observed,
        metadata_reads: report.metadata_reads,
        unchanged: report.unchanged,
        added_or_updated: report.applied.inserted_or_updated,
        removed_mappings: report.applied.removed_source_mappings,
        error_count: error_count as u64,
        errors_truncated: error_count > errors.len(),
        errors,
    }
}

fn source_sync_error_details(
    source_errors: &[MediaSourceError],
    metadata_errors: &[(String, TrackMetadataError)],
) -> Vec<SourceSyncError> {
    let mut details = Vec::with_capacity(
        source_errors
            .len()
            .saturating_add(metadata_errors.len())
            .min(SOURCE_SYNC_ERROR_DETAILS_LIMIT),
    );
    for error in source_errors {
        if details.len() == SOURCE_SYNC_ERROR_DETAILS_LIMIT {
            return details;
        }
        details.push(SourceSyncError {
            item: error
                .source_item_id
                .as_deref()
                .map(display_source_sync_item),
            message: error.message.clone(),
        });
    }
    for (item, error) in metadata_errors {
        if details.len() == SOURCE_SYNC_ERROR_DETAILS_LIMIT {
            return details;
        }
        details.push(SourceSyncError {
            item: Some(display_source_sync_item(item)),
            message: error.message.clone(),
        });
    }
    details
}

fn display_source_sync_item(item: &str) -> String {
    #[cfg(target_os = "windows")]
    if let Some(path) = decode_windows_locator_key(item) {
        return path;
    }

    item.to_owned()
}

#[cfg(target_os = "windows")]
fn decode_windows_locator_key(key: &str) -> Option<String> {
    const PREFIX: &str = "windows-u16-v1:";
    let encoded = key.strip_prefix(PREFIX)?;
    if encoded.is_empty() || !encoded.len().is_multiple_of(4) {
        return None;
    }

    let mut code_units = Vec::with_capacity(encoded.len() / 4);
    for chunk in encoded.as_bytes().chunks_exact(4) {
        let hex = std::str::from_utf8(chunk).ok()?;
        code_units.push(u16::from_str_radix(hex, 16).ok()?);
    }
    let path = String::from_utf16(&code_units).ok()?;
    if path.contains('\0') {
        return None;
    }

    // Only render tokens that round-trip through the canonical Windows identity encoder.
    (windows_locator_key(std::path::Path::new(&path)) == key).then_some(path)
}

#[cfg(target_os = "android")]
fn unavailable_source_summary(root: &LibraryRoot, reason: impl Into<String>) -> SourceSyncResult {
    SourceSyncResult {
        source_id: root.id.to_string(),
        display_name: root.display_name.clone(),
        state: "unavailable".to_owned(),
        observed: 0,
        metadata_reads: 0,
        unchanged: 0,
        added_or_updated: 0,
        removed_mappings: 0,
        error_count: 1,
        errors: vec![SourceSyncError {
            item: None,
            message: reason.into(),
        }],
        errors_truncated: false,
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
    let _database_work = enter_database_work(&state)?;
    #[cfg(target_os = "windows")]
    {
        let playback = windows_playback_service(&state)?;
        if let Some(database) = state.database.as_ref() {
            if let Some(change) = advance_ended_playback(database, playback)? {
                let PreparedTrackChange {
                    ticket,
                    track,
                    replacement_queue,
                    target_cursor,
                } = change;
                let result = await_playback_ack(ticket).await;
                if playback.player.snapshot().state != AudioPlaybackState::Ended {
                    *playback
                        .queue_advance_pending
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
                }
                result?;
                commit_track_change(database, playback, track, replacement_queue, target_cursor)?;
            }
            checkpoint_playback_position(database, playback, false)?;
        }
        if let Some(system_media) = state.system_media.as_ref() {
            system_media.pump(playback, state.database.as_ref());
        }
        Ok(playback.snapshot())
    }
    #[cfg(target_os = "android")]
    {
        Ok(android_playback_service(&state)?.snapshot())
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = state;
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
fn playback_get_queue_page(
    state: State<'_, AppState>,
    offset: u64,
    limit: u32,
) -> Result<PlaybackQueuePage, String> {
    let _database_work = enter_database_work(&state)?;
    #[cfg(target_os = "windows")]
    {
        let service = windows_playback_service(&state)?;
        let (revision, queue) = snapshot_playback_queue(service);
        let temporary_database = service
            .temporary_catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut page =
            build_playback_queue_page(state.database.as_ref(), revision, queue, offset, limit)?;
        if let Some(temporary_database) = temporary_database.as_ref() {
            let temporary_ids = page
                .items
                .iter()
                .filter(|item| item.track.is_none())
                .map(|item| item.track_id)
                .collect::<Vec<_>>();
            let temporary_tracks = temporary_database
                .get_track_summaries(&temporary_ids)
                .map_err(|error| error.to_string())?;
            let temporary_by_id = temporary_ids
                .into_iter()
                .zip(temporary_tracks)
                .filter_map(|(id, track)| track.map(|track| (id, track)))
                .collect::<HashMap<_, _>>();
            for item in &mut page.items {
                if let Some(track) = temporary_by_id.get(&item.track_id) {
                    item.track = Some(track.clone());
                }
            }
        }
        let transient = service
            .external_tracks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for item in &mut page.items {
            if item.track.is_none() {
                item.track = transient
                    .get(&item.track_id)
                    .map(|track| track.summary.clone());
            }
        }
        Ok(page)
    }
    #[cfg(target_os = "android")]
    {
        let service = android_playback_service(&state)?;
        let (revision, queue) = service
            .queue_snapshot()?
            .map_or((0, None), |(revision, queue)| (revision, Some(queue)));
        build_playback_queue_page(state.database.as_ref(), revision, queue, offset, limit)
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (state, offset, limit);
        Err("此平台尚未提供本機播放佇列。".to_owned())
    }
}

#[tauri::command]
fn playback_get_queue_quick_index_groupings(
    state: State<'_, AppState>,
    field: TrackSortField,
) -> Result<player_db::QuickIndexSnapshot, String> {
    let _database_work = enter_database_work(&state)?;
    let Some((kind, _)) = quick_index_key(field, None) else {
        return Ok(player_db::QuickIndexSnapshot::default());
    };
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    #[cfg(target_os = "windows")]
    let (_revision, queue) = {
        let service = windows_playback_service(&state)?;
        snapshot_playback_queue(service)
    };
    #[cfg(target_os = "android")]
    let (_revision, queue) = {
        let service = android_playback_service(&state)?;
        match service.queue_snapshot()? {
            Some((revision, snapshot)) => (revision, {
                // Validate like playback does, but read the snapshot's public fields (same shape as Windows).
                PlaybackQueue::restore(snapshot.clone()).map_err(|error| error.to_owned())?;
                Some(snapshot)
            }),
            None => (0, None),
        }
    };
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    let (_revision, queue) = (0_u64, None);
    let Some(queue) = queue else {
        return Ok(player_db::QuickIndexSnapshot::default());
    };
    let track_ids = queue
        .entries
        .iter()
        .map(|entry| entry.track_id)
        .collect::<Vec<_>>();
    let mut summaries = HashMap::<TrackId, TrackSummary>::new();
    for (id, summary) in track_ids.iter().copied().zip(
        database
            .get_track_summaries(&track_ids)
            .map_err(|error| error.to_string())?,
    ) {
        if let Some(summary) = summary {
            summaries.insert(id, summary);
        }
    }
    #[cfg(target_os = "windows")]
    {
        let service = windows_playback_service(&state)?;
        let temporary_database = service
            .temporary_catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(temporary_database) = temporary_database {
            let unresolved = track_ids
                .iter()
                .filter(|id| !summaries.contains_key(id))
                .copied()
                .collect::<Vec<_>>();
            for (id, summary) in unresolved.iter().copied().zip(
                temporary_database
                    .get_track_summaries(&unresolved)
                    .map_err(|error| error.to_string())?,
            ) {
                if let Some(summary) = summary {
                    summaries.insert(id, summary);
                }
            }
        }
        let transient_tracks = service
            .external_tracks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (id, external) in transient_tracks.iter() {
            summaries
                .entry(*id)
                .or_insert_with(|| external.summary.clone());
        }
    }
    let keys = queue
        .play_order
        .iter()
        .filter_map(|entry_index| queue.entries.get(*entry_index))
        .map(|entry| {
            quick_index_key(field, summaries.get(&entry.track_id))
                .map(|(_, key)| key)
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    database
        .quick_index_for_ordered_keys(kind, &keys)
        .map_err(|error| error.to_string())
}

/// Read the active track's original encoded cover bytes on a blocking worker.
/// An empty binary response means that no supported cover is available.
#[tauri::command]
async fn library_get_track_artwork(
    app: AppHandle,
    state: State<'_, AppState>,
    track_id: String,
) -> Result<Response, String> {
    #[cfg(target_os = "windows")]
    {
        let _ = &app;
        let id = TrackId::parse(&track_id).map_err(|error| format!("曲目 ID 無效：{error}"))?;
        let external_path = state.playback.as_ref().and_then(|service| {
            service
                .external_tracks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&id)
                .map(|track| track.path.clone())
        });
        let locators = if let Some(path) = external_path {
            vec![MediaLocator::FileSystem(path)]
        } else {
            let database = state.database.as_ref().ok_or_else(|| {
                state
                    .database_error
                    .clone()
                    .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
            })?;
            database
                .track_locators(id)
                .map_err(|error| error.to_string())?
        };
        let locators_for_check = locators.clone();
        let artwork = tauri::async_runtime::spawn_blocking(move || find_artwork(&locators))
            .await
            .map_err(|error| format!("封面讀取工作失敗：{error}"))?;
        match artwork {
            ArtworkLookup::Found(image) => Ok(Response::new(image.into_bytes())),
            ArtworkLookup::Missing => {
                if remote_cover_lookup_may_have_failed(&locators_for_check) {
                    return Err("REMOTE_UNAVAILABLE".to_owned());
                }
                Ok(Response::new(Vec::new()))
            }
            ArtworkLookup::Oversized => Err("ARTWORK_TOO_LARGE".to_owned()),
        }
    }
    #[cfg(target_os = "android")]
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
        let tree_uris: Vec<String> = Vec::new();
        let locators_for_check = locators.clone();
        let artwork = tauri::async_runtime::spawn_blocking(move || {
            find_android_artwork(&app, &locators, &tree_uris)
        })
        .await
        .map_err(|error| format!("封面讀取工作失敗：{error}"))?;
        eprintln!(
            "[art] track_artwork {} -> {}",
            &track_id[..track_id.len().min(8)],
            match &artwork {
                Ok(image) => format!("{} bytes", image.bytes().len()),
                Err(error) => format!("{error:?}"),
            }
        );
        match artwork {
            Ok(image) => Ok(Response::new(image.into_bytes())),
            Err(android_artwork::ArtworkError::Missing) => {
                if remote_cover_lookup_may_have_failed(&locators_for_check) {
                    return Err("REMOTE_UNAVAILABLE".to_owned());
                }
                Ok(Response::new(Vec::new()))
            }
            Err(android_artwork::ArtworkError::Oversized) => Err("ARTWORK_TOO_LARGE".to_owned()),
            Err(android_artwork::ArtworkError::Invalid) => Ok(Response::new(Vec::new())),
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (app, state, track_id);
        Err("此平台尚未提供本機封面讀取服務。".to_owned())
    }
}

#[cfg(target_os = "android")]
fn find_android_artwork(
    app: &AppHandle,
    locators: &[MediaLocator],
    tree_uris: &[String],
) -> Result<player_core::ArtworkImage, android_artwork::ArtworkError> {
    use std::io::Read;
    use tauri_plugin_media_index::MediaIndexExt as _;

    let media_index = app.media_index();
    // Embedded artwork: content URIs as-is, real paths as file:// URIs (ADR 0014).
    let content_uris = locators
        .iter()
        .filter_map(|locator| match locator {
            MediaLocator::ContentUri(uri) if uri.starts_with("content://") => Some(uri.clone()),
            MediaLocator::FileSystem(path) if path.is_file() => {
                player_core::file_uri_for_path(path)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut oversized = false;
    // A remote track's cover comes through the remote reader (ADR 0026).
    for locator in locators {
        let MediaLocator::Remote { source_id, path } = locator else {
            continue;
        };
        let Some(bytes) = remote_media::remote_cover_bytes(*source_id, path) else {
            continue;
        };
        match android_artwork::image_from_bytes(&bytes, None) {
            Ok(image) => return Ok(image),
            Err(android_artwork::ArtworkError::Oversized) => oversized = true,
            Err(_) => {}
        }
    }
    // WavPack keeps its cover in an APEv2 item that Android's MediaMetadataRetriever cannot read (ADR 0025).
    for locator in locators {
        let MediaLocator::FileSystem(path) = locator else {
            continue;
        };
        let Some(picture) = player_tags::ape_cover::ape_front_cover_of_path(path) else {
            continue;
        };
        if picture.data().len() > player_core::MAX_ARTWORK_BYTES {
            oversized = true;
            continue;
        }
        match android_artwork::image_from_bytes(
            picture.data(),
            picture.mime_type().map(|mime| mime.as_str()),
        ) {
            Ok(image) => return Ok(image),
            Err(android_artwork::ArtworkError::Oversized) => oversized = true,
            Err(_) => {}
        }
    }
    for uri in &content_uris {
        let uri = uri.as_str();
        let trees = if tree_uris.is_empty() {
            std::iter::once(None).collect::<Vec<_>>()
        } else {
            tree_uris
                .iter()
                .map(|tree| Some(tree.as_str()))
                .collect::<Vec<_>>()
        };
        for tree_uri in trees {
            let lease = match media_index.cache_artwork_bytes(
                uri,
                tree_uri,
                player_core::MAX_ARTWORK_BYTES as u64,
            ) {
                Ok(lease) => lease,
                Err(error) => {
                    let message = error.to_string();
                    oversized |= message.contains("ARTWORK_TOO_LARGE");
                    continue;
                }
            };
            let metadata = match std::fs::metadata(lease.path()) {
                Ok(metadata) if metadata.len() <= player_core::MAX_ARTWORK_BYTES as u64 => metadata,
                Ok(_) => {
                    oversized = true;
                    continue;
                }
                Err(_) => continue,
            };
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            if std::fs::File::open(lease.path())
                .and_then(|file| {
                    file.take(player_core::MAX_ARTWORK_BYTES as u64 + 1)
                        .read_to_end(&mut bytes)
                })
                .is_err()
            {
                continue;
            }
            match android_artwork::image_from_bytes(&bytes, lease.mime_type()) {
                Ok(image) => return Ok(image),
                Err(android_artwork::ArtworkError::Oversized) => oversized = true,
                Err(
                    android_artwork::ArtworkError::Missing | android_artwork::ArtworkError::Invalid,
                ) => {}
            }
        }
    }
    // Folder artwork next to a real-path track (cover.jpg, folder.jpg, cover.png, folder.png).
    for locator in locators {
        let MediaLocator::FileSystem(path) = locator else {
            continue;
        };
        let Some(cover) = tauri_plugin_media_index::fs_scan::folder_artwork(path) else {
            continue;
        };
        let mut bytes = Vec::new();
        if std::fs::File::open(&cover)
            .and_then(|file| {
                file.take(player_core::MAX_ARTWORK_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
            })
            .is_err()
        {
            continue;
        }
        if bytes.len() > player_core::MAX_ARTWORK_BYTES {
            oversized = true;
            continue;
        }
        let mime = match cover.extension().and_then(|extension| extension.to_str()) {
            Some("png") => "image/png",
            _ => "image/jpeg",
        };
        match android_artwork::image_from_bytes(&bytes, Some(mime)) {
            Ok(image) => return Ok(image),
            Err(android_artwork::ArtworkError::Oversized) => oversized = true,
            Err(_) => {}
        }
    }
    if oversized {
        Err(android_artwork::ArtworkError::Oversized)
    } else {
        Err(android_artwork::ArtworkError::Missing)
    }
}

#[tauri::command]
async fn playback_play(
    state: State<'_, AppState>,
    track_id: String,
    queue_source: Option<PlaybackQueueSource>,
) -> Result<PlaybackSnapshot, String> {
    let _database_work = enter_database_work(&state)?;
    if let Some(PlaybackQueueSource::Playlist { playlist_id, .. }) = queue_source.as_ref() {
        let playlist_id = PlaylistId::parse(playlist_id)
            .map_err(|_| "播放清單 ID 無效，請重新整理。".to_owned())?;
        let settings = state
            .settings
            .snapshot()
            .map_err(|error| error.to_string())?;
        require_enabled_playlist_source(&settings.sources, playlist_id)?;
    }
    #[cfg(target_os = "windows")]
    {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let service = windows_playback_service(&state)?;
        let change = play_track_from_database(database, service, &track_id, queue_source)?;
        let PreparedTrackChange {
            ticket,
            track,
            replacement_queue,
            target_cursor,
        } = change;
        await_playback_ack(ticket).await?;
        commit_track_change(database, service, track, replacement_queue, target_cursor)?;
        Ok(service.snapshot())
    }
    #[cfg(target_os = "android")]
    {
        android_playback_service(&state)?
            .execute(android_playback::PlaybackAction::select_track(
                track_id,
                queue_source,
            ))
            .await
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (state, track_id, queue_source);
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

/// Event sent to every window with the id of the track whose summary, cover and embedded lyrics were just reloaded.
const CURRENT_TRACK_REFRESHED_EVENT: &str = "current-track-refreshed";

/// Reload the current track from the library after its tags were saved (the caller syncs the library first): its
/// summary in the playback snapshot, the system media metadata and cover, and, through the event, each window's
/// cover and lyrics.
#[tauri::command]
async fn playback_refresh_current_track(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshot, String> {
    let _database_work = enter_database_work(&state)?;
    #[cfg(target_os = "windows")]
    let snapshot = {
        let service = windows_playback_service(&state)?;
        let current = service
            .current_track
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|track| track.id);
        if let (Some(track_id), Some(database)) = (current, state.database.as_ref()) {
            let external = service
                .external_tracks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains_key(&track_id);
            if !external {
                if let Some(summary) = database
                    .get_track_summary(track_id)
                    .map_err(|error| error.to_string())?
                {
                    *service
                        .current_track
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(summary);
                }
            }
        }
        if let Some(system_media) = state.system_media.as_ref() {
            system_media.refresh_current_track();
        }
        bump_queue_revision(service);
        service.snapshot()
    };
    #[cfg(target_os = "android")]
    let snapshot = android_playback_service(&state)?
        .execute(android_playback::PlaybackAction::RefreshCurrentTrack)
        .await?;
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    let snapshot = {
        let _ = &state;
        return Err("此平台尚未提供本機音訊播放服務。".to_owned());
    };
    let _ = app.emit(
        CURRENT_TRACK_REFRESHED_EVENT,
        snapshot
            .current_track
            .as_ref()
            .map(|track| track.id.to_string()),
    );
    Ok(snapshot)
}

#[tauri::command]
async fn playback_pause(state: State<'_, AppState>) -> Result<PlaybackSnapshot, String> {
    let _database_work = enter_database_work(&state)?;
    #[cfg(target_os = "windows")]
    {
        let service = windows_playback_service(&state)?;
        await_playback_ack(pause_playback(service)?).await?;
        if let Some(database) = state.database.as_ref() {
            checkpoint_playback_position(database, service, true)?;
        }
        Ok(service.snapshot())
    }
    #[cfg(target_os = "android")]
    {
        android_playback_service(&state)?
            .execute(android_playback::PlaybackAction::Pause)
            .await
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = state;
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
async fn playback_next(state: State<'_, AppState>) -> Result<PlaybackSnapshot, String> {
    let _database_work = enter_database_work(&state)?;
    #[cfg(target_os = "windows")]
    {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let service = windows_playback_service(&state)?;
        if let Some(change) = navigate_queue(database, service, true)? {
            let PreparedTrackChange {
                ticket,
                track,
                replacement_queue,
                target_cursor,
            } = change;
            await_playback_ack(ticket).await?;
            commit_track_change(database, service, track, replacement_queue, target_cursor)?;
        }
        Ok(service.snapshot())
    }
    #[cfg(target_os = "android")]
    {
        android_playback_service(&state)?
            .execute(android_playback::PlaybackAction::Next { natural: false })
            .await
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = state;
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
async fn playback_previous(state: State<'_, AppState>) -> Result<PlaybackSnapshot, String> {
    let _database_work = enter_database_work(&state)?;
    #[cfg(target_os = "windows")]
    {
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let service = windows_playback_service(&state)?;
        if let Some(change) = navigate_queue(database, service, false)? {
            let PreparedTrackChange {
                ticket,
                track,
                replacement_queue,
                target_cursor,
            } = change;
            await_playback_ack(ticket).await?;
            commit_track_change(database, service, track, replacement_queue, target_cursor)?;
        }
        Ok(service.snapshot())
    }
    #[cfg(target_os = "android")]
    {
        android_playback_service(&state)?
            .execute(android_playback::PlaybackAction::Previous)
            .await
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
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
    let _database_work = enter_database_work(&state)?;
    #[cfg(target_os = "windows")]
    {
        let service = windows_playback_service(&state)?;
        await_playback_ack(seek_playback(service, position_ms)?).await?;
        if let Some(database) = state.database.as_ref() {
            checkpoint_playback_position(database, service, true)?;
        }
        Ok(service.snapshot())
    }
    #[cfg(target_os = "android")]
    {
        android_playback_service(&state)?
            .execute(android_playback::PlaybackAction::Seek(position_ms))
            .await
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
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
    #[cfg(target_os = "android")]
    {
        android_playback_service(&state)?
            .execute(android_playback::PlaybackAction::SetVolume(volume))
            .await
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (state, volume);
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
async fn playback_set_repeat(
    state: State<'_, AppState>,
    mode: RepeatMode,
) -> Result<PlaybackSnapshot, String> {
    let _database_work = enter_database_work(&state)?;
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
        if let Some(database) = state.database.as_ref() {
            save_playback_session(database, service)?;
        }
        Ok(service.snapshot())
    }
    #[cfg(target_os = "android")]
    {
        android_playback_service(&state)?
            .execute(android_playback::PlaybackAction::SetRepeat(mode))
            .await
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (state, mode);
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
async fn playback_reorder_queue(
    state: State<'_, AppState>,
    from: u64,
    to: u64,
) -> Result<PlaybackSnapshot, String> {
    let _database_work = enter_database_work(&state)?;
    let from_index =
        usize::try_from(from).map_err(|_| "播放佇列來源位置超出支援範圍。".to_owned())?;
    let to_index = usize::try_from(to).map_err(|_| "播放佇列目標位置超出支援範圍。".to_owned())?;
    #[cfg(target_os = "windows")]
    {
        let service = windows_playback_service(&state)?;
        let _gate = service
            .command_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        {
            let mut queue = service
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(queue) = queue.as_mut() else {
                return Err("目前沒有播放佇列可調整順序。".to_owned());
            };
            let length = queue.len();
            if from_index >= length || to_index >= length {
                return Err("播放佇列位置超出範圍。".to_owned());
            }
            let before = queue.snapshot();
            queue
                .move_traversal(from_index, to_index)
                .map_err(|_| "播放佇列位置超出範圍。".to_owned())?;
            let after = queue.snapshot();
            if before.play_order != after.play_order || before.cursor != after.cursor {
                bump_queue_revision(service);
            }
        }
        if let Some(database) = state.database.as_ref() {
            save_playback_session(database, service)?;
        }
        Ok(service.snapshot())
    }
    #[cfg(target_os = "android")]
    {
        if let Some((_, queue)) = android_playback_service(&state)?.queue_snapshot()? {
            if from_index >= queue.play_order.len() || to_index >= queue.play_order.len() {
                return Err("播放佇列位置超出範圍。".to_owned());
            }
        } else {
            return Err("目前沒有播放佇列可調整順序。".to_owned());
        }
        let snapshot = android_playback_service(&state)?
            .execute(android_playback::PlaybackAction::ReorderQueue {
                from: from_index,
                to: to_index,
            })
            .await?;
        Ok(snapshot)
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (state, from_index, to_index);
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
async fn playback_sort_queue(
    state: State<'_, AppState>,
    sort: TrackListSort,
) -> Result<PlaybackSnapshot, String> {
    let _database_work = enter_database_work(&state)?;
    #[cfg(target_os = "windows")]
    {
        let service = windows_playback_service(&state)?;
        let _gate = service
            .command_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let revision = service.queue_revision.load(Ordering::Relaxed);
        let queue_snapshot = service
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(PlaybackQueue::snapshot)
            .ok_or_else(|| "目前沒有播放佇列可排序。".to_owned())?;
        let temporary_database = service
            .temporary_catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let external_tracks = service
            .external_tracks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|(id, track)| (*id, track.summary.clone()))
            .collect::<HashMap<_, _>>();
        let order = cached_queue_display_order(
            &state,
            revision,
            &queue_snapshot,
            sort,
            state.database.as_ref(),
            temporary_database.as_deref(),
            Some(&external_tracks),
        )?;
        state
            .settings
            .update(|settings| {
                settings.shuffle = false;
                Ok(())
            })
            .map_err(|error| error.to_string())?;
        {
            let mut queue = service
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let queue = queue
                .as_mut()
                .ok_or_else(|| "播放佇列已清除，請重新排序。".to_owned())?;
            let before = queue.snapshot();
            queue
                .set_traversal_order(order, sort)
                .map_err(|error| format!("無法套用播放佇列排序：{error}"))?;
            let after = queue.snapshot();
            if before.play_order != after.play_order || before.cursor != after.cursor {
                bump_queue_revision(service);
            }
        }
        *service
            .shuffle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
        if let Some(database) = state.database.as_ref() {
            save_playback_session(database, service)?;
        }
        Ok(service.snapshot())
    }
    #[cfg(target_os = "android")]
    {
        let service = android_playback_service(&state)?;
        let (revision, queue) = service
            .queue_snapshot()?
            .ok_or_else(|| "目前沒有播放佇列可排序。".to_owned())?;
        let order = cached_queue_display_order(
            &state,
            revision,
            &queue,
            sort,
            state.database.as_ref(),
            None,
            None,
        )?;
        state
            .settings
            .update(|settings| {
                settings.shuffle = false;
                Ok(())
            })
            .map_err(|error| error.to_string())?;
        service
            .execute(android_playback::PlaybackAction::SetQueueOrder {
                order,
                sort,
                expected_revision: revision,
            })
            .await
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (state, sort);
        Err("此平台尚未提供本機音訊播放服務。".to_owned())
    }
}

#[tauri::command]
async fn playback_set_shuffle(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<PlaybackSnapshot, String> {
    let _database_work = enter_database_work(&state)?;
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
        let temporary_database = service
            .temporary_catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(queue) = service
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            let before = queue.snapshot();
            if enabled && !queue.shuffle() {
                let database = state.database.as_ref().ok_or_else(|| {
                    state
                        .database_error
                        .clone()
                        .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
                })?;
                if service.external_queue_active.load(Ordering::Acquire) {
                    set_queue_shuffle_with_mixed_stats(
                        database,
                        temporary_database.as_deref(),
                        queue,
                        true,
                    )?;
                } else {
                    set_queue_shuffle_with_latest_stats(database, queue, true)?;
                }
            } else {
                queue.set_shuffle(enabled);
            }
            let after = queue.snapshot();
            if before.play_order != after.play_order || before.cursor != after.cursor {
                bump_queue_revision(service);
            }
        }
        if let Some(database) = state.database.as_ref() {
            save_playback_session(database, service)?;
        }
        Ok(service.snapshot())
    }
    #[cfg(target_os = "android")]
    {
        android_playback_service(&state)?
            .execute(android_playback::PlaybackAction::SetShuffle(enabled))
            .await
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
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
fn first_supported_audio_argument(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
    working_directory: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    arguments.into_iter().find_map(|argument| {
        let path = std::path::PathBuf::from(argument);
        let path = if path.is_absolute() {
            path
        } else {
            working_directory?.join(path)
        };
        is_supported_audio_file(&path).then_some(path)
    })
}

#[cfg(target_os = "windows")]
fn dispatch_external_audio_open(app: AppHandle, path: std::path::PathBuf) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let _database_work = match enter_database_work(&state) {
            Ok(guard) => guard,
            Err(error) => {
                eprintln!("無法開啟檔案總管傳入的音樂：{error}");
                return;
            }
        };
        let service = match windows_playback_service(&state) {
            Ok(service) => service,
            Err(error) => {
                eprintln!("無法開啟檔案總管傳入的音樂：{error}");
                return;
            }
        };
        if let Err(error) = open_external_audio_path(state.database.as_ref(), service, &path) {
            eprintln!("無法開啟檔案總管傳入的音樂：{error}");
        }
    });
}

#[cfg(target_os = "windows")]
fn snapshot_playback_queue(
    service: &WindowsPlaybackService,
) -> (u64, Option<PlaybackQueueSnapshot>) {
    let queue = service
        .queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let revision = service.queue_revision.load(Ordering::Relaxed);
    (revision, queue.as_ref().map(PlaybackQueue::snapshot))
}

fn cached_queue_display_order(
    state: &AppState,
    revision: u64,
    queue: &PlaybackQueueSnapshot,
    sort: TrackListSort,
    database: Option<&Database>,
    secondary_database: Option<&Database>,
    external_tracks: Option<&HashMap<TrackId, TrackSummary>>,
) -> Result<Vec<usize>, String> {
    {
        let cached = state
            .queue_display_sort_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cached) = cached.as_ref() {
            if cached.revision == revision && cached.sort == sort {
                return Ok(cached.order.clone());
            }
        }
    }

    let track_ids = queue
        .entries
        .iter()
        .map(|entry| entry.track_id)
        .collect::<Vec<_>>();
    let mut summaries = HashMap::<TrackId, TrackSummary>::new();
    if let Some(database) = database {
        for (id, track) in track_ids.iter().copied().zip(
            database
                .get_track_summaries(&track_ids)
                .map_err(|error| error.to_string())?,
        ) {
            if let Some(track) = track {
                summaries.insert(id, track);
            }
        }
    }
    if let Some(database) = secondary_database {
        let unresolved = track_ids
            .iter()
            .filter(|id| !summaries.contains_key(id))
            .copied()
            .collect::<Vec<_>>();
        for (id, track) in unresolved.iter().copied().zip(
            database
                .get_track_summaries(&unresolved)
                .map_err(|error| error.to_string())?,
        ) {
            if let Some(track) = track {
                summaries.insert(id, track);
            }
        }
    }
    if let Some(external_tracks) = external_tracks {
        for (id, track) in external_tracks {
            summaries.entry(*id).or_insert_with(|| track.clone());
        }
    }

    let order = if let Some(sort_database) = database.or(secondary_database) {
        sort_queue_entry_order_sql(queue, &summaries, sort, sort_database)?
    } else {
        sort_queue_entry_order(queue, &summaries, sort, None)
    };
    let mut cached = state
        .queue_display_sort_cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *cached = Some(QueueDisplaySortCache {
        revision,
        sort,
        order: order.clone(),
    });
    Ok(order)
}

fn sort_queue_entry_order(
    queue: &PlaybackQueueSnapshot,
    summaries: &HashMap<TrackId, TrackSummary>,
    sort: TrackListSort,
    database: Option<&Database>,
) -> Vec<usize> {
    let mut order = queue.play_order.clone();
    order.sort_by(|left_index, right_index| {
        let left_id = queue.entries[*left_index].track_id;
        let right_id = queue.entries[*right_index].track_id;
        match (summaries.get(&left_id), summaries.get(&right_id)) {
            (Some(left), Some(right)) => compare_track_summaries(left, right, sort, database),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
    order
}

fn sort_queue_entry_order_sql(
    queue: &PlaybackQueueSnapshot,
    summaries: &HashMap<TrackId, TrackSummary>,
    sort: TrackListSort,
    database: &Database,
) -> Result<Vec<usize>, String> {
    let candidates = queue
        .play_order
        .iter()
        .copied()
        .map(|entry_index| player_db::QueueSortCandidate {
            entry_index,
            summary: queue
                .entries
                .get(entry_index)
                .and_then(|entry| summaries.get(&entry.track_id))
                .cloned(),
        })
        .collect::<Vec<_>>();
    database
        .sort_queue_candidates(&candidates, sort)
        .map_err(|error| error.to_string())
}

fn build_playback_queue_page(
    database: Option<&Database>,
    revision: u64,
    queue: Option<PlaybackQueueSnapshot>,
    offset: u64,
    requested_limit: u32,
) -> Result<PlaybackQueuePage, String> {
    let Some(queue) = queue else {
        return Ok(PlaybackQueuePage {
            revision,
            total: 0,
            offset,
            cursor: None,
            current_entry_position: None,
            items: Vec::new(),
        });
    };

    let total = u64::try_from(queue.play_order.len())
        .map_err(|_| "播放佇列項目數量超出支援範圍。".to_owned())?;
    let queue_cursor =
        u64::try_from(queue.cursor).map_err(|_| "播放佇列游標超出支援範圍。".to_owned())?;
    let current_entry_position = queue
        .play_order
        .get(queue.cursor)
        .copied()
        .map(u64::try_from)
        .transpose()
        .map_err(|_| "播放佇列項目位置超出支援範圍。".to_owned())?;
    let cursor = queue
        .play_order
        .iter()
        .position(|entry_index| Some(*entry_index as u64) == current_entry_position)
        .map(|index| index as u64)
        .unwrap_or(queue_cursor);
    let offset_index = usize::try_from(offset.min(total)).unwrap_or(usize::MAX);
    let limit = requested_limit.clamp(1, PLAYBACK_QUEUE_PAGE_MAX_LIMIT) as usize;
    let page_entries = queue
        .play_order
        .iter()
        .skip(offset_index)
        .take(limit)
        .enumerate()
        .map(|(page_index, entry_index)| {
            let traversal_position = offset
                .checked_add(page_index as u64)
                .ok_or_else(|| "播放佇列分頁位置超出支援範圍。".to_owned())?;
            let entry_position = u64::try_from(*entry_index)
                .map_err(|_| "播放佇列項目位置超出支援範圍。".to_owned())?;
            let entry = queue
                .entries
                .get(*entry_index)
                .ok_or_else(|| "播放佇列 traversal 指向無效項目。".to_owned())?;
            Ok((traversal_position, entry_position, entry))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let track_ids = page_entries
        .iter()
        .map(|(_, _, entry)| entry.track_id)
        .collect::<Vec<_>>();
    let tracks = match database {
        Some(database) => database
            .get_track_summaries(&track_ids)
            .map_err(|error| error.to_string())?,
        None => vec![None; page_entries.len()],
    };

    let items = page_entries
        .into_iter()
        .zip(tracks)
        .map(
            |((traversal_position, entry_position, entry), track)| PlaybackQueuePageItem {
                traversal_position,
                entry_position,
                source_position: entry.source_position,
                track_id: entry.track_id,
                track,
                is_current: entry_position == current_entry_position.unwrap_or(u64::MAX),
            },
        )
        .collect();

    Ok(PlaybackQueuePage {
        revision,
        total,
        offset,
        cursor: Some(cursor),
        current_entry_position,
        items,
    })
}

#[cfg(target_os = "windows")]
fn bump_queue_revision(service: &WindowsPlaybackService) {
    service.queue_revision.fetch_add(1, Ordering::Relaxed);
}

#[cfg(any(target_os = "windows", target_os = "android"))]
fn queue_repeat_mode(mode: RepeatMode) -> QueueRepeatMode {
    match mode {
        RepeatMode::Off => QueueRepeatMode::Off,
        RepeatMode::One => QueueRepeatMode::One,
        RepeatMode::All => QueueRepeatMode::All,
    }
}

fn set_queue_shuffle_with_latest_stats(
    database: &Database,
    queue: &mut PlaybackQueue,
    enabled: bool,
) -> Result<(), String> {
    if !enabled || queue.shuffle() {
        queue.set_shuffle(enabled);
        return Ok(());
    }
    let snapshot = queue.snapshot();
    let track_ids = snapshot
        .entries
        .iter()
        .map(|entry| entry.track_id)
        .collect::<Vec<_>>();
    let stored = database
        .playback_statistics_for(&track_ids)
        .map_err(|error| format!("讀取曲目播放時長失敗：{error}"))?;
    let stats_by_track = stored
        .into_iter()
        .map(|(track_id, stats)| {
            (
                track_id,
                QueueTrackListeningStats {
                    played_ms: stats.played_ms,
                    duration_ms: stats.duration_ms,
                },
            )
        })
        .collect();
    queue.set_shuffle_with_stats(true, &stats_by_track);
    Ok(())
}

#[cfg(target_os = "windows")]
fn set_queue_shuffle_with_mixed_stats(
    database: &Database,
    temporary_database: Option<&Database>,
    queue: &mut PlaybackQueue,
    enabled: bool,
) -> Result<(), String> {
    if !enabled || queue.shuffle() {
        queue.set_shuffle(enabled);
        return Ok(());
    }
    let track_ids = queue
        .snapshot()
        .entries
        .into_iter()
        .map(|entry| entry.track_id)
        .collect::<Vec<_>>();
    let mut statistics = HashMap::new();
    let mut library_ids = Vec::new();
    let mut temporary_ids = Vec::new();
    for track_id in track_ids {
        if temporary_database
            .map(|temporary| temporary.get_track_summary(track_id))
            .transpose()
            .map_err(|error| error.to_string())?
            .flatten()
            .is_some()
        {
            temporary_ids.push(track_id);
        } else {
            library_ids.push(track_id);
        }
    }
    if !library_ids.is_empty() {
        statistics.extend(
            database
                .playback_statistics_for(&library_ids)
                .map_err(|error| format!("讀取正式曲庫播放時長失敗：{error}"))?,
        );
    }
    if let Some(temporary_database) = temporary_database {
        if !temporary_ids.is_empty() {
            statistics.extend(
                temporary_database
                    .playback_statistics_for(&temporary_ids)
                    .map_err(|error| format!("讀取暫存曲庫播放時長失敗：{error}"))?,
            );
        }
    }
    let stats_by_track = statistics
        .into_iter()
        .map(|(track_id, stats)| {
            (
                track_id,
                QueueTrackListeningStats {
                    played_ms: stats.played_ms,
                    duration_ms: stats.duration_ms,
                },
            )
        })
        .collect();
    queue.set_shuffle_with_stats(true, &stats_by_track);
    Ok(())
}

#[tauri::command]
fn windows_register_audio_associations() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        windows_file_associations::register_file_associations(&executable)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("此功能僅適用於 Windows。".to_owned())
    }
}

#[tauri::command]
fn windows_open_default_apps_settings() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_file_associations::open_default_apps_settings()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("此功能僅適用於 Windows。".to_owned())
    }
}

fn queue_for_track(
    database: &Database,
    track_id: TrackId,
    source: Option<PlaybackQueueSource>,
) -> Result<PlaybackQueue, String> {
    let (entries, context, selected_index) = match source {
        Some(PlaybackQueueSource::Library {
            query,
            query_alternates,
            field_filter,
            sort,
        }) => {
            let track_ids = database
                .list_track_ids_filtered_with_alternates(
                    query.as_deref(),
                    &query_alternates,
                    field_filter.as_ref(),
                    sort,
                )
                .map_err(|error| error.to_string())?;
            let selected = track_ids
                .iter()
                .position(|candidate| *candidate == track_id)
                .ok_or_else(|| "所選曲目已不在目前的曲庫查詢結果中。".to_owned())?;
            let entries = track_ids
                .into_iter()
                .map(|track_id| PlaybackQueueEntry {
                    track_id,
                    source_position: None,
                })
                .collect();
            (
                entries,
                PlaybackQueueContext::Library {
                    query,
                    field_filter,
                    sort,
                },
                selected,
            )
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
            let entries = entries
                .into_iter()
                .map(|(position, track_id)| PlaybackQueueEntry {
                    track_id,
                    source_position: Some(position),
                })
                .collect();
            (
                entries,
                PlaybackQueueContext::Playlist {
                    playlist_id: playlist_id.to_string(),
                },
                selected,
            )
        }
        Some(PlaybackQueueSource::Queue { .. }) => {
            return Err("播放佇列選曲不會重建佇列。".to_owned());
        }
        None => {
            let track_ids = database
                .list_track_ids(None)
                .map_err(|error| error.to_string())?;
            let selected = track_ids
                .iter()
                .position(|candidate| *candidate == track_id)
                .ok_or_else(|| "所選曲目已不在曲庫中。".to_owned())?;
            let entries = track_ids
                .into_iter()
                .map(|track_id| PlaybackQueueEntry {
                    track_id,
                    source_position: None,
                })
                .collect();
            (
                entries,
                PlaybackQueueContext::Library {
                    query: None,
                    field_filter: None,
                    sort: Default::default(),
                },
                selected,
            )
        }
    };
    PlaybackQueue::with_entries(entries, context, selected_index)
        .ok_or_else(|| "播放清單沒有可播放的曲目。".to_owned())
}

#[cfg(target_os = "windows")]
fn external_track_summary(path: &std::path::Path, id: TrackId) -> TrackSummary {
    let metadata = read_standalone_audio_metadata(path).unwrap_or_default();
    let file_name = path
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned());
    TrackSummary {
        id,
        title: metadata.title,
        artist: metadata.artist,
        album: metadata.album,
        album_artist: metadata.album_artist,
        track_number: metadata.track_number,
        disc_number: metadata.disc_number,
        duration_ms: metadata.duration_ms,
        codec: metadata.codec,
        bitrate_bps: metadata.bitrate_bps,
        sample_rate_hz: metadata.sample_rate_hz,
        release_date: metadata.release_date,
        year: metadata.year,
        created_at_utc_ms: std::fs::metadata(path)
            .ok()
            .and_then(|metadata| metadata.created().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|duration| i64::try_from(duration.as_millis()).ok()),
        bit_depth: metadata.bit_depth,
        played_ms: 0,
        file_name,
        // A file opened from outside the library has no library source.
        source_name: None,
    }
}

#[cfg(target_os = "windows")]
fn same_folder_audio_files(
    selected_path: &std::path::Path,
) -> Result<Vec<std::path::PathBuf>, String> {
    let selected_path = std::fs::canonicalize(selected_path)
        .map_err(|error| format!("無法開啟音樂檔案：{error}"))?;
    if !selected_path.is_file() || !is_supported_audio_file(&selected_path) {
        return Err("選取的檔案不是支援的音訊格式。".to_owned());
    }
    let parent = selected_path
        .parent()
        .ok_or_else(|| "無法判斷音樂檔案所在資料夾。".to_owned())?;
    let mut paths = std::fs::read_dir(parent)
        .map_err(|error| format!("無法讀取音樂檔案所在資料夾：{error}"))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_file() || !is_supported_audio_file(&entry.path()) {
                return None;
            }
            std::fs::canonicalize(entry.path()).ok()
        })
        .collect::<Vec<_>>();
    if !paths.iter().any(|path| path == &selected_path) {
        paths.push(selected_path);
    }
    paths.sort_by(|left, right| {
        let left_name = left.file_name().unwrap_or_default().to_string_lossy();
        let right_name = right.file_name().unwrap_or_default().to_string_lossy();
        natural_title_cmp(&left_name, &right_name).then_with(|| left_name.cmp(&right_name))
    });
    paths.dedup();
    Ok(paths)
}

#[cfg(target_os = "windows")]
fn open_external_audio_path(
    database: Option<&Database>,
    service: &WindowsPlaybackService,
    selected_path: &std::path::Path,
) -> Result<(), String> {
    let selected_path = std::fs::canonicalize(selected_path)
        .map_err(|error| format!("無法開啟音樂檔案：{error}"))?;
    let paths = same_folder_audio_files(&selected_path)?;

    let mut entries = Vec::with_capacity(paths.len());
    let mut transient = HashMap::new();
    let mut temporary_records = Vec::new();
    let mut selected_index = None;
    for (index, path) in paths.iter().enumerate() {
        let library_track = database
            .map(|database| {
                database
                    .resolve_track_id_for_locator(&MediaLocator::FileSystem(path.clone()))
                    .map_err(|error| format!("檢查曲庫路徑失敗：{error}"))?
                    .map(|id| {
                        database
                            .get_track_summary(id)
                            .map_err(|error| format!("讀取曲庫曲目失敗：{error}"))?
                            .map(|summary| (id, summary))
                            .ok_or_else(|| "曲庫路徑指向不存在的曲目。".to_owned())
                    })
                    .transpose()
            })
            .transpose()?
            .flatten();
        let (track_id, summary, transient_track) = if let Some((id, summary)) = library_track {
            (id, summary, false)
        } else {
            let id = TrackId::new();
            let summary = external_track_summary(path, id);
            let file_metadata = std::fs::metadata(path)
                .map_err(|error| format!("讀取音樂檔案資訊失敗：{error}"))?;
            temporary_records.push(MediaTrackRecord {
                identity: TrackIdentity {
                    source_id: SourceId::new(),
                    source_item_id: path.to_string_lossy().into_owned(),
                    locator_key: Some(path.to_string_lossy().into_owned()),
                },
                locator: MediaLocator::FileSystem(path.clone()),
                fingerprint: FileFingerprint {
                    size_bytes: file_metadata.len(),
                    modified_at_utc_ms: file_metadata.modified().ok().and_then(|modified| {
                        modified
                            .duration_since(UNIX_EPOCH)
                            .ok()
                            .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
                    }),
                    created_at_utc_ms: file_metadata.created().ok().and_then(|created| {
                        created
                            .duration_since(UNIX_EPOCH)
                            .ok()
                            .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
                    }),
                },
                metadata: Some(TrackMetadata {
                    // A track opened from outside the library keeps no lyrics of its own here.
                    embedded_lyrics: None,
                    title: summary.title.clone(),
                    artist: summary.artist.clone(),
                    album: summary.album.clone(),
                    album_artist: summary.album_artist.clone(),
                    track_number: summary.track_number,
                    disc_number: summary.disc_number,
                    duration_ms: summary.duration_ms,
                    codec: summary.codec.clone(),
                    bitrate_bps: summary.bitrate_bps,
                    sample_rate_hz: summary.sample_rate_hz,
                    release_date: summary.release_date.clone(),
                    year: summary.year,
                    bit_depth: summary.bit_depth,
                }),
            });
            (id, summary, true)
        };
        transient.insert(
            track_id,
            ExternalPlaybackTrack {
                path: path.clone(),
                summary,
                transient: transient_track,
            },
        );
        if path == &selected_path {
            selected_index = Some(index);
        }
        entries.push(PlaybackQueueEntry {
            track_id,
            source_position: None,
        });
    }
    let selected_index =
        selected_index.ok_or_else(|| "選取的音樂檔案不在資料夾清單中。".to_owned())?;

    let temporary_database = if temporary_records.is_empty() {
        None
    } else {
        let mut temporary_database =
            Database::open_in_memory().map_err(|error| format!("建立暫存曲庫失敗：{error}"))?;
        let parent = selected_path
            .parent()
            .ok_or_else(|| "無法判斷音樂檔案所在資料夾。".to_owned())?;
        let root = temporary_database
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                parent
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "檔案總管佇列".to_owned()),
                MediaLocator::FileSystem(parent.to_path_buf()),
            )
            .map_err(|error| format!("建立暫存曲庫來源失敗：{error}"))?;
        for record in &mut temporary_records {
            record.identity.source_id = root.id;
        }
        temporary_database
            .apply_source_scan(
                &root,
                &SourceScanState::Complete,
                &temporary_records,
                &temporary_records,
                &[],
                now_utc_epoch_ms().unwrap_or_default(),
            )
            .map_err(|error| format!("載入暫存曲庫失敗：{error}"))?;
        let runtime_id = *service
            .statistics_runtime_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(runtime_id) = runtime_id {
            temporary_database
                .register_playback_statistics_runtime(runtime_id, None, None)
                .map_err(|error| format!("登記暫存曲庫統計工作階段失敗：{error}"))?;
        }
        let temporary_database = Arc::new(temporary_database);
        for record in &temporary_records {
            let MediaLocator::FileSystem(path) = &record.locator else {
                continue;
            };
            let transient_id = transient
                .iter()
                .find_map(|(id, track)| (track.transient && track.path == *path).then_some(*id));
            let Some(transient_id) = transient_id else {
                continue;
            };
            let catalog_id = temporary_database
                .resolve_track_id_for_locator(&record.locator)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "暫存曲庫沒有建立曲目對應。".to_owned())?;
            let mut track = transient
                .remove(&transient_id)
                .ok_or_else(|| "暫存曲目在建立資料庫時消失。".to_owned())?;
            track.summary = temporary_database
                .get_track_summary(catalog_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "暫存曲庫曲目摘要不存在。".to_owned())?;
            transient.insert(catalog_id, track);
            for entry in &mut entries {
                if entry.track_id == transient_id {
                    entry.track_id = catalog_id;
                }
            }
        }
        Some(temporary_database)
    };

    let mut queue = PlaybackQueue::with_entries(
        entries,
        PlaybackQueueContext::Library {
            query: None,
            field_filter: None,
            sort: Default::default(),
        },
        selected_index,
    )
    .ok_or_else(|| "此資料夾沒有可播放的音訊檔案。".to_owned())?;
    let repeat_mode = *service
        .repeat_mode
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    queue.set_repeat_mode(queue_repeat_mode(repeat_mode));
    let shuffle = *service
        .shuffle
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if shuffle {
        if let Some(database) = database {
            set_queue_shuffle_with_mixed_stats(
                database,
                temporary_database.as_deref(),
                &mut queue,
                true,
            )?;
        } else {
            queue.set_shuffle(true);
        }
    }
    let selected_id = queue.current();
    let selected_track = transient
        .get(&selected_id)
        .map(|track| track.summary.clone())
        .ok_or_else(|| "無法讀取所選音樂檔案的曲目資訊。".to_owned())?;
    let selected_file = transient
        .get(&selected_id)
        .map(|track| track.path.clone())
        .ok_or_else(|| "無法解析所選音樂檔案路徑。".to_owned())?;
    let ticket = if transient
        .get(&selected_id)
        .is_some_and(|track| track.transient)
    {
        let accounting_id = temporary_database
            .as_ref()
            .map(|database| service.accounting_id_for_temporary(selected_id, Arc::clone(database)));
        if let Some(accounting_id) = accounting_id {
            service
                .player
                .request_load_and_play_accounted(selected_file, accounting_id)
        } else {
            service.player.request_load_and_play(selected_file)
        }
    } else {
        service
            .player
            .request_load_and_play_accounted(selected_file, service.accounting_id_for(selected_id))
    }
    .map_err(|error| playback_audio_error_message(&error))?;
    ticket
        .wait(Duration::from_secs(10))
        .map_err(|error| playback_audio_error_message(&error))?;

    *service
        .external_tracks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = transient;
    let previous_temporary_database = {
        let mut catalog = service
            .temporary_catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::replace(&mut *catalog, temporary_database.clone())
    };
    if let Some(previous) = previous_temporary_database {
        let old_ids = {
            let accounting = service
                .accounting_track_ids
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            accounting
                .iter()
                .filter_map(|(id, target)| {
                    target
                        .temporary_database
                        .as_ref()
                        .is_some_and(|database| Arc::ptr_eq(database, &previous))
                        .then_some((*id, target.track_id))
                })
                .collect::<Vec<_>>()
        };
        let old_accounting_ids = old_ids
            .iter()
            .map(|(id, _)| *id)
            .collect::<std::collections::HashSet<_>>();
        if let Some(runtime_id) = *service
            .statistics_runtime_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            let deadline = Instant::now() + Duration::from_secs(6);
            service.player.request_playback_accounting_flush();
            loop {
                let pending = service
                    .player
                    .playback_accounting_checkpoints()
                    .iter()
                    .any(|checkpoint| old_accounting_ids.contains(&checkpoint.id));
                if !pending || Instant::now() >= deadline {
                    break;
                }
                service.player.request_playback_accounting_flush();
                std::thread::sleep(Duration::from_millis(100));
            }
            let still_pending = service
                .player
                .playback_accounting_checkpoints()
                .iter()
                .any(|checkpoint| old_accounting_ids.contains(&checkpoint.id));
            if !still_pending {
                let _ = previous.finish_playback_statistics_runtime(runtime_id, &[]);
                let mut accounting = service
                    .accounting_track_ids
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                accounting.retain(|id, _| !old_accounting_ids.contains(id));
            }
        } else {
            let mut accounting = service
                .accounting_track_ids
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            accounting.retain(|_, target| {
                !target
                    .temporary_database
                    .as_ref()
                    .is_some_and(|database| Arc::ptr_eq(database, &previous))
            });
        }
    }
    *service
        .queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(queue);
    service.external_queue_active.store(true, Ordering::Release);
    *service
        .current_track
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(selected_track);
    bump_queue_revision(service);
    Ok(())
}

#[cfg(target_os = "windows")]
fn prepare_track_load(
    database: &Database,
    service: &WindowsPlaybackService,
    track_id: TrackId,
    force_play: bool,
) -> Result<(CommandTicket, TrackSummary), String> {
    if let Some(track) = service
        .external_tracks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&track_id)
        .cloned()
    {
        let ticket = if track.transient {
            let temporary_database = service
                .temporary_catalog
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .cloned();
            let accounting_id = temporary_database
                .map(|database| service.accounting_id_for_temporary(track_id, database));
            if force_play {
                if let Some(accounting_id) = accounting_id {
                    service
                        .player
                        .request_load_and_play_accounted(track.path, accounting_id)
                } else {
                    service.player.request_load_and_play(track.path)
                }
            } else if let Some(accounting_id) = accounting_id {
                service
                    .player
                    .request_load_preserving_playback_accounted(track.path, accounting_id)
            } else {
                service.player.request_load_preserving_playback(track.path)
            }
        } else {
            let accounting_id = service.accounting_id_for(track_id);
            if force_play {
                service
                    .player
                    .request_load_and_play_accounted(track.path, accounting_id)
            } else {
                service
                    .player
                    .request_load_preserving_playback_accounted(track.path, accounting_id)
            }
        }
        .map_err(|error| playback_audio_error_message(&error))?;
        return Ok((ticket, track.summary));
    }
    let track = database
        .get_track_summary(track_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "曲目已不在曲庫中，請重新整理列表。".to_owned())?;
    let path = database
        .resolve_playable_filesystem_locator(track_id)
        .map_err(|error| error.to_string())?;
    let accounting_id = service.accounting_id_for(track_id);
    let ticket = if force_play {
        service
            .player
            .request_load_and_play_accounted(path, accounting_id)
    } else {
        service
            .player
            .request_load_preserving_playback_accounted(path, accounting_id)
    }
    .map_err(|error| playback_audio_error_message(&error))?;
    Ok((ticket, track))
}

#[cfg(target_os = "windows")]
fn navigate_queue(
    database: &Database,
    service: &WindowsPlaybackService,
    next: bool,
) -> Result<Option<PreparedTrackChange>, String> {
    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let target = {
        let queue = service
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(queue) = queue.as_ref() else {
            return Ok(None);
        };
        if next {
            queue.preview_next(true)
        } else {
            queue.preview_previous()
        }
    };
    target
        .map(|(track_id, cursor)| {
            let (ticket, track) = prepare_track_load(database, service, track_id, false)?;
            Ok(PreparedTrackChange {
                ticket,
                track,
                replacement_queue: None,
                target_cursor: Some(cursor),
            })
        })
        .transpose()
}

#[cfg(target_os = "windows")]
fn advance_ended_playback(
    database: &Database,
    service: &WindowsPlaybackService,
) -> Result<Option<PreparedTrackChange>, String> {
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
        .as_ref()
        .and_then(|queue| queue.preview_next(false));
    let Some((track_id, cursor)) = target else {
        *service
            .queue_advance_pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
        return Ok(None);
    };
    match prepare_track_load(database, service, track_id, true) {
        Ok((ticket, track)) => Ok(Some(PreparedTrackChange {
            ticket,
            track,
            replacement_queue: None,
            target_cursor: Some(cursor),
        })),
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
fn output_device_settings(
    entries: &[settings::OutputDeviceEntry],
) -> Vec<player_audio_windows::output_devices::OutputDeviceSetting> {
    entries
        .iter()
        .map(
            |entry| player_audio_windows::output_devices::OutputDeviceSetting {
                id: entry.id.clone(),
                enabled: entry.enabled,
                delay_ms: entry.delay_ms,
                channel_map: entry.channel_map.as_ref().map(|routes| {
                    routes
                        .iter()
                        .map(|route| player_audio_windows::output_devices::ChannelRoute {
                            source: route.source,
                            gain_percent: route.gain_percent,
                        })
                        .collect()
                }),
            },
        )
        .collect()
}

#[cfg(target_os = "windows")]
fn set_playback_exclusive(
    service: &WindowsPlaybackService,
    exclusive: bool,
) -> Result<CommandTicket, String> {
    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    service
        .player
        .request_set_exclusive(exclusive)
        .map_err(|error| playback_audio_error_message(&error))
}

#[cfg(target_os = "windows")]
fn set_playback_output_devices(
    service: &WindowsPlaybackService,
    entries: &[settings::OutputDeviceEntry],
) -> Result<CommandTicket, String> {
    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    service
        .player
        .request_set_output_devices(output_device_settings(entries))
        .map_err(|error| playback_audio_error_message(&error))
}

#[cfg(target_os = "windows")]
fn set_playback_dsd(
    service: &WindowsPlaybackService,
    saved: settings::DsdPlaybackSettings,
) -> Result<CommandTicket, String> {
    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    service
        .player
        .request_set_dsd_settings(player_audio_windows::DsdSettings {
            gain_db: saved.gain_db,
            output_rate: saved.output_rate_hz,
            dop: saved.dop,
        })
        .map_err(|error| playback_audio_error_message(&error))
}

#[cfg(target_os = "windows")]
fn set_playback_dsee_hx(
    service: &WindowsPlaybackService,
    enabled: bool,
) -> Result<CommandTicket, String> {
    let _gate = service
        .command_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    service
        .player
        .request_set_dsee_hx(enabled)
        .map_err(|error| playback_audio_error_message(&error))
}

#[cfg(target_os = "windows")]
async fn await_playback_ack(ticket: CommandTicket) -> Result<(), String> {
    await_playback_ack_within(ticket, Duration::from_secs(2)).await
}

#[cfg(target_os = "windows")]
async fn await_playback_ack_within(ticket: CommandTicket, timeout: Duration) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || ticket.wait(timeout))
        .await
        .map_err(|error| format!("等待播放命令工作失敗：{error}"))?
        .map(|_| ())
        .map_err(|error| playback_audio_error_message(&error))
}

#[cfg(target_os = "windows")]
fn commit_track_change(
    database: &Database,
    service: &WindowsPlaybackService,
    track: TrackSummary,
    replacement_queue: Option<PlaybackQueue>,
    target_cursor: Option<usize>,
) -> Result<(), String> {
    let queue_replaced = replacement_queue.is_some();
    if let Some(queue) = replacement_queue {
        let contains_temporary_tracks = queue.snapshot().entries.iter().any(|entry| {
            service
                .external_tracks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&entry.track_id)
                .is_some_and(|track| track.transient)
        });
        let mut current_queue = service
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *current_queue = Some(queue);
        bump_queue_revision(service);
        if !contains_temporary_tracks {
            *service
                .external_tracks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = HashMap::new();
            service
                .external_queue_active
                .store(false, Ordering::Release);
        }
    } else if let Some(cursor) = target_cursor {
        let mut queue = service
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queue
            .as_mut()
            .ok_or_else(|| "播放佇列在曲目切換期間消失。".to_owned())?
            .set_cursor(cursor)
            .map_err(|error| format!("播放佇列游標無效：{error}"))?;
        bump_queue_revision(service);
    }
    *service
        .current_track
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(track);
    *service
        .restore_warning
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    if queue_replaced {
        save_playback_session(database, service)?;
    } else {
        checkpoint_playback_position(database, service, true)?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn checkpoint_playback_position(
    database: &Database,
    service: &WindowsPlaybackService,
    force: bool,
) -> Result<(), String> {
    if service.external_queue_active.load(Ordering::Acquire) {
        return Ok(());
    }
    if !force {
        let mut last = service
            .last_position_checkpoint
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if service.player.snapshot().state != AudioPlaybackState::Playing
            || last.elapsed() < Duration::from_secs(5)
        {
            return Ok(());
        }
        *last = Instant::now();
    }
    let queue = service
        .queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .map(PlaybackQueue::snapshot);
    let Some(queue) = queue else {
        return Ok(());
    };
    let position_ms = duration_millis(service.player.snapshot().position);
    database
        .checkpoint_playback_position(&queue, position_ms)
        .map_err(|error| format!("保存播放狀態失敗：{error}"))?;
    *service
        .last_position_checkpoint
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Instant::now();
    Ok(())
}

#[cfg(target_os = "windows")]
fn capture_window_geometry(window: &tauri::Window) -> Option<WindowGeometry> {
    if window.is_minimized().unwrap_or(false) {
        return None;
    }
    let position = window.outer_position().ok()?;
    let size = window.inner_size().ok()?;
    let maximized = window.is_maximized().unwrap_or(false);
    let geometry = WindowGeometry {
        x: position.x,
        y: position.y,
        width: size.width.max(1),
        height: size.height.max(1),
        maximized,
    };
    geometry.is_valid_restore_snapshot().then_some(geometry)
}

#[cfg(target_os = "windows")]
fn remember_main_window_normal_frame(window: &tauri::Window) {
    if window.is_maximized().unwrap_or(false) {
        return;
    }
    let Some(geometry) = capture_window_geometry(window) else {
        return;
    };
    let state = window.app_handle().state::<AppState>();
    let Ok(mut frame) = state.window_frame.lock() else {
        return;
    };
    frame.normal = Some(WindowGeometry {
        maximized: false,
        ..geometry
    });
}

#[cfg(target_os = "windows")]
fn persist_main_window_geometry(window: &tauri::Window) {
    let maximized = window.is_maximized().unwrap_or(false);
    let state = window.app_handle().state::<AppState>();
    let saved_normal = || {
        state
            .window_frame
            .lock()
            .ok()
            .and_then(|frame| frame.normal.clone())
    };
    let geometry = if maximized {
        saved_normal().or_else(|| capture_window_geometry(window))
    } else {
        capture_window_geometry(window).or_else(saved_normal)
    };
    let Some(mut geometry) = geometry else {
        return;
    };
    geometry.maximized = maximized;
    if let Err(error) = state.settings.update(|settings| {
        settings.window_geometry = Some(geometry.clone());
        Ok(())
    }) {
        eprintln!("無法保存視窗位置與大小：{error}");
        return;
    }
    let Ok(mut frame) = state.window_frame.lock() else {
        return;
    };
    frame.normal = Some(WindowGeometry {
        maximized: false,
        ..geometry
    });
}

#[cfg(target_os = "windows")]
fn apply_saved_window_geometry(window: &WebviewWindow, geometry: WindowGeometry) {
    if !geometry.is_valid_restore_snapshot() {
        eprintln!("忽略最小化或無效的視窗快照，使用預設視窗大小。");
        return;
    }
    let monitors: Vec<(i32, i32, u32, u32)> = window
        .available_monitors()
        .unwrap_or_default()
        .into_iter()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            (position.x, position.y, size.width, size.height)
        })
        .collect();
    let geometry = clamp_window_geometry_to_monitors(geometry, &monitors);
    if let Err(error) = window.set_size(Size::Physical(PhysicalSize::new(
        geometry.width,
        geometry.height,
    ))) {
        eprintln!("無法還原視窗大小：{error}");
    }
    if let Err(error) = window.set_position(Position::Physical(PhysicalPosition::new(
        geometry.x, geometry.y,
    ))) {
        eprintln!("無法還原視窗位置：{error}");
    }
    if geometry.maximized {
        if let Err(error) = window.maximize() {
            eprintln!("無法還原最大化狀態：{error}");
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn install_downloaded_update(
    app: &AppHandle,
    staged: &updater::StagedUpdate,
    restart: bool,
) -> Result<updater::UpdateState, String> {
    // Validate executable/helper and paths while playback is still running.
    updater::validate_for_apply(staged)?;
    let service = app.state::<updater::UpdateService>();
    if !restart {
        // Rename the running files and put the new ones in place now; the new version starts on the next launch and
        // playback is not touched.
        updater::install_in_place(staged)?;
        return Ok(updater::publish(app, &service, |state| {
            state.status = "deferred".into();
        }));
    }
    let state = app.state::<AppState>();
    if state.shutdown.request() != database_lifecycle::ShutdownRequest::Start {
        return Err("程式正在關閉，無法開始更新。".into());
    }
    // The update restarts the program: a library sync in progress saves what it has read and stops.
    cancel_library_sync(&state);
    let was_playing = state
        .playback
        .as_ref()
        .is_some_and(|playback| playback.player.snapshot().state == AudioPlaybackState::Playing);
    if was_playing {
        set_resume_playback(&state.settings, true);
    }
    let result = database_lifecycle::finish_update_shutdown(
        &state.shutdown,
        &state.database_work,
        Duration::from_secs(30),
        || {
            if let (Some(database), Some(playback)) =
                (state.database.as_ref(), state.playback.as_ref())
            {
                let _command_gate = playback
                    .command_gate
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let has_track = playback
                    .current_track
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .is_some();
                if has_track || !playback.player.playback_accounting_checkpoints().is_empty() {
                    playback
                        .player
                        .request_pause()
                        .and_then(|ticket| ticket.wait(Duration::from_secs(2)))
                        .map_err(|error| format!("無法安全暫停播放，已取消更新：{error}"))?;
                    playback
                        .player
                        .request_playback_accounting_checkpoint()
                        .and_then(|ticket| ticket.wait(Duration::from_secs(2)))
                        .map_err(|error| format!("無法取得播放統計尾段，已取消更新：{error}"))?;
                    playback.player.request_playback_accounting_flush();
                    let deadline = Instant::now() + Duration::from_secs(10);
                    while !playback.player.playback_accounting_checkpoints().is_empty() {
                        if Instant::now() >= deadline {
                            return Err("播放時長尚未保存，已取消更新；程式仍可繼續使用。".into());
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
                checkpoint_playback_position(database, playback, true)?;
                database
                    .checkpoint_wal()
                    .map_err(|error| format!("SQLite checkpoint 失敗，已取消更新：{error}"))?;
            } else if let Some(database) = state.database.as_ref() {
                database
                    .checkpoint_wal()
                    .map_err(|error| error.to_string())?;
            }
            // Do not stop SMTC/the statistics collector here: a failed strict save
            // must leave the existing services usable. Paused/ACKed counters are
            // already durable, so no unbounded collector join is needed for update.
            Ok(())
        },
    );
    // Strictly saved: replace the files, then start the new executable (it waits for this process to exit).
    let result = result.and_then(|()| {
        updater::install_in_place(staged)?;
        updater::spawn_relaunch(&staged.target)
    });
    if let Err(error) = result {
        database_lifecycle::abort_update_shutdown(&state.shutdown, &state.database_work);
        set_resume_playback(&state.settings, false);
        if was_playing {
            if let Some(playback) = state.playback.as_ref() {
                let _ = playback.player.request_play();
            }
        }
        return Err(error);
    }
    let result = updater::publish(app, &service, |state| state.status = "restarting".into());
    // Normal process teardown; the new executable is already waiting for this process to exit.
    app.exit(0);
    Ok(result)
}

#[cfg(target_os = "windows")]
fn schedule_database_shutdown(app: AppHandle, request: database_lifecycle::ShutdownRequest) {
    if request != database_lifecycle::ShutdownRequest::Start {
        return;
    }

    std::thread::spawn(move || {
        const DATABASE_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
        let state = app.state::<AppState>();
        let result = database_lifecycle::finish_shutdown(
            &state.shutdown,
            &state.database_work,
            DATABASE_DRAIN_TIMEOUT,
            || {
                if let Some(system_media) = state.system_media.as_ref() {
                    system_media.shutdown();
                }
                if let (Some(database), Some(playback)) =
                    (state.database.as_ref(), state.playback.as_ref())
                {
                    if playback.player.snapshot().state == AudioPlaybackState::Playing {
                        set_resume_playback(&state.settings, true);
                    }
                    if let Err(error) = checkpoint_playback_position(database, playback, true) {
                        eprintln!("無法在關閉時保存播放狀態：{error}");
                    }
                    if let Ok(ticket) = playback.player.request_playback_accounting_checkpoint() {
                        if let Err(error) = ticket.wait(Duration::from_secs(2)) {
                            eprintln!("無法在關閉時取得播放時長尾段：{error}");
                        }
                    }
                    playback.shutdown_statistics_collector();
                    if let Some(status) = playback
                        .statistics_status
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .as_ref()
                    {
                        eprintln!("播放時長統計狀態：{status}");
                    }
                    if let Err(error) = database.checkpoint_wal() {
                        eprintln!("無法在關閉時完成 SQLite WAL checkpoint：{error}");
                    }
                } else if let Some(database) = state.database.as_ref() {
                    if let Err(error) = database.checkpoint_wal() {
                        eprintln!("無法在關閉時完成 SQLite WAL checkpoint：{error}");
                    }
                }
            },
        );

        if result.is_err() {
            eprintln!(
                "關閉已取消：資料庫工作未能在 {} 秒內完成；應用程式仍保持開啟。",
                DATABASE_DRAIN_TIMEOUT.as_secs()
            );
            let _ = app.emit(
                "database-shutdown-blocked",
                "曲庫工作仍在執行，關閉已取消；請稍後再試。",
            );
            return;
        }

        if let Some(window) = app.get_webview_window("main") {
            if let Err(error) = window.close() {
                eprintln!("關閉主視窗失敗：{error}");
                app.exit(0);
            }
        } else {
            app.exit(0);
        }
    });
}

/// Record in settings.json whether audio was playing when the run ended (normal close or update restart); the next
/// launch consumes it to resume playback.
#[cfg(target_os = "windows")]
fn set_resume_playback(settings: &SettingsStore, resume: bool) {
    if let Err(error) = settings.update(|settings| {
        settings.resume_playback = resume;
        Ok(())
    }) {
        eprintln!("無法記錄關閉時的播放狀態：{error}");
    }
}

/// Reset the flag and report whether it was set.
#[cfg(target_os = "windows")]
fn take_resume_playback(settings: &SettingsStore) -> bool {
    let was_set = settings
        .snapshot()
        .map(|snapshot| snapshot.resume_playback)
        .unwrap_or(false);
    if was_set {
        set_resume_playback(settings, false);
    }
    was_set
}

/// Resume playback of the restored session when the last run ended while playing.
#[cfg(target_os = "windows")]
fn resume_playback_if_marked(settings: &SettingsStore, service: &WindowsPlaybackService) {
    if !take_resume_playback(settings) {
        return;
    }
    if service.snapshot().state != "ready" {
        return;
    }
    if let Err(error) = service
        .player
        .request_play()
        .and_then(|ticket| ticket.wait(Duration::from_secs(2)))
    {
        eprintln!(
            "無法接續上次的播放：{}",
            playback_audio_error_message(&error)
        );
    }
}

#[cfg(target_os = "windows")]
fn save_playback_session(
    database: &Database,
    service: &WindowsPlaybackService,
) -> Result<(), String> {
    if service.external_queue_active.load(Ordering::Acquire) {
        return Ok(());
    }
    let queue = service
        .queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .map(PlaybackQueue::snapshot);
    let Some(queue) = queue else {
        return Ok(());
    };
    let position_ms = duration_millis(service.player.snapshot().position);
    database
        .save_playback_session(&PlaybackSessionCheckpoint { queue, position_ms })
        .map_err(|error| format!("保存播放佇列失敗：{error}"))?;
    *service
        .last_position_checkpoint
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Instant::now();
    Ok(())
}

#[cfg(target_os = "windows")]
fn restore_playback_session(
    database: &Database,
    service: &WindowsPlaybackService,
) -> Result<(), String> {
    let Some(checkpoint) = database
        .load_playback_session()
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let mut queue = PlaybackQueue::restore(checkpoint.queue)
        .map_err(|error| format!("保存的播放佇列無法還原：{error}"))?;
    // The queue may have been last checkpointed before a temporary Explorer-open
    // queue was played. Mode changes in that temporary queue still update the
    // user's settings.json, so restore the latest preferences over the saved
    // traversal while retaining its current track and position.
    let preferred_shuffle = *service
        .shuffle
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let preferred_repeat = *service
        .repeat_mode
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let queue_modes_changed = queue.shuffle() != preferred_shuffle
        || queue.repeat_mode() != queue_repeat_mode(preferred_repeat);
    if queue.shuffle() != preferred_shuffle {
        if preferred_shuffle {
            set_queue_shuffle_with_latest_stats(database, &mut queue, true)?;
        } else {
            queue.set_shuffle(false);
        }
    }
    queue.set_repeat_mode(queue_repeat_mode(preferred_repeat));
    if queue_modes_changed {
        database
            .save_playback_session(&PlaybackSessionCheckpoint {
                queue: queue.snapshot(),
                position_ms: checkpoint.position_ms,
            })
            .map_err(|error| format!("保存播放模式失敗：{error}"))?;
    }
    let track_id = queue.current();
    let track = database
        .get_track_summary(track_id)
        .map_err(|error| error.to_string())?;
    let mut current_queue = service
        .queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *current_queue = Some(queue.clone());
    bump_queue_revision(service);
    *service
        .current_track
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = track.clone();
    *service
        .shuffle
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = queue.shuffle();
    *service
        .repeat_mode
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = match queue.repeat_mode() {
        QueueRepeatMode::Off => RepeatMode::Off,
        QueueRepeatMode::One => RepeatMode::One,
        QueueRepeatMode::All => RepeatMode::All,
    };
    if track.is_none() {
        *service
            .restore_warning
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some("已保留播放佇列，但目前曲目已不在曲庫中。".to_owned());
        return Ok(());
    }
    let path = match database.resolve_playable_filesystem_locator(track_id) {
        Ok(path) => path,
        Err(error) => {
            *service
                .restore_warning
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(format!("已保留播放狀態；目前來源暫不可用：{error}"));
            return Ok(());
        }
    };
    let accounting_id = service.accounting_id_for(track_id);
    service
        .player
        .request_load_accounted(path, accounting_id)
        .and_then(|ticket| ticket.wait(Duration::from_secs(2)))
        .map_err(|error| playback_audio_error_message(&error))?;
    // The audio backend duration is authoritative for seek clamping. Track tag
    // duration is only display fallback and may be stale or approximate.
    let restore_position = checkpoint.position_ms;
    let seek_succeeded = if restore_position == 0 {
        true
    } else {
        service
            .player
            .request_seek(Duration::from_millis(restore_position))
            .and_then(|ticket| ticket.wait(Duration::from_secs(2)))
            .is_ok()
    };
    if !seek_succeeded {
        *service
            .restore_warning
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(
            "已還原曲目與佇列，但此音訊無法跳回保存的位置；按播放可從目前位置開始。".to_owned(),
        );
    }
    *service
        .last_position_checkpoint
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Instant::now();
    Ok(())
}

#[cfg(target_os = "windows")]
fn play_track_from_database(
    database: &Database,
    service: &WindowsPlaybackService,
    track_id: &str,
    source: Option<PlaybackQueueSource>,
) -> Result<PreparedTrackChange, String> {
    let track_id = TrackId::parse(track_id).map_err(|error| format!("曲目 ID 無效：{error}"))?;
    let external_track = service
        .external_tracks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&track_id)
        .cloned();
    let (track, path) = if let Some(external_track) = external_track.as_ref() {
        (external_track.summary.clone(), external_track.path.clone())
    } else {
        let track = database
            .get_track_summary(track_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "曲目已不在曲庫中，請重新整理列表。".to_owned())?;
        let path = database
            .resolve_playable_filesystem_locator(track_id)
            .map_err(|error| error.to_string())?;
        (track, path)
    };

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
    let existing_queue = service
        .queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (queue_cursor, same_queue_slot) = match &source {
        Some(PlaybackQueueSource::Queue { traversal_position }) => {
            let queue = existing_queue
                .as_ref()
                .ok_or_else(|| "沒有播放佇列。".to_owned())?;
            let cursor = usize::try_from(*traversal_position)
                .map_err(|_| "播放佇列位置無效。".to_owned())?;
            let entry_track = queue
                .track_at_cursor(cursor)
                .ok_or_else(|| "播放佇列項目已不存在，請重新整理。".to_owned())?;
            if entry_track != track_id {
                return Err("播放佇列項目已變更，請重新整理。".to_owned());
            }
            let same_slot = queue.cursor() == cursor && current_track_id == Some(track_id);
            (Some(cursor), same_slot)
        }
        _ => (None, false),
    };
    let explicit_play = source.is_none();
    let preserve_queue = queue_cursor.is_some()
        || (!service.external_queue_active.load(Ordering::Acquire)
            && explicit_play
            && existing_queue
                .as_ref()
                .is_some_and(|queue| queue.current() == track_id));
    let replacement_queue = if preserve_queue {
        None
    } else {
        let repeat_mode = *service
            .repeat_mode
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let shuffle = *service
            .shuffle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temporary_database = service
            .temporary_catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let used_merged_search = service.external_queue_active.load(Ordering::Acquire)
            && matches!(&source, Some(PlaybackQueueSource::Library { .. }) | None);
        let mut queue = match (
            &source,
            service.external_queue_active.load(Ordering::Acquire),
        ) {
            (
                Some(PlaybackQueueSource::Library {
                    query,
                    query_alternates,
                    field_filter,
                    sort,
                }),
                true,
            ) => queue_for_merged_library_search(
                database,
                temporary_database.as_deref(),
                track_id,
                LibraryQueueSearch {
                    query: query.clone(),
                    query_alternates: query_alternates.clone(),
                    field_filter: field_filter.clone(),
                    sort: *sort,
                },
                shuffle,
            )?,
            (None, true) => queue_for_merged_library_search(
                database,
                temporary_database.as_deref(),
                track_id,
                LibraryQueueSearch {
                    query: None,
                    query_alternates: Vec::new(),
                    field_filter: None,
                    sort: TrackListSort::default(),
                },
                shuffle,
            )?,
            _ => queue_for_track(database, track_id, source)?,
        };
        queue.set_repeat_mode(queue_repeat_mode(repeat_mode));
        if !used_merged_search {
            set_queue_shuffle_with_latest_stats(database, &mut queue, shuffle)?;
        }
        Some(queue)
    };
    drop(existing_queue);
    let audio_snapshot = service.player.snapshot();
    let resume_loaded = (same_queue_slot
        && matches!(
            audio_snapshot.state,
            AudioPlaybackState::Ready | AudioPlaybackState::Paused | AudioPlaybackState::Playing
        ))
        || (explicit_play
            && current_track_id == Some(track_id)
            && matches!(
                audio_snapshot.state,
                AudioPlaybackState::Ready | AudioPlaybackState::Paused
            ));
    let ticket = if resume_loaded {
        service
            .player
            .request_play()
            .map_err(|error| playback_audio_error_message(&error))?
    } else if external_track.as_ref().is_some_and(|track| track.transient) {
        let accounting_id = service
            .temporary_catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .cloned()
            .map(|database| service.accounting_id_for_temporary(track_id, database));
        if let Some(accounting_id) = accounting_id {
            service
                .player
                .request_load_and_play_accounted(path, accounting_id)
                .map_err(|error| playback_audio_error_message(&error))?
        } else {
            service
                .player
                .request_load_and_play(path)
                .map_err(|error| playback_audio_error_message(&error))?
        }
    } else {
        // Library, playlist, and queue-row clicks start playback immediately.
        service
            .player
            .request_load_and_play_accounted(path, service.accounting_id_for(track_id))
            .map_err(|error| playback_audio_error_message(&error))?
    };
    Ok(PreparedTrackChange {
        ticket,
        track,
        replacement_queue,
        target_cursor: if same_queue_slot { None } else { queue_cursor },
    })
}

#[cfg(target_os = "windows")]
pub fn initialize_process_diagnostics() {
    let user_data_dir = std::env::current_exe().ok().and_then(|executable| {
        windows_data_directory::user_data_dir_for_executable(&executable).ok()
    });
    if let Some(user_data_dir) = user_data_dir {
        diagnostics::install_panic_hook(user_data_dir);
    }
}

#[cfg(target_os = "windows")]
pub fn diagnostics_json() -> Result<String, String> {
    diagnostics::report_json()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "windows")]
    let initial_audio_path = {
        let working_directory = std::env::current_dir().ok();
        first_supported_audio_argument(std::env::args_os().skip(1), working_directory.as_deref())
    };
    let builder = tauri::Builder::default();
    #[cfg(target_os = "windows")]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, arguments, cwd| {
        let working_directory = std::path::PathBuf::from(cwd);
        if let Some(path) = first_supported_audio_argument(
            arguments.into_iter().map(std::ffi::OsString::from),
            Some(&working_directory),
        ) {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
            dispatch_external_audio_open(app.clone(), path);
        }
    }));
    let builder = builder.plugin(tauri_plugin_media_index::init());
    #[cfg(target_os = "windows")]
    let builder = builder.plugin(tauri_plugin_dialog::init());

    builder
        .on_window_event(|window, event| {
            // A close request sent to the taskbar-lyrics window (tools that close "the process's main window" can
            // pick it, and Windows sends WM_CLOSE to every window at logoff) means the app should quit: forward it
            // to the main window's normal close flow instead of destroying only the lyrics window.
            #[cfg(target_os = "windows")]
            if window.label() == taskbar_lyrics::WINDOW_LABEL {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    if let Some(main) = window.app_handle().get_webview_window("main") {
                        let _ = main.close();
                    }
                }
            }
            #[cfg(target_os = "windows")]
            if window.label() == "main" {
                match event {
                    tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                        remember_main_window_normal_frame(window);
                    }
                    tauri::WindowEvent::Destroyed => {
                        taskbar_lyrics::shutdown(window.app_handle());
                    }
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        persist_main_window_geometry(window);
                        let app = window.app_handle().clone();
                        let state = app.state::<AppState>();
                        cancel_library_sync(&state);
                        let request = state.shutdown.request();
                        if request != database_lifecycle::ShutdownRequest::Ready {
                            api.prevent_close();
                            schedule_database_shutdown(app, request);
                        }
                    }
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            backup::backup_export,
            backup::backup_import,
            backup::backup_pending,
            backup::backup_remote_pending,
            backup::backup_remote_accounts_import,
            backup::backup_remote_accounts_discard,
            track_details::track_details_get,
            track_details::track_details_cover,
            track_details::track_details_save,
            track_details::track_details_pick_cover,
            track_details::track_details_read_image,
            track_details::track_details_export_cover,
            track_details::track_details_reveal,
            track_details::track_details_clipboard_has_image,
            track_details::track_details_read_clipboard_image,
            track_details::track_details_copy_cover,
            track_details::track_spectrogram,
            track_details::spectrogram_render_file,
            track_details::spectrogram_pick_file,
            track_details::spectrogram_window_open,
            updater::about_get_info,
            updater::update_get_state,
            updater::update_check,
            updater::update_download,
            updater::update_ignore,
            updater::app_open_external_url,
            settings_get_recovery_warning,
            settings_source_registry_authoritative,
            settings_confirm_source_registry,
            theme_get_preferences,
            theme_set_preferences,
            appearance_pick_main_background,
            appearance_pick_slideshow_folder,
            appearance_clear_main_background,
            appearance_get_main_background,
            appearance_next_slideshow_background,
            appearance_set_android_wallpaper_backdrop,
            android_all_files_access_status,
            android_request_all_files_access,
            settings_get_lyrics_preferences,
            settings_set_lyrics_preferences,
            settings_get_now_playing_appearance_preferences,
            settings_set_now_playing_appearance_preferences,
            settings_get_track_list_columns,
            settings_set_track_list_columns,
            settings_get_artist_list_sort,
            settings_set_artist_list_sort,
            settings_get_album_index_sort,
            settings_set_album_index_sort,
            settings_get_album_detail_sort,
            settings_set_album_detail_sort,
            settings_get_now_playing_layout,
            settings_set_now_playing_layout,
            settings_get_dsee_hx,
            settings_set_dsee_hx,
            settings_get_dsd,
            audio_list_output_devices,
            audio_output_device_status,
            audio_get_device_volumes,
            audio_set_device_volume,
            settings_get_output_devices,
            settings_set_output_devices,
            settings_get_output_exclusive,
            settings_get_output_volume_sync,
            settings_set_output_volume_sync,
            settings_set_output_exclusive,
            settings_get_output_presets,
            settings_save_output_preset,
            settings_clear_output_preset,
            settings_apply_output_preset,
            settings_get_language,
            settings_set_language,
            settings_get_taskbar_lyrics,
            settings_set_taskbar_lyrics,
            taskbar_lyrics_theme,
            settings_set_dsd,
            settings_get_cover_zoom,
            settings_set_cover_zoom,
            settings_get_shortcuts,
            settings_set_shortcuts,
            get_runtime_capabilities,
            library_get_page,
            library_get_track_position,
            playlist_get_track_position,
            library_create_artist_index_snapshot,
            library_get_artist_index_snapshot_page,
            library_resolve_artist_index_entry,
            library_resolve_artist_index_entry_by_name,
            library_get_quick_index_for_tracks,
            library_get_quick_index_for_artists,
            library_get_quick_index_for_album_tracks,
            playlist_get_quick_index_groupings,
            library_create_album_index_snapshot,
            library_get_album_index_snapshot_page,
            library_resolve_album_index_entry,
            library_resolve_album_index_entry_by_name,
            library_get_album_page,
            library_get_album_detail_facets,
            library_get_album_detail_artists,
            album_artwork_cache_get,
            album_artwork_cache_put,
            album_artwork_selection_ensure,
            album_artwork_cache_get_for_track,
            album_artwork_cache_put_for_track,
            album_artwork_cache_mark_missing_for_track,
            album_artwork_cache_clear,
            db_slow_locks,
            album_artwork_cache_stats,
            album_artwork_cache_set_limit,
            library_get_artist_page,
            playlist_list,
            playlist_get_page,
            playlist_import_m3u,
            playlist_export_m3u,
            favorites_get_playlists,
            favorites_set_membership,
            favorites_create_my_love,
            favorites_create_playlist,
            library_list_sources,
            library_add_windows_folder,
            library_pick_windows_folder,
            library_list_excluded_folders,
            library_pick_excluded_folder,
            library_remove_excluded_folder,
            library_set_source_enabled,
            library_remove_source,
            android_pick_folder_source,
            android_folder_browser::android_list_directories,
            remote_accounts::remote_accounts_list,
            remote_accounts::remote_account_save,
            remote_sources::remote_account_test,
            remote_sources::remote_smb_shares,
            remote_sources::remote_browse,
            remote_sources::remote_browse_playlists,
            remote_sources::remote_playlist_add,
            remote_sources::remote_cache_get,
            remote_sources::remote_cache_set_limit,
            remote_sources::remote_cache_clear,
            remote_sources::remote_excluded_list,
            remote_sources::remote_excluded_add,
            remote_sources::remote_excluded_remove,
            remote_sources::remote_browse_draft,
            remote_sources::remote_source_add,
            remote_accounts::remote_account_delete,
            remote_accounts::remote_accounts_export,
            remote_accounts::remote_accounts_import,
            android_folder_browser::android_folder_picker_result,
            library_sync,
            playback_get_snapshot,
            playback_refresh_current_track,
            playback_get_queue_page,
            playback_get_queue_quick_index_groupings,
            library_get_track_artwork,
            playback_play,
            playback_pause,
            sleep_timer::sleep_timer_get,
            sleep_timer::sleep_timer_set,
            playback_next,
            playback_previous,
            playback_seek,
            playback_set_volume,
            playback_set_repeat,
            playback_set_shuffle,
            playback_reorder_queue,
            playback_sort_queue,
            windows_register_audio_associations,
            windows_open_default_apps_settings,
            lyrics_service::lyrics_get_track,
            lyrics_service::lyrics_search,
            lyrics_service::lyrics_load_more,
            lyrics_service::lyrics_select_candidate,
            lyrics_service::lyrics_cancel_search,
            lyrics_service::lyrics_clear_track,
        ])
        .setup(move |app| {
            // Windows reports playback devices coming, going or becoming the default: the settings panel re-reads
            // its list from this event instead of polling.
            #[cfg(target_os = "windows")]
            {
                let handle = app.handle().clone();
                player_audio_windows::device_notify::set_device_change_sink(move || {
                    let _ = handle.emit("audio-devices-changed", ());
                    volume_sync::service::refresh();
                });
                player_audio_windows::device_notify::start_device_watch();
            }
            #[cfg(target_os = "windows")]
            let app_data_dir = windows_data_directory::user_data_dir_for_executable(
                &std::env::current_exe().map_err(|error| error.to_string())?,
            )
            .map_err(|error| format!("無法使用執行檔旁的 UserData 資料夾：{error}"))?;
            #[cfg(not(target_os = "windows"))]
            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| error.to_string())?;
            // A backup imported in the last run is put in place now, before the settings and the database are opened.
            match backup::apply_pending_import(&app_data_dir) {
                Ok(parts) if !parts.is_empty() => {
                    eprintln!("已套用匯入的備份：{}", parts.join("、"))
                }
                Ok(_) => {}
                Err(error) => eprintln!("無法套用匯入的備份，沿用目前的資料：{error}"),
            }
            let database_path = app_data_dir.join("moemusicplayer.sqlite3");
            let settings_path = database_path.with_file_name("settings.json");
            let database_result =
                Database::open_checked(&database_path).map_err(|error| error.to_string());
            let (database, stored_path, database_error) = match database_result {
                Ok(database) => (Some(database), Some(database_path.clone()), None),
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
                    // A playlist-file source is rebuilt with the playlist it feeds (kept in SQLite).
                    let playlist_ids = roots
                        .iter()
                        .filter(|root| root.kind == MediaSourceKind::PlaylistFile)
                        .filter_map(|root| {
                            database
                                .playlist_file_sync_state(root.id)
                                .ok()
                                .flatten()
                                .map(|state| (root.id, state.playlist_id))
                        })
                        .collect::<Vec<_>>();
                    AppSettings::from_legacy(theme, roots, &playlist_ids)
                        .map_err(|error| format!("轉換舊版設定失敗：{error}"))?
                } else {
                    AppSettings::default()
                }
            } else {
                AppSettings::default()
            };
            let settings = Arc::new(
                SettingsStore::open(&settings_path, legacy_settings)
                    .map_err(|error| format!("開啟使用者設定失敗：{error}"))?,
            );
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
            volume_sync::service::start(
                app.handle().clone(),
                Arc::clone(&settings),
                initial_settings.output_volume_sync,
            );
            #[cfg(target_os = "android")]
            let initial_settings = settings
                .snapshot()
                .map_err(|error| format!("讀取使用者設定失敗：{error}"))?;
            #[cfg(target_os = "android")]
            android_dsd::set_gain_db(initial_settings.dsd.gain_db);
            #[cfg(target_os = "windows")]
            let (playback, playback_error) = match PlayerHandle::with_output_options(
                // Sample-rate conversion is always the rubato FFT resampler (the setting was removed).
                AudioResamplingMode::HighQuality,
                initial_settings.dsee_hx,
                output_device_settings(&initial_settings.output_devices),
                initial_settings.output_exclusive,
            ) {
                Ok(player) => {
                    player.configure_dsd(player_audio_windows::DsdSettings {
                        gain_db: initial_settings.dsd.gain_db,
                        output_rate: initial_settings.dsd.output_rate_hz,
                        dop: initial_settings.dsd.dop,
                    });
                    let service = WindowsPlaybackService::with_modes(
                        player,
                        initial_settings.shuffle,
                        initial_settings.repeat_mode,
                    );
                    if let Some(database) = database.as_ref() {
                        if let Err(error) = restore_playback_session(database, &service) {
                            *service
                                .restore_warning
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                                Some(error.clone());
                            eprintln!("無法完整還原播放狀態，已保留檢查點：{error}");
                        } else {
                            resume_playback_if_marked(&settings, &service);
                        }
                        if let Err(error) = service.start_statistics_collector(&database_path) {
                            *service
                                .statistics_status
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                                Some(error.clone());
                            eprintln!("播放時長統計未啟動：{error}");
                        }
                    } else {
                        let reason = database_error
                            .clone()
                            .unwrap_or_else(|| "曲庫資料庫未通過啟動安全檢查。".to_owned());
                        *service
                            .statistics_status
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            Some(format!("曲庫資料庫未安全開啟，播放統計暫停：{reason}"));
                        eprintln!("播放時長統計未啟動：曲庫資料庫未安全開啟：{reason}");
                    }
                    (Some(service), None)
                }
                Err(error) => (None, Some(playback_audio_error_message(&error))),
            };
            #[cfg(target_os = "windows")]
            let (system_media, system_media_error) =
                create_windows_system_media_service(app.handle());
            #[cfg(target_os = "android")]
            let (android_playback, android_playback_error) = match stored_path.as_deref() {
                Some(path) => match android_playback::AndroidPlaybackService::start(
                    path,
                    initial_settings.shuffle,
                    initial_settings.repeat_mode,
                    android_playback::ResumeHook {
                        initial: initial_settings.resume_playback,
                        sink: Some({
                            let settings = Arc::clone(&settings);
                            Arc::new(move |playing| {
                                if let Err(error) = settings.update(|settings| {
                                    settings.resume_playback = playing;
                                    Ok(())
                                }) {
                                    eprintln!("無法記錄播放狀態：{error}");
                                }
                            })
                        }),
                    },
                ) {
                    Ok(service) => {
                        match app.handle().media_index().ensure_playback_service_started() {
                            Ok(()) => (Some(service), None),
                            Err(error) => (Some(service), Some(error.to_string())),
                        }
                    }
                    Err(error) => (None, Some(error)),
                },
                None => (
                    None,
                    Some(
                        database_error
                            .clone()
                            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned()),
                    ),
                ),
            };
            #[cfg(target_os = "windows")]
            let diagnostic_runtime_status = (
                database.is_some(),
                database_error.clone(),
                playback.is_some(),
                playback_error.clone(),
            );
            let remote_connections: Arc<remote_connections::RemoteConnections> = Arc::default();
            app.manage(updater::UpdateService::default());
            app.manage(sleep_timer::SleepTimer::default());
            // Every reader of remote files (engine, covers, lyrics) goes through this one provider.
            album_artwork_cache::set_limit_bytes(
                None,
                u64::from(initial_settings.cover_cache_limit_mb) * 1_000_000,
            );
            let remote_cache = settings.data_directory().and_then(|dir| {
                player_remote::cache::DiskCache::open(
                    dir.join("remote-cache"),
                    u64::from(initial_settings.remote_cache_limit_mb) * 1_000_000,
                )
                .ok()
                .map(Arc::new)
            });
            player_core::set_remote_content(Arc::new(remote_connections::RemoteFiles {
                connections: Arc::clone(&remote_connections),
                settings: Arc::clone(&settings),
                cache: remote_cache.clone(),
            }));
            app.manage(AppState {
                database,
                database_path: stored_path.clone(),
                database_error,
                database_work: database_lifecycle::DatabaseWorkGate::default(),
                queue_display_sort_cache: std::sync::Mutex::new(None),
                active_sync: std::sync::Mutex::new(None),
                #[cfg(target_os = "windows")]
                shutdown: database_lifecycle::ShutdownCoordinator::default(),
                settings,
                main_background: main_background::MainBackgroundRuntime::default(),
                #[cfg(target_os = "windows")]
                window_frame: std::sync::Mutex::new(WindowFrameMemory {
                    normal: initial_settings
                        .window_geometry
                        .as_ref()
                        .filter(|geometry| geometry.is_valid_restore_snapshot())
                        .map(|geometry| WindowGeometry {
                            maximized: false,
                            ..geometry.clone()
                        }),
                }),
                lyrics_service: lyrics_service::LyricsService::default(),
                remote_connections,
                remote_cache,
                #[cfg(target_os = "windows")]
                playback,
                #[cfg(target_os = "windows")]
                playback_error,
                #[cfg(target_os = "windows")]
                system_media,
                #[cfg(target_os = "windows")]
                system_media_error,
                #[cfg(target_os = "android")]
                android_playback,
                #[cfg(target_os = "android")]
                android_playback_error,
            });

            #[cfg(target_os = "windows")]
            if let Err(error) = diagnostics::write_runtime_snapshot(
                &app_data_dir,
                diagnostic_runtime_status.0,
                diagnostic_runtime_status.1.as_deref(),
                diagnostic_runtime_status.2,
                diagnostic_runtime_status.3.as_deref(),
            ) {
                eprintln!("無法寫入診斷執行狀態：{error}");
            }

            if let Some(window) = app.get_webview_window("main") {
                window.set_title(concat!("MoeMusicPlayer ", env!("MOE_BUILD_VERSION")))?;
                #[cfg(target_os = "windows")]
                window.set_always_on_top(false)?;
                #[cfg(target_os = "windows")]
                if let Ok(hwnd) = window.hwnd() {
                    window_icon::apply_embedded_taskbar_icon(hwnd);
                }
                // Window starts with visible:false so we can apply saved geometry before first paint.
                #[cfg(target_os = "windows")]
                if let Some(geometry) = initial_settings.window_geometry.clone() {
                    apply_saved_window_geometry(&window, geometry);
                }
                window.show()?;
                let _ = window.set_focus();
            }
            #[cfg(target_os = "windows")]
            if initial_settings.taskbar_lyrics.enabled {
                let handle = app.handle().clone();
                let taskbar_lyrics = initial_settings.taskbar_lyrics.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = taskbar_lyrics::apply(&handle, &taskbar_lyrics).await {
                        eprintln!("{error}");
                    }
                });
            }
            #[cfg(target_os = "windows")]
            if let Some(path) = initial_audio_path {
                dispatch_external_audio_open(app.handle().clone(), path);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("MoeMusicPlayer could not start")
        .run(|app_handle, event| {
            #[cfg(target_os = "windows")]
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                let state = app_handle.state::<AppState>();
                cancel_library_sync(&state);
                let request = state.shutdown.request();
                if request != database_lifecycle::ShutdownRequest::Ready {
                    api.prevent_exit();
                    schedule_database_shutdown(app_handle.clone(), request);
                }
            }
            #[cfg(not(target_os = "windows"))]
            let _ = (app_handle, event);
        });
}

#[cfg(all(test, target_os = "windows"))]
mod windows_library_integration_tests {
    use super::{
        add_windows_folder_path_to_database, add_windows_folder_to_database, sync_windows_sources,
    };
    use super::{
        apply_system_media_event, build_playback_queue_page, checkpoint_playback_position,
        commit_track_change, first_supported_audio_argument, lyrics_database_from_databases,
        merged_library_page, navigate_queue, pause_playback, play_track_from_database,
        playback_audio_error_message, queue_for_track, restore_playback_session,
        same_folder_audio_files, save_playback_session, set_playback_volume,
        set_queue_shuffle_with_latest_stats, snapshot_playback_queue, sort_queue_entry_order,
        PlaybackQueueSource, PreparedTrackChange, TrackSummary, WindowsPlaybackService,
    };
    use player_audio_windows::{
        system_media::{SystemMediaController, SystemMediaEvent},
        AudioBackend, AudioError, PlaybackState as AudioPlaybackState, PlayerHandle,
    };
    use player_core::{
        FileFingerprint, LibraryRepository, ListTracksQuery, MediaLocator, MediaSourceKind,
        MediaTrackRecord, PlaybackQueue, PlaybackQueueContext, PlaybackQueueEntry, Playlist,
        PlaylistEntry, QueueRepeatMode, SourceId, SourceScanState, TrackField, TrackFieldFilter,
        TrackId, TrackIdentity, TrackListSort, TrackMetadata, TrackSortDirection, TrackSortField,
    };
    use player_db::{Database, PlaybackSessionCheckpoint};
    use std::sync::atomic::Ordering;
    use std::{
        collections::HashMap,
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
    fn shell_open_sorts_supported_sibling_files_by_natural_filename_order() {
        let directory = TestDirectory::new();
        fs::create_dir_all(directory.0.join("subfolder")).unwrap();
        fs::write(directory.0.join("z-last.mp3"), b"audio").unwrap();
        fs::write(directory.0.join("a-first.flac"), b"audio").unwrap();
        fs::write(directory.0.join("middle.m4a"), b"audio").unwrap();
        fs::write(directory.0.join("movie.mp4"), b"video").unwrap();
        fs::write(directory.0.join("subfolder\u{005c}nested.wav"), b"audio").unwrap();
        fs::write(directory.0.join("AUDIO_Ex_Dorm_CN_1.flac"), b"audio").unwrap();
        fs::write(directory.0.join("AUDIO_Ex_Dorm_CN_2.flac"), b"audio").unwrap();
        fs::write(directory.0.join("AUDIO_Ex_Dorm_CN_10.flac"), b"audio").unwrap();
        fs::write(directory.0.join("AUDIO_Ex_Dorm_CN_100.flac"), b"audio").unwrap();

        let files = same_folder_audio_files(&directory.0.join("middle.m4a")).unwrap();
        let names = files
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "a-first.flac",
                "AUDIO_Ex_Dorm_CN_1.flac",
                "AUDIO_Ex_Dorm_CN_2.flac",
                "AUDIO_Ex_Dorm_CN_10.flac",
                "AUDIO_Ex_Dorm_CN_100.flac",
                "middle.m4a",
                "z-last.mp3"
            ]
        );
    }

    #[test]
    fn shell_open_argument_resolves_relative_paths_against_the_forwarded_working_directory() {
        let directory = TestDirectory::new();
        let path = first_supported_audio_argument(
            [std::ffi::OsString::from(r"Music\picked song.m4a")],
            Some(&directory.0),
        )
        .unwrap();
        assert_eq!(path, directory.0.join(r"Music\picked song.m4a"));
        assert!(first_supported_audio_argument(
            [std::ffi::OsString::from("video.mp4")],
            Some(&directory.0),
        )
        .is_none());
    }

    #[test]
    fn external_open_queue_never_replaces_or_checkpoints_the_saved_session() {
        let database = Database::open_in_memory().unwrap();
        let player = PlayerHandle::with_backend_factory(|| {
            Ok(Box::new(CapturingAudioBackend {
                loaded_path: Arc::new(Mutex::new(None)),
                position: Duration::ZERO,
            }) as Box<dyn AudioBackend>)
        })
        .unwrap();
        let service = WindowsPlaybackService::new(player);
        let queue = PlaybackQueue::with_entries(
            vec![PlaybackQueueEntry {
                track_id: TrackId::new(),
                source_position: None,
            }],
            PlaybackQueueContext::Library {
                query: None,
                field_filter: None,
                sort: Default::default(),
            },
            0,
        )
        .unwrap();
        *service.queue.lock().unwrap() = Some(queue);
        service.external_queue_active.store(true, Ordering::Release);

        save_playback_session(&database, &service).unwrap();
        checkpoint_playback_position(&database, &service, true).unwrap();

        assert!(database.load_playback_session().unwrap().is_none());
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
    fn authoritative_orphan_cleanup_recovers_playlist_when_sync_state_already_cleared() {
        let temporary = TestDirectory::new();
        let media_folder = temporary.0.join("曲目");
        fs::create_dir_all(&media_folder).expect("create media folder");
        let media_path = media_folder.join("保留曲目.mp3");
        write_tagged_mp3(&media_path);
        let playlist_path = temporary.0.join("hanser hr.m3u8");
        fs::write(
            &playlist_path,
            format!("#EXTM3U\n{}\n", media_path.display()),
        )
        .expect("write playlist source");
        let playlist_id = super::PlaylistId::new();
        let source = super::SourceEntry {
            id: SourceId::new(),
            display_name: "hanser hr".to_owned(),
            enabled: true,
            kind: super::SourceEntryKind::PlaylistFile {
                playlist_id,
                path: super::settings::StoredPath::from_path(&playlist_path)
                    .expect("store native playlist path"),
                tree_uri: None,
                remote_account: None,
            },
        };
        let mut database = Database::open_in_memory().expect("database");
        super::playlist_source_sync::sync_playlist_file_source(
            &mut database,
            &source,
            &[],
            1_800_000_000_000,
        )
        .expect("create playlist projection")
        .expect("playlist source is readable");
        // Older remove path cleared sync_state / mappings but left the sidebar playlist.
        database
            .remove_playlist_file_source(source.id)
            .expect("simulate prior incomplete source removal");
        assert!(database
            .playlist_file_sync_state(source.id)
            .expect("read cleared sync state")
            .is_none());
        assert!(database
            .get_playlist(playlist_id)
            .expect("stale playlist remains")
            .is_some());

        super::sync_windows_configured_sources(&mut database, &[], &[], true, &mut |_, _, _, _| {})
            .expect("name-fallback orphan cleanup");
        assert!(database
            .get_playlist(playlist_id)
            .expect("stale sidebar playlist removed")
            .is_none());
    }

    #[test]
    fn authoritative_orphan_playlist_cleanup_deletes_sidebar_playlist_but_keeps_tracks() {
        let temporary = TestDirectory::new();
        let media_folder = temporary.0.join("曲目");
        fs::create_dir_all(&media_folder).expect("create media folder");
        let media_path = media_folder.join("保留曲目.mp3");
        write_tagged_mp3(&media_path);
        let playlist_path = temporary.0.join("側欄清單.m3u8");
        fs::write(
            &playlist_path,
            format!("#EXTM3U\n{}\n", media_path.display()),
        )
        .expect("write playlist source");
        let playlist_id = super::PlaylistId::new();
        let source = super::SourceEntry {
            id: SourceId::new(),
            display_name: "側欄清單".to_owned(),
            enabled: true,
            kind: super::SourceEntryKind::PlaylistFile {
                playlist_id,
                path: super::settings::StoredPath::from_path(&playlist_path)
                    .expect("store native playlist path"),
                tree_uri: None,
                remote_account: None,
            },
        };
        let mut database = Database::open_in_memory().expect("database");
        super::playlist_source_sync::sync_playlist_file_source(
            &mut database,
            &source,
            &[],
            1_800_000_000_000,
        )
        .expect("create playlist projection")
        .expect("playlist source is readable");
        assert!(database
            .get_playlist(playlist_id)
            .expect("read playlist")
            .is_some());
        assert_eq!(database.count_tracks(None).expect("track count"), 1);
        let track_id = database
            .resolve_track_id_for_locator(&player_core::MediaLocator::FileSystem(
                media_path.clone(),
            ))
            .expect("resolve track")
            .expect("track exists after playlist sync");

        let result = super::sync_windows_configured_sources(
            &mut database,
            &[],
            &[],
            true,
            &mut |_, _, _, _| {},
        )
        .expect("authoritative empty registry cleans orphans");
        assert!(result.sources.is_empty());
        assert!(database
            .get_playlist(playlist_id)
            .expect("playlist removed from sidebar")
            .is_none());
        // Library pages only count mapped tracks; the Track row itself must remain.
        assert_eq!(
            database
                .count_tracks(None)
                .expect("unmapped tracks leave library"),
            0
        );
        assert!(database
            .get_track_summary(track_id)
            .expect("read preserved track row")
            .is_some());
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
                tree_uri: None,
                remote_account: None,
            },
        };
        let mut database = Database::open_in_memory().expect("database");
        super::playlist_source_sync::sync_playlist_file_source(
            &mut database,
            &source,
            &[],
            1_800_000_000_000,
        )
        .expect("create an existing playlist projection")
        .expect("playlist source is readable");
        assert_eq!(
            database.count_tracks(None).expect("initial track mapping"),
            1
        );

        let result = super::sync_windows_configured_sources(
            &mut database,
            &[],
            &[],
            false,
            &mut |_, _, _, _| {},
        );
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

    struct ClockedAudioBackend {
        loaded: bool,
        playing: bool,
        position: Duration,
        started_at: Option<Instant>,
    }

    impl ClockedAudioBackend {
        fn current_position(&self) -> Duration {
            if self.playing {
                self.position.saturating_add(
                    self.started_at
                        .map(|started| started.elapsed())
                        .unwrap_or_default(),
                )
            } else {
                self.position
            }
        }
    }

    impl AudioBackend for ClockedAudioBackend {
        fn load(&mut self, _path: &Path) -> Result<Option<Duration>, AudioError> {
            self.loaded = true;
            self.playing = false;
            self.position = Duration::ZERO;
            self.started_at = None;
            Ok(Some(Duration::from_secs(60)))
        }

        fn play(&mut self) -> Result<(), AudioError> {
            if !self.loaded {
                return Err(AudioError::NoTrackLoaded);
            }
            if !self.playing {
                self.playing = true;
                self.started_at = Some(Instant::now());
            }
            Ok(())
        }

        fn pause(&mut self) -> Result<(), AudioError> {
            self.position = self.current_position();
            self.playing = false;
            self.started_at = None;
            Ok(())
        }

        fn stop(&mut self) -> Result<(), AudioError> {
            self.playing = false;
            self.position = Duration::ZERO;
            self.started_at = None;
            Ok(())
        }

        fn seek(&mut self, position: Duration) -> Result<Duration, AudioError> {
            self.position = position.min(Duration::from_secs(60));
            self.started_at = self.playing.then(Instant::now);
            Ok(self.position)
        }

        fn set_volume(&mut self, _volume: f32) -> Result<(), AudioError> {
            Ok(())
        }

        fn position(&self) -> Duration {
            self.current_position()
        }

        fn is_empty(&self) -> bool {
            !self.loaded
        }
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
                    field_filter: None,
                    offset: 0,
                    limit: 25,
                    sort: Default::default(),
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
                    field_filter: None,
                    offset: 0,
                    limit: 25,
                    sort: Default::default(),
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
                field_filter: None,
                offset: 0,
                limit: 25,
                sort: Default::default(),
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
    fn library_queue_uses_the_exact_field_filtered_page_set_and_persists_its_context() {
        let mut database = Database::open_in_memory().expect("isolated database");
        let root = database
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                "library filter fixture",
                MediaLocator::FileSystem(PathBuf::from(r"C:\Music\filter-fixture")),
            )
            .expect("add fixture root");
        let fixture = |item: &str, title: &str, artist: &str| MediaTrackRecord {
            identity: TrackIdentity {
                source_id: root.id,
                source_item_id: item.to_owned(),
                locator_key: Some(format!("fixture:{item}")),
            },
            locator: MediaLocator::FileSystem(PathBuf::from(format!(r"C:\Music\{item}.flac"))),
            fingerprint: FileFingerprint {
                size_bytes: 1,
                modified_at_utc_ms: Some(1),
                created_at_utc_ms: None,
            },
            metadata: Some(TrackMetadata {
                title: Some(title.to_owned()),
                artist: Some(artist.to_owned()),
                album: Some("Album".to_owned()),
                ..TrackMetadata::default()
            }),
        };
        let records = vec![
            fixture("first", "Needle One", "hanser"),
            fixture("second", "Needle Two", "HANSER"),
            fixture("excluded", "Needle Three", "other artist"),
        ];
        database
            .apply_source_scan(
                &root,
                &SourceScanState::Complete,
                &records,
                &records,
                &[],
                1,
            )
            .expect("persist fixture tracks");
        let filter = TrackFieldFilter {
            field: TrackField::Artist,
            value: "hanser|HANSER".to_owned(),
        };
        let page = database
            .list_tracks_page(ListTracksQuery {
                query: Some("needle".to_owned()),
                field_filter: Some(filter.clone()),
                offset: 0,
                limit: 10,
                sort: Default::default(),
            })
            .expect("read exact-filtered page");
        assert_eq!(page.total_count, 2);
        let selected_track = page.items[0].id;
        let queue = queue_for_track(
            &database,
            selected_track,
            Some(PlaybackQueueSource::Library {
                query: Some("needle".to_owned()),
                query_alternates: Vec::new(),
                field_filter: Some(filter.clone()),
                sort: Default::default(),
            }),
        )
        .expect("construct library queue from same filter");
        assert_eq!(queue.current(), selected_track);
        assert_eq!(queue.snapshot().entries.len(), page.total_count as usize);
        assert_eq!(
            queue.snapshot().context,
            PlaybackQueueContext::Library {
                query: Some("needle".to_owned()),
                field_filter: Some(filter),
                sort: Default::default(),
            }
        );
    }

    #[test]
    fn explorer_catalog_search_merges_naturally_paginates_and_deduplicates_paths() {
        let directory = TestDirectory::new();
        fs::create_dir_all(&directory.0).unwrap();
        let duplicate_path = directory.0.join("duplicate.flac");
        let first_path = directory.0.join("Song 1.flac");
        let second_path = directory.0.join("Song 2.flac");
        let last_path = directory.0.join("Song 10.flac");
        for path in [&duplicate_path, &first_path, &second_path, &last_path] {
            fs::write(path, b"fixture").unwrap();
        }

        let mut library = Database::open_in_memory().unwrap();
        let library_root = library
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                "persistent",
                MediaLocator::FileSystem(directory.0.clone()),
            )
            .unwrap();
        let mut temporary = Database::open_in_memory().unwrap();
        let temporary_root = temporary
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                "temporary",
                MediaLocator::FileSystem(directory.0.clone()),
            )
            .unwrap();
        let record = |root: SourceId, item: &str, title: &str, path: &Path| MediaTrackRecord {
            identity: TrackIdentity {
                source_id: root,
                source_item_id: item.to_owned(),
                locator_key: Some(item.to_owned()),
            },
            locator: MediaLocator::FileSystem(path.to_path_buf()),
            fingerprint: FileFingerprint {
                size_bytes: 1,
                modified_at_utc_ms: Some(1),
                created_at_utc_ms: None,
            },
            metadata: Some(TrackMetadata {
                title: Some(title.to_owned()),
                ..TrackMetadata::default()
            }),
        };
        let persistent_records = vec![
            record(library_root.id, "first", "Song 1", &first_path),
            record(library_root.id, "duplicate", "Duplicate", &duplicate_path),
            record(library_root.id, "last", "Song 10", &last_path),
        ];
        library
            .apply_source_scan(
                &library_root,
                &SourceScanState::Complete,
                &persistent_records,
                &persistent_records,
                &[],
                1,
            )
            .unwrap();
        let transient_records = vec![
            record(temporary_root.id, "middle", "Song 2", &second_path),
            record(
                temporary_root.id,
                "duplicate-again",
                "Duplicate",
                &duplicate_path,
            ),
        ];
        temporary
            .apply_source_scan(
                &temporary_root,
                &SourceScanState::Complete,
                &transient_records,
                &transient_records,
                &[],
                1,
            )
            .unwrap();

        let first_page = merged_library_page(
            &library,
            &temporary,
            ListTracksQuery {
                offset: 0,
                limit: 2,
                sort: TrackListSort::default(),
                ..Default::default()
            },
            &[],
        )
        .unwrap();
        assert_eq!(first_page.total_count, 4);
        assert_eq!(
            first_page
                .items
                .iter()
                .map(|track| track.title.as_deref().unwrap())
                .collect::<Vec<_>>(),
            vec!["Duplicate", "Song 1"]
        );
        let second_page = merged_library_page(
            &library,
            &temporary,
            ListTracksQuery {
                offset: 2,
                limit: 2,
                sort: TrackListSort::default(),
                ..Default::default()
            },
            &[],
        )
        .unwrap();
        assert_eq!(
            second_page
                .items
                .iter()
                .map(|track| track.title.as_deref().unwrap())
                .collect::<Vec<_>>(),
            vec!["Song 2", "Song 10"]
        );
    }

    #[test]
    fn lyrics_database_router_selects_memory_catalog_for_transient_track_id() {
        let directory = TestDirectory::new();
        fs::create_dir_all(&directory.0).unwrap();
        let audio_path = directory.0.join("transient.mp3");
        fs::write(&audio_path, b"fixture").unwrap();
        let library = Database::open_in_memory().unwrap();
        let mut temporary = Database::open_in_memory().unwrap();
        let root = temporary
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                "temporary lyrics fixture",
                MediaLocator::FileSystem(directory.0.clone()),
            )
            .unwrap();
        let record = MediaTrackRecord {
            identity: TrackIdentity {
                source_id: root.id,
                source_item_id: "transient.mp3".to_owned(),
                locator_key: None,
            },
            locator: MediaLocator::FileSystem(audio_path.clone()),
            fingerprint: FileFingerprint {
                size_bytes: 1,
                modified_at_utc_ms: None,
                created_at_utc_ms: None,
            },
            metadata: Some(TrackMetadata {
                title: Some("Transient lyrics target".to_owned()),
                ..TrackMetadata::default()
            }),
        };
        temporary
            .apply_source_scan(
                &root,
                &SourceScanState::Complete,
                std::slice::from_ref(&record),
                std::slice::from_ref(&record),
                &[],
                1,
            )
            .unwrap();
        let track_id = temporary
            .resolve_track_id_for_locator(&MediaLocator::FileSystem(audio_path))
            .unwrap()
            .unwrap();
        let selected = lyrics_database_from_databases(
            Some(&library),
            Some(Arc::new(temporary)),
            track_id,
            None,
        )
        .unwrap();
        assert_eq!(
            selected
                .as_database()
                .get_track_summary(track_id)
                .unwrap()
                .unwrap()
                .title
                .as_deref(),
            Some("Transient lyrics target")
        );
        assert!(library.get_track_summary(track_id).unwrap().is_none());
    }

    #[test]
    fn playback_queue_page_preserves_traversal_and_does_not_mutate_playback_or_session() {
        let database = Database::open_in_memory().expect("open isolated database");
        let first_track = TrackId::new();
        let second_track = TrackId::new();
        let queue = PlaybackQueue::with_entries(
            vec![
                PlaybackQueueEntry {
                    track_id: first_track,
                    source_position: Some(0),
                },
                PlaybackQueueEntry {
                    track_id: second_track,
                    source_position: Some(1),
                },
                PlaybackQueueEntry {
                    track_id: first_track,
                    source_position: Some(2),
                },
            ],
            PlaybackQueueContext::Playlist {
                playlist_id: "read-only-page-test".to_owned(),
            },
            1,
        )
        .expect("create duplicate queue");
        let mut queue_snapshot = queue.snapshot();
        queue_snapshot.play_order = vec![2, 0, 1];
        queue_snapshot.cursor = 1;
        let queue = PlaybackQueue::restore(queue_snapshot.clone()).expect("restore traversal");
        let player = PlayerHandle::with_backend_factory(|| {
            Ok(Box::new(CapturingAudioBackend {
                loaded_path: Arc::new(Mutex::new(None)),
                position: Duration::ZERO,
            }) as Box<dyn AudioBackend>)
        })
        .expect("create silent test backend");
        let initialization_deadline = Instant::now() + Duration::from_secs(2);
        while player.snapshot().state == AudioPlaybackState::Initializing {
            assert!(
                Instant::now() < initialization_deadline,
                "silent playback worker did not finish initialization"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(player.snapshot().state, AudioPlaybackState::Empty);
        let service = WindowsPlaybackService::new(player);
        *service.queue.lock().unwrap() = Some(queue);
        service
            .queue_revision
            .store(7, std::sync::atomic::Ordering::Relaxed);
        *service.current_track.lock().unwrap() = Some(TrackSummary {
            id: first_track,
            title: Some("Current Track".to_owned()),
            artist: None,
            album: None,
            album_artist: None,
            track_number: None,
            disc_number: None,
            duration_ms: Some(9_000),
            codec: None,
            bitrate_bps: None,
            sample_rate_hz: None,
            release_date: None,
            year: None,
            created_at_utc_ms: None,
            bit_depth: None,
            played_ms: 0,
            file_name: None,
            source_name: None,
        });
        let before_playback = serde_json::to_value(service.snapshot())
            .expect("serialize playback snapshot before query");
        let before_audio = service.player.snapshot();
        database
            .save_playback_session(&PlaybackSessionCheckpoint {
                queue: queue_snapshot.clone(),
                position_ms: 4_321,
            })
            .expect("save baseline session");
        let before_session = database
            .load_playback_session()
            .expect("read baseline session");

        let (revision, queue) = snapshot_playback_queue(&service);
        let first_page = build_playback_queue_page(Some(&database), revision, queue, 0, 2)
            .expect("read first queue page");
        assert_eq!(first_page.revision, 7);
        assert_eq!(first_page.total, 3);
        assert_eq!(first_page.offset, 0);
        assert_eq!(first_page.cursor, Some(1));
        assert_eq!(first_page.current_entry_position, Some(0));
        assert_eq!(first_page.items.len(), 2);
        assert_eq!(first_page.items[0].traversal_position, 0);
        assert_eq!(first_page.items[0].entry_position, 2);
        assert_eq!(first_page.items[0].source_position, Some(2));
        assert_eq!(first_page.items[0].track_id, first_track);
        assert!(!first_page.items[0].is_current);
        assert!(first_page.items[0].track.is_none());
        assert_eq!(first_page.items[1].traversal_position, 1);
        assert_eq!(first_page.items[1].entry_position, 0);
        assert_eq!(first_page.items[1].track_id, first_track);
        assert!(first_page.items[1].is_current);

        let (revision, queue) = snapshot_playback_queue(&service);
        let second_page = build_playback_queue_page(Some(&database), revision, queue, 2, 2)
            .expect("read final queue page");
        assert_eq!(second_page.total, 3);
        assert_eq!(second_page.offset, 2);
        assert_eq!(second_page.items.len(), 1);
        assert_eq!(second_page.items[0].entry_position, 1);
        assert_eq!(second_page.items[0].track_id, second_track);
        assert!(second_page.items[0].track.is_none());

        let (revision, queue) = snapshot_playback_queue(&service);
        let past_end = build_playback_queue_page(Some(&database), revision, queue, 3, 2)
            .expect("read page beyond queue end");
        assert!(past_end.items.is_empty());
        assert_eq!(past_end.total, 3);
        let empty = build_playback_queue_page(None, 8, None, 0, 2)
            .expect("empty queue remains readable without database");
        assert_eq!(empty.total, 0);
        assert_eq!(empty.cursor, None);
        assert_eq!(empty.current_entry_position, None);
        assert!(empty.items.is_empty());

        let after_playback = serde_json::to_value(service.snapshot())
            .expect("serialize playback snapshot after query");
        assert_eq!(after_playback, before_playback);
        assert_eq!(service.player.snapshot().state, before_audio.state);
        assert_eq!(service.player.snapshot().position, before_audio.position);
        assert_eq!(
            database
                .load_playback_session()
                .expect("read session after query"),
            before_session,
            "a queue page query must not alter the persisted session"
        );
    }

    #[test]
    fn queue_column_sort_changes_playback_order_and_keeps_current_duplicate_slot() {
        let first_track = TrackId::new();
        let second_track = TrackId::new();
        let mut queue = PlaybackQueue::with_entries(
            vec![
                PlaybackQueueEntry {
                    track_id: first_track,
                    source_position: Some(4),
                },
                PlaybackQueueEntry {
                    track_id: second_track,
                    source_position: Some(8),
                },
                PlaybackQueueEntry {
                    track_id: first_track,
                    source_position: Some(9),
                },
            ],
            PlaybackQueueContext::Library {
                query: None,
                field_filter: None,
                sort: TrackListSort::default(),
            },
            2,
        )
        .expect("create queue with duplicate track");
        let snapshot = queue.snapshot();
        let make_summary =
            |id: TrackId, title: &str, created_at_utc_ms: Option<i64>| TrackSummary {
                id,
                title: Some(title.to_owned()),
                artist: None,
                album: None,
                album_artist: None,
                track_number: None,
                disc_number: None,
                duration_ms: Some(10_000),
                codec: None,
                bitrate_bps: None,
                sample_rate_hz: None,
                release_date: None,
                year: None,
                created_at_utc_ms,
                bit_depth: None,
                played_ms: 0,
                file_name: None,
                source_name: None,
            };
        let summaries = HashMap::from([
            (first_track, make_summary(first_track, "Zebra", Some(200))),
            (second_track, make_summary(second_track, "Alpha", Some(100))),
        ]);

        let ascending = sort_queue_entry_order(
            &snapshot,
            &summaries,
            TrackListSort {
                field: TrackSortField::Title,
                direction: TrackSortDirection::Asc,
            },
            None,
        );
        assert_eq!(ascending, [1, 0, 2]);
        queue
            .set_traversal_order(
                ascending,
                TrackListSort {
                    field: TrackSortField::Title,
                    direction: TrackSortDirection::Asc,
                },
            )
            .expect("apply sorted queue order");
        assert_eq!(queue.snapshot().play_order, [1, 0, 2]);
        assert_eq!(queue.snapshot().cursor, 2);
        assert_eq!(queue.current_entry_index(), 2);
        assert!(!queue.shuffle(), "manual sort disables shuffle mode");

        let descending = sort_queue_entry_order(
            &queue.snapshot(),
            &summaries,
            TrackListSort {
                field: TrackSortField::CreatedAt,
                direction: TrackSortDirection::Desc,
            },
            None,
        );
        assert_eq!(descending, [0, 2, 1]);
        queue
            .set_traversal_order(
                descending,
                TrackListSort {
                    field: TrackSortField::CreatedAt,
                    direction: TrackSortDirection::Desc,
                },
            )
            .expect("apply reverse sort");
        assert_eq!(queue.snapshot().play_order, [0, 2, 1]);
        assert_eq!(queue.current_entry_index(), 2);
        assert_eq!(queue.snapshot().entries[0].source_position, Some(4));
        assert_eq!(queue.snapshot().entries[2].source_position, Some(9));
        queue.set_shuffle(true);
        assert!(
            queue.shuffle(),
            "the shuffle button may create a fresh traversal afterward"
        );
    }

    #[test]
    fn queue_unknown_text_values_reverse_with_text_sort_direction() {
        let ids = [
            TrackId::new(),
            TrackId::new(),
            TrackId::new(),
            TrackId::new(),
        ];
        let queue = PlaybackQueue::with_entries(
            ids.iter()
                .copied()
                .map(|track_id| PlaybackQueueEntry {
                    track_id,
                    source_position: None,
                })
                .collect(),
            PlaybackQueueContext::Library {
                query: None,
                field_filter: None,
                sort: TrackListSort::default(),
            },
            0,
        )
        .expect("create queue");
        let make_summary = |id: TrackId, artist: Option<&str>| TrackSummary {
            id,
            title: Some("Track".to_owned()),
            artist: artist.map(str::to_owned),
            album: None,
            album_artist: None,
            track_number: None,
            disc_number: None,
            duration_ms: Some(10_000),
            codec: None,
            bitrate_bps: None,
            sample_rate_hz: None,
            release_date: None,
            year: None,
            created_at_utc_ms: None,
            bit_depth: None,
            played_ms: 0,
            file_name: None,
            source_name: None,
        };
        let summaries = HashMap::from([
            (ids[0], make_summary(ids[0], None)),
            (ids[1], make_summary(ids[1], Some("Alpha"))),
            (ids[2], make_summary(ids[2], Some("Zulu"))),
            (ids[3], make_summary(ids[3], Some("  \t"))),
        ]);

        for (direction, named, unknown_first) in [
            (TrackSortDirection::Asc, [1, 2], false),
            (TrackSortDirection::Desc, [2, 1], true),
        ] {
            let sorted = sort_queue_entry_order(
                &queue.snapshot(),
                &summaries,
                TrackListSort {
                    field: TrackSortField::Artist,
                    direction,
                },
                None,
            );
            assert_eq!(sorted.len(), 4);
            if unknown_first {
                assert!(sorted[..2].iter().all(|index| matches!(*index, 0 | 3)));
                assert_eq!([sorted[2], sorted[3]], named);
            } else {
                assert_eq!([sorted[0], sorted[1]], named);
                assert!(sorted[2..].iter().all(|index| matches!(*index, 0 | 3)));
            }
        }
    }

    #[test]
    fn playback_statistics_flush_on_pause_reopen_and_feed_new_shuffle_generation() {
        let temporary = TestDirectory::new();
        fs::create_dir_all(&temporary.0).expect("create isolated test directory");
        let database_path = temporary.0.join("statistics.sqlite3");
        let database = Database::open(&database_path).expect("open isolated statistics database");
        let track_id = TrackId::new();
        let player = PlayerHandle::with_backend_factory(|| {
            Ok(Box::new(ClockedAudioBackend {
                loaded: false,
                playing: false,
                position: Duration::ZERO,
                started_at: None,
            }) as Box<dyn AudioBackend>)
        })
        .expect("create clocked audio backend");
        let ready_deadline = Instant::now() + Duration::from_secs(2);
        while player.snapshot().state == AudioPlaybackState::Initializing {
            assert!(
                Instant::now() < ready_deadline,
                "audio actor did not initialize"
            );
            thread::sleep(Duration::from_millis(2));
        }
        let service = WindowsPlaybackService::with_modes(player, false, super::RepeatMode::Off);
        service
            .start_statistics_collector(&database_path)
            .expect("start isolated statistics collector");

        let accounting_id = service.accounting_id_for(track_id);
        service
            .player
            .request_load_and_play_accounted("synthetic-track.wav", accounting_id)
            .expect("enqueue tracked synthetic playback")
            .wait(Duration::from_secs(2))
            .expect("tracked playback starts");
        thread::sleep(Duration::from_millis(180));
        service
            .player
            .request_pause()
            .expect("enqueue pause")
            .wait(Duration::from_secs(2))
            .expect("pause and sample actor tail");

        let flush_deadline = Instant::now() + Duration::from_secs(3);
        let stats = loop {
            let stats = database
                .playback_statistics_for(&[track_id])
                .expect("query isolated stats");
            if stats[&track_id].played_ms > 0 {
                break stats[&track_id];
            }
            assert!(
                Instant::now() < flush_deadline,
                "pause boundary did not flush stats"
            );
            thread::sleep(Duration::from_millis(10));
        };
        assert!(stats.played_ms >= 80, "unexpected credited time: {stats:?}");
        assert_eq!(stats.duration_ms, Some(60_000));

        let second_track = TrackId::new();
        let mut queue = PlaybackQueue::with_seed(vec![track_id, second_track, track_id], 0, 17)
            .expect("make a queue with a duplicate playlist slot");
        set_queue_shuffle_with_latest_stats(&database, &mut queue, true)
            .expect("build weighted shuffle from persisted stats");
        let shuffled = queue.snapshot();
        assert!(shuffled.shuffle);
        assert_eq!(shuffled.play_order.first().copied(), Some(0));
        let mut permutation = shuffled.play_order.clone();
        permutation.sort_unstable();
        assert_eq!(permutation, vec![0, 1, 2]);
        assert_eq!(shuffled.entries[0].track_id, shuffled.entries[2].track_id);

        service.shutdown_statistics_collector();
        drop(database);
        let reopened = Database::open(&database_path).expect("reopen isolated statistics DB");
        assert_eq!(
            reopened.playback_statistics_for(&[track_id]).unwrap()[&track_id],
            stats,
            "runtime retries/close must not double-count flushed milliseconds"
        );
        assert!(reopened
            .list_playback_statistics_runtimes()
            .expect("inspect statistics runtimes")
            .is_empty());
    }

    #[test]
    fn explorer_queue_statistics_are_written_only_to_the_temporary_catalog() {
        let temporary = TestDirectory::new();
        fs::create_dir_all(&temporary.0).unwrap();
        let permanent_path = temporary.0.join("library.sqlite3");
        let permanent = Database::open(&permanent_path).unwrap();
        let audio_path = temporary.0.join("transient.wav");
        fs::write(&audio_path, b"fixture").unwrap();
        let mut temporary_database = Database::open_in_memory().unwrap();
        let root = temporary_database
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                "explorer queue",
                MediaLocator::FileSystem(temporary.0.clone()),
            )
            .unwrap();
        let record = MediaTrackRecord {
            identity: TrackIdentity {
                source_id: root.id,
                source_item_id: "transient.wav".to_owned(),
                locator_key: None,
            },
            locator: MediaLocator::FileSystem(audio_path.clone()),
            fingerprint: FileFingerprint {
                size_bytes: 1,
                modified_at_utc_ms: None,
                created_at_utc_ms: None,
            },
            metadata: Some(TrackMetadata {
                title: Some("Transient".to_owned()),
                duration_ms: Some(60_000),
                ..TrackMetadata::default()
            }),
        };
        temporary_database
            .apply_source_scan(
                &root,
                &SourceScanState::Complete,
                std::slice::from_ref(&record),
                std::slice::from_ref(&record),
                &[],
                1,
            )
            .unwrap();
        let resolved_id = temporary_database
            .resolve_track_id_for_locator(&MediaLocator::FileSystem(audio_path.clone()))
            .unwrap()
            .unwrap();
        let temporary_database = Arc::new(temporary_database);
        let player = PlayerHandle::with_backend_factory(|| {
            Ok(Box::new(ClockedAudioBackend {
                loaded: false,
                playing: false,
                position: Duration::ZERO,
                started_at: None,
            }) as Box<dyn AudioBackend>)
        })
        .unwrap();
        let ready_deadline = Instant::now() + Duration::from_secs(2);
        while player.snapshot().state == AudioPlaybackState::Initializing {
            assert!(Instant::now() < ready_deadline);
            thread::sleep(Duration::from_millis(2));
        }
        let service = WindowsPlaybackService::with_modes(player, false, super::RepeatMode::Off);
        service.start_statistics_collector(&permanent_path).unwrap();
        let runtime_id = service
            .statistics_runtime_id
            .lock()
            .unwrap()
            .expect("collector registered runtime");
        temporary_database
            .register_playback_statistics_runtime(runtime_id, None, None)
            .unwrap();
        *service.temporary_catalog.lock().unwrap() = Some(Arc::clone(&temporary_database));
        service.external_tracks.lock().unwrap().insert(
            resolved_id,
            super::ExternalPlaybackTrack {
                path: audio_path.clone(),
                summary: temporary_database
                    .get_track_summary(resolved_id)
                    .unwrap()
                    .unwrap(),
                transient: true,
            },
        );
        let accounting_id =
            service.accounting_id_for_temporary(resolved_id, Arc::clone(&temporary_database));
        service
            .player
            .request_load_and_play_accounted(
                audio_path.to_string_lossy().into_owned(),
                accounting_id,
            )
            .unwrap()
            .wait(Duration::from_secs(2))
            .unwrap();
        thread::sleep(Duration::from_millis(180));
        service
            .player
            .request_pause()
            .unwrap()
            .wait(Duration::from_secs(2))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let played_ms = loop {
            let stats = temporary_database
                .playback_statistics_for(&[resolved_id])
                .unwrap();
            if stats[&resolved_id].played_ms > 0 {
                break stats[&resolved_id].played_ms;
            }
            assert!(
                Instant::now() < deadline,
                "temporary statistics did not flush"
            );
            thread::sleep(Duration::from_millis(10));
        };
        assert!(played_ms >= 80);
        assert_eq!(
            temporary_database
                .get_track_summary(resolved_id)
                .unwrap()
                .expect("temporary catalog keeps the transient track")
                .played_ms,
            played_ms,
            "temporary catalog track summaries must expose the persisted play-count input"
        );
        assert_eq!(
            permanent.playback_statistics_for(&[resolved_id]).unwrap()[&resolved_id].played_ms,
            0,
            "temporary listening time must not enter the persistent library database"
        );
        service.shutdown_statistics_collector();
        assert!(temporary_database
            .list_playback_statistics_runtimes()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn paused_next_keeps_audio_nonplaying() {
        let temporary = TestDirectory::new();
        let source_path = temporary.0.join("paused-navigation-source");
        fs::create_dir_all(&source_path).expect("create isolated source folder");
        write_tagged_mp3(&source_path.join("track-one.mp3"));
        write_tagged_mp3(&source_path.join("track-two.mp3"));
        let mut database =
            Database::open(temporary.0.join("library.sqlite3")).expect("open isolated database");
        add_windows_folder_to_database(
            &database,
            source_path.to_str().expect("source folder path"),
        )
        .expect("register isolated folder");
        sync_windows_sources(&mut database).expect("index isolated tracks");
        let tracks = database
            .list_tracks_page(ListTracksQuery {
                query: None,
                field_filter: None,
                offset: 0,
                limit: 10,
                sort: Default::default(),
            })
            .expect("read isolated tracks");
        assert_eq!(tracks.total_count, 2);

        let player = PlayerHandle::with_backend_factory(|| {
            Ok(Box::new(CapturingAudioBackend {
                loaded_path: Arc::new(Mutex::new(None)),
                position: Duration::ZERO,
            }) as Box<dyn AudioBackend>)
        })
        .expect("create isolated audio worker");
        let service = WindowsPlaybackService::new(player);
        let first_id = tracks.items[0].id;
        let change = play_track_from_database(
            &database,
            &service,
            &first_id.to_string(),
            Some(PlaybackQueueSource::Library {
                query: None,
                query_alternates: Vec::new(),
                field_filter: None,
                sort: Default::default(),
            }),
        )
        .expect("play selected initial track");
        change
            .ticket
            .wait(Duration::from_secs(2))
            .expect("initial track starts");
        commit_track_change(
            &database,
            &service,
            change.track,
            change.replacement_queue,
            change.target_cursor,
        )
        .expect("commit initial track");
        assert!(service.snapshot().is_playing);

        pause_playback(&service)
            .expect("queue pause")
            .wait(Duration::from_secs(2))
            .expect("pause initial track");
        assert!(!service.snapshot().is_playing);
        service
            .queue
            .lock()
            .unwrap()
            .as_mut()
            .expect("active queue")
            .set_repeat_mode(QueueRepeatMode::All);

        let change = navigate_queue(&database, &service, true)
            .expect("prepare next track")
            .expect("repeat-all queue has a next track");
        change
            .ticket
            .wait(Duration::from_secs(2))
            .expect("next track ACK");
        commit_track_change(
            &database,
            &service,
            change.track,
            change.replacement_queue,
            change.target_cursor,
        )
        .expect("commit next track");

        assert_eq!(service.snapshot().state, "paused");
        assert!(!service.snapshot().is_playing);

        let change = navigate_queue(&database, &service, false)
            .expect("prepare previous track")
            .expect("previous track exists after next");
        change
            .ticket
            .wait(Duration::from_secs(2))
            .expect("previous track ACK");
        commit_track_change(
            &database,
            &service,
            change.track,
            change.replacement_queue,
            change.target_cursor,
        )
        .expect("commit previous track");
        assert_eq!(service.snapshot().state, "paused");

        // Selecting another library or playlist row starts playback immediately,
        // including while paused. Next and previous still keep a paused session paused.
        let selected_id = tracks.items[1].id;
        let selected = play_track_from_database(
            &database,
            &service,
            &selected_id.to_string(),
            Some(PlaybackQueueSource::Library {
                query: None,
                query_alternates: Vec::new(),
                field_filter: None,
                sort: Default::default(),
            }),
        )
        .expect("select library row while paused");
        selected
            .ticket
            .wait(Duration::from_secs(2))
            .expect("selection ACK");
        commit_track_change(
            &database,
            &service,
            selected.track,
            selected.replacement_queue,
            selected.target_cursor,
        )
        .expect("commit paused selection");
        assert_eq!(service.snapshot().state, "playing");

        let mut playlist = Playlist::new("paused playlist selection");
        for (position, id) in [first_id, first_id, selected_id].into_iter().enumerate() {
            playlist.entries.push(PlaylistEntry {
                track_id: Some(id),
                locator: MediaLocator::FileSystem(
                    database
                        .resolve_playable_filesystem_locator(id)
                        .expect("resolve playlist fixture path"),
                ),
                title: None,
                duration_ms: None,
            });
            assert_eq!(playlist.entries.len() - 1, position);
        }
        database
            .save_playlist(&playlist)
            .expect("save test playlist");
        let playlist_selection = play_track_from_database(
            &database,
            &service,
            &first_id.to_string(),
            Some(PlaybackQueueSource::Playlist {
                playlist_id: playlist.id.to_string(),
                entry_position: 1,
            }),
        )
        .expect("select repeated playlist entry while paused");
        playlist_selection
            .ticket
            .wait(Duration::from_secs(2))
            .expect("playlist selection ACK");
        commit_track_change(
            &database,
            &service,
            playlist_selection.track,
            playlist_selection.replacement_queue,
            playlist_selection.target_cursor,
        )
        .expect("commit duplicate playlist selection");
        assert_eq!(service.snapshot().state, "playing");
        assert_eq!(
            service
                .queue
                .lock()
                .unwrap()
                .as_ref()
                .expect("playlist queue")
                .snapshot()
                .cursor,
            1,
            "selecting the repeated playlist row keeps its exact entry position"
        );

        pause_playback(&service)
            .expect("pause before queue-row selection")
            .wait(Duration::from_secs(2))
            .expect("pause playlist track");
        let queue_before_jump = service
            .queue
            .lock()
            .unwrap()
            .as_ref()
            .expect("playlist queue")
            .snapshot();
        let queue_jump = play_track_from_database(
            &database,
            &service,
            &first_id.to_string(),
            Some(PlaybackQueueSource::Queue {
                traversal_position: 0,
            }),
        )
        .expect("play a paused queue row");
        queue_jump
            .ticket
            .wait(Duration::from_secs(2))
            .expect("queue row ACK");
        commit_track_change(
            &database,
            &service,
            queue_jump.track,
            queue_jump.replacement_queue,
            queue_jump.target_cursor,
        )
        .expect("commit queue row");
        assert_eq!(service.snapshot().state, "playing");
        let queue_after_jump = service
            .queue
            .lock()
            .unwrap()
            .as_ref()
            .expect("queue after row click")
            .snapshot();
        assert_eq!(queue_after_jump.cursor, 0);
        assert_eq!(queue_after_jump.entries, queue_before_jump.entries);
        assert_eq!(queue_after_jump.play_order, queue_before_jump.play_order);

        let explicit_play =
            play_track_from_database(&database, &service, &selected_id.to_string(), None)
                .expect("explicit Play of current track");
        explicit_play
            .ticket
            .wait(Duration::from_secs(2))
            .expect("explicit Play ACK");
        assert_eq!(service.snapshot().state, "playing");

        let selected_while_playing = play_track_from_database(
            &database,
            &service,
            &first_id.to_string(),
            Some(PlaybackQueueSource::Library {
                query: None,
                query_alternates: Vec::new(),
                field_filter: None,
                sort: Default::default(),
            }),
        )
        .expect("select library row while playing");
        selected_while_playing
            .ticket
            .wait(Duration::from_secs(2))
            .expect("playing selection ACK");
        commit_track_change(
            &database,
            &service,
            selected_while_playing.track,
            selected_while_playing.replacement_queue,
            selected_while_playing.target_cursor,
        )
        .expect("commit playing selection");
        assert_eq!(service.snapshot().state, "playing");

        // Restored sessions stay Ready until the user clicks a list row.
        // That click starts playback instead of leaving the player paused or ready.
        let restored_path = database
            .resolve_playable_filesystem_locator(first_id)
            .expect("resolve restored fixture path");
        service
            .player
            .load(restored_path)
            .expect("load restored track");
        let ready_deadline = Instant::now() + Duration::from_secs(2);
        while service.player.snapshot().state != AudioPlaybackState::Ready {
            assert!(
                Instant::now() < ready_deadline,
                "restored player did not reach Ready"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let change = play_track_from_database(
            &database,
            &service,
            &selected_id.to_string(),
            Some(PlaybackQueueSource::Library {
                query: None,
                query_alternates: Vec::new(),
                field_filter: None,
                sort: Default::default(),
            }),
        )
        .expect("select track from restored Ready state");
        change
            .ticket
            .wait(Duration::from_secs(2))
            .expect("Ready selection ACK");
        assert_eq!(service.snapshot().state, "playing");
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
                field_filter: None,
                offset: 0,
                limit: 10,
                sort: Default::default(),
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
        let change = play_track_from_database(&database, &service, &track_id.to_string(), None)
            .expect("resolve internal ID and enqueue playback");
        let PreparedTrackChange {
            ticket,
            track,
            replacement_queue,
            target_cursor,
        } = change;
        assert_eq!(track.id, track_id);
        ticket
            .wait(Duration::from_secs(2))
            .expect("worker should acknowledge playback");
        commit_track_change(&database, &service, track, replacement_queue, target_cursor)
            .expect("commit queue only after audio ACK");

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

        let change = play_track_from_database(
            &database,
            &service,
            &track_id.to_string(),
            Some(PlaybackQueueSource::Playlist {
                playlist_id: playlist.id.to_string(),
                entry_position: 0,
            }),
        )
        .expect("start exact duplicate playlist queue");
        let PreparedTrackChange {
            ticket,
            track,
            replacement_queue,
            target_cursor,
        } = change;
        ticket
            .wait(Duration::from_secs(2))
            .expect("load duplicate queue track");
        commit_track_change(&database, &service, track, replacement_queue, target_cursor)
            .expect("persist duplicate queue after ACK");
        let change = navigate_queue(&database, &service, true)
            .expect("prepare next duplicate queue entry")
            .expect("next duplicate queue entry exists");
        let PreparedTrackChange {
            ticket,
            track,
            replacement_queue,
            target_cursor,
        } = change;
        ticket
            .wait(Duration::from_secs(2))
            .expect("load duplicate entry");
        commit_track_change(&database, &service, track, replacement_queue, target_cursor)
            .expect("commit duplicate entry only after ACK");
        let committed_queue = service.queue.lock().unwrap().as_ref().unwrap().snapshot();
        assert_eq!(
            committed_queue.entries[committed_queue.play_order[committed_queue.cursor]]
                .source_position,
            Some(1)
        );
        let saved_queue = database
            .load_playback_session()
            .expect("read saved queue")
            .unwrap()
            .queue;
        assert_eq!(saved_queue, committed_queue);

        let hidden_path = track_path.with_extension("mp3.temporarily-offline");
        fs::rename(&track_path, &hidden_path).expect("make source temporarily unavailable");
        assert!(navigate_queue(&database, &service, false).is_err());
        let queue_after_failure = service.queue.lock().unwrap().as_ref().unwrap().snapshot();
        assert_eq!(queue_after_failure, committed_queue);
        assert_eq!(
            database
                .load_playback_session()
                .expect("retained checkpoint")
                .unwrap()
                .queue,
            committed_queue
        );
        fs::rename(&hidden_path, &track_path).expect("restore isolated fixture");

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

        apply_system_media_event(&SystemMediaEvent::PauseRequested, Some(&database), &service)
            .expect("pause before checkpoint restore test");
        apply_system_media_event(
            &SystemMediaEvent::SeekRequested(Duration::from_millis(1_250)),
            Some(&database),
            &service,
        )
        .expect("persist successful seek ACK");
        let expected_checkpoint = database
            .load_playback_session()
            .expect("read checkpoint")
            .unwrap();
        assert_eq!(expected_checkpoint.position_ms, 1_250);

        let restored_path = Arc::new(Mutex::new(None));
        let path_for_backend = Arc::clone(&restored_path);
        let restored_player = PlayerHandle::with_backend_factory(move || {
            Ok(Box::new(CapturingAudioBackend {
                loaded_path: path_for_backend,
                position: Duration::ZERO,
            }) as Box<dyn AudioBackend>)
        })
        .expect("create isolated restore worker");
        let restored_service = WindowsPlaybackService::new(restored_player);
        restore_playback_session(&database, &restored_service).expect("restore paused session");
        assert_eq!(restored_service.snapshot().state, "ready");
        assert!(!restored_service.snapshot().is_playing);
        assert_eq!(restored_service.snapshot().position_ms, 1_250);
        assert_eq!(
            restored_service
                .snapshot()
                .current_track
                .as_ref()
                .map(|track| track.id),
            Some(track_id)
        );
        assert_eq!(
            restored_service
                .queue
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .snapshot(),
            expected_checkpoint.queue
        );

        // A temporary Explorer-open queue does not replace the saved queue,
        // but its mode controls still update settings.json. On the next launch
        // those settings are the service's initial modes and must win over an
        // older queue checkpoint.
        let preference_path = Arc::new(Mutex::new(None));
        let path_for_backend = Arc::clone(&preference_path);
        let preference_player = PlayerHandle::with_backend_factory(move || {
            Ok(Box::new(CapturingAudioBackend {
                loaded_path: path_for_backend,
                position: Duration::ZERO,
            }) as Box<dyn AudioBackend>)
        })
        .expect("create mode-preference restore worker");
        let preference_service =
            WindowsPlaybackService::with_modes(preference_player, true, super::RepeatMode::One);
        restore_playback_session(&database, &preference_service)
            .expect("apply current mode preferences while restoring the saved queue");
        let preference_queue = preference_service
            .queue
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .snapshot();
        assert!(preference_queue.shuffle);
        assert_eq!(preference_queue.repeat, QueueRepeatMode::One);
        assert_eq!(preference_queue.entries, expected_checkpoint.queue.entries);
        let saved_current_source_index =
            expected_checkpoint.queue.play_order[expected_checkpoint.queue.cursor];
        let preferred_current_source_index = preference_queue.play_order[preference_queue.cursor];
        assert_eq!(
            preference_queue.entries[preferred_current_source_index],
            expected_checkpoint.queue.entries[saved_current_source_index]
        );
        let expected_checkpoint = database
            .load_playback_session()
            .expect("read queue after applying latest modes")
            .unwrap();
        assert_eq!(expected_checkpoint.queue, preference_queue);
        assert_eq!(expected_checkpoint.position_ms, 1_250);

        let unavailable_path = track_path.with_extension("mp3.offline-fixture");
        fs::rename(&track_path, &unavailable_path).expect("make restored source unavailable");
        let offline_player = PlayerHandle::with_backend_factory(|| {
            Ok(Box::new(CapturingAudioBackend {
                loaded_path: Arc::new(Mutex::new(None)),
                position: Duration::ZERO,
            }) as Box<dyn AudioBackend>)
        })
        .expect("create offline restore worker");
        let initialization_deadline = Instant::now() + Duration::from_secs(2);
        while offline_player.snapshot().state == AudioPlaybackState::Initializing {
            assert!(
                Instant::now() < initialization_deadline,
                "offline restore worker did not initialize"
            );
            thread::sleep(Duration::from_millis(2));
        }
        let offline_service =
            WindowsPlaybackService::with_modes(offline_player, true, super::RepeatMode::One);
        restore_playback_session(&database, &offline_service)
            .expect("offline restore is fail-soft");
        assert!(offline_service.queue.lock().unwrap().is_some());
        assert_eq!(offline_service.snapshot().state, "empty");
        assert!(offline_service.snapshot().last_error.is_some());
        assert_eq!(
            database
                .load_playback_session()
                .expect("checkpoint remains offline")
                .unwrap(),
            expected_checkpoint
        );
        fs::rename(&unavailable_path, &track_path).expect("restore isolated fixture");
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
            artwork: Mutex::new(super::ArtworkPump::new()),
            published_metadata: Mutex::new(None),
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
