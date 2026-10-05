#[cfg(any(target_os = "windows", test))]
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "windows")]
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    time::Instant,
};

#[cfg(target_os = "windows")]
use player_core::PlaybackCheckpoint;
use player_core::{
    LibraryRoot, ListTracksQuery, MediaLocator, MediaSourceError, MediaSourceKind, Page,
    PlaybackQueue, PlaybackQueueContext, PlaybackQueueEntry, PlaybackQueueSnapshot, PlaylistId,
    PlaylistPage, PlaylistSummary, QueueRepeatMode, QueueTrackListeningStats, SourceId,
    SourceScanState, SyncEngine, SyncProgress, SyncReport, TrackFieldFilter, TrackId,
    TrackMetadataError, TrackSummary,
};
#[cfg(target_os = "windows")]
use player_db::PlaybackSessionCheckpoint;
use player_db::{Database, ThemePreferences};
use serde::{Deserialize, Serialize};
use tauri::{ipc::Response, AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Position, Size, State, WebviewWindow};
use tauri_plugin_media_index::MediaIndexExt;

mod database_lifecycle;
mod playlist_exchange;
use playlist_exchange::{export_playlist_file, PlaylistExportResult, PlaylistImportResult};
#[cfg(target_os = "windows")]
use playlist_exchange::{import_playlist_file_with_id, read_playlist_file};
mod settings;
use settings::{
    clamp_window_geometry_to_monitors, AppSettings, LyricsPreferences,
    NowPlayingAppearancePreferences, NowPlayingLayout, RepeatMode, SettingsStore, SourceEntry,
    SourceEntryKind, ThemeSettings, TrackListColumnSettings, WindowGeometry,
};
#[cfg(any(target_os = "android", test))]
mod android_artwork;
#[cfg(any(target_os = "android", test))]
#[cfg_attr(test, allow(dead_code))]
mod android_playback;
#[cfg(any(target_os = "android", test))]
mod android_playlist_source_sync;
mod lyrics_provider;
mod lyrics_service;
#[cfg(any(target_os = "windows", test))]
mod playlist_source_sync;
#[cfg(target_os = "windows")]
mod windows_data_directory;

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
    AudioError, CommandTicket, PlaybackAccountingCheckpoint, PlaybackAccountingId,
    PlaybackState as AudioPlaybackState, PlayerHandle,
};

const LIBRARY_SYNC_FINISHED_EVENT: &str = "library-sync-finished";
const LIBRARY_SYNC_PROGRESS_EVENT: &str = "library-sync-progress";
const PLAYBACK_QUEUE_PAGE_MAX_LIMIT: u32 = 100;
const SOURCE_SYNC_ERROR_DETAILS_LIMIT: usize = 100;

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
    #[cfg(target_os = "windows")]
    shutdown: database_lifecycle::ShutdownCoordinator,
    settings: SettingsStore,
    #[cfg(target_os = "windows")]
    window_frame: std::sync::Mutex<WindowFrameMemory>,
    lyrics_service: lyrics_service::LyricsService,
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
    accounting_track_ids: Arc<Mutex<HashMap<PlaybackAccountingId, TrackId>>>,
    statistics_status: Arc<Mutex<Option<String>>>,
    statistics_collector: Mutex<Option<PlaybackStatisticsCollector>>,
    current_track: Mutex<Option<TrackSummary>>,
    command_gate: Mutex<()>,
    queue: Mutex<Option<PlaybackQueue>>,
    queue_revision: AtomicU64,
    repeat_mode: Mutex<RepeatMode>,
    shuffle: Mutex<bool>,
    queue_advance_pending: Mutex<bool>,
    last_position_checkpoint: Mutex<Instant>,
    restore_warning: Mutex<Option<String>>,
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

    fn update(
        &mut self,
        track_id: Option<TrackId>,
        database: Option<&Database>,
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
            if let (Some(track_id), Some(database)) = (track_id, database) {
                match database.track_locators(track_id) {
                    Ok(locators) => {
                        self.pending = Some(ArtworkRequest { track_id, locators });
                        self.lookup_complete = false;
                    }
                    Err(_) => self.lookup_complete = true,
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
            statistics_status: Arc::new(Mutex::new(None)),
            statistics_collector: Mutex::new(None),
            current_track: Mutex::new(None),
            command_gate: Mutex::new(()),
            queue: Mutex::new(None),
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
            .insert(id, track_id);
        id
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
            can_next,
            can_previous,
        }
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
    accounting_track_ids: Arc<Mutex<HashMap<PlaybackAccountingId, TrackId>>>,
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
        let mut database_checkpoints = Vec::with_capacity(audio_checkpoints.len());
        let mut mapped_audio_checkpoints = Vec::with_capacity(audio_checkpoints.len());
        let mut unmapped = false;
        for checkpoint in &audio_checkpoints {
            if let Some(track_id) = track_map.get(&checkpoint.id) {
                mapped_audio_checkpoints.push(*checkpoint);
                database_checkpoints.push(PlaybackCheckpoint {
                    track_id: *track_id,
                    played_ms: checkpoint.played_ms,
                    duration_ms: checkpoint.duration_ms,
                });
            } else {
                unmapped = true;
            }
        }
        drop(track_map);

        if shutting_down {
            if unmapped {
                set_statistics_status(
                    &status,
                    Some(
                        "部分播放時長無法對應曲目；未保存增量可能遺失，已提交統計仍保留。"
                            .to_owned(),
                    ),
                );
                return;
            }
            match database.finish_playback_statistics_runtime(runtime_id, &database_checkpoints) {
                Ok(()) => {
                    acknowledge_statistics_batch(&player, &mapped_audio_checkpoints);
                    set_statistics_status(&status, None);
                    return;
                }
                Err(error) => {
                    shutdown_failures = shutdown_failures.saturating_add(1);
                    set_statistics_status(
                        &status,
                        Some(format!("播放時長仍待保存，關閉時將重試：{error}")),
                    );
                    if shutdown_failures >= 10 {
                        eprintln!(
                            "播放時長檢查點仍待保存；已提交統計保留，未保存增量可能遺失：{error}"
                        );
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
            }
        }

        if audio_checkpoints.is_empty() && !unmapped {
            continue;
        }
        if unmapped {
            set_statistics_status(
                &status,
                Some("部分播放時長仍待保存：找不到對應的曲目識別。".to_owned()),
            );
        }
        if database_checkpoints.is_empty() {
            if unmapped {
                std::thread::sleep(Duration::from_secs(1));
            }
            continue;
        }
        match database.record_playback_checkpoints(runtime_id, &database_checkpoints) {
            Ok(()) => {
                acknowledge_statistics_batch(&player, &mapped_audio_checkpoints);
                if unmapped {
                    set_statistics_status(
                        &status,
                        Some("部分播放時長仍待保存：找不到對應的曲目識別。".to_owned()),
                    );
                } else {
                    set_statistics_status(&status, None);
                }
            }
            Err(error) => {
                set_statistics_status(
                    &status,
                    Some(format!("播放時長檢查點仍待保存，背景工作會重試：{error}")),
                );
                eprintln!("播放時長檢查點仍待保存，背景工作會重試：{error}");
                std::thread::sleep(Duration::from_secs(1));
            }
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
        let track_id = current_track.as_ref().map(|track| track.id);
        let artwork = self
            .artwork
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .update(track_id, database);
        let current_metadata = current_track.map(|track| MediaControlMetadata {
            title: track.title,
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
    last_error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaStoreVolumeOption {
    volume_name: String,
    display_name: String,
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
        #[serde(default, rename = "fieldFilter")]
        field_filter: Option<TrackFieldFilter>,
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
        feature, playback_duration_ms, sync_summary, FeatureState, LibraryRoot, LibrarySource,
        MediaLocator, MediaSourceError, MediaSourceKind, PlaybackQueuePage, PlaybackQueuePageItem,
        PlaybackQueueSource, PlaybackSnapshot, RepeatMode, RuntimeCapabilities, SourceId,
        SourceScanState, SyncReport, TrackId, TrackMetadataError,
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
            PlaybackQueueSource::Library {
                query: Some(query),
                field_filter: None,
            } if query == "ambient"
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
                field_filter: Some(player_core::TrackFieldFilter {
                    field: player_core::TrackField::Artist,
                    value: "演出者 🎧".to_owned(),
                }),
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
fn library_get_page(
    state: State<'_, AppState>,
    query: Option<String>,
    field_filter: Option<TrackFieldFilter>,
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
            field_filter,
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
    let _database_work = enter_database_work(&state)?;
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

    #[cfg(not(target_os = "windows"))]
    {
        #[cfg(target_os = "android")]
        {
            let _ = window;
            let Some(selected) = app
                .media_index()
                .pick_playlist_import()
                .map_err(|error| error.to_string())?
            else {
                return Ok(None);
            };
            if !app
                .media_index()
                .has_saf_permission(&selected.tree_uri)
                .map_err(|error| error.to_string())?
            {
                return Err("Android 未保留此播放清單文件樹的讀取授權。".to_owned());
            }
            let playlist_uri = selected.playlist_uri.clone();
            let app_for_read = app.clone();
            let playlist_bytes = tauri::async_runtime::spawn_blocking(move || {
                let lease = app_for_read
                    .media_index()
                    .cache_content_uri(
                        &playlist_uri,
                        android_playlist_source_sync::MAX_ANDROID_PLAYLIST_BYTES as u64,
                    )
                    .map_err(|error| error.to_string())?;
                let metadata = std::fs::metadata(lease.path())
                    .map_err(|error| format!("無法讀取播放清單快取：{error}"))?;
                if metadata.len() > android_playlist_source_sync::MAX_ANDROID_PLAYLIST_BYTES as u64
                {
                    return Err("Android 播放清單超過 32 MiB 限制。".to_owned());
                }
                let mut bytes = Vec::with_capacity(metadata.len() as usize);
                {
                    use std::io::Read;
                    std::fs::File::open(lease.path())
                        .and_then(|file| {
                            file.take(
                                android_playlist_source_sync::MAX_ANDROID_PLAYLIST_BYTES as u64 + 1,
                            )
                            .read_to_end(&mut bytes)
                        })
                        .map_err(|error| format!("讀取播放清單快取失敗：{error}"))?;
                }
                if bytes.len() > android_playlist_source_sync::MAX_ANDROID_PLAYLIST_BYTES {
                    return Err("Android 播放清單超過 32 MiB 限制。".to_owned());
                }
                Ok(bytes)
            })
            .await
            .map_err(|error| format!("讀取 Android 播放清單工作失敗：{error}"))??;

            let file_name = std::path::Path::new(&selected.display_name)
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| {
                    name.rsplit_once('.').is_some_and(|(_, ext)| {
                        matches!(ext.to_ascii_lowercase().as_str(), "m3u" | "m3u8")
                    })
                })
                .ok_or_else(|| "選取的文件不是 .m3u 或 .m3u8 播放清單。".to_owned())?;
            let parse_path = std::path::PathBuf::from(file_name);
            let parsed = playlist_exchange::read_playlist_bytes(&playlist_bytes, &parse_path)?;
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
            let existing = settings.sources.iter().find(|source| {
                matches!(
                    &source.kind,
                    SourceEntryKind::PlaylistFile {
                        path: settings::StoredPath::Uri(path),
                        tree_uri: Some(tree),
                        ..
                    } if path == &selected.playlist_uri && tree == &selected.tree_uri
                )
            });
            let (source_id, playlist_id) = if let Some(source) = existing {
                let SourceEntryKind::PlaylistFile { playlist_id, .. } = source.kind else {
                    unreachable!();
                };
                (source.id, playlist_id)
            } else {
                let playlist_id = database
                    .unique_playlist_id_matching_locators(
                        &parsed.name,
                        &parsed
                            .entries
                            .iter()
                            .map(|entry| entry.locator.clone())
                            .collect::<Vec<_>>(),
                    )
                    .map_err(|error| error.to_string())?
                    .unwrap_or(parsed.id);
                (SourceId::new(), playlist_id)
            };
            state
                .settings
                .upsert_source(SourceEntry {
                    id: source_id,
                    display_name: file_name.to_owned(),
                    enabled: true,
                    kind: SourceEntryKind::PlaylistFile {
                        playlist_id,
                        path: settings::StoredPath::Uri(selected.playlist_uri),
                        tree_uri: Some(selected.tree_uri),
                    },
                })
                .map_err(|error| error.to_string())?;
            drop(_database_work);
            let sync = library_sync(app.clone()).await?;
            let _post_sync_database_work = enter_database_work(&state)?;
            let saved = database
                .get_playlist(playlist_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "播放清單同步後無法重新讀取。".to_owned())?;
            let result = PlaylistImportResult {
                playlist: PlaylistSummary {
                    id: saved.id,
                    name: saved.name,
                    entry_count: saved.entries.len() as u64,
                },
                matched_entries: saved
                    .entries
                    .iter()
                    .filter(|entry| entry.track_id.is_some())
                    .count() as u64,
            };
            let _ = sync;
            Ok(Some(result))
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (app, window, state);
            Err("M3U/M3U8 原生檔案匯入目前只支援 Windows 與 Android。".to_owned())
        }
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
            if relative_paths {
                return Err(
                    "Android 匯出使用已授權的 content URI；目前不支援相對路徑匯出。".to_owned(),
                );
            }
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
                    let media_index = app_for_export.media_index();
                    let Some(destination_uri) = media_index
                        .pick_playlist_export(&suggested_name)
                        .map_err(|error| error.to_string())?
                    else {
                        return Ok(None);
                    };
                    let lease = media_index
                        .create_cache_lease()
                        .map_err(|error| error.to_string())?;
                    let database =
                        Database::open(database_path).map_err(|error| error.to_string())?;
                    let result = export_playlist_file(
                        &database,
                        &playlist_id.to_string(),
                        lease.path(),
                        &extension,
                        None,
                    )?;
                    media_index
                        .write_content_cache_lease(&lease, &destination_uri)
                        .map_err(|error| error.to_string())?;
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
    #[cfg(target_os = "android")]
    if let Some(tree_uri) = source_tree_uri(&removed) {
        let tree_still_used = settings
            .sources
            .iter()
            .any(|source| source_tree_uri(source) == Some(tree_uri));
        if !tree_still_used {
            if let Err(error) = app.media_index().release_saf_tree(tree_uri) {
                eprintln!("無法釋放已移除來源的 SAF 授權：{error}");
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = app;
    settings
        .sources
        .iter()
        .map(|source| source_entry_summary(state.database.as_ref(), source))
        .collect()
}

#[cfg(target_os = "android")]
fn source_tree_uri(source: &SourceEntry) -> Option<&str> {
    match &source.kind {
        SourceEntryKind::Folder {
            media_kind: MediaSourceKind::AndroidSaf,
            path: settings::StoredPath::Uri(uri),
        } => Some(uri),
        SourceEntryKind::PlaylistFile {
            tree_uri: Some(uri),
            ..
        } => Some(uri),
        _ => None,
    }
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
    let _database_work = enter_database_work(&state)?;
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

    let _database_work = enter_database_work(&state)?;
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
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _database_work = database_work;
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
        let playlist_sources = configured_sources
            .iter()
            .filter(|source| source.enabled)
            .filter(|source| matches!(source.kind, SourceEntryKind::PlaylistFile { .. }))
            .cloned()
            .collect::<Vec<_>>();
        let source_count = roots.len() + playlist_sources.len();
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
        for source in playlist_sources {
            let root = source
                .database_projection()
                .map_err(|error| error.to_string())?;
            let SourceEntryKind::PlaylistFile {
                path: settings::StoredPath::Uri(playlist_uri),
                tree_uri: Some(tree_uri),
                ..
            } = &source.kind
            else {
                sources.push(unavailable_source_summary(
                    &root,
                    "播放清單來源缺少有效的 SAF URI 或文件樹授權。",
                ));
                continue;
            };
            let media_index = app.media_index();
            match media_index.has_saf_permission(tree_uri) {
                Ok(true) => {}
                Ok(false) => {
                    sources.push(unavailable_source_summary(
                        &root,
                        "已撤銷此播放清單的 SAF 文件樹權限。",
                    ));
                    continue;
                }
                Err(error) => {
                    sources.push(unavailable_source_summary(
                        &root,
                        format!("無法確認 SAF 文件樹權限：{error}"),
                    ));
                    continue;
                }
            }
            let lease = match media_index.cache_content_uri(
                playlist_uri,
                android_playlist_source_sync::MAX_ANDROID_PLAYLIST_BYTES as u64,
            ) {
                Ok(lease) => lease,
                Err(error) => {
                    sources.push(unavailable_source_summary(
                        &root,
                        format!("無法讀取 SAF 播放清單：{error}"),
                    ));
                    continue;
                }
            };
            let file_metadata = match std::fs::metadata(lease.path()) {
                Ok(metadata)
                    if metadata.len()
                        <= android_playlist_source_sync::MAX_ANDROID_PLAYLIST_BYTES as u64 =>
                {
                    metadata
                }
                Ok(metadata) => {
                    sources.push(unavailable_source_summary(
                        &root,
                        format!(
                            "播放清單大小 {} bytes 超過 {} bytes 上限。",
                            metadata.len(),
                            android_playlist_source_sync::MAX_ANDROID_PLAYLIST_BYTES
                        ),
                    ));
                    continue;
                }
                Err(error) => {
                    sources.push(unavailable_source_summary(
                        &root,
                        format!("無法讀取 SAF 播放清單檔案資訊：{error}"),
                    ));
                    continue;
                }
            };
            let mut playlist_bytes = Vec::with_capacity(file_metadata.len() as usize);
            let read_result = {
                use std::io::Read;
                std::fs::File::open(lease.path()).and_then(|file| {
                    file.take(android_playlist_source_sync::MAX_ANDROID_PLAYLIST_BYTES as u64 + 1)
                        .read_to_end(&mut playlist_bytes)
                })
            };
            if let Err(error) = read_result {
                sources.push(unavailable_source_summary(
                    &root,
                    format!("無法讀取暫存的 SAF 播放清單：{error}"),
                ));
                continue;
            }
            if playlist_bytes.len() > android_playlist_source_sync::MAX_ANDROID_PLAYLIST_BYTES {
                sources.push(unavailable_source_summary(
                    &root,
                    format!(
                        "讀取的播放清單大小 {} bytes 超過 {} bytes 上限。",
                        playlist_bytes.len(),
                        android_playlist_source_sync::MAX_ANDROID_PLAYLIST_BYTES
                    ),
                ));
                continue;
            }
            let authorized_content_uris =
                authorized_android_playlist_content_uris(database, app, &source, &playlist_bytes)?;
            let report = android_playlist_source_sync::sync_android_playlist_source(
                database,
                &source,
                &playlist_bytes,
                None,
                synced_at_utc_ms,
                |locator| {
                    media_index
                        .resolve_saf_playlist_entry(tree_uri, playlist_uri, locator)
                        .map(|entry| {
                            entry.map(|entry| {
                                android_playlist_source_sync::ResolvedSafPlaylistEntry {
                                    content_uri: entry.content_uri,
                                    size_bytes: entry.size_bytes,
                                    modified_at_utc_ms: entry
                                        .modified_at_utc_ms
                                        .and_then(|value| i64::try_from(value).ok()),
                                }
                            })
                        })
                        .map_err(|error| error.to_string())
                },
                |uri| Ok(authorized_content_uris.get(uri).cloned().flatten()),
                |record| media_index.read_metadata(record),
            );
            drop(lease);
            match report {
                Ok(Some(report)) => sources.push(sync_summary(&root, report)),
                Ok(None) => sources.push(unavailable_source_summary(
                    &root,
                    "播放清單同步沒有產生來源結果。",
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
        let _ = (app, database);
        return Err("目前平台尚未提供媒體來源同步。".to_owned());
    }
}

#[cfg(target_os = "android")]
fn authorized_android_playlist_content_uris(
    database: &Database,
    app: &AppHandle,
    source: &SourceEntry,
    bytes: &[u8],
) -> Result<
    std::collections::HashMap<
        String,
        Option<android_playlist_source_sync::ResolvedSafPlaylistEntry>,
    >,
    String,
> {
    use std::path::PathBuf;

    let extension = source.display_name.rsplit('.').next().unwrap_or_default();
    let synthetic_file =
        PathBuf::from("__android_saf_playlist_parent__").join(format!("playlist.{extension}"));
    let parsed = if extension.eq_ignore_ascii_case("m3u8") {
        player_core::parse_m3u8(bytes, &synthetic_file)
    } else if extension.eq_ignore_ascii_case("m3u") {
        player_core::parse_m3u(bytes, &synthetic_file)
    } else {
        return Ok(std::collections::HashMap::new());
    };
    let Ok(parsed) = parsed else {
        // The sync module will report the parse error and leave the old projection untouched.
        return Ok(std::collections::HashMap::new());
    };

    let mut authorized = std::collections::HashMap::new();
    for entry in parsed.entries {
        let MediaLocator::ContentUri(uri) = entry.locator else {
            continue;
        };
        if authorized.contains_key(&uri) {
            continue;
        }
        let locator = MediaLocator::ContentUri(uri.clone());
        let track_id = database
            .resolve_track_id_for_locator(&locator)
            .map_err(|error| error.to_string())?;
        let value = if let Some(track_id) = track_id {
            let locators = database
                .track_locators(track_id)
                .map_err(|error| error.to_string())?;
            if locators.contains(&locator) {
                app.media_index()
                    .probe_content_uri(&uri)
                    .ok()
                    .flatten()
                    .filter(|metadata| metadata.content_uri == uri)
                    .map(
                        |metadata| android_playlist_source_sync::ResolvedSafPlaylistEntry {
                            content_uri: metadata.content_uri,
                            size_bytes: metadata.size_bytes,
                            modified_at_utc_ms: metadata.modified_at_utc_ms,
                        },
                    )
            } else {
                None
            }
        } else {
            None
        };
        authorized.insert(uri, value);
    }
    Ok(authorized)
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
        build_playback_queue_page(state.database.as_ref(), revision, queue, offset, limit)
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
        let settings = state
            .settings
            .snapshot()
            .map_err(|error| error.to_string())?;
        let mut tree_uris = settings
            .sources
            .iter()
            .filter(|source| source.enabled)
            .filter_map(|source| match &source.kind {
                SourceEntryKind::Folder {
                    media_kind: MediaSourceKind::AndroidSaf,
                    path: settings::StoredPath::Uri(tree_uri),
                } => Some(tree_uri.clone()),
                SourceEntryKind::PlaylistFile {
                    tree_uri: Some(tree_uri),
                    ..
                } => Some(tree_uri.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        tree_uris.sort();
        tree_uris.dedup();
        let artwork = tauri::async_runtime::spawn_blocking(move || {
            find_android_artwork(&app, &locators, &tree_uris)
        })
        .await
        .map_err(|error| format!("封面讀取工作失敗：{error}"))?;
        match artwork {
            Ok(image) => Ok(Response::new(image.into_bytes())),
            Err(android_artwork::ArtworkError::Missing) => Ok(Response::new(Vec::new())),
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
    let content_uris = locators
        .iter()
        .filter_map(|locator| match locator {
            MediaLocator::ContentUri(uri) if uri.starts_with("content://") => Some(uri.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut oversized = false;
    for uri in content_uris {
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
                set_queue_shuffle_with_latest_stats(database, queue, true)?;
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
    let cursor =
        u64::try_from(queue.cursor).map_err(|_| "播放佇列游標超出支援範圍。".to_owned())?;
    let current_entry_position = queue
        .play_order
        .get(queue.cursor)
        .copied()
        .map(u64::try_from)
        .transpose()
        .map_err(|_| "播放佇列項目位置超出支援範圍。".to_owned())?;
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
                is_current: traversal_position == cursor,
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

fn queue_for_track(
    database: &Database,
    track_id: TrackId,
    source: Option<PlaybackQueueSource>,
) -> Result<PlaybackQueue, String> {
    let (entries, context, selected_index) = match source {
        Some(PlaybackQueueSource::Library {
            query,
            field_filter,
        }) => {
            let track_ids = database
                .list_track_ids_filtered(query.as_deref(), field_filter.as_ref())
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
                },
                selected,
            )
        }
    };
    PlaybackQueue::with_entries(entries, context, selected_index)
        .ok_or_else(|| "播放清單沒有可播放的曲目。".to_owned())
}

#[cfg(target_os = "windows")]
fn prepare_track_load(
    database: &Database,
    service: &WindowsPlaybackService,
    track_id: TrackId,
    force_play: bool,
) -> Result<(CommandTicket, TrackSummary), String> {
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
async fn await_playback_ack(ticket: CommandTicket) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || ticket.wait(Duration::from_secs(2)))
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
        let mut current_queue = service
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *current_queue = Some(queue);
        bump_queue_revision(service);
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
    let position = window.outer_position().ok()?;
    let size = window.inner_size().ok()?;
    let maximized = window.is_maximized().unwrap_or(false);
    Some(WindowGeometry {
        x: position.x,
        y: position.y,
        width: size.width.max(1),
        height: size.height.max(1),
        maximized,
    })
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
    let geometry = if maximized {
        state
            .window_frame
            .lock()
            .ok()
            .and_then(|frame| frame.normal.clone())
            .or_else(|| capture_window_geometry(window))
    } else {
        capture_window_geometry(window)
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
    if let Err(error) = window.set_size(Size::Physical(PhysicalSize::new(geometry.width, geometry.height))) {
        eprintln!("無法還原視窗大小：{error}");
    }
    if let Err(error) = window.set_position(Position::Physical(PhysicalPosition::new(geometry.x, geometry.y))) {
        eprintln!("無法還原視窗位置：{error}");
    }
    if geometry.maximized {
        if let Err(error) = window.maximize() {
            eprintln!("無法還原最大化狀態：{error}");
        }
    }
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

#[cfg(target_os = "windows")]
fn save_playback_session(
    database: &Database,
    service: &WindowsPlaybackService,
) -> Result<(), String> {
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
    let queue = PlaybackQueue::restore(checkpoint.queue)
        .map_err(|error| format!("保存的播放佇列無法還原：{error}"))?;
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
    let track = database
        .get_track_summary(track_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "曲目已不在曲庫中，請重新整理列表。".to_owned())?;
    let path = database
        .resolve_playable_filesystem_locator(track_id)
        .map_err(|error| error.to_string())?;
    let selection_intent = source.is_some();

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
    let preserve_queue = source.is_none()
        && existing_queue
            .as_ref()
            .is_some_and(|queue| queue.current() == track_id);
    let replacement_queue = if preserve_queue {
        None
    } else {
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
        set_queue_shuffle_with_latest_stats(database, &mut queue, shuffle)?;
        Some(queue)
    };
    drop(existing_queue);
    let audio_snapshot = service.player.snapshot();
    let ticket = if !selection_intent
        && current_track_id == Some(track_id)
        && matches!(
            audio_snapshot.state,
            AudioPlaybackState::Ready | AudioPlaybackState::Paused
        ) {
        service
            .player
            .request_play()
            .map_err(|error| playback_audio_error_message(&error))?
    } else if selection_intent {
        service
            .player
            .request_load_preserving_playback_accounted(path, service.accounting_id_for(track_id))
            .map_err(|error| playback_audio_error_message(&error))?
    } else {
        service
            .player
            .request_load_and_play_accounted(path, service.accounting_id_for(track_id))
            .map_err(|error| playback_audio_error_message(&error))?
    };
    Ok(PreparedTrackChange {
        ticket,
        track,
        replacement_queue,
        target_cursor: None,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_media_index::init());
    #[cfg(target_os = "windows")]
    let builder = builder.plugin(tauri_plugin_dialog::init());

    builder
        .on_window_event(|window, event| {
            #[cfg(target_os = "windows")]
            if window.label() == "main" {
                match event {
                    tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                        remember_main_window_normal_frame(window);
                    }
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        persist_main_window_geometry(window);
                        let app = window.app_handle().clone();
                        let state = app.state::<AppState>();
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
            settings_get_recovery_warning,
            settings_source_registry_authoritative,
            settings_confirm_source_registry,
            theme_get_preferences,
            theme_set_preferences,
            settings_get_lyrics_preferences,
            settings_set_lyrics_preferences,
            settings_get_now_playing_appearance_preferences,
            settings_set_now_playing_appearance_preferences,
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
            playback_get_queue_page,
            library_get_track_artwork,
            playback_play,
            playback_pause,
            playback_next,
            playback_previous,
            playback_seek,
            playback_set_volume,
            playback_set_repeat,
            playback_set_shuffle,
            lyrics_service::lyrics_get_track,
            lyrics_service::lyrics_search,
            lyrics_service::lyrics_select_candidate,
            lyrics_service::lyrics_cancel_search,
            lyrics_service::lyrics_clear_track,
        ])
        .setup(|app| {
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
            #[cfg(target_os = "android")]
            let initial_settings = settings
                .snapshot()
                .map_err(|error| format!("讀取使用者設定失敗：{error}"))?;
            #[cfg(target_os = "windows")]
            let (playback, playback_error) = match PlayerHandle::new() {
                Ok(player) => {
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
            app.manage(AppState {
                database,
                database_path: stored_path.clone(),
                database_error,
                database_work: database_lifecycle::DatabaseWorkGate::default(),
                #[cfg(target_os = "windows")]
                shutdown: database_lifecycle::ShutdownCoordinator::default(),
                settings,
                #[cfg(target_os = "windows")]
                window_frame: std::sync::Mutex::new(WindowFrameMemory {
                    normal: initial_settings.window_geometry.as_ref().map(|geometry| WindowGeometry {
                        maximized: false,
                        ..geometry.clone()
                    }),
                }),
                lyrics_service: lyrics_service::LyricsService::default(),
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

            if let Some(window) = app.get_webview_window("main") {
                window.set_title("MoeMusicPlayer")?;
                #[cfg(target_os = "windows")]
                window.set_always_on_top(false)?;
                // Window starts with visible:false so we can apply saved geometry before first paint.
                #[cfg(target_os = "windows")]
                if let Some(geometry) = initial_settings.window_geometry.clone() {
                    apply_saved_window_geometry(&window, geometry);
                }
                window.show()?;
                let _ = window.set_focus();
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("MoeMusicPlayer could not start")
        .run(|app_handle, event| {
            #[cfg(target_os = "windows")]
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                let state = app_handle.state::<AppState>();
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
        apply_system_media_event, build_playback_queue_page, commit_track_change, navigate_queue,
        pause_playback, play_track_from_database, playback_audio_error_message, queue_for_track,
        restore_playback_session, set_playback_volume, set_queue_shuffle_with_latest_stats,
        snapshot_playback_queue, PlaybackQueueSource, PreparedTrackChange, TrackSummary,
        WindowsPlaybackService,
    };
    use player_audio_windows::{
        system_media::{SystemMediaController, SystemMediaEvent},
        AudioBackend, AudioError, PlaybackState as AudioPlaybackState, PlayerHandle,
    };
    use player_core::{
        FileFingerprint, LibraryRepository, ListTracksQuery, MediaLocator, MediaSourceKind,
        MediaTrackRecord, PlaybackQueue, PlaybackQueueContext, PlaybackQueueEntry, Playlist,
        PlaylistEntry, QueueRepeatMode, SourceId, SourceScanState, TrackField, TrackFieldFilter,
        TrackId, TrackIdentity, TrackMetadata,
    };
    use player_db::{Database, PlaybackSessionCheckpoint};
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
                tree_uri: None,
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
            fixture("second", "Needle Two", "hanser"),
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
            value: "hanser".to_owned(),
        };
        let page = database
            .list_tracks_page(ListTracksQuery {
                query: Some("needle".to_owned()),
                field_filter: Some(filter.clone()),
                offset: 0,
                limit: 10,
            })
            .expect("read exact-filtered page");
        assert_eq!(page.total_count, 2);
        let selected_track = page.items[0].id;
        let queue = queue_for_track(
            &database,
            selected_track,
            Some(PlaybackQueueSource::Library {
                query: Some("needle".to_owned()),
                field_filter: Some(filter.clone()),
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
            }
        );
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
            year: None,
            bit_depth: None,
            played_ms: 0,
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
                field_filter: None,
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

        // Selecting another library row while paused must keep the intent,
        // while selecting during playback must leave playback active.
        let selected_id = tracks.items[1].id;
        let selected = play_track_from_database(
            &database,
            &service,
            &selected_id.to_string(),
            Some(PlaybackQueueSource::Library {
                query: None,
                field_filter: None,
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
        assert_eq!(service.snapshot().state, "paused");

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
        assert_eq!(service.snapshot().state, "paused");
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
                field_filter: None,
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

        // Restored sessions load to Ready; replacing their selected track must
        // not turn startup restoration into autoplay.
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
                field_filter: None,
            }),
        )
        .expect("select track from restored Ready state");
        change
            .ticket
            .wait(Duration::from_secs(2))
            .expect("Ready selection ACK");
        assert_eq!(service.snapshot().state, "ready");
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

        let unavailable_path = track_path.with_extension("mp3.offline-fixture");
        fs::rename(&track_path, &unavailable_path).expect("make restored source unavailable");
        let offline_player = PlayerHandle::with_backend_factory(|| {
            Ok(Box::new(CapturingAudioBackend {
                loaded_path: Arc::new(Mutex::new(None)),
                position: Duration::ZERO,
            }) as Box<dyn AudioBackend>)
        })
        .expect("create offline restore worker");
        let offline_service = WindowsPlaybackService::new(offline_player);
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
