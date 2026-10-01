use super::*;

use std::{
    fs,
    path::PathBuf,
    sync::{mpsc::RecvTimeoutError, Arc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use player_core::{
    FileFingerprint, LibraryRepository, MediaSourceKind, MediaTrackRecord, Playlist, PlaylistEntry,
    SourceScanState, TrackIdentity, TrackMetadata,
};

const WAIT: Duration = Duration::from_secs(3);

struct TemporaryDatabase {
    directory: PathBuf,
    path: PathBuf,
}

impl TemporaryDatabase {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "moemusic-android-actor-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("create isolated actor test directory");
        Self {
            path: directory.join("actor-test.sqlite3"),
            directory,
        }
    }

    fn open(&self) -> Database {
        Database::open(&self.path).expect("open isolated actor database")
    }
}

impl Drop for TemporaryDatabase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[derive(Clone)]
struct FixtureTracks {
    root: player_core::LibraryRoot,
    playlist_id: player_core::PlaylistId,
    first: TrackId,
    second: TrackId,
}

fn seed_library(database: &mut Database) -> FixtureTracks {
    let root = database
        .add_library_root(
            MediaSourceKind::AndroidSaf,
            "fake SAF source",
            MediaLocator::ContentUri("content://media/external/audio".to_owned()),
        )
        .expect("add enabled fake media root");
    let first_uri = "content://media/external/audio/media/101";
    let second_uri = "content://media/external/audio/media/202";
    let records = [
        ("first", first_uri, "First Track"),
        ("second", second_uri, "Second Track"),
    ]
    .into_iter()
    .map(|(item, uri, title)| MediaTrackRecord {
        identity: TrackIdentity {
            source_id: root.id,
            source_item_id: item.to_owned(),
            locator_key: Some(uri.to_owned()),
        },
        locator: MediaLocator::ContentUri(uri.to_owned()),
        fingerprint: FileFingerprint {
            size_bytes: 128,
            modified_at_utc_ms: Some(1_800_000_000_000),
        },
        metadata: Some(TrackMetadata {
            title: Some(title.to_owned()),
            artist: Some("Actor Test".to_owned()),
            duration_ms: Some(8_000),
            ..TrackMetadata::default()
        }),
    })
    .collect::<Vec<_>>();
    database
        .apply_source_scan(
            &root,
            &SourceScanState::Complete,
            &records,
            &records,
            &[],
            1,
        )
        .expect("persist isolated media rows");
    let first = database
        .resolve_track_id_for_locator(&MediaLocator::ContentUri(first_uri.to_owned()))
        .expect("resolve first track")
        .expect("first track ID");
    let second = database
        .resolve_track_id_for_locator(&MediaLocator::ContentUri(second_uri.to_owned()))
        .expect("resolve second track")
        .expect("second track ID");
    let playlist_id = player_core::PlaylistId::new();
    database
        .save_playlist(&Playlist {
            id: playlist_id,
            name: "actor duplicate fixture".to_owned(),
            entries: vec![
                PlaylistEntry {
                    track_id: Some(first),
                    locator: MediaLocator::ContentUri(first_uri.to_owned()),
                    title: Some("First slot".to_owned()),
                    duration_ms: None,
                },
                PlaylistEntry {
                    track_id: Some(first),
                    locator: MediaLocator::ContentUri(first_uri.to_owned()),
                    title: Some("Duplicate slot".to_owned()),
                    duration_ms: None,
                },
                PlaylistEntry {
                    track_id: Some(second),
                    locator: MediaLocator::ContentUri(second_uri.to_owned()),
                    title: Some("Next slot".to_owned()),
                    duration_ms: None,
                },
            ],
        })
        .expect("persist duplicate-entry playlist");
    FixtureTracks {
        root,
        playlist_id,
        first,
        second,
    }
}

struct ActorFixture {
    actor: Option<AndroidActor>,
    shared: Arc<SharedService>,
    _request_sender: SyncSender<ActorRequest>,
    fake: FakeNative,
}

impl ActorFixture {
    fn new(database: Database) -> Self {
        Self::with_snapshot(database, NativePlaybackSnapshot::default())
    }

    fn with_snapshot(database: Database, native: NativePlaybackSnapshot) -> Self {
        let (request_sender, request_receiver) = mpsc::sync_channel(8);
        let (command_sender, command_receiver) = mpsc::sync_channel(8);
        let (event_sender, event_receiver) = mpsc::sync_channel(16);
        let shared = Arc::new(SharedService {
            requests: request_sender.clone(),
            native_commands: Mutex::new(command_receiver),
            native_events: event_sender.clone(),
            snapshot: RwLock::new(empty_snapshot(false, RepeatMode::Off)),
            queue: RwLock::new(None),
            queue_revision: AtomicU64::new(0),
        });
        let runtime_id = uuid::Uuid::new_v4();
        let mut actor = AndroidActor::new(
            database,
            Arc::clone(&shared),
            request_receiver,
            command_sender,
            event_receiver,
            runtime_id,
            false,
            RepeatMode::Off,
        );
        actor.native = native.clone();
        if native.service_generation > 0 {
            assert!(actor
                .event_fence
                .adopt_generation(native.service_generation));
        }
        let fake = FakeNative::new(shared.clone(), event_sender.clone(), native);
        Self {
            actor: Some(actor),
            shared,
            _request_sender: request_sender,
            fake,
        }
    }

    fn actor(&self) -> &AndroidActor {
        self.actor.as_ref().expect("actor is available")
    }

    fn actor_mut(&mut self) -> &mut AndroidActor {
        self.actor.as_mut().expect("actor is available")
    }

    fn take_actor(&mut self) -> AndroidActor {
        self.actor.take().expect("actor is available")
    }

    fn publish_fixture_state(&mut self) {
        if let Some(queue) = self.actor().queue.as_ref() {
            *self.shared.queue.write().expect("queue lock") = Some(queue.snapshot());
        }
        *self.shared.snapshot.write().expect("snapshot lock") = self.actor().publish_snapshot();
    }
}

struct FakeNative {
    shared: Arc<SharedService>,
    event_sender: SyncSender<NativeEvent>,
    snapshot: NativePlaybackSnapshot,
    event_id: u64,
}

impl FakeNative {
    fn new(
        shared: Arc<SharedService>,
        event_sender: SyncSender<NativeEvent>,
        snapshot: NativePlaybackSnapshot,
    ) -> Self {
        Self {
            shared,
            event_sender,
            snapshot,
            event_id: 0,
        }
    }

    fn next_command(&self) -> NativeCommand {
        self.shared
            .native_commands
            .lock()
            .expect("native command receiver lock")
            .recv_timeout(WAIT)
            .expect("actor should issue a native command")
    }

    fn no_command(&self) {
        match self
            .shared
            .native_commands
            .lock()
            .expect("native command receiver lock")
            .recv_timeout(Duration::from_millis(100))
        {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(command) => panic!("unexpected native command: {}", command.op),
            Err(RecvTimeoutError::Disconnected) => panic!("native command channel disconnected"),
        }
    }

    fn ack(&mut self, command: &NativeCommand) {
        let serialized_command = serde_json::to_value(command).expect("command JSON DTO");
        assert_eq!(serialized_command["kind"], "command");
        assert_eq!(serialized_command["op"], command.op);
        if command.op == "load" {
            self.snapshot.service_generation = self.snapshot.service_generation.max(1);
            self.snapshot.load_instance = self.snapshot.load_instance.saturating_add(1).max(1);
            self.snapshot.track_id = command.track_id.clone();
            self.snapshot.media_key = Some(command.command_id.clone());
            self.snapshot.error = None;
            self.snapshot.position_ms = command.position_ms.unwrap_or(0);
            self.snapshot.duration_ms = command
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.duration_ms);
            self.snapshot.state = if command.play_when_ready == Some(true) {
                "playing"
            } else {
                "ready"
            }
            .to_owned();
        } else {
            match command.op {
                "play" => self.snapshot.state = "playing".to_owned(),
                "pause" => self.snapshot.state = "paused".to_owned(),
                "stop" => self.snapshot.state = "stopped".to_owned(),
                "seek" => self.snapshot.position_ms = command.position_ms.unwrap_or(0),
                _ => {}
            }
        }
        if let Some(volume) = command.volume {
            self.snapshot.volume = volume;
        }
        self.event_id = self.event_id.saturating_add(1).max(1);
        let event_json = serde_json::json!({
            "v": 1,
            "kind": "ack",
            "eventId": self.event_id.to_string(),
            "commandId": command.command_id,
            "requestId": command.request_id,
            "serviceGeneration": self.snapshot.service_generation,
            "loadInstance": self.snapshot.load_instance,
            "mediaKey": self.snapshot.media_key,
            "trackId": self.snapshot.track_id,
            "snapshot": self.snapshot,
        });
        let event: NativeEvent =
            serde_json::from_value(event_json).expect("Kotlin-compatible ACK DTO");
        self.event_sender.send(event).expect("send fake native ACK");
    }

    fn fail_load(&mut self, command: &NativeCommand, error: &str) {
        assert_eq!(command.op, "load");
        self.snapshot.service_generation = self.snapshot.service_generation.max(1);
        self.snapshot.load_instance = self.snapshot.load_instance.saturating_add(1).max(1);
        self.snapshot.track_id = None;
        self.snapshot.media_key = None;
        self.snapshot.state = "ready".to_owned();
        self.snapshot.position_ms = 0;
        self.snapshot.duration_ms = None;
        self.snapshot.error = Some(error.to_owned());
        self.event_id = self.event_id.saturating_add(1).max(1);
        let event_json = serde_json::json!({
            "v": 1,
            "kind": "ack",
            "eventId": self.event_id.to_string(),
            "commandId": command.command_id,
            "requestId": command.request_id,
            "serviceGeneration": self.snapshot.service_generation,
            "loadInstance": self.snapshot.load_instance,
            "mediaKey": self.snapshot.media_key,
            "trackId": self.snapshot.track_id,
            "error": error,
            "snapshot": self.snapshot,
        });
        let event: NativeEvent =
            serde_json::from_value(event_json).expect("Kotlin-compatible load error ACK DTO");
        self.event_sender
            .send(event)
            .expect("send fake load error ACK");
    }
}

fn perform_on_thread<T: Send + 'static>(
    mut actor: AndroidActor,
    operation: impl FnOnce(&mut AndroidActor) -> T + Send + 'static,
) -> thread::JoinHandle<(AndroidActor, T)> {
    thread::spawn(move || {
        let result = operation(&mut actor);
        (actor, result)
    })
}

fn playlist_source(
    playlist_id: player_core::PlaylistId,
    entry_position: u64,
) -> PlaybackQueueSource {
    PlaybackQueueSource::Playlist {
        playlist_id: playlist_id.to_string(),
        entry_position,
    }
}

fn queue_for_duplicate_slot(database: &Database, fixture: &FixtureTracks) -> PlaybackQueue {
    super::queue_for_track(
        database,
        fixture.first,
        Some(playlist_source(fixture.playlist_id, 1)),
    )
    .expect("construct queue from duplicate playlist slot")
}

fn playing_snapshot(
    track: TrackId,
    load_instance: u64,
    state: &str,
    position_ms: u64,
) -> NativePlaybackSnapshot {
    NativePlaybackSnapshot {
        service_generation: 1,
        load_instance,
        track_id: Some(track.to_string()),
        media_key: Some(format!("load-{load_instance}")),
        state: state.to_owned(),
        position_ms,
        duration_ms: Some(8_000),
        volume: 1.0,
        error: None,
    }
}

#[test]
fn select_commits_duplicate_entry_queue_only_after_native_load_ack() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    drop(seed_db);
    let mut harness = ActorFixture::new(temporary.open());
    let actor = harness.take_actor();
    let action = PlaybackAction::SelectTrack {
        track_id: fixture.first.to_string(),
        source: Some(playlist_source(fixture.playlist_id, 1)),
    };
    let handle = perform_on_thread(actor, move |actor| {
        actor.handle_action("load-duplicate", action)
    });

    let load = harness.fake.next_command();
    assert_eq!(load.op, "load");
    assert_eq!(
        load.track_id.as_deref(),
        Some(fixture.first.to_string().as_str())
    );
    assert_eq!(load.play_when_ready, Some(true));
    assert!(harness.shared.queue.read().expect("queue lock").is_none());
    harness.fake.ack(&load);

    let capabilities = harness.fake.next_command();
    assert_eq!(capabilities.op, "set_capabilities");
    let published = harness
        .shared
        .queue
        .read()
        .expect("queue lock")
        .clone()
        .expect("queue commits after successful load ACK");
    assert_eq!(published.entries.len(), 3);
    assert_eq!(published.entries[0].track_id, fixture.first);
    assert_eq!(published.entries[1].track_id, fixture.first);
    assert_eq!(published.entries[2].track_id, fixture.second);
    assert_eq!(published.entries[0].source_position, Some(0));
    assert_eq!(published.entries[1].source_position, Some(1));
    assert_eq!(published.cursor, 1);
    harness.fake.ack(&capabilities);

    let (actor, result) = handle.join().expect("actor action thread");
    assert!(result.is_ok(), "selection result: {:?}", result.err());
    harness.actor = Some(actor);
    let saved = temporary
        .open()
        .load_playback_session()
        .expect("read persisted queue")
        .expect("saved queue");
    assert_eq!(saved.queue.cursor, 1);
    assert_eq!(saved.queue.entries[0].track_id, fixture.first);
    assert_eq!(saved.queue.entries[1].track_id, fixture.first);
    assert_eq!(saved.queue.entries[1].source_position, Some(1));
}

#[test]
fn paused_next_load_ack_sets_play_when_ready_false_and_advances_duplicate_traversal() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    let queue = queue_for_duplicate_slot(&seed_db, &fixture);
    seed_db
        .save_playback_session(&PlaybackSessionCheckpoint {
            queue: queue.snapshot(),
            position_ms: 1_234,
        })
        .expect("save paused queue baseline");
    let actor_db = temporary.open();
    let native = playing_snapshot(fixture.first, 4, "paused", 1_234);
    let mut harness = ActorFixture::with_snapshot(actor_db, native.clone());
    let current_track = harness
        .actor()
        .database
        .get_track_summary(fixture.first)
        .expect("read current track")
        .expect("current track exists");
    harness.actor_mut().current_track = Some(current_track);
    harness.actor_mut().queue = Some(queue.clone());
    harness.publish_fixture_state();

    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| {
        actor.handle_action("paused-next", PlaybackAction::Next { natural: false })
    });
    let load = harness.fake.next_command();
    assert_eq!(load.op, "load");
    assert_eq!(
        load.track_id.as_deref(),
        Some(fixture.second.to_string().as_str())
    );
    assert_eq!(
        load.play_when_ready,
        Some(false),
        "paused navigation must not autostart"
    );
    assert_eq!(harness.fake.snapshot.state, "paused");
    harness.fake.ack(&load);
    let capabilities = harness.fake.next_command();
    assert_eq!(capabilities.op, "set_capabilities");
    harness.fake.ack(&capabilities);
    let (actor, result) = handle.join().expect("actor navigation thread");
    assert!(result.is_ok(), "next result: {:?}", result.err());
    harness.actor = Some(actor);
    let saved = temporary
        .open()
        .load_playback_session()
        .expect("read checkpoint")
        .expect("queue checkpoint");
    assert_eq!(saved.position_ms, 0);
    assert_eq!(saved.queue.cursor, 2);
    assert_eq!(saved.queue.entries[0].track_id, fixture.first);
    assert_eq!(saved.queue.entries[1].track_id, fixture.first);
    assert_eq!(saved.queue.entries[2].track_id, fixture.second);
}

#[test]
fn failed_load_keeps_queue_cursor_and_saved_position_then_play_reloads_logical_current() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    let queue = super::queue_for_track(
        &seed_db,
        fixture.first,
        Some(playlist_source(fixture.playlist_id, 0)),
    )
    .expect("construct baseline queue");
    seed_db
        .save_playback_session(&PlaybackSessionCheckpoint {
            queue: queue.snapshot(),
            position_ms: 2_345,
        })
        .expect("save baseline position");
    drop(seed_db);

    let native = playing_snapshot(fixture.first, 3, "playing", 2_345);
    let mut harness = ActorFixture::with_snapshot(temporary.open(), native.clone());
    let current_track = harness
        .actor()
        .database
        .get_track_summary(fixture.first)
        .expect("read current track")
        .expect("track exists");
    harness.actor_mut().current_track = Some(current_track);
    harness.actor_mut().queue = Some(queue);
    harness.actor_mut().meter.observe(&native, Instant::now());
    harness.publish_fixture_state();

    let selected_track = fixture.second.to_string();
    let source = playlist_source(fixture.playlist_id, 2);
    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, move |actor| {
        actor.handle_action(
            "failed-replacement-load",
            PlaybackAction::SelectTrack {
                track_id: selected_track,
                source: Some(source),
            },
        )
    });
    let failed_load = harness.fake.next_command();
    assert_eq!(failed_load.op, "load");
    let load_error = "Android source became unavailable during load.";
    harness.fake.fail_load(&failed_load, load_error);
    let (actor, result) = handle.join().expect("failed replacement action thread");
    assert_eq!(result.err().as_deref(), Some(load_error));
    harness.actor = Some(actor);

    let failed_snapshot = harness.actor().publish_snapshot();
    assert!(!failed_snapshot.is_playing);
    assert_eq!(failed_snapshot.state, "ready");
    assert_eq!(harness.actor().native.track_id, None);
    assert_eq!(harness.actor().logical_position_ms(), 2_345);
    let saved = temporary
        .open()
        .load_playback_session()
        .expect("read queue after failed load")
        .expect("baseline queue remains persisted");
    assert_eq!(saved.queue.cursor, 0, "failed target cannot commit cursor");
    assert_eq!(saved.queue.entries[0].track_id, fixture.first);
    assert_eq!(saved.position_ms, 2_345);
    harness
        .actor_mut()
        .flush_statistics()
        .expect("flush stopped segment");
    let stats = temporary
        .open()
        .playback_statistics_for(&[fixture.first, fixture.second])
        .expect("read totals after failed load");
    assert_eq!(stats[&fixture.first].played_ms, 0);
    assert_eq!(stats[&fixture.second].played_ms, 0);

    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| {
        actor.handle_action("retry-play", PlaybackAction::Play)
    });
    let retry = harness.fake.next_command();
    assert_eq!(
        retry.op, "load",
        "Play must retry logical current, not unloaded native"
    );
    assert_eq!(
        retry.track_id.as_deref(),
        Some(fixture.first.to_string().as_str())
    );
    assert_eq!(retry.position_ms, Some(2_345));
    assert_eq!(retry.play_when_ready, Some(true));
    harness.fake.ack(&retry);
    let capabilities = harness.fake.next_command();
    assert_eq!(capabilities.op, "set_capabilities");
    harness.fake.ack(&capabilities);
    let (actor, result) = handle.join().expect("retry Play thread");
    assert!(result.is_ok(), "retry result: {:?}", result.err());
    harness.actor = Some(actor);
    assert_eq!(
        harness.actor().native.track_id.as_deref(),
        Some(fixture.first.to_string().as_str())
    );
    assert_eq!(harness.actor().native.state, "playing");
}

#[test]
fn seek_ack_updates_only_session_position_and_does_not_change_listening_totals() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    let queue = queue_for_duplicate_slot(&seed_db, &fixture);
    seed_db
        .save_playback_session(&PlaybackSessionCheckpoint {
            queue: queue.snapshot(),
            position_ms: 1_234,
        })
        .expect("save seek baseline");
    let baseline_queue = queue.snapshot();
    let native = playing_snapshot(fixture.first, 7, "playing", 1_234);
    let mut harness = ActorFixture::with_snapshot(temporary.open(), native.clone());
    let current_track = harness
        .actor()
        .database
        .get_track_summary(fixture.first)
        .expect("read current track")
        .expect("track exists");
    harness.actor_mut().current_track = Some(current_track);
    harness.actor_mut().queue = Some(queue);
    harness.actor_mut().meter.observe(&native, Instant::now());
    harness.publish_fixture_state();

    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| {
        actor.handle_action("seek-ack", PlaybackAction::Seek(4_321))
    });
    let seek = harness.fake.next_command();
    assert_eq!(seek.op, "seek");
    assert_eq!(seek.position_ms, Some(4_321));
    let before_ack = temporary
        .open()
        .load_playback_session()
        .expect("read baseline before ACK")
        .expect("baseline exists");
    assert_eq!(before_ack.position_ms, 1_234);
    harness.fake.ack(&seek);
    let (actor, result) = handle.join().expect("actor seek thread");
    assert!(result.is_ok(), "seek result: {:?}", result.err());
    harness.actor = Some(actor);

    let saved = temporary
        .open()
        .load_playback_session()
        .expect("read saved position")
        .expect("position checkpoint exists");
    assert_eq!(saved.position_ms, 4_321);
    assert_eq!(
        saved.queue, baseline_queue,
        "seek must not mutate queue traversal"
    );
    let stats = temporary
        .open()
        .playback_statistics_for(&[fixture.first])
        .expect("read listening totals");
    assert_eq!(
        stats[&fixture.first].played_ms, 0,
        "seeking contributes no listening time"
    );
}

#[test]
fn unavailable_restore_stays_ready_then_reloads_at_saved_position_without_autoplay() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    let queue = queue_for_duplicate_slot(&seed_db, &fixture);
    seed_db
        .save_playback_session(&PlaybackSessionCheckpoint {
            queue: queue.snapshot(),
            position_ms: 3_456,
        })
        .expect("save restore baseline");
    let mut disabled_root = fixture.root.clone();
    disabled_root.enabled = false;
    seed_db
        .save_library_root(&disabled_root)
        .expect("temporarily disable source");
    drop(seed_db);

    let mut harness = ActorFixture::new(temporary.open());
    harness.actor_mut().restore_session();
    assert_eq!(harness.actor().native.state, "ready");
    assert!(
        harness.actor().queue.is_some(),
        "unavailable source retains queue"
    );
    assert_eq!(harness.actor().logical_position_ms(), 3_456);
    harness.fake.no_command();

    let mut enabled_root = fixture.root.clone();
    enabled_root.enabled = true;
    temporary
        .open()
        .save_library_root(&enabled_root)
        .expect("restore source availability");
    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| actor.restore_session());
    let load = harness.fake.next_command();
    assert_eq!(load.op, "load");
    assert_eq!(load.position_ms, Some(3_456));
    assert_eq!(load.play_when_ready, Some(false));
    harness.fake.ack(&load);
    let capabilities = harness.fake.next_command();
    assert_eq!(capabilities.op, "set_capabilities");
    harness.fake.ack(&capabilities);
    let (actor, ()) = handle.join().expect("restore actor thread");
    harness.actor = Some(actor);
    assert_eq!(harness.actor().native.state, "ready");
    assert_eq!(harness.actor().native.position_ms, 3_456);

    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| {
        actor.handle_action("explicit-play", PlaybackAction::Play)
    });
    let play = harness.fake.next_command();
    assert_eq!(play.op, "play", "audio starts only after explicit Play");
    harness.fake.ack(&play);
    let (actor, result) = handle.join().expect("explicit Play thread");
    assert!(result.is_ok(), "Play result: {:?}", result.err());
    harness.actor = Some(actor);
    assert_eq!(harness.actor().native.state, "playing");
}
