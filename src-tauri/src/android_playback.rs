//! Process-lifetime Android playback bridge.
//!
//! Media3 owns the active decoder/output and MediaSession. This module owns
//! queue policy, session persistence, request ordering, and playback statistics.
//! The JNI entry points deliberately do not retain a Tauri `AppHandle`, so the
//! Rust worker remains alive while the foreground media service outlives the
//! Activity/WebView.

use std::{
    collections::{HashSet, VecDeque},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
        Arc, Mutex, OnceLock, RwLock,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use player_core::{
    MediaLocator, PlaybackCheckpoint, PlaybackQueue, PlaybackQueueSnapshot, QueueRepeatMode,
    QueueTrackListeningStats, TrackId, TrackSummary,
};
use player_db::{Database, PlaybackSessionCheckpoint};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::queue_for_track;
use crate::{
    queue_repeat_mode, set_queue_shuffle_with_latest_stats, PlaybackQueueSource, PlaybackSnapshot,
    RepeatMode,
};
#[cfg(target_os = "android")]
use std::sync::mpsc::TryRecvError;

const NATIVE_COMMAND_CAPACITY: usize = 64;
const ACTOR_REQUEST_CAPACITY: usize = 64;
const NATIVE_EVENT_CAPACITY: usize = 256;
const COMMAND_ACK_TIMEOUT: Duration = Duration::from_secs(15);
const STATS_FLUSH_INTERVAL: Duration = Duration::from_secs(5);

static PROCESS_SERVICE: OnceLock<Arc<SharedService>> = OnceLock::new();
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_COMMAND_ID: AtomicU64 = AtomicU64::new(1);

struct SharedService {
    requests: SyncSender<ActorRequest>,
    native_commands: Mutex<Receiver<NativeCommand>>,
    native_events: SyncSender<NativeEvent>,
    snapshot: RwLock<PlaybackSnapshot>,
    queue: RwLock<Option<PlaybackQueueSnapshot>>,
    queue_revision: AtomicU64,
}

/// Cloneable Tauri-facing handle. The actor itself is detached and retained by
/// the process-global service even if Tauri tears down and recreates its Activity.
#[derive(Clone)]
pub(crate) struct AndroidPlaybackService(Arc<SharedService>);

#[derive(Debug)]
enum ActorRequest {
    Command {
        request_id: String,
        action: PlaybackAction,
        response: SyncSender<Result<PlaybackSnapshot, String>>,
    },
}

#[derive(Debug)]
pub(crate) enum PlaybackAction {
    SelectTrack {
        track_id: String,
        source: Option<PlaybackQueueSource>,
    },
    #[allow(dead_code)]
    // Explicit play is kept for native transport restoration and future callers.
    Play,
    Pause,
    #[allow(dead_code)]
    // Native stop remains part of the adapter contract, even without a UI button.
    Stop,
    Next {
        natural: bool,
    },
    Previous,
    Seek(u64),
    SetVolume(f64),
    SetRepeat(RepeatMode),
    SetShuffle(bool),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativePlaybackSnapshot {
    service_generation: u64,
    load_instance: u64,
    track_id: Option<String>,
    media_key: Option<String>,
    state: String,
    position_ms: u64,
    duration_ms: Option<u64>,
    volume: f64,
    error: Option<String>,
}

impl Default for NativePlaybackSnapshot {
    fn default() -> Self {
        Self {
            service_generation: 0,
            load_instance: 0,
            track_id: None,
            media_key: None,
            state: "empty".into(),
            position_ms: 0,
            duration_ms: None,
            volume: 1.0,
            error: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeCommand {
    v: u8,
    kind: &'static str,
    command_id: String,
    request_id: String,
    op: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    track_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<NativeTrackMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    position_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    volume: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    play_when_ready: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_next: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_previous: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeTrackMetadata {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artwork_uri: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeEvent {
    v: u8,
    kind: String,
    command_id: Option<String>,
    request_id: Option<String>,
    event_id: Option<String>,
    service_generation: Option<u64>,
    load_instance: Option<u64>,
    media_key: Option<String>,
    action: Option<String>,
    error: Option<String>,
    track_id: Option<String>,
    position_ms: Option<u64>,
    snapshot: Option<NativePlaybackSnapshot>,
}

fn parse_event_id(value: &str) -> Option<u64> {
    value.parse::<u64>().ok().filter(|value| *value > 0)
}

fn command_snapshot_matches(
    current: &NativePlaybackSnapshot,
    command: &NativeCommand,
    command_id: &str,
    generation: u64,
    snapshot: &NativePlaybackSnapshot,
) -> bool {
    if snapshot.service_generation != generation {
        return false;
    }
    if command.op == "load" {
        snapshot.track_id == command.track_id
            && snapshot.media_key.as_deref() == Some(command_id)
            && snapshot.load_instance > 0
            && (current.service_generation != generation
                || snapshot.load_instance > current.load_instance)
    } else {
        snapshot.track_id == current.track_id
            && snapshot.media_key == current.media_key
            && snapshot.load_instance == current.load_instance
    }
}

impl AndroidPlaybackService {
    /// Start or attach to the app-process playback actor. The actor reopens the
    /// same App SQLite path so its lifetime is independent of `AppState`.
    pub(crate) fn start(
        database_path: &Path,
        shuffle: bool,
        repeat_mode: RepeatMode,
    ) -> Result<Self, String> {
        if let Some(existing) = PROCESS_SERVICE.get() {
            return Ok(Self(Arc::clone(existing)));
        }

        let database = Database::open(database_path).map_err(|error| error.to_string())?;
        finish_stale_statistics_runtimes(&database);
        let runtime_id = uuid::Uuid::new_v4();
        let owner_started_utc_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok());
        database
            .register_playback_statistics_runtime(
                runtime_id,
                Some(std::process::id()),
                owner_started_utc_ms,
            )
            .map_err(|error| format!("無法啟動 Android 播放統計：{error}"))?;

        let (request_tx, request_rx) = mpsc::sync_channel(ACTOR_REQUEST_CAPACITY);
        let (native_command_tx, native_command_rx) = mpsc::sync_channel(NATIVE_COMMAND_CAPACITY);
        let (native_event_tx, native_event_rx) = mpsc::sync_channel(NATIVE_EVENT_CAPACITY);
        let shared = Arc::new(SharedService {
            requests: request_tx,
            native_commands: Mutex::new(native_command_rx),
            native_events: native_event_tx,
            snapshot: RwLock::new(empty_snapshot(shuffle, repeat_mode)),
            queue: RwLock::new(None),
            queue_revision: AtomicU64::new(0),
        });

        PROCESS_SERVICE
            .set(Arc::clone(&shared))
            .map_err(|_| "Android 播放服務已在本程序中初始化。".to_owned())?;

        let actor_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("moe-android-playback".to_owned())
            .spawn(move || {
                AndroidActor::new(
                    database,
                    actor_shared,
                    request_rx,
                    native_command_tx,
                    native_event_rx,
                    runtime_id,
                    shuffle,
                    repeat_mode,
                )
                .run();
            })
            .map_err(|error| format!("無法啟動 Android 播放 worker：{error}"))?;

        Ok(Self(shared))
    }

    pub(crate) fn snapshot(&self) -> PlaybackSnapshot {
        self.0
            .snapshot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn queue_snapshot(&self) -> Result<Option<(u64, PlaybackQueueSnapshot)>, String> {
        let queue = self
            .0
            .queue
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        Ok(queue.map(|queue| (self.0.queue_revision.load(Ordering::Relaxed), queue)))
    }

    pub(crate) async fn execute(&self, action: PlaybackAction) -> Result<PlaybackSnapshot, String> {
        let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed).to_string();
        let (tx, rx) = mpsc::sync_channel(1);
        self.0
            .requests
            .try_send(ActorRequest::Command {
                request_id,
                action,
                response: tx,
            })
            .map_err(|error| format!("Android 播放命令佇列已滿：{error}"))?;
        tokio::task::spawn_blocking(move || rx.recv_timeout(COMMAND_ACK_TIMEOUT))
            .await
            .map_err(|error| format!("Android 播放等待工作失敗：{error}"))?
            .map_err(|error| format!("Android 播放命令等候原生回覆逾時：{error}"))?
    }
}

impl PlaybackAction {
    pub(crate) fn select_track(track_id: String, source: Option<PlaybackQueueSource>) -> Self {
        Self::SelectTrack { track_id, source }
    }
}

fn empty_snapshot(shuffle: bool, repeat_mode: RepeatMode) -> PlaybackSnapshot {
    PlaybackSnapshot {
        current_track: None,
        state: "empty".to_owned(),
        is_playing: false,
        position_ms: 0,
        duration_ms: None,
        volume: 1.0,
        last_error: None,
        repeat_mode,
        shuffle,
        can_next: false,
        can_previous: false,
    }
}

fn finish_stale_statistics_runtimes(database: &Database) {
    if let Ok(runtimes) = database.list_playback_statistics_runtimes() {
        for runtime in runtimes {
            let _ = database.finish_playback_statistics_runtime(runtime.runtime_id, &[]);
        }
    }
}

struct AndroidActor {
    database: Database,
    shared: Arc<SharedService>,
    requests: Receiver<ActorRequest>,
    commands: SyncSender<NativeCommand>,
    events: Receiver<NativeEvent>,
    runtime_id: uuid::Uuid,
    shuffle: bool,
    repeat_mode: RepeatMode,
    current_track: Option<TrackSummary>,
    restored_position_ms: Option<u64>,
    queue: Option<PlaybackQueue>,
    native: NativePlaybackSnapshot,
    command_sequence: u64,
    event_fence: NativeEventFence,
    deferred_events: VecDeque<NativeEvent>,
    meter: ListeningMeter,
    last_flush: Instant,
    stats_status: Option<String>,
    last_capabilities: Option<(bool, bool)>,
}

impl AndroidActor {
    #[allow(clippy::too_many_arguments)]
    fn new(
        database: Database,
        shared: Arc<SharedService>,
        requests: Receiver<ActorRequest>,
        commands: SyncSender<NativeCommand>,
        events: Receiver<NativeEvent>,
        runtime_id: uuid::Uuid,
        shuffle: bool,
        repeat_mode: RepeatMode,
    ) -> Self {
        Self {
            database,
            shared,
            requests,
            commands,
            events,
            runtime_id,
            shuffle,
            repeat_mode,
            current_track: None,
            restored_position_ms: None,
            queue: None,
            native: NativePlaybackSnapshot::default(),
            command_sequence: 0,
            event_fence: NativeEventFence::default(),
            deferred_events: VecDeque::new(),
            meter: ListeningMeter::default(),
            last_flush: Instant::now(),
            stats_status: None,
            last_capabilities: None,
        }
    }

    fn run(mut self) {
        self.restore_session();
        loop {
            match self.requests.recv_timeout(Duration::from_millis(250)) {
                Ok(ActorRequest::Command {
                    request_id,
                    action,
                    response,
                }) => {
                    let result = self.handle_action(&request_id, action);
                    self.process_deferred_events();
                    let result = result.map(|_| self.publish_snapshot());
                    let _ = response.send(result);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    // The Tauri AppState may be detached while MediaSession keeps
                    // the process alive. JNI still owns the service channel; keep
                    // sampling and persisting until the process itself exits.
                    thread::sleep(Duration::from_millis(250));
                }
            }
            self.drain_native_events();
            self.process_deferred_events();
            self.periodic_checkpoint();
        }
    }

    fn handle_action(
        &mut self,
        request_id: &str,
        action: PlaybackAction,
    ) -> Result<PlaybackSnapshot, String> {
        match action {
            PlaybackAction::SelectTrack { track_id, source } => {
                self.select_track(request_id, &track_id, source)
            }
            PlaybackAction::Play => {
                if self.native.track_id.is_none() || self.native.media_key.is_none() {
                    let logical_track_id = self
                        .queue
                        .as_ref()
                        .map(|queue| queue.current())
                        .or_else(|| self.current_track.as_ref().map(|track| track.id));
                    if let Some(track_id) = logical_track_id {
                        self.reload_current_for_explicit_play(request_id, track_id)
                    } else {
                        self.native_command(request_id, "play", None, None, None)
                    }
                } else {
                    self.native_command(request_id, "play", None, None, None)
                }
            }
            PlaybackAction::Pause => self.native_command(request_id, "pause", None, None, None),
            PlaybackAction::Stop => self.native_command(request_id, "stop", None, None, None),
            PlaybackAction::Seek(position_ms) => {
                self.meter.seal_segment(&self.native, Instant::now());
                self.native_command(request_id, "seek", None, Some(position_ms), None)
            }
            PlaybackAction::SetVolume(volume) => {
                if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
                    return Err("音量必須介於 0 到 1。".to_owned());
                }
                self.native_command(request_id, "set_volume", None, None, Some(volume))
            }
            PlaybackAction::Next { natural } => self.navigate(request_id, true, natural),
            PlaybackAction::Previous => self.navigate(request_id, false, false),
            PlaybackAction::SetRepeat(mode) => {
                self.repeat_mode = mode;
                if let Some(queue) = self.queue.as_mut() {
                    queue.set_repeat_mode(queue_repeat_mode(mode));
                    self.save_session()?;
                }
                self.update_capabilities(request_id)?;
                Ok(self.publish_snapshot())
            }
            PlaybackAction::SetShuffle(enabled) => {
                self.shuffle = enabled;
                if let Some(queue) = self.queue.as_mut() {
                    if enabled && !queue.shuffle() {
                        let snapshot = queue.snapshot();
                        let ids = snapshot
                            .entries
                            .iter()
                            .map(|entry| entry.track_id)
                            .collect::<Vec<_>>();
                        let statistics = self
                            .database
                            .playback_statistics_for(&ids)
                            .map_err(|error| format!("讀取播放統計失敗：{error}"))?;
                        let weights = statistics
                            .into_iter()
                            .map(|(track_id, value)| {
                                (
                                    track_id,
                                    QueueTrackListeningStats {
                                        played_ms: value.played_ms,
                                        duration_ms: value.duration_ms,
                                    },
                                )
                            })
                            .collect();
                        queue.set_shuffle_with_stats(true, &weights);
                    } else {
                        queue.set_shuffle(enabled);
                    }
                    self.publish_queue();
                    self.save_session()?;
                }
                self.update_capabilities(request_id)?;
                Ok(self.publish_snapshot())
            }
        }
    }

    fn select_track(
        &mut self,
        request_id: &str,
        track_id_text: &str,
        source: Option<PlaybackQueueSource>,
    ) -> Result<PlaybackSnapshot, String> {
        let track_id = TrackId::parse(track_id_text).map_err(|error| error.to_string())?;
        if source.is_none()
            && self
                .current_track
                .as_ref()
                .is_some_and(|track| track.id == track_id)
        {
            if self.native.track_id.as_deref() == Some(track_id_text)
                && self.native.media_key.is_some()
            {
                return self.native_command(request_id, "play", None, None, None);
            }
            return self.reload_current_for_explicit_play(request_id, track_id);
        }
        let track = self
            .database
            .get_track_summary(track_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "曲目已不在曲庫中，請重新整理列表。".to_owned())?;
        let uri = self
            .database
            .resolve_playable_content_uri(track_id)
            .map_err(|error| error.to_string())?;
        let replacement_queue = super::queue_for_track(&self.database, track_id, source.clone())?;
        let mut replacement_queue = replacement_queue;
        replacement_queue.set_repeat_mode(queue_repeat_mode(self.repeat_mode));
        set_queue_shuffle_with_latest_stats(&self.database, &mut replacement_queue, self.shuffle)?;
        let was_playing = self.native.state == "playing";
        let play_when_ready =
            was_playing || !matches!(self.native.state.as_str(), "paused" | "ready" | "stopped");

        self.meter.seal_segment(&self.native, Instant::now());
        self.flush_statistics()?;
        let command = self.new_command(
            request_id,
            "load",
            Some(track_id.to_string()),
            Some(uri),
            Some(self.native_metadata(&track)),
            Some(0),
            None,
            Some(play_when_ready),
            None,
            None,
        );
        let native = match self.transact(command) {
            Ok(native) => native,
            Err(error) => {
                self.meter.reset_segment(&self.native, Instant::now());
                return Err(error);
            }
        };
        self.native = native;
        self.restored_position_ms = None;
        self.current_track = Some(track);
        self.queue = Some(replacement_queue);
        self.publish_queue();
        self.save_session()?;
        self.last_flush = Instant::now();
        self.update_capabilities(request_id)?;
        self.publish_snapshot_checked(false)
    }

    fn navigate(
        &mut self,
        request_id: &str,
        next: bool,
        natural: bool,
    ) -> Result<PlaybackSnapshot, String> {
        let Some(queue) = self.queue.as_ref() else {
            return Ok(self.publish_snapshot());
        };
        let preview = if next {
            queue.preview_next(!natural)
        } else {
            queue.preview_previous()
        };
        let Some((track_id, target_cursor)) = preview else {
            if natural {
                self.native.state = "ended".to_owned();
                self.native.position_ms =
                    self.native.duration_ms.unwrap_or(self.native.position_ms);
                self.meter.seal_segment(&self.native, Instant::now());
                self.checkpoint_position(true)?;
                self.flush_statistics()?;
            }
            return Ok(self.publish_snapshot());
        };
        let track = self
            .database
            .get_track_summary(track_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "佇列中的曲目暫時不在曲庫中。".to_owned())?;
        let uri = self
            .database
            .resolve_playable_content_uri(track_id)
            .map_err(|error| error.to_string())?;
        let play_when_ready = natural || self.native.state == "playing";
        self.meter.seal_segment(&self.native, Instant::now());
        self.flush_statistics()?;
        let command = self.new_command(
            request_id,
            "load",
            Some(track_id.to_string()),
            Some(uri),
            Some(self.native_metadata(&track)),
            Some(0),
            None,
            Some(play_when_ready),
            None,
            None,
        );
        let native = match self.transact(command) {
            Ok(native) => native,
            Err(error) => {
                self.meter.reset_segment(&self.native, Instant::now());
                return Err(error);
            }
        };
        self.native = native;
        self.restored_position_ms = None;
        self.current_track = Some(track);
        self.queue
            .as_mut()
            .ok_or_else(|| "播放佇列在切換期間消失。".to_owned())?
            .set_cursor(target_cursor)
            .map_err(|error| format!("播放佇列游標無效：{error}"))?;
        self.publish_queue();
        self.checkpoint_position(true)?;
        self.update_capabilities(request_id)?;
        self.publish_snapshot_checked(false)
    }

    fn reload_current_for_explicit_play(
        &mut self,
        request_id: &str,
        track_id: TrackId,
    ) -> Result<PlaybackSnapshot, String> {
        let track = self
            .database
            .get_track_summary(track_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "曲目已不在曲庫中，請重新整理列表。".to_owned())?;
        let uri = self
            .database
            .resolve_playable_content_uri(track_id)
            .map_err(|error| error.to_string())?;
        let track_id_text = track_id.to_string();
        let position_ms = if self.native.track_id.as_deref() == Some(track_id_text.as_str()) {
            self.native.position_ms
        } else {
            self.restored_position_ms.unwrap_or(self.native.position_ms)
        };
        self.meter.seal_segment(&self.native, Instant::now());
        let command = self.new_command(
            request_id,
            "load",
            Some(track_id.to_string()),
            Some(uri),
            Some(self.native_metadata(&track)),
            Some(position_ms),
            None,
            Some(true),
            None,
            None,
        );
        self.native = self.transact(command)?;
        self.current_track = Some(track);
        self.restored_position_ms = None;
        self.meter.reset_segment(&self.native, Instant::now());
        self.update_capabilities(request_id)?;
        if self.queue.is_some() {
            self.checkpoint_position(true)?;
        }
        self.publish_snapshot_checked(false)
    }

    fn native_command(
        &mut self,
        request_id: &str,
        op: &'static str,
        track_id: Option<String>,
        position_ms: Option<u64>,
        volume: Option<f64>,
    ) -> Result<PlaybackSnapshot, String> {
        let command = self.new_command(
            request_id,
            op,
            track_id,
            None,
            None,
            position_ms,
            volume,
            None,
            None,
            None,
        );
        self.native = self.transact(command)?;
        if matches!(op, "pause" | "stop") {
            self.meter.seal_segment(&self.native, Instant::now());
            self.checkpoint_position(true)?;
        } else if op == "seek" {
            self.meter.reset_segment(&self.native, Instant::now());
            self.checkpoint_position(true)?;
        }
        self.publish_snapshot_checked(false)
    }

    fn update_capabilities(&mut self, request_id: &str) -> Result<(), String> {
        let can_next = self.queue.as_ref().is_some_and(PlaybackQueue::can_next);
        let can_previous = self.queue.as_ref().is_some_and(PlaybackQueue::can_previous);
        if self.last_capabilities == Some((can_next, can_previous)) {
            return Ok(());
        }
        let command = self.new_command(
            request_id,
            "set_capabilities",
            None,
            None,
            None,
            None,
            None,
            None,
            Some(can_next),
            Some(can_previous),
        );
        self.native = self.transact(command)?;
        self.last_capabilities = Some((can_next, can_previous));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn new_command(
        &mut self,
        request_id: &str,
        op: &'static str,
        track_id: Option<String>,
        content_uri: Option<String>,
        metadata: Option<NativeTrackMetadata>,
        position_ms: Option<u64>,
        volume: Option<f64>,
        play_when_ready: Option<bool>,
        can_next: Option<bool>,
        can_previous: Option<bool>,
    ) -> NativeCommand {
        self.command_sequence = self.command_sequence.wrapping_add(1).max(1);
        NativeCommand {
            v: 1,
            kind: "command",
            command_id: NEXT_COMMAND_ID.fetch_add(1, Ordering::Relaxed).to_string(),
            request_id: request_id.to_owned(),
            op,
            track_id,
            content_uri,
            metadata,
            position_ms,
            volume,
            play_when_ready,
            can_next,
            can_previous,
        }
    }

    fn transact(&mut self, command: NativeCommand) -> Result<NativePlaybackSnapshot, String> {
        let command_id = command.command_id.clone();
        let request_id = command.request_id.clone();
        self.commands
            .try_send(command.clone())
            .map_err(|error| format!("Android 播放服務命令佇列已滿：{error}"))?;
        let deadline = Instant::now() + COMMAND_ACK_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Android 播放服務沒有在期限內確認命令。".to_owned());
            }
            match self
                .events
                .recv_timeout(remaining.min(Duration::from_millis(250)))
            {
                Ok(event) => {
                    if event.v != 1 {
                        continue;
                    }
                    match event.kind.as_str() {
                        "ack"
                            if event.command_id.as_deref() == Some(command_id.as_str())
                                && event.request_id.as_deref() == Some(request_id.as_str()) =>
                        {
                            if let Some(error) = event.error.clone() {
                                self.invalidate_failed_load(
                                    &command,
                                    &error,
                                    event.service_generation,
                                    event.snapshot.as_ref(),
                                    event.event_id.as_deref(),
                                );
                                return Err(error);
                            }
                            let Some(snapshot) = event.snapshot else {
                                let error = "Android 播放服務 ACK 缺少快照。".to_owned();
                                self.invalidate_failed_load(
                                    &command,
                                    &error,
                                    event.service_generation,
                                    None,
                                    event.event_id.as_deref(),
                                );
                                return Err(error);
                            };
                            let generation = event.service_generation.ok_or_else(|| {
                                "Android 播放服務 ACK 缺少 serviceGeneration。".to_owned()
                            })?;
                            if self
                                .event_fence
                                .generation
                                .is_some_and(|current| current != generation)
                            {
                                return Err("Android 播放服務 ACK 來自過期的 service generation。"
                                    .to_owned());
                            }
                            if let Some(error) = snapshot.error.clone() {
                                self.invalidate_failed_load(
                                    &command,
                                    &error,
                                    Some(generation),
                                    Some(&snapshot),
                                    event.event_id.as_deref(),
                                );
                                return Err(error);
                            }
                            if !command_snapshot_matches(
                                &self.native,
                                &command,
                                &command_id,
                                generation,
                                &snapshot,
                            ) {
                                let error = "Android 播放服務 ACK 與要求的載入項目不符。";
                                self.invalidate_failed_load(
                                    &command,
                                    error,
                                    Some(generation),
                                    Some(&snapshot),
                                    event.event_id.as_deref(),
                                );
                                return Err(error.to_owned());
                            }
                            if !self.event_fence.adopt_generation(generation) {
                                return Err("Android 播放服務 ACK 來自過期的 service generation。"
                                    .to_owned());
                            }
                            if let Some(event_id) =
                                event.event_id.as_deref().and_then(parse_event_id)
                            {
                                self.event_fence
                                    .advance_snapshot_watermark(generation, event_id);
                            }
                            self.meter.observe(&snapshot, Instant::now());
                            self.native = snapshot.clone();
                            return Ok(snapshot);
                        }
                        _ => self.deferred_events.push_back(event),
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.periodic_checkpoint();
                    if Instant::now() >= deadline {
                        let error = "Android 播放服務沒有在期限內確認命令。";
                        self.invalidate_failed_load(
                            &command,
                            error,
                            self.event_fence.generation,
                            None,
                            None,
                        );
                        return Err(error.to_owned());
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("Android MediaSession bridge 已關閉。".to_owned());
                }
            }
        }
    }

    fn invalidate_failed_load(
        &mut self,
        command: &NativeCommand,
        error: &str,
        generation: Option<u64>,
        failed_snapshot: Option<&NativePlaybackSnapshot>,
        event_id: Option<&str>,
    ) {
        if command.op != "load"
            || generation.is_some_and(|generation| {
                self.event_fence
                    .generation
                    .is_some_and(|current| current != generation)
            })
        {
            return;
        }

        let logical_position = if self.native.track_id.is_some() {
            self.native.position_ms
        } else {
            self.restored_position_ms.unwrap_or(self.native.position_ms)
        };
        let next_generation = generation
            .or(self.event_fence.generation)
            .unwrap_or(self.native.service_generation);
        let next_load_instance =
            failed_snapshot.map_or(self.native.load_instance, |snapshot| snapshot.load_instance);
        if let Some(event_id) = event_id.and_then(parse_event_id) {
            self.event_fence
                .advance_snapshot_watermark(next_generation, event_id);
        }
        self.restored_position_ms = Some(logical_position);
        self.native = NativePlaybackSnapshot {
            service_generation: next_generation,
            load_instance: next_load_instance,
            track_id: None,
            media_key: None,
            state: "ready".to_owned(),
            position_ms: 0,
            duration_ms: None,
            volume: self.native.volume,
            error: Some(error.to_owned()),
        };
        self.meter.reset_segment(&self.native, Instant::now());
        if self.queue.is_some() {
            if let Err(checkpoint_error) = self.checkpoint_position(true) {
                self.stats_status = Some(checkpoint_error);
            }
        }
        self.publish_snapshot();
    }

    fn drain_native_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.deferred_events.push_back(event);
        }
    }

    fn process_deferred_events(&mut self) {
        while let Some(event) = self.deferred_events.pop_front() {
            match event.kind.as_str() {
                "snapshot" => self.process_snapshot_event(event),
                "transport" | "ended" => self.process_transport_event(event),
                "service_ready" => self.process_service_ready(event),
                "ack" => {}
                _ => {}
            }
        }
    }

    fn process_snapshot_event(&mut self, event: NativeEvent) {
        let Some(generation) = self.event_fence.generation else {
            return;
        };
        if event.service_generation != Some(generation) {
            return;
        }
        let Some(event_id) = event.event_id.as_deref().and_then(parse_event_id) else {
            return;
        };
        let Some(snapshot) = event.snapshot else {
            return;
        };
        if !self.snapshot_matches_current(&snapshot, generation) {
            return;
        }
        if !self.event_fence.accept_snapshot(generation, event_id) {
            return;
        }
        self.observe_snapshot(snapshot, Instant::now());
    }

    fn snapshot_matches_current(&self, snapshot: &NativePlaybackSnapshot, generation: u64) -> bool {
        self.event_fence.generation == Some(generation)
            && self.event_fence.matches_snapshot(&self.native, snapshot)
    }

    fn process_service_ready(&mut self, event: NativeEvent) {
        let Some(generation) = event.service_generation else {
            return;
        };
        let previous_generation = self.event_fence.generation;
        if !self.event_fence.adopt_generation(generation) {
            return;
        }
        if let Some(snapshot) = event.snapshot.as_ref() {
            if snapshot.service_generation != generation {
                return;
            }
        }
        if let Some(event_id) = event.event_id.as_deref().and_then(parse_event_id) {
            if !self.event_fence.accept_snapshot(generation, event_id) {
                return;
            }
        } else {
            return;
        }
        if let Some(snapshot) = event.snapshot.clone() {
            self.native = snapshot;
            self.meter.reset_segment(&self.native, Instant::now());
        }
        if previous_generation == Some(generation) {
            if let Some(snapshot) = event.snapshot {
                if self.snapshot_matches_current(&snapshot, generation) {
                    self.observe_snapshot(snapshot, Instant::now());
                }
            }
            return;
        }
        if self.queue.is_some() && self.current_track.is_some() {
            let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed).to_string();
            if let Err(error) = self.reload_after_service_restart(&request_id) {
                self.native.state = "ready".to_owned();
                self.native.error = Some(format!(
                    "已保留播放狀態；Android 音訊來源目前不可用：{error}"
                ));
                self.publish_snapshot();
            }
        } else {
            self.publish_snapshot();
        }
    }

    fn reload_after_service_restart(&mut self, request_id: &str) -> Result<(), String> {
        let (Some(queue), Some(track)) = (self.queue.as_ref(), self.current_track.clone()) else {
            return Ok(());
        };
        let track_id = queue.current();
        if track.id != track_id {
            return Err("還原的播放游標與目前曲目不一致。".to_owned());
        }
        let uri = self
            .database
            .resolve_playable_content_uri(track_id)
            .map_err(|error| format!("找不到目前曲目的有效來源：{error}"))?;
        let track_id_text = track_id.to_string();
        let position_ms = if self.native.track_id.as_deref() == Some(track_id_text.as_str()) {
            self.native.position_ms
        } else {
            self.restored_position_ms.unwrap_or(self.native.position_ms)
        };
        self.meter.seal_segment(&self.native, Instant::now());
        let command = self.new_command(
            request_id,
            "load",
            Some(track_id.to_string()),
            Some(uri),
            Some(self.native_metadata(&track)),
            Some(position_ms),
            None,
            Some(false),
            None,
            None,
        );
        self.native = self.transact(command)?;
        self.restored_position_ms = None;
        self.native.state = "ready".to_owned();
        self.publish_snapshot_checked(true)?;
        self.update_capabilities(request_id)
    }

    fn process_transport_event(&mut self, event: NativeEvent) {
        let Some(generation) = self.event_fence.generation else {
            return;
        };
        if !self.event_fence.matches_transport(&self.native, &event) {
            return;
        }
        let Some(event_id) = event.event_id.as_deref().and_then(parse_event_id) else {
            return;
        };
        if !self.event_fence.accept_transport(generation, event_id) {
            return;
        }

        if event.kind == "ended" {
            if event.track_id.as_deref() == self.native.track_id.as_deref() {
                self.native.state = "ended".to_owned();
                self.native.position_ms =
                    self.native.duration_ms.unwrap_or(self.native.position_ms);
                self.meter.seal_segment(&self.native, Instant::now());
                let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed).to_string();
                let _ = self.navigate(&request_id, true, true);
            }
            return;
        }

        match event.action.as_deref() {
            Some("next") => {
                let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed).to_string();
                let _ = self.navigate(&request_id, true, false);
            }
            Some("previous") => {
                let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed).to_string();
                let _ = self.navigate(&request_id, false, false);
            }
            Some("seek") => {
                if let Some(position) = event.position_ms {
                    self.meter.seal_segment(&self.native, Instant::now());
                    self.native.position_ms = position;
                    self.meter.reset_segment(&self.native, Instant::now());
                    let _ = self.checkpoint_position(true);
                }
            }
            Some("pause" | "stop") => {
                if let Some(snapshot) = event.snapshot {
                    self.observe_snapshot(snapshot, Instant::now());
                } else {
                    self.native.state = event.action.unwrap_or_default();
                    self.meter.seal_segment(&self.native, Instant::now());
                }
                let _ = self.checkpoint_position(true);
                let _ = self.flush_statistics();
            }
            Some("play") => {
                if let Some(snapshot) = event.snapshot {
                    self.observe_snapshot(snapshot, Instant::now());
                } else {
                    self.native.state = "playing".to_owned();
                    self.meter.reset_segment(&self.native, Instant::now());
                }
            }
            _ => {
                if let Some(snapshot) = event.snapshot {
                    self.observe_snapshot(snapshot, Instant::now());
                }
            }
        }
    }

    fn observe_snapshot(&mut self, snapshot: NativePlaybackSnapshot, observed_at: Instant) {
        self.meter.observe(&snapshot, observed_at);
        self.native = snapshot;
        let track_id = self
            .native
            .track_id
            .as_deref()
            .and_then(|value| TrackId::parse(value).ok());
        if track_id.is_some_and(|track_id| {
            self.current_track
                .as_ref()
                .is_none_or(|current| current.id != track_id)
        }) {
            self.current_track = track_id
                .and_then(|track_id| self.database.get_track_summary(track_id).ok().flatten());
        }
        self.publish_snapshot();
    }

    fn publish_snapshot(&self) -> PlaybackSnapshot {
        let queue = self.queue.as_ref();
        let duration_ms = self.native.duration_ms.or_else(|| {
            self.current_track
                .as_ref()
                .and_then(|track| track.duration_ms)
        });
        let snapshot = PlaybackSnapshot {
            current_track: self.current_track.clone(),
            state: self.native.state.clone(),
            is_playing: self.native.state == "playing",
            position_ms: if self.native.track_id.is_some() {
                self.native.position_ms
            } else {
                self.restored_position_ms.unwrap_or(self.native.position_ms)
            },
            duration_ms,
            volume: self.native.volume,
            last_error: self
                .native
                .error
                .clone()
                .or_else(|| self.stats_status.clone()),
            repeat_mode: self.repeat_mode,
            shuffle: self.shuffle,
            can_next: queue.is_some_and(PlaybackQueue::can_next),
            can_previous: queue.is_some_and(PlaybackQueue::can_previous),
        };
        *self
            .shared
            .snapshot
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = snapshot.clone();
        snapshot
    }

    fn publish_snapshot_checked(&mut self, checkpoint: bool) -> Result<PlaybackSnapshot, String> {
        if checkpoint {
            self.checkpoint_position(true)?;
        }
        Ok(self.publish_snapshot())
    }

    fn publish_queue(&self) {
        *self
            .shared
            .queue
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            self.queue.as_ref().map(PlaybackQueue::snapshot);
        self.shared.queue_revision.fetch_add(1, Ordering::Relaxed);
    }

    fn native_metadata(&self, track: &TrackSummary) -> NativeTrackMetadata {
        let artwork_uri = self
            .database
            .track_locators(track.id)
            .ok()
            .and_then(|locators| {
                locators.into_iter().find_map(|locator| match locator {
                    MediaLocator::ContentUri(uri) => Some(uri),
                    MediaLocator::FileSystem(_) => None,
                })
            });
        NativeTrackMetadata {
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_ms: track.duration_ms,
            artwork_uri,
        }
    }

    fn save_session(&self) -> Result<(), String> {
        let Some(queue) = self.queue.as_ref() else {
            return Ok(());
        };
        self.database
            .save_playback_session(&PlaybackSessionCheckpoint {
                queue: queue.snapshot(),
                position_ms: self.logical_position_ms(),
            })
            .map_err(|error| format!("保存播放狀態失敗：{error}"))
    }

    fn logical_position_ms(&self) -> u64 {
        if self.native.track_id.is_some() {
            self.native.position_ms
        } else {
            self.restored_position_ms.unwrap_or(self.native.position_ms)
        }
    }

    fn checkpoint_position(&mut self, force: bool) -> Result<(), String> {
        if !force
            && (self.native.state != "playing" || self.last_flush.elapsed() < STATS_FLUSH_INTERVAL)
        {
            return Ok(());
        }
        if let Some(queue) = self.queue.as_ref() {
            self.database
                .checkpoint_playback_position(&queue.snapshot(), self.logical_position_ms())
                .map_err(|error| format!("保存播放狀態失敗：{error}"))?;
        }
        self.flush_statistics()?;
        self.last_flush = Instant::now();
        Ok(())
    }

    fn periodic_checkpoint(&mut self) {
        if self.native.state == "playing" && self.last_flush.elapsed() >= STATS_FLUSH_INTERVAL {
            if let Err(error) = self.checkpoint_position(true) {
                self.stats_status = Some(error);
                self.publish_snapshot();
            }
        }
    }

    fn flush_statistics(&mut self) -> Result<(), String> {
        let checkpoints = self.meter.dirty_checkpoints();
        if checkpoints.is_empty() {
            return Ok(());
        }
        self.database
            .record_playback_checkpoints(self.runtime_id, &checkpoints)
            .map_err(|error| format!("保存播放時長統計失敗：{error}"))?;
        self.meter.acknowledge(&checkpoints);
        self.stats_status = None;
        Ok(())
    }

    fn restore_session(&mut self) {
        let Ok(Some(checkpoint)) = self.database.load_playback_session() else {
            return;
        };
        let Ok(queue) = PlaybackQueue::restore(checkpoint.queue) else {
            return;
        };
        self.restored_position_ms = Some(checkpoint.position_ms);
        let track_id = queue.current();
        self.current_track = self.database.get_track_summary(track_id).ok().flatten();
        self.shuffle = queue.shuffle();
        self.repeat_mode = match queue.repeat_mode() {
            QueueRepeatMode::Off => RepeatMode::Off,
            QueueRepeatMode::All => RepeatMode::All,
            QueueRepeatMode::One => RepeatMode::One,
        };
        self.queue = Some(queue);
        self.publish_queue();
        let Some(track) = self.current_track.clone() else {
            self.native.state = "ready".into();
            self.native.error = Some("已保留播放佇列，但目前曲目已不在曲庫中。".to_owned());
            self.publish_snapshot();
            return;
        };
        let Ok(uri) = self.database.resolve_playable_content_uri(track_id) else {
            self.native.state = "ready".into();
            self.native.error = Some("已保留播放狀態；目前 Android 來源暫不可用。".to_owned());
            self.publish_snapshot();
            return;
        };
        let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed).to_string();
        let command = self.new_command(
            &request_id,
            "load",
            Some(track_id.to_string()),
            Some(uri),
            Some(self.native_metadata(&track)),
            Some(checkpoint.position_ms),
            None,
            Some(false),
            None,
            None,
        );
        match self.transact(command) {
            Ok(snapshot) => {
                self.native = snapshot;
                if let Err(error) = self.update_capabilities(&request_id) {
                    self.native.error = Some(error);
                }
                self.native.state = "ready".into();
                self.publish_snapshot();
            }
            Err(error) => {
                self.native.state = "ready".into();
                self.native.error = Some(format!("已保留播放狀態；無法載入目前曲目：{error}"));
                self.publish_snapshot();
            }
        }
    }
}

#[derive(Debug, Default)]
struct NativeEventFence {
    generation: Option<u64>,
    snapshot_event_id: u64,
    transport_event_id: u64,
}

impl NativeEventFence {
    fn adopt_generation(&mut self, generation: u64) -> bool {
        if self.generation.is_some_and(|current| generation < current) {
            return false;
        }
        if self.generation != Some(generation) {
            self.generation = Some(generation);
            self.snapshot_event_id = 0;
            self.transport_event_id = 0;
        }
        true
    }

    fn accept_snapshot(&mut self, generation: u64, event_id: u64) -> bool {
        if self.generation != Some(generation) || event_id <= self.snapshot_event_id {
            return false;
        }
        self.snapshot_event_id = event_id;
        true
    }

    fn advance_snapshot_watermark(&mut self, generation: u64, event_id: u64) -> bool {
        if self.generation != Some(generation) {
            return false;
        }
        self.snapshot_event_id = self.snapshot_event_id.max(event_id);
        true
    }

    fn accept_transport(&mut self, generation: u64, event_id: u64) -> bool {
        if self.generation != Some(generation) || event_id <= self.transport_event_id {
            return false;
        }
        self.transport_event_id = event_id;
        true
    }

    fn matches_snapshot(
        &self,
        current: &NativePlaybackSnapshot,
        candidate: &NativePlaybackSnapshot,
    ) -> bool {
        candidate.service_generation == self.generation.unwrap_or_default()
            && candidate.load_instance >= current.load_instance
            && (current.track_id.is_none()
                || (candidate.track_id == current.track_id
                    && candidate.media_key == current.media_key
                    && candidate.load_instance == current.load_instance))
    }

    fn matches_transport(&self, current: &NativePlaybackSnapshot, event: &NativeEvent) -> bool {
        event.service_generation == self.generation
            && event.load_instance == Some(current.load_instance)
            && event.media_key == current.media_key
            && event.track_id == current.track_id
    }
}

#[derive(Default)]
struct ListeningMeter {
    totals: std::collections::HashMap<TrackId, MeterTotal>,
    dirty: HashSet<TrackId>,
    anchor: Option<MeterAnchor>,
}

#[derive(Clone, Copy, Default)]
struct MeterTotal {
    played_ms: u64,
    duration_ms: Option<u64>,
}

#[derive(Clone)]
struct MeterAnchor {
    track_id: TrackId,
    started_at: Instant,
    start_position_ms: u64,
    credited_ms: u64,
    duration_ms: Option<u64>,
}

impl ListeningMeter {
    fn observe(&mut self, snapshot: &NativePlaybackSnapshot, at: Instant) {
        if snapshot.state != "playing" {
            self.seal_segment(snapshot, at);
            return;
        }
        let Some(track_id) = snapshot
            .track_id
            .as_deref()
            .and_then(|value| TrackId::parse(value).ok())
        else {
            self.anchor = None;
            return;
        };
        let anchor_matches = self
            .anchor
            .as_ref()
            .is_some_and(|anchor| anchor.track_id == track_id);
        if !anchor_matches {
            self.anchor = Some(MeterAnchor {
                track_id,
                started_at: at,
                start_position_ms: snapshot.position_ms,
                credited_ms: 0,
                duration_ms: snapshot.duration_ms,
            });
            return;
        }
        let anchor = self.anchor.as_mut().expect("matching anchor");
        let elapsed_ms = u64::try_from(at.saturating_duration_since(anchor.started_at).as_millis())
            .unwrap_or(u64::MAX);
        let natural_ms = snapshot
            .position_ms
            .saturating_sub(anchor.start_position_ms);
        let credited_now = elapsed_ms.min(natural_ms);
        if credited_now > anchor.credited_ms {
            let increment = credited_now - anchor.credited_ms;
            let total = self.totals.entry(track_id).or_default();
            total.played_ms = total.played_ms.saturating_add(increment);
            total.duration_ms = snapshot.duration_ms.or(total.duration_ms);
            self.dirty.insert(track_id);
            anchor.credited_ms = credited_now;
        }
    }

    fn seal_segment(&mut self, snapshot: &NativePlaybackSnapshot, at: Instant) {
        self.credit_position(snapshot, at);
        self.anchor = None;
    }

    fn reset_segment(&mut self, snapshot: &NativePlaybackSnapshot, at: Instant) {
        self.anchor = None;
        if snapshot.state == "playing" {
            if let Some(track_id) = snapshot
                .track_id
                .as_deref()
                .and_then(|value| TrackId::parse(value).ok())
            {
                self.anchor = Some(MeterAnchor {
                    track_id,
                    started_at: at,
                    start_position_ms: snapshot.position_ms,
                    credited_ms: 0,
                    duration_ms: snapshot.duration_ms,
                });
            }
        }
    }

    fn credit_position(&mut self, snapshot: &NativePlaybackSnapshot, at: Instant) {
        let Some(track_id) = snapshot
            .track_id
            .as_deref()
            .and_then(|value| TrackId::parse(value).ok())
        else {
            return;
        };
        let Some(anchor) = self
            .anchor
            .as_mut()
            .filter(|anchor| anchor.track_id == track_id)
        else {
            return;
        };
        let elapsed_ms = u64::try_from(at.saturating_duration_since(anchor.started_at).as_millis())
            .unwrap_or(u64::MAX);
        let natural_ms = snapshot
            .position_ms
            .saturating_sub(anchor.start_position_ms);
        let credited_now = elapsed_ms.min(natural_ms);
        if credited_now > anchor.credited_ms {
            let increment = credited_now - anchor.credited_ms;
            let total = self.totals.entry(track_id).or_default();
            total.played_ms = total.played_ms.saturating_add(increment);
            total.duration_ms = snapshot
                .duration_ms
                .or(anchor.duration_ms)
                .or(total.duration_ms);
            self.dirty.insert(track_id);
            anchor.credited_ms = credited_now;
        }
    }

    fn dirty_checkpoints(&self) -> Vec<PlaybackCheckpoint> {
        self.dirty
            .iter()
            .filter_map(|track_id| {
                self.totals.get(track_id).map(|total| PlaybackCheckpoint {
                    track_id: *track_id,
                    played_ms: total.played_ms,
                    duration_ms: total.duration_ms,
                })
            })
            .collect()
    }

    fn acknowledge(&mut self, saved: &[PlaybackCheckpoint]) {
        for checkpoint in saved {
            let unchanged = self.totals.get(&checkpoint.track_id).is_some_and(|total| {
                total.played_ms == checkpoint.played_ms
                    && total.duration_ms == checkpoint.duration_ms
            });
            if unchanged {
                self.dirty.remove(&checkpoint.track_id);
            }
        }
    }
}

#[cfg(target_os = "android")]
mod jni_exports {
    use super::*;
    use jni::{
        objects::{JObject, JString},
        sys::jstring,
        JNIEnv,
    };

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_PlaybackService_nativePollPlaybackCommand(
        env: JNIEnv,
        _service: JObject,
    ) -> jstring {
        let Some(shared) = PROCESS_SERVICE.get() else {
            return std::ptr::null_mut();
        };
        let receiver = shared
            .native_commands
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match receiver.try_recv() {
            Ok(command) => match serde_json::to_string(&command)
                .ok()
                .and_then(|json| env.new_string(json).ok())
            {
                Some(value) => value.into_raw(),
                None => std::ptr::null_mut(),
            },
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => std::ptr::null_mut(),
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_PlaybackService_nativeOnPlaybackEvent(
        mut env: JNIEnv,
        _service: JObject,
        raw: JString,
    ) {
        let Ok(json) = env.get_string(&raw) else {
            return;
        };
        let Ok(event) = serde_json::from_str::<NativeEvent>(&json.to_string_lossy()) else {
            return;
        };
        let Some(shared) = PROCESS_SERVICE.get() else {
            return;
        };
        let _ = shared.native_events.try_send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_command_contract_uses_camel_case_and_ephemeral_content_uri() {
        let command = NativeCommand {
            v: 1,
            kind: "command",
            command_id: "12".into(),
            request_id: "11".into(),
            op: "load",
            track_id: Some("track-id".into()),
            content_uri: Some("content://media/external/audio/media/123".into()),
            metadata: Some(NativeTrackMetadata {
                title: Some("Title".into()),
                artist: Some("Artist".into()),
                album: None,
                duration_ms: Some(1000),
                artwork_uri: Some("content://media/external/audio/albumart/123".into()),
            }),
            position_ms: Some(400),
            volume: None,
            play_when_ready: Some(false),
            can_next: None,
            can_previous: None,
        };
        let json = serde_json::to_value(command).expect("serialize command");
        assert_eq!(
            json["contentUri"],
            "content://media/external/audio/media/123"
        );
        assert_eq!(json["trackId"], "track-id");
        assert_eq!(json["playWhenReady"], false);
        assert_eq!(json["positionMs"], 400);
        assert_eq!(
            json["metadata"]["artworkUri"],
            "content://media/external/audio/albumart/123"
        );
    }

    #[test]
    fn parses_kotlin_service_ready_ack_snapshot_and_ended_json_fixtures() {
        let fixtures = [
            r#"{"v":1,"kind":"service_ready","eventId":"1","serviceGeneration":2,"snapshot":{"serviceGeneration":2,"loadInstance":0,"trackId":null,"mediaKey":null,"state":"empty","positionMs":0,"durationMs":null,"volume":1.0,"error":null}}"#,
            r#"{"v":1,"kind":"ack","eventId":"3","commandId":"load-9","requestId":"8","serviceGeneration":2,"snapshot":{"serviceGeneration":2,"loadInstance":1,"trackId":"track-1","mediaKey":"load-9","state":"paused","positionMs":1200,"durationMs":5000,"volume":1.0,"error":null}}"#,
            r#"{"v":1,"kind":"snapshot","eventId":"4","serviceGeneration":2,"snapshot":{"serviceGeneration":2,"loadInstance":1,"trackId":"track-1","mediaKey":"load-9","state":"playing","positionMs":1450,"durationMs":5000,"volume":1.0,"error":null}}"#,
            r#"{"v":1,"kind":"ended","eventId":"5","serviceGeneration":2,"trackId":"track-1","mediaKey":"load-9","loadInstance":1}"#,
        ];
        let ready: NativeEvent =
            serde_json::from_str(fixtures[0]).expect("Kotlin service_ready fixture");
        assert_eq!(ready.service_generation, Some(2));
        assert_eq!(ready.snapshot.as_ref().unwrap().media_key, None);

        let ack: NativeEvent = serde_json::from_str(fixtures[1]).expect("Kotlin ACK fixture");
        assert_eq!(ack.command_id.as_deref(), Some("load-9"));
        assert_eq!(ack.event_id.as_deref(), Some("3"));
        assert_eq!(ack.service_generation, Some(2));
        assert_eq!(
            ack.snapshot.as_ref().unwrap().media_key.as_deref(),
            Some("load-9")
        );
        assert_eq!(ack.snapshot.as_ref().unwrap().load_instance, 1);

        let snapshot: NativeEvent =
            serde_json::from_str(fixtures[2]).expect("Kotlin snapshot fixture");
        assert_eq!(snapshot.snapshot.as_ref().unwrap().position_ms, 1450);
        let ended: NativeEvent = serde_json::from_str(fixtures[3]).expect("Kotlin ended fixture");
        assert_eq!(ended.load_instance, Some(1));
        assert_eq!(ended.media_key.as_deref(), Some("load-9"));
    }

    #[test]
    fn event_fence_separates_stale_snapshots_from_queued_transport_and_old_service_events() {
        let track_id = "track-1".to_owned();
        let active = NativePlaybackSnapshot {
            service_generation: 7,
            load_instance: 4,
            track_id: Some(track_id.clone()),
            media_key: Some("load-4".into()),
            state: "playing".into(),
            position_ms: 1_000,
            duration_ms: Some(5_000),
            volume: 1.0,
            error: None,
        };
        let stale_position = NativePlaybackSnapshot {
            position_ms: 950,
            ..active.clone()
        };
        let mut fence = NativeEventFence::default();
        assert!(fence.adopt_generation(7));
        assert!(fence.accept_snapshot(7, 10));
        assert!(fence.advance_snapshot_watermark(7, 12)); // a post-command ACK
        assert!(!fence.accept_snapshot(7, 11)); // pre-ACK position cannot roll back seek
        assert!(!fence.matches_snapshot(
            &active,
            &NativePlaybackSnapshot {
                media_key: Some("old-load-same-track".into()),
                ..stale_position.clone()
            }
        ));

        let queued_next = NativeEvent {
            v: 1,
            kind: "transport".into(),
            command_id: None,
            request_id: None,
            event_id: Some("11".into()),
            service_generation: Some(7),
            load_instance: Some(4),
            media_key: Some("load-4".into()),
            action: Some("next".into()),
            error: None,
            track_id: Some(track_id),
            position_ms: None,
            snapshot: None,
        };
        assert!(fence.matches_transport(&active, &queued_next));
        assert!(fence.accept_transport(7, 11)); // ACK watermark does not consume transport
        assert!(!fence.accept_transport(7, 11)); // duplicate transport is dropped
        assert!(fence.adopt_generation(8));
        assert!(!fence.accept_snapshot(7, 20));
        assert!(!fence.matches_transport(&active, &queued_next));
    }

    #[test]
    fn command_ack_must_match_generation_track_and_load_instance_identity() {
        let current = NativePlaybackSnapshot {
            service_generation: 3,
            load_instance: 7,
            track_id: Some("track-a".into()),
            media_key: Some("load-7".into()),
            state: "playing".into(),
            position_ms: 900,
            duration_ms: Some(4000),
            volume: 1.0,
            error: None,
        };
        let load = NativeCommand {
            v: 1,
            kind: "command",
            command_id: "load-8".into(),
            request_id: "req-8".into(),
            op: "load",
            track_id: Some("track-a".into()),
            content_uri: Some("content://audio/8".into()),
            metadata: None,
            position_ms: Some(0),
            volume: None,
            play_when_ready: Some(false),
            can_next: None,
            can_previous: None,
        };
        let delayed_old_slot = NativePlaybackSnapshot {
            media_key: Some("load-7".into()),
            ..current.clone()
        };
        assert!(!command_snapshot_matches(
            &current,
            &load,
            "load-8",
            3,
            &delayed_old_slot
        ));
        let correct_duplicate_slot = NativePlaybackSnapshot {
            media_key: Some("load-8".into()),
            load_instance: 8,
            ..current.clone()
        };
        assert!(command_snapshot_matches(
            &current,
            &load,
            "load-8",
            3,
            &correct_duplicate_slot
        ));
        assert!(!command_snapshot_matches(
            &current,
            &load,
            "load-8",
            2,
            &correct_duplicate_slot
        ));

        let pause = NativeCommand {
            op: "pause",
            ..load
        };
        assert!(!command_snapshot_matches(
            &current,
            &pause,
            "load-8",
            3,
            &correct_duplicate_slot
        ));
        assert!(command_snapshot_matches(
            &current, &pause, "pause-9", 3, &current
        ));
    }

    #[test]
    fn meter_counts_only_elapsed_time_with_natural_progress_and_resets_on_seek() {
        let track_id = TrackId::new();
        let start = Instant::now();
        let mut meter = ListeningMeter::default();
        let playing = |position_ms| NativePlaybackSnapshot {
            service_generation: 1,
            load_instance: 1,
            track_id: Some(track_id.to_string()),
            media_key: Some("load-1".into()),
            state: "playing".into(),
            position_ms,
            duration_ms: Some(10_000),
            volume: 1.0,
            error: None,
        };
        meter.observe(&playing(100), start);
        meter.observe(&playing(349), start + Duration::from_millis(250));
        assert_eq!(meter.totals[&track_id].played_ms, 249);
        meter.reset_segment(&playing(3000), start + Duration::from_millis(300));
        meter.observe(&playing(3050), start + Duration::from_millis(550));
        assert_eq!(meter.totals[&track_id].played_ms, 299);
    }

    #[test]
    fn meter_does_not_attribute_old_track_tail_to_new_track() {
        let first = TrackId::new();
        let second = TrackId::new();
        let start = Instant::now();
        let mut meter = ListeningMeter::default();
        let snapshot = |track_id: TrackId, state: &str, position_ms: u64| NativePlaybackSnapshot {
            service_generation: 1,
            load_instance: 1,
            track_id: Some(track_id.to_string()),
            media_key: Some("load-1".into()),
            state: state.into(),
            position_ms,
            duration_ms: Some(10_000),
            volume: 1.0,
            error: None,
        };
        meter.observe(&snapshot(first, "playing", 100), start);
        meter.observe(
            &snapshot(first, "playing", 350),
            start + Duration::from_millis(250),
        );
        meter.seal_segment(
            &snapshot(first, "paused", 350),
            start + Duration::from_millis(300),
        );
        meter.observe(
            &snapshot(second, "playing", 0),
            start + Duration::from_millis(300),
        );
        assert_eq!(meter.totals[&first].played_ms, 250);
        assert!(!meter.totals.contains_key(&second));
    }
}

#[cfg(test)]
#[path = "android_playback_integration_tests.rs"]
mod integration_tests;
