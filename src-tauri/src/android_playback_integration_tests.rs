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
            created_at_utc_ms: None,
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

    /// The service refused the load before switching to it (its file could not be opened): the track that plays is
    /// left alone, which the acknowledgement says with `retained`.
    fn refuse_load(&mut self, command: &NativeCommand, error: &str) {
        assert_eq!(command.op, "load");
        self.event_id = self.event_id.saturating_add(1).max(1);
        let event_json = serde_json::json!({
            "v": 1,
            "kind": "ack",
            "eventId": self.event_id.to_string(),
            "commandId": command.command_id,
            "requestId": command.request_id,
            "serviceGeneration": self.snapshot.service_generation,
            "error": error,
            "retained": true,
            "snapshot": self.snapshot,
        });
        let event: NativeEvent =
            serde_json::from_value(event_json).expect("Kotlin-compatible refused load ACK DTO");
        self.event_sender
            .send(event)
            .expect("send fake refused load ACK");
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
        sort: Default::default(),
    }
}

fn queue_for_duplicate_slot(database: &Database, fixture: &FixtureTracks) -> PlaybackQueue {
    super::queue_for_track(
        database,
        player_db::LibrarySourceScope::All,
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
        scope: player_db::LibrarySourceScope::All,
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

/// Adds a remote source with one song beside the local fixture songs; returns that song.
fn seed_remote_song(database: &mut Database) -> TrackId {
    let root = database
        .add_library_root(
            MediaSourceKind::Remote,
            "fake NAS",
            MediaLocator::Remote {
                source_id: player_core::SourceId::new(),
                path: String::new(),
            },
        )
        .expect("add remote root");
    let locator = MediaLocator::Remote {
        source_id: root.id,
        path: "Music/cloud.flac".to_owned(),
    };
    let record = MediaTrackRecord {
        identity: TrackIdentity {
            source_id: root.id,
            source_item_id: "cloud".to_owned(),
            locator_key: Some("remote-acct:fake:Music/cloud.flac".to_owned()),
        },
        locator: locator.clone(),
        fingerprint: FileFingerprint {
            size_bytes: 128,
            modified_at_utc_ms: Some(1_800_000_000_000),
            created_at_utc_ms: None,
        },
        metadata: Some(TrackMetadata {
            title: Some("Cloud Track".to_owned()),
            artist: Some("Actor Test".to_owned()),
            duration_ms: Some(8_000),
            ..TrackMetadata::default()
        }),
    };
    database
        .apply_source_scan(
            &root,
            &SourceScanState::Complete,
            std::slice::from_ref(&record),
            std::slice::from_ref(&record),
            &[],
            1,
        )
        .expect("persist remote song");
    database
        .resolve_track_id_for_locator(&locator)
        .expect("resolve remote song")
        .expect("remote song ID")
}

fn library_source() -> PlaybackQueueSource {
    PlaybackQueueSource::Library {
        query: None,
        query_alternates: Vec::new(),
        field_filter: None,
        sort: Default::default(),
    }
}

/// The user's report: in local mode a song picked on Android started a queue holding cloud songs. The actor reads the
/// library through its own database handle; the scope is the queue query's argument, carried by the action, so no
/// handle has a scope of its own to fall out of step with the lists.
#[test]
fn the_actor_builds_a_list_queue_under_the_scope_the_lists_show() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    let cloud = seed_remote_song(&mut seed_db);
    let actor_db = temporary.open();
    // The same query on the app's handle and on the actor's: the scope argument alone decides the rows.
    let everything = super::queue_for_track(
        &actor_db,
        player_db::LibrarySourceScope::All,
        fixture.first,
        Some(library_source()),
    )
    .expect("queue of every source");
    assert!(everything
        .snapshot()
        .entries
        .iter()
        .any(|entry| entry.track_id == cloud));
    let page: Vec<_> = seed_db
        .list_tracks_page(
            player_db::LibrarySourceScope::Local,
            player_core::ListTracksQuery {
                offset: 0,
                limit: 100,
                query: None,
                field_filter: None,
                sort: Default::default(),
            },
        )
        .expect("local page")
        .items
        .into_iter()
        .map(|track| track.id)
        .collect();
    assert!(!page.contains(&cloud));
    let local_on_actor_handle = super::queue_for_track(
        &actor_db,
        player_db::LibrarySourceScope::Local,
        fixture.first,
        Some(library_source()),
    )
    .expect("local queue on the actor's handle");
    assert_eq!(
        local_on_actor_handle
            .snapshot()
            .entries
            .iter()
            .map(|entry| entry.track_id)
            .collect::<Vec<_>>(),
        page
    );
    drop(seed_db);

    let mut harness = ActorFixture::new(actor_db);
    let actor = harness.take_actor();
    let action = PlaybackAction::SelectTrack {
        track_id: fixture.first.to_string(),
        source: Some(library_source()),
        scope: player_db::LibrarySourceScope::Local,
    };
    let handle = perform_on_thread(actor, move |actor| {
        actor.handle_action("local-pick", action)
    });
    let load = harness.fake.next_command();
    assert_eq!(load.op, "load");
    harness.fake.ack(&load);
    let capabilities = harness.fake.next_command();
    let published = harness
        .shared
        .queue
        .read()
        .expect("queue lock")
        .clone()
        .expect("queue commits after the load");
    let queued: Vec<_> = published
        .entries
        .iter()
        .map(|entry| entry.track_id)
        .collect();
    assert_eq!(queued, page, "the queue holds exactly the local page");
    harness.fake.ack(&capabilities);
    let (_actor, result) = handle.join().expect("actor action thread");
    assert!(result.is_ok(), "selection result: {:?}", result.err());
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
        player_db::LibrarySourceScope::All,
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
                scope: player_db::LibrarySourceScope::All,
            },
        )
    });
    let failed_load = harness.fake.next_command();
    assert_eq!(failed_load.op, "load");
    let load_error = "Android source became unavailable during load.";
    harness.fake.fail_load(&failed_load, load_error);
    let (actor, result) = handle.join().expect("failed replacement action thread");
    // A load that failed after the switch (the track that played is gone) is reported in words the front end shows.
    assert_eq!(
        result.err().as_deref(),
        Some(player_core::TRACK_UNPLAYABLE),
        "the native text was {load_error}"
    );
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

#[test]
fn a_seek_buffering_snapshot_keeps_playing_without_flipping_resume_or_counting_time() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    let native = playing_snapshot(fixture.first, 7, "playing", 1_000);
    let mut harness = ActorFixture::with_snapshot(temporary.open(), native.clone());
    let reports = Arc::new(Mutex::new(Vec::<bool>::new()));
    let sink_reports = Arc::clone(&reports);
    harness.actor_mut().resume_reported.set(true);
    harness.actor_mut().resume_sink = Some(Arc::new(move |playing| sink_reports.lock().unwrap().push(playing)));
    let start = Instant::now();
    harness.actor_mut().meter.observe(&native, start);

    // What Kotlin sends right after seekTo while playing (playWhenReady, STATE_BUFFERING).
    let event: NativeEvent = serde_json::from_str(&format!(
        r#"{{"v":1,"kind":"snapshot","eventId":"9","serviceGeneration":1,"snapshot":{{"serviceGeneration":1,"loadInstance":7,"trackId":"{}","mediaKey":"load-7","state":"buffering","positionMs":4000,"durationMs":8000,"volume":1.0,"error":null}}}}"#,
        fixture.first
    ))
    .expect("Kotlin buffering snapshot fixture");
    let buffering = event.snapshot.expect("snapshot");
    assert_eq!(buffering.state, "buffering");
    harness.actor_mut().observe_snapshot(buffering.clone(), start + Duration::from_millis(100));
    let published = harness.actor().publish_snapshot();
    assert!(published.is_playing, "buffering after a seek keeps the play button on pause");
    assert_eq!(published.state, "buffering");
    assert!(reports.lock().unwrap().is_empty(), "resumePlayback must not flip during a seek");

    // Buffering for a second, then playing again from 4000.
    let resumed = NativePlaybackSnapshot { state: "playing".into(), ..buffering.clone() };
    harness.actor_mut().observe_snapshot(resumed.clone(), start + Duration::from_millis(1_100));
    harness.actor_mut().observe_snapshot(
        NativePlaybackSnapshot { position_ms: 4_200, ..resumed },
        start + Duration::from_millis(1_300),
    );
    let counted = harness.actor().meter.totals.get(&fixture.first).map_or(0, |total| total.played_ms);
    // 100 ms playing before the seek + 200 ms after it; the 1000 ms of buffering in between is not counted.
    assert_eq!(counted, 300, "buffering time is not listening time");
    assert!(reports.lock().unwrap().is_empty(), "still playing: nothing to record");
}
#[test]
fn a_newer_choice_of_track_takes_over_from_a_load_that_is_still_awaited() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    drop(seed_db);
    let mut harness = ActorFixture::new(temporary.open());
    let actor = harness.take_actor();

    // The first track is a slow remote one: its load is sent and never acknowledged.
    let first = fixture.first.to_string();
    let handle = perform_on_thread(actor, move |actor| {
        actor.handle_action(
            "slow",
            PlaybackAction::select_track(first, None, player_db::LibrarySourceScope::All),
        )
    });
    let stale_load = harness.fake.next_command();
    assert_eq!(stale_load.op, "load");

    // The user chooses a local track 0.4 s later; the actor must not wait out the stale load.
    let (response, answer) = mpsc::sync_channel(1);
    harness
        .shared
        .requests
        .try_send(ActorRequest::Command {
            request_id: "local".to_owned(),
            action: PlaybackAction::select_track(
                fixture.second.to_string(),
                None,
                player_db::LibrarySourceScope::All,
            ),
            response,
        })
        .expect("queue the newer choice");
    let started = Instant::now();
    let (mut actor, result) = handle.join().expect("actor action thread");
    assert_eq!(result.err().as_deref(), Some(SUPERSEDED_MESSAGE));
    assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
    assert!(answer.try_recv().is_err(), "the newer request is still to be handled");
    // Nothing of the stale load was committed.
    assert!(actor.current_track.is_none());
    assert!(actor.queue.is_none());

    // The service is told the stale load is not wanted (so a file it is still checking is not switched to).
    let cancel = harness.fake.next_command();
    assert_eq!(cancel.op, "cancel_load");
    assert_eq!(cancel.target_command_id.as_deref(), Some(stale_load.command_id.as_str()));
    // The stale load is acknowledged late: it is ignored.
    harness.fake.ack(&stale_load);

    let Ok(ActorRequest::Command { request_id, action, .. }) =
        actor.next_request(Duration::from_millis(10))
    else {
        panic!("the newer request was kept for the actor");
    };
    assert_eq!(request_id, "local");
    let handle = perform_on_thread(actor, move |actor| actor.handle_action("local", action));
    let load = harness.fake.next_command();
    assert_eq!(load.op, "load");
    assert_eq!(load.track_id.as_deref(), Some(fixture.second.to_string().as_str()));
    harness.fake.ack(&load);
    let capabilities = harness.fake.next_command();
    assert_eq!(capabilities.op, "set_capabilities");
    harness.fake.ack(&capabilities);
    let (actor, result) = handle.join().expect("actor action thread");
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(actor.current_track.as_ref().map(|track| track.id), Some(fixture.second));
}

#[test]
fn a_pause_during_a_load_does_not_cancel_it() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    drop(seed_db);
    let mut harness = ActorFixture::new(temporary.open());
    let actor = harness.take_actor();
    let first = fixture.first.to_string();
    let handle = perform_on_thread(actor, move |actor| {
        actor.handle_action(
            "slow",
            PlaybackAction::select_track(first, None, player_db::LibrarySourceScope::All),
        )
    });
    let load = harness.fake.next_command();
    let (response, _answer) = mpsc::sync_channel(1);
    harness
        .shared
        .requests
        .try_send(ActorRequest::Command {
            request_id: "pause".to_owned(),
            action: PlaybackAction::Pause,
            response,
        })
        .expect("queue a pause");
    harness.fake.no_command();
    harness.fake.ack(&load);
    let capabilities = harness.fake.next_command();
    harness.fake.ack(&capabilities);
    let (actor, result) = handle.join().expect("actor action thread");
    assert!(result.is_ok(), "{:?}", result.err());
    assert_eq!(actor.held_requests.len(), 1, "the pause waits its turn");
}

#[test]
fn a_failed_load_of_a_remote_track_says_the_server_is_unreachable() {
    for error in [
        "Android media source did not become ready within 8000ms",
        "伺服器無法連線",
        "Android 播放服務沒有在期限內確認命令。",
        "無法連線到伺服器：No route to host",
    ] {
        assert!(remote_load_failure_is_unreachable(error), "{error}");
    }
    assert!(!remote_load_failure_is_unreachable(SUPERSEDED_MESSAGE));
    assert!(!remote_load_failure_is_unreachable("Source error"));
}

/// An actor playing the first entry of the duplicate-entry playlist (`first`, `first`, `second`).
fn playing_first_entry(
    temporary: &TemporaryDatabase,
    fixture: &FixtureTracks,
    repeat: QueueRepeatMode,
) -> ActorFixture {
    let seed_db = temporary.open();
    let mut queue = super::queue_for_track(
        &seed_db,
        player_db::LibrarySourceScope::All,
        fixture.first,
        Some(playlist_source(fixture.playlist_id, 0)),
    )
    .expect("construct queue from the first playlist slot");
    queue.set_repeat_mode(repeat);
    seed_db
        .save_playback_session(&PlaybackSessionCheckpoint {
            queue: queue.snapshot(),
            position_ms: 1_000,
        })
        .expect("save the baseline session");
    let native = playing_snapshot(fixture.first, 3, "playing", 1_000);
    let mut harness = ActorFixture::with_snapshot(temporary.open(), native.clone());
    let current_track = harness
        .actor()
        .database
        .get_track_summary(fixture.first)
        .expect("read the current track")
        .expect("the track exists");
    harness.actor_mut().current_track = Some(current_track);
    harness.actor_mut().queue = Some(queue);
    harness.actor_mut().meter.observe(&native, Instant::now());
    let runtime_id = harness.actor().runtime_id;
    harness
        .actor()
        .database
        .register_playback_statistics_runtime(runtime_id, Some(std::process::id()), None)
        .expect("register the statistics runtime");
    harness.publish_fixture_state();
    harness
}

#[test]
fn a_load_the_service_refuses_leaves_the_playing_track_the_queue_and_the_session_alone() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    drop(seed_db);
    let mut harness = playing_first_entry(&temporary, &fixture, QueueRepeatMode::Off);
    let before = harness.actor().publish_snapshot();
    assert!(before.start_failure.is_none());

    let selected = fixture.second.to_string();
    let source = playlist_source(fixture.playlist_id, 2);
    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, move |actor| {
        actor.handle_action(
            "tap-a-bad-track",
            PlaybackAction::SelectTrack {
                track_id: selected,
                source: Some(source),
                scope: player_db::LibrarySourceScope::All,
            },
        )
    });
    let load = harness.fake.next_command();
    assert_eq!(load.op, "load");
    harness.fake.refuse_load(&load, "Cannot open the file");
    let (actor, result) = handle.join().expect("tap thread");
    // One attempt: the notice text, no other command (no skipping to a next entry).
    assert_eq!(result.err().as_deref(), Some(player_core::TRACK_UNPLAYABLE));
    harness.fake.no_command();
    harness.actor = Some(actor);

    // The music that plays is untouched.
    let after = harness.actor().publish_snapshot();
    assert_eq!(
        harness.actor().native.track_id.as_deref(),
        Some(fixture.first.to_string().as_str())
    );
    assert_eq!(harness.actor().native.state, "playing");
    assert_eq!(after.state, "playing");
    assert!(after.is_playing);
    assert_eq!(
        after.current_track.as_ref().map(|track| track.id),
        Some(fixture.first)
    );
    assert_eq!(after.position_ms, 1_000);
    assert!(after.last_error.is_none(), "{:?}", after.last_error);
    assert_eq!(
        harness.actor().queue.as_ref().map(PlaybackQueue::cursor),
        Some(0)
    );
    // The failure is noted once for the front end.
    let notice = after.start_failure.expect("the failure is in the snapshot");
    assert_eq!((notice.kind, notice.sequence), ("unplayable", 1));
    // The session was not rewritten.
    let saved = temporary
        .open()
        .load_playback_session()
        .expect("read the session")
        .expect("the baseline session");
    assert_eq!(saved.queue.cursor, 0);
    assert_eq!(saved.position_ms, 1_000);
    // The listening of the track that plays goes on and counts.
    let playing_later = NativePlaybackSnapshot {
        position_ms: 1_600,
        ..harness.actor().native.clone()
    };
    harness
        .actor_mut()
        .meter
        .observe(&playing_later, Instant::now() + Duration::from_millis(600));
    assert!(
        harness.actor().meter.anchor.is_some(),
        "the meter still follows the playing track"
    );
}

#[test]
fn a_server_that_cannot_be_reached_is_told_as_such_and_changes_nothing() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    drop(seed_db);
    let mut harness = playing_first_entry(&temporary, &fixture, QueueRepeatMode::Off);
    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| {
        actor.handle_action("next", PlaybackAction::Next { natural: false })
    });
    let load = harness.fake.next_command();
    harness
        .fake
        .refuse_load(&load, player_core::REMOTE_LINK_LOST);
    let (actor, result) = handle.join().expect("next thread");
    assert_eq!(result.err().as_deref(), Some(player_core::REMOTE_LINK_LOST));
    harness.fake.no_command();
    harness.actor = Some(actor);
    let after = harness.actor().publish_snapshot();
    assert_eq!(after.state, "playing");
    assert_eq!(
        after.start_failure.map(|notice| notice.kind),
        Some("unreachable")
    );
    assert_eq!(
        harness.actor().queue.as_ref().map(PlaybackQueue::cursor),
        Some(0)
    );
}

#[test]
fn when_a_track_ends_the_walk_goes_past_a_failure_to_the_next_entry_that_loads() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    drop(seed_db);
    let mut harness = playing_first_entry(&temporary, &fixture, QueueRepeatMode::Off);
    harness.actor_mut().native.state = "ended".to_owned();
    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| actor.navigate("end", true, true));

    // Entry 1 (the same file again) cannot be opened.
    let first_try = harness.fake.next_command();
    assert_eq!(first_try.op, "load");
    assert_eq!(first_try.play_when_ready, Some(true));
    harness.fake.refuse_load(&first_try, "Cannot open the file");
    // The walk goes on: entry 2 opens.
    let second_try = harness.fake.next_command();
    assert_eq!(second_try.op, "load");
    assert_eq!(
        second_try.track_id.as_deref(),
        Some(fixture.second.to_string().as_str())
    );
    harness.fake.ack(&second_try);
    let capabilities = harness.fake.next_command();
    assert_eq!(capabilities.op, "set_capabilities");
    harness.fake.ack(&capabilities);
    let (actor, result) = handle.join().expect("walk thread");
    assert!(result.is_ok(), "{:?}", result.err());
    harness.actor = Some(actor);

    // Only the entry that started was committed.
    let queue = harness.actor().queue.as_ref().expect("queue");
    assert_eq!(queue.cursor(), 2);
    assert_eq!(
        harness.actor().current_track.as_ref().map(|track| track.id),
        Some(fixture.second)
    );
    assert!(
        harness.actor().start_failures.1.is_none(),
        "an entry started: nothing to report"
    );
    let saved = temporary
        .open()
        .load_playback_session()
        .expect("read the session")
        .expect("session");
    assert_eq!(saved.queue.cursor, 2);
}

#[test]
fn a_seek_past_the_end_is_the_end_of_the_track_and_the_walk_starts_the_next_entry() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    drop(seed_db);
    let mut harness = playing_first_entry(&temporary, &fixture, QueueRepeatMode::Off);
    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| {
        actor.handle_action("seek-end", PlaybackAction::Seek(9_999))
    });
    // The track that is over is silenced at its end (clamped to the duration), never sought past it.
    let pause = harness.fake.next_command();
    assert_eq!(pause.op, "pause");
    harness.fake.ack(&pause);
    let seek = harness.fake.next_command();
    assert_eq!(seek.op, "seek");
    assert_eq!(seek.position_ms, Some(8_000));
    harness.fake.ack(&seek);
    // Then the natural-end walk: the next entry starts playing.
    let load = harness.fake.next_command();
    assert_eq!(load.op, "load");
    assert_eq!(load.play_when_ready, Some(true));
    harness.fake.ack(&load);
    let capabilities = harness.fake.next_command();
    assert_eq!(capabilities.op, "set_capabilities");
    harness.fake.ack(&capabilities);
    let (actor, result) = handle.join().expect("seek thread");
    let snapshot = result.expect("a seek to the end is not an error");
    harness.actor = Some(actor);
    assert_eq!(snapshot.state, "playing");
    assert!(snapshot.last_error.is_none(), "{:?}", snapshot.last_error);
    assert_eq!(
        harness.actor().queue.as_ref().map(PlaybackQueue::cursor),
        Some(1)
    );
}

#[test]
fn a_seek_to_the_end_with_nothing_to_go_on_to_rests_at_the_end_without_an_error() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    drop(seed_db);
    let mut harness = playing_first_entry(&temporary, &fixture, QueueRepeatMode::Off);
    harness.actor_mut().queue = None;
    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| {
        actor.handle_action("seek-end", PlaybackAction::Seek(8_000))
    });
    for op in ["pause", "seek"] {
        let command = harness.fake.next_command();
        assert_eq!(command.op, op);
        harness.fake.ack(&command);
    }
    let (actor, result) = handle.join().expect("seek thread");
    let snapshot = result.expect("a seek to the end is not an error");
    harness.fake.no_command();
    harness.actor = Some(actor);
    assert_eq!(snapshot.state, "ended");
    assert_eq!(snapshot.position_ms, 8_000);
    assert!(!snapshot.is_playing);
    assert!(snapshot.last_error.is_none(), "{:?}", snapshot.last_error);
    assert!(snapshot.start_failure.is_none());
}

#[test]
fn a_seek_to_the_end_while_paused_loads_the_next_entry_paused_at_its_start() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    drop(seed_db);
    let mut harness = playing_first_entry(&temporary, &fixture, QueueRepeatMode::Off);
    harness.actor_mut().native.state = "paused".to_owned();
    harness.fake.snapshot.state = "paused".to_owned();
    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| {
        actor.handle_action("seek-end", PlaybackAction::Seek(8_000))
    });
    for op in ["pause", "seek"] {
        let command = harness.fake.next_command();
        assert_eq!(command.op, op);
        harness.fake.ack(&command);
    }
    // The natural-end walk, but the entry is loaded without playing, at its start.
    let load = harness.fake.next_command();
    assert_eq!(load.op, "load");
    assert_eq!(load.play_when_ready, Some(false));
    assert_eq!(load.position_ms.unwrap_or(0), 0);
    harness.fake.ack(&load);
    let capabilities = harness.fake.next_command();
    assert_eq!(capabilities.op, "set_capabilities");
    harness.fake.ack(&capabilities);
    let (actor, result) = handle.join().expect("seek thread");
    let snapshot = result.expect("a seek to the end is not an error");
    harness.actor = Some(actor);
    assert!(!snapshot.is_playing, "{}", snapshot.state);
    assert_eq!(snapshot.position_ms, 0);
    assert!(snapshot.last_error.is_none(), "{:?}", snapshot.last_error);
    assert_eq!(
        harness.actor().queue.as_ref().map(PlaybackQueue::cursor),
        Some(1)
    );
}

#[test]
fn after_a_seek_to_the_end_the_service_ended_event_does_not_walk_again() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    drop(seed_db);
    let mut harness = playing_first_entry(&temporary, &fixture, QueueRepeatMode::Off);
    harness.actor_mut().native.state = "paused".to_owned();
    harness.fake.snapshot.state = "paused".to_owned();
    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| {
        actor.handle_action("seek-end", PlaybackAction::Seek(8_000))
    });
    for op in ["pause", "seek"] {
        let command = harness.fake.next_command();
        assert_eq!(command.op, op);
        harness.fake.ack(&command);
    }
    // Neither remaining entry opens: the player rests at the end of the first track.
    for _ in 0..2 {
        let attempt = harness.fake.next_command();
        assert_eq!(attempt.op, "load");
        assert_eq!(attempt.play_when_ready, Some(false));
        harness.fake.refuse_load(&attempt, "Cannot open the file");
    }
    let (actor, result) = handle.join().expect("seek thread");
    let snapshot = result.expect("a seek to the end is not an error");
    harness.actor = Some(actor);
    assert_eq!(snapshot.state, "ended");
    assert!(snapshot.last_error.is_none(), "{:?}", snapshot.last_error);

    // ExoPlayer then reports the end of the same item: that is not a second natural end.
    let ended: NativeEvent = serde_json::from_value(serde_json::json!({
        "v": 1,
        "kind": "ended",
        "eventId": "1000",
        "serviceGeneration": 1,
        "trackId": fixture.first.to_string(),
        "mediaKey": "load-3",
        "loadInstance": 3,
    }))
    .expect("Kotlin-compatible ended event");
    harness.actor_mut().process_transport_event(ended);
    harness.fake.no_command();
}

#[test]
fn a_seek_just_before_the_end_is_an_ordinary_seek() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    drop(seed_db);
    let mut harness = playing_first_entry(&temporary, &fixture, QueueRepeatMode::Off);
    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| {
        actor.handle_action("seek-tail", PlaybackAction::Seek(7_999))
    });
    let seek = harness.fake.next_command();
    assert_eq!(seek.op, "seek");
    assert_eq!(seek.position_ms, Some(7_999));
    harness.fake.ack(&seek);
    let (actor, result) = handle.join().expect("seek thread");
    assert_eq!(result.expect("seek").state, "playing");
    harness.fake.no_command();
    harness.actor = Some(actor);
}

#[test]
fn when_every_entry_fails_the_player_rests_at_the_end_and_the_last_failure_is_reported() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    drop(seed_db);
    let mut harness = playing_first_entry(&temporary, &fixture, QueueRepeatMode::Off);
    harness.actor_mut().native.state = "ended".to_owned();
    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| actor.navigate("end", true, true));
    for message in ["Cannot open the file", player_core::REMOTE_LINK_LOST] {
        let attempt = harness.fake.next_command();
        assert_eq!(attempt.op, "load");
        harness.fake.refuse_load(&attempt, message);
    }
    let (actor, result) = handle.join().expect("walk thread");
    assert!(result.is_ok(), "{:?}", result.err());
    harness.fake.no_command();
    harness.actor = Some(actor);
    let snapshot = harness.actor().publish_snapshot();
    assert_eq!(snapshot.state, "ended");
    assert!(!snapshot.is_playing);
    // The unreachable server is what the user is told, whichever failed last.
    assert_eq!(
        snapshot.start_failure.map(|notice| notice.kind),
        Some("unreachable")
    );
    assert_eq!(
        harness.actor().queue.as_ref().map(PlaybackQueue::cursor),
        Some(0)
    );
}

#[test]
fn repeat_one_retries_only_the_same_track_and_repeat_off_stops_after_the_last_entry() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    drop(seed_db);

    // Repeat-one: one attempt at the track that ended, no walking.
    let mut harness = playing_first_entry(&temporary, &fixture, QueueRepeatMode::One);
    harness.actor_mut().native.state = "ended".to_owned();
    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| actor.navigate("end", true, true));
    let attempt = harness.fake.next_command();
    assert_eq!(
        attempt.track_id.as_deref(),
        Some(fixture.first.to_string().as_str())
    );
    harness.fake.refuse_load(&attempt, "Cannot open the file");
    let (actor, result) = handle.join().expect("walk thread");
    assert!(result.is_ok());
    harness.fake.no_command();
    assert_eq!(actor.publish_snapshot().state, "ended");

    // Repeat-off at the last entry: nothing to try, an ordinary end without a notice.
    let mut harness = playing_first_entry(&temporary, &fixture, QueueRepeatMode::Off);
    harness
        .actor_mut()
        .queue
        .as_mut()
        .expect("queue")
        .set_cursor(2)
        .expect("last entry");
    harness.actor_mut().native.state = "ended".to_owned();
    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| actor.navigate("end", true, true));
    let (actor, result) = handle.join().expect("walk thread");
    assert!(result.is_ok());
    harness.fake.no_command();
    let snapshot = actor.publish_snapshot();
    assert_eq!(snapshot.state, "ended");
    assert!(snapshot.start_failure.is_none());
}

#[test]
fn a_choice_of_the_user_ends_the_walk_before_its_next_entry() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    drop(seed_db);
    let mut harness = playing_first_entry(&temporary, &fixture, QueueRepeatMode::Off);
    harness.actor_mut().native.state = "ended".to_owned();
    let actor = harness.take_actor();
    let handle = perform_on_thread(actor, |actor| actor.navigate("end", true, true));
    let attempt = harness.fake.next_command();
    assert_eq!(attempt.op, "load");
    // The user taps a track while the first entry is still being opened.
    let (response, _answer) = mpsc::sync_channel(1);
    harness
        .shared
        .requests
        .try_send(ActorRequest::Command {
            request_id: "tap".to_owned(),
            action: PlaybackAction::select_track(
                fixture.second.to_string(),
                None,
                player_db::LibrarySourceScope::All,
            ),
            response,
        })
        .expect("queue the user's choice");
    let (actor, result) = handle.join().expect("walk thread");
    assert_eq!(result.err().as_deref(), Some(SUPERSEDED_MESSAGE));
    assert_eq!(harness.fake.next_command().op, "cancel_load");
    assert_eq!(
        actor.held_requests.len(),
        1,
        "the user's choice is handled next"
    );
    assert!(
        actor.start_failures.1.is_none(),
        "a replaced start reports nothing"
    );
}

fn external_entry(
    name: &str,
    title: &str,
) -> (PlaybackQueueEntry, crate::external_open::ExternalTrack) {
    let id = TrackId::new();
    let path = PathBuf::from(format!("/storage/emulated/0/Opened/{name}"));
    let summary = crate::external_open::external_summary(
        &path,
        id,
        TrackMetadata {
            title: Some(title.to_owned()),
            duration_ms: Some(8_000),
            ..TrackMetadata::default()
        },
        None,
        None,
    );
    (
        PlaybackQueueEntry {
            track_id: id,
            source_position: None,
        },
        crate::external_open::ExternalTrack { path, summary },
    )
}

#[test]
fn a_file_another_app_opened_plays_at_once_as_the_current_entry_of_its_folders_queue() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    let actor_db = temporary.open();
    drop(seed_db);
    let (first_entry, first_track) = external_entry("1a.flac", "Opened A");
    let (second_entry, second_track) = external_entry("3c.flac", "Opened C");
    let library_entry = PlaybackQueueEntry {
        track_id: fixture.first,
        source_position: None,
    };
    let opened = first_entry.track_id;
    crate::external_open::registry().register([first_track, second_track]);

    let mut harness = ActorFixture::new(actor_db);
    let actor = harness.take_actor();
    let entries = vec![first_entry, library_entry, second_entry];
    let handle = perform_on_thread(actor, move |actor| {
        actor.handle_action(
            "open-with",
            PlaybackAction::OpenExternal {
                entries,
                selected_index: 0,
            },
        )
    });
    let load = harness.fake.next_command();
    assert_eq!(load.op, "load");
    assert_eq!(load.track_id.as_deref(), Some(opened.to_string().as_str()));
    let uri = load.content_uri.as_deref().expect("a URI to load");
    assert!(
        uri.starts_with("file://") || uri.starts_with("moeflac://"),
        "{uri}"
    );
    assert!(uri.ends_with("/Opened/1a.flac"), "{uri}");
    assert_eq!(
        load.play_when_ready,
        Some(true),
        "the opened file plays at once"
    );
    assert_eq!(
        load.metadata
            .as_ref()
            .and_then(|metadata| metadata.title.as_deref()),
        Some("Opened A")
    );
    harness.fake.ack(&load);
    let capabilities = harness.fake.next_command();
    harness.fake.ack(&capabilities);
    let (actor, result) = handle.join().expect("actor action thread");
    assert!(result.is_ok(), "open result: {:?}", result.err());
    let queue = actor.queue.as_ref().expect("queue");
    assert_eq!(queue.current(), opened);
    assert_eq!(queue.snapshot().entries.len(), 3);
    assert_eq!(
        actor.current_track.as_ref().map(|track| track.id),
        Some(opened)
    );
    // The queue is not saved: its tracks are not in the database, and the last library session stays.
    assert!(actor.database.load_playback_session().unwrap().is_none());
    // Navigation reaches the library track and the next opened file through the same lookups.
    let next = queue.preview_next(true).expect("next entry").0;
    assert_eq!(next, fixture.first);
    assert!(crate::external_open::registry().contains(opened));
}

#[test]
fn a_file_the_service_refuses_leaves_the_playing_queue_and_forgets_the_new_tracks() {
    let temporary = TemporaryDatabase::new();
    let mut seed_db = temporary.open();
    let fixture = seed_library(&mut seed_db);
    let actor_db = temporary.open();
    drop(seed_db);
    let (entry, track) = external_entry("x.flac", "Refused");
    let refused = entry.track_id;
    crate::external_open::registry().register([track]);

    let mut harness = ActorFixture::new(actor_db);
    let mut actor = harness.take_actor();
    actor.queue = PlaybackQueue::with_entries(
        vec![PlaybackQueueEntry {
            track_id: fixture.first,
            source_position: None,
        }],
        PlaybackQueueContext::Library {
            query: None,
            field_filter: None,
            sort: Default::default(),
        },
        0,
    );
    let handle = perform_on_thread(actor, move |actor| {
        actor.handle_action(
            "open-with",
            PlaybackAction::OpenExternal {
                entries: vec![entry],
                selected_index: 0,
            },
        )
    });
    let load = harness.fake.next_command();
    harness.fake.refuse_load(&load, "cannot open");
    let (actor, result) = handle.join().expect("actor action thread");
    assert!(result.is_err());
    assert_eq!(
        actor.queue.as_ref().map(PlaybackQueue::current),
        Some(fixture.first)
    );
    assert!(!crate::external_open::registry().contains(refused));
}
