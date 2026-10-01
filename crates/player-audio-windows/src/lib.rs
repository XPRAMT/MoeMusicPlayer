//! Background-thread Windows audio playback for local files.
//!
//! The handle exposes a small, backend-independent command surface. Native
//! device access, file opening, decoding, and seeking run on a dedicated worker
//! thread. Callers poll a bounded snapshot and may drain state/error events.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(windows)]
mod rodio_backend;

#[cfg(windows)]
pub mod system_media;

/// Query a Windows process creation time as UTC milliseconds since the Unix
/// epoch. This identity is used with a PID to distinguish a reused PID from
/// the process that registered a playback-statistics runtime.
#[cfg(windows)]
pub fn process_started_utc_ms(pid: u32) -> Result<Option<i64>, String> {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let process = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(process) => process,
        Err(error) if error.code().0 as u32 == 0x8007_0057 => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let result =
        unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) };
    let _ = unsafe { CloseHandle(process) };
    result.map_err(|error| error.to_string())?;

    const WINDOWS_TO_UNIX_EPOCH_100NS: u64 = 116_444_736_000_000_000;
    let filetime = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
    let Some(unix_100ns) = filetime.checked_sub(WINDOWS_TO_UNIX_EPOCH_100NS) else {
        return Ok(None);
    };
    Ok(i64::try_from(unix_100ns / 10_000).ok())
}

const COMMAND_CAPACITY: usize = 64;
const EVENT_CAPACITY: usize = 128;
const POSITION_POLL_INTERVAL: Duration = Duration::from_millis(40);

/// The coarse playback lifecycle exposed to application code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaybackState {
    Initializing,
    Empty,
    Loading,
    Ready,
    Playing,
    Paused,
    Stopped,
    Ended,
    Error,
}

/// A point-in-time view of the active playback session.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackSnapshot {
    pub state: PlaybackState,
    pub position: Duration,
    pub duration: Option<Duration>,
    /// Linear gain in the range `0.0..=1.0`.
    pub volume: f32,
    pub last_error: Option<AudioError>,
}

impl Default for PlaybackSnapshot {
    fn default() -> Self {
        Self {
            state: PlaybackState::Initializing,
            position: Duration::ZERO,
            duration: None,
            volume: 1.0,
            last_error: None,
        }
    }
}

/// State changes and failures are bounded notifications; the snapshot remains
/// authoritative when an event consumer is temporarily behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioEvent {
    StateChanged(PlaybackState),
    Error(AudioError),
}

/// Errors returned by the player command queue or reported asynchronously by
/// the backend worker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AudioError {
    OutputDevice(String),
    WorkerStart(String),
    WorkerStopped,
    CommandQueueFull,
    CommandTimeout,
    UnsupportedPlatform,
    FileOpen { path: PathBuf, message: String },
    UnsupportedFormat { path: PathBuf },
    Decode { path: PathBuf, message: String },
    Seek(String),
    Backend(String),
    BackendUnavailable,
    NoTrackLoaded,
    InvalidVolume,
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutputDevice(message) => {
                write!(f, "could not open the default output device: {message}")
            }
            Self::WorkerStart(message) => write!(f, "could not start the audio worker: {message}"),
            Self::WorkerStopped => f.write_str("the audio worker has stopped"),
            Self::CommandQueueFull => f.write_str("the audio command queue is full"),
            Self::CommandTimeout => {
                f.write_str("the audio worker did not acknowledge the command in time")
            }
            Self::UnsupportedPlatform => {
                f.write_str("the Windows audio backend is only available on Windows")
            }
            Self::FileOpen { path, message } => write!(f, "could not open {path:?}: {message}"),
            Self::UnsupportedFormat { path } => write!(f, "unsupported audio format: {path:?}"),
            Self::Decode { path, message } => write!(f, "could not decode {path:?}: {message}"),
            Self::Seek(message) => write!(f, "could not seek in the current track: {message}"),
            Self::Backend(message) => f.write_str(message),
            Self::BackendUnavailable => f.write_str("the audio backend is unavailable"),
            Self::NoTrackLoaded => f.write_str("no track is loaded"),
            Self::InvalidVolume => f.write_str("volume must be a finite value between 0.0 and 1.0"),
        }
    }
}

impl Error for AudioError {}

/// Synchronous backend operations. Implementations are created and called only
/// by the worker thread, so an application can replace Rodio without changing
/// the IPC-facing handle.
pub trait AudioBackend: 'static {
    /// Load a file and leave it ready but paused. Return its duration if known.
    fn load(&mut self, path: &Path) -> Result<Option<Duration>, AudioError>;
    fn play(&mut self) -> Result<(), AudioError>;
    fn pause(&mut self) -> Result<(), AudioError>;
    fn stop(&mut self) -> Result<(), AudioError>;
    /// Seek and return the actual position reached by the backend.
    fn seek(&mut self, position: Duration) -> Result<Duration, AudioError>;
    fn set_volume(&mut self, volume: f32) -> Result<(), AudioError>;
    fn position(&self) -> Duration;
    fn is_empty(&self) -> bool;
    /// Return a runtime output failure, if the backend has observed one.
    fn take_error(&mut self) -> Option<AudioError> {
        None
    }
}

/// Cloneable, non-blocking control surface for one playback worker.
#[derive(Clone)]
pub struct PlayerHandle {
    inner: Arc<PlayerInner>,
}

struct PlayerInner {
    commands: SyncSender<QueuedCommand>,
    snapshot: Arc<RwLock<PlaybackSnapshot>>,
    events: Mutex<Receiver<AudioEvent>>,
    accounting: Arc<PlaybackAccountingShared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

/// Opaque identity assigned by the application to one TrackId for this
/// process. The audio worker never interprets it as a path or database key.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PlaybackAccountingId(u128);

impl PlaybackAccountingId {
    pub const fn new(value: u128) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u128 {
        self.0
    }
}

/// Cumulative, retry-safe audio-thread listening checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaybackAccountingCheckpoint {
    pub id: PlaybackAccountingId,
    pub played_ms: u64,
    pub duration_ms: Option<u64>,
}

#[derive(Default)]
struct PlaybackAccountingState {
    counters: std::collections::HashMap<PlaybackAccountingId, PlaybackCounter>,
    dirty: std::collections::HashSet<PlaybackAccountingId>,
    flush_requested: bool,
}

#[derive(Default)]
struct PlaybackCounter {
    played_nanos: u128,
    acknowledged_ms: u64,
    duration_ms: Option<u64>,
    acknowledged_duration_ms: Option<u64>,
}

#[derive(Default)]
struct PlaybackAccountingShared {
    state: Mutex<PlaybackAccountingState>,
    changed: Condvar,
}

enum Command {
    Load(PathBuf, Option<PlaybackAccountingId>),
    LoadAndPlay(PathBuf, Option<PlaybackAccountingId>),
    LoadPreservingPlayback(PathBuf, Option<PlaybackAccountingId>),
    Play,
    Pause,
    Stop,
    Seek(Duration),
    SetVolume(f32),
    CheckpointAccounting,
    Shutdown,
}

/// A queued command acknowledgement. Submitting a request is non-blocking;
/// callers choose where to wait for the worker's authoritative post-command
/// snapshot.
pub struct CommandTicket {
    response: Receiver<Result<PlaybackSnapshot, AudioError>>,
}

impl CommandTicket {
    pub fn wait(self, timeout: Duration) -> Result<PlaybackSnapshot, AudioError> {
        match self.response.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(AudioError::CommandTimeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(AudioError::WorkerStopped),
        }
    }
}

impl PlayerHandle {
    /// Create the Windows Rodio/CPAL backend on a worker thread.
    pub fn new() -> Result<Self, AudioError> {
        #[cfg(windows)]
        {
            Self::with_backend_factory(|| {
                Ok(Box::new(rodio_backend::RodioBackend::new()?) as Box<dyn AudioBackend>)
            })
        }

        #[cfg(not(windows))]
        {
            Err(AudioError::UnsupportedPlatform)
        }
    }

    /// Spawn a worker with an injected backend factory. This is also useful for
    /// deterministic tests and for replacing the native backend later.
    pub fn with_backend_factory<F>(factory: F) -> Result<Self, AudioError>
    where
        F: FnOnce() -> Result<Box<dyn AudioBackend>, AudioError> + Send + 'static,
    {
        let snapshot = Arc::new(RwLock::new(PlaybackSnapshot::default()));
        let worker_snapshot = Arc::clone(&snapshot);
        let (command_tx, command_rx) = mpsc::sync_channel(COMMAND_CAPACITY);
        let (event_tx, event_rx) = mpsc::sync_channel(EVENT_CAPACITY);
        let accounting = Arc::new(PlaybackAccountingShared::default());
        let worker_accounting = Arc::clone(&accounting);
        let worker = thread::Builder::new()
            .name("moe-audio-player".to_owned())
            .spawn(move || {
                worker_loop(
                    factory,
                    command_rx,
                    worker_snapshot,
                    event_tx,
                    worker_accounting,
                )
            })
            .map_err(|error| AudioError::WorkerStart(error.to_string()))?;

        Ok(Self {
            inner: Arc::new(PlayerInner {
                commands: command_tx,
                snapshot,
                events: Mutex::new(event_rx),
                accounting,
                worker: Mutex::new(Some(worker)),
            }),
        })
    }

    /// Queue a local file for playback. The path is preserved as an OS path;
    /// it is not converted to a lossy UTF-8 string.
    pub fn load(&self, path: impl Into<PathBuf>) -> Result<(), AudioError> {
        self.enqueue(Command::Load(path.into(), None))
    }

    pub fn play(&self) -> Result<(), AudioError> {
        self.enqueue(Command::Play)
    }

    pub fn pause(&self) -> Result<(), AudioError> {
        self.enqueue(Command::Pause)
    }

    pub fn stop(&self) -> Result<(), AudioError> {
        self.enqueue(Command::Stop)
    }

    /// Queue a seek request. The worker clamps it to the known duration.
    pub fn seek(&self, position: Duration) -> Result<(), AudioError> {
        self.enqueue(Command::Seek(position))
    }

    /// Queue a linear volume change in the range `0.0..=1.0`.
    pub fn set_volume(&self, volume: f32) -> Result<(), AudioError> {
        if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
            return Err(AudioError::InvalidVolume);
        }
        self.enqueue(Command::SetVolume(volume))
    }

    pub fn request_play(&self) -> Result<CommandTicket, AudioError> {
        self.enqueue_with_ack(Command::Play)
    }

    /// Atomically load and start a track as one bounded worker command.
    pub fn request_load_and_play(
        &self,
        path: impl Into<PathBuf>,
    ) -> Result<CommandTicket, AudioError> {
        self.enqueue_with_ack(Command::LoadAndPlay(path.into(), None))
    }

    /// Load a track paused while associating all subsequent audio accounting
    /// with the caller's opaque per-track identity.
    pub fn request_load_accounted(
        &self,
        path: impl Into<PathBuf>,
        accounting_id: PlaybackAccountingId,
    ) -> Result<CommandTicket, AudioError> {
        self.enqueue_with_ack(Command::Load(path.into(), Some(accounting_id)))
    }

    pub fn request_load_and_play_accounted(
        &self,
        path: impl Into<PathBuf>,
        accounting_id: PlaybackAccountingId,
    ) -> Result<CommandTicket, AudioError> {
        self.enqueue_with_ack(Command::LoadAndPlay(path.into(), Some(accounting_id)))
    }

    /// Load a selected track while preserving the playback intent observed by
    /// the worker when this command executes. An empty player starts the first
    /// selected track; a paused/ready/restored player stays nonplaying.
    pub fn request_load_preserving_playback(
        &self,
        path: impl Into<PathBuf>,
    ) -> Result<CommandTicket, AudioError> {
        self.enqueue_with_ack(Command::LoadPreservingPlayback(path.into(), None))
    }

    pub fn request_load_preserving_playback_accounted(
        &self,
        path: impl Into<PathBuf>,
        accounting_id: PlaybackAccountingId,
    ) -> Result<CommandTicket, AudioError> {
        self.enqueue_with_ack(Command::LoadPreservingPlayback(
            path.into(),
            Some(accounting_id),
        ))
    }

    pub fn request_pause(&self) -> Result<CommandTicket, AudioError> {
        self.enqueue_with_ack(Command::Pause)
    }

    pub fn request_seek(&self, position: Duration) -> Result<CommandTicket, AudioError> {
        self.enqueue_with_ack(Command::Seek(position))
    }

    pub fn request_set_volume(&self, volume: f32) -> Result<CommandTicket, AudioError> {
        if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
            return Err(AudioError::InvalidVolume);
        }
        self.enqueue_with_ack(Command::SetVolume(volume))
    }

    /// Sample and request persistence of the final active playback interval
    /// without changing its state.
    pub fn request_playback_accounting_checkpoint(&self) -> Result<CommandTicket, AudioError> {
        self.enqueue_with_ack(Command::CheckpointAccounting)
    }

    /// Return the most recent playback snapshot. Audio operations never hold
    /// this lock while opening files, decoding, or talking to the device.
    pub fn snapshot(&self) -> PlaybackSnapshot {
        read_snapshot(&self.inner.snapshot)
    }

    /// Read all counters whose cumulative value or duration has not yet been
    /// acknowledged by the persistence worker. Reading never clears data.
    pub fn playback_accounting_checkpoints(&self) -> Vec<PlaybackAccountingCheckpoint> {
        accounting_checkpoints(&self.inner.accounting)
    }

    /// Acknowledge only the exact checkpoint accepted by the database. Newer
    /// audio-thread progress remains pending for the next batch.
    pub fn acknowledge_playback_accounting(
        &self,
        id: PlaybackAccountingId,
        played_ms: u64,
        duration_ms: Option<u64>,
    ) {
        acknowledge_accounting(&self.inner.accounting, id, played_ms, duration_ms);
    }

    /// Wait until an audio boundary requests an immediate persistence pass or
    /// until the caller's bounded periodic interval expires.
    pub fn wait_for_playback_accounting_flush(&self, timeout: Duration) -> bool {
        let mut state = self
            .inner
            .accounting
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.flush_requested {
            let (next, _) = self
                .inner
                .accounting
                .changed
                .wait_timeout(state, timeout)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
        }
        let requested = state.flush_requested;
        state.flush_requested = false;
        requested
    }

    pub fn request_playback_accounting_flush(&self) {
        let mut state = self
            .inner
            .accounting
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.flush_requested = true;
        self.inner.accounting.changed.notify_one();
    }

    /// Drain one pending state/error notification without waiting.
    pub fn try_recv_event(&self) -> Option<AudioEvent> {
        self.inner
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .try_recv()
            .ok()
    }

    fn enqueue(&self, command: Command) -> Result<(), AudioError> {
        self.enqueue_message(command, None)
    }

    fn enqueue_with_ack(&self, command: Command) -> Result<CommandTicket, AudioError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.enqueue_message(command, Some(response_tx))?;
        Ok(CommandTicket {
            response: response_rx,
        })
    }

    fn enqueue_message(
        &self,
        command: Command,
        acknowledgement: Option<SyncSender<Result<PlaybackSnapshot, AudioError>>>,
    ) -> Result<(), AudioError> {
        match self.inner.commands.try_send(QueuedCommand {
            command,
            acknowledgement,
        }) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(AudioError::CommandQueueFull),
            Err(TrySendError::Disconnected(_)) => Err(AudioError::WorkerStopped),
        }
    }
}

struct QueuedCommand {
    command: Command,
    acknowledgement: Option<SyncSender<Result<PlaybackSnapshot, AudioError>>>,
}

impl Drop for PlayerInner {
    fn drop(&mut self) {
        // Sending shutdown through the bounded queue may wait for the worker to
        // drain earlier commands, but public playback methods remain non-blocking.
        let _ = self.commands.send(QueuedCommand {
            command: Command::Shutdown,
            acknowledgement: None,
        });
        if let Some(worker) = self
            .worker
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = worker.join();
        }
    }
}

fn worker_loop<F>(
    factory: F,
    commands: Receiver<QueuedCommand>,
    snapshot: Arc<RwLock<PlaybackSnapshot>>,
    events: SyncSender<AudioEvent>,
    accounting: Arc<PlaybackAccountingShared>,
) where
    F: FnOnce() -> Result<Box<dyn AudioBackend>, AudioError>,
{
    let mut backend = match factory() {
        Ok(mut backend) => {
            if let Err(error) = backend.set_volume(1.0) {
                set_error(&snapshot, &events, error);
                None
            } else {
                set_state(&snapshot, &events, PlaybackState::Empty);
                Some(backend)
            }
        }
        Err(error) => {
            set_error(&snapshot, &events, error);
            None
        }
    };
    let mut current_path: Option<PathBuf> = None;
    let mut meter = PlaybackMeter::new(Arc::clone(&accounting));

    loop {
        match commands.recv_timeout(POSITION_POLL_INTERVAL) {
            Ok(queued) if matches!(queued.command, Command::Shutdown) => {
                sample_before_command(&mut meter, &mut backend, &snapshot);
                meter.request_flush();
                break;
            }
            Ok(queued) => {
                // Sample the old identity before a command can pause, seek,
                // stop, or replace it. AudioEvent delivery is intentionally
                // not involved in the accounting path.
                sample_before_command(&mut meter, &mut backend, &snapshot);
                let result = handle_command(
                    queued.command,
                    &mut backend,
                    &mut current_path,
                    &mut meter,
                    &snapshot,
                    &events,
                );
                update_position(&mut backend, &snapshot, &events, &mut meter);
                if let Some(acknowledgement) = queued.acknowledgement {
                    let response = result.map(|()| read_snapshot(&snapshot));
                    let _ = acknowledgement.try_send(response);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        update_position(&mut backend, &snapshot, &events, &mut meter);
    }
}

fn handle_command(
    command: Command,
    backend: &mut Option<Box<dyn AudioBackend>>,
    current_path: &mut Option<PathBuf>,
    meter: &mut PlaybackMeter,
    snapshot: &Arc<RwLock<PlaybackSnapshot>>,
    events: &SyncSender<AudioEvent>,
) -> Result<(), AudioError> {
    if matches!(
        &command,
        Command::Load(_, _)
            | Command::LoadAndPlay(_, _)
            | Command::LoadPreservingPlayback(_, _)
            | Command::Pause
            | Command::Stop
            | Command::Seek(_)
            | Command::CheckpointAccounting
            | Command::Shutdown
    ) {
        meter.request_flush();
    }
    let Some(backend) = backend.as_mut() else {
        let error = AudioError::BackendUnavailable;
        set_error(snapshot, events, error.clone());
        return Err(error);
    };

    match command {
        Command::Load(path, accounting_id) => {
            update_snapshot(snapshot, events, |state| {
                state.state = PlaybackState::Loading;
                state.position = Duration::ZERO;
                state.duration = None;
                state.last_error = None;
            });
            match backend.load(&path) {
                Ok(duration) => {
                    *current_path = Some(path);
                    meter.activate(accounting_id, duration);
                    update_snapshot(snapshot, events, |state| {
                        state.state = PlaybackState::Ready;
                        state.position = Duration::ZERO;
                        state.duration = duration;
                        state.last_error = None;
                    });
                }
                Err(error) => {
                    let _ = backend.stop();
                    *current_path = None;
                    meter.activate(None, None);
                    set_error(snapshot, events, error.clone());
                    return Err(error);
                }
            }
        }
        Command::LoadAndPlay(path, accounting_id) => {
            update_snapshot(snapshot, events, |state| {
                state.state = PlaybackState::Loading;
                state.position = Duration::ZERO;
                state.duration = None;
                state.last_error = None;
            });
            match backend.load(&path) {
                Ok(duration) => {
                    *current_path = Some(path);
                    meter.activate(accounting_id, duration);
                    update_snapshot(snapshot, events, |state| {
                        state.state = PlaybackState::Ready;
                        state.position = Duration::ZERO;
                        state.duration = duration;
                        state.last_error = None;
                    });
                }
                Err(error) => {
                    let _ = backend.stop();
                    *current_path = None;
                    meter.activate(None, None);
                    set_error(snapshot, events, error.clone());
                    return Err(error);
                }
            }
            if let Err(error) = backend.play() {
                set_error(snapshot, events, error.clone());
                return Err(error);
            }
            set_state(snapshot, events, PlaybackState::Playing);
        }
        Command::LoadPreservingPlayback(path, accounting_id) => {
            // Sample intent on the audio worker, after earlier queued commands
            // (such as Pause) have completed. A Tauri-side snapshot would race
            // with the bounded actor queue.
            let prior_state = read_snapshot(snapshot).state;
            update_snapshot(snapshot, events, |state| {
                state.state = PlaybackState::Loading;
                state.position = Duration::ZERO;
                state.duration = None;
                state.last_error = None;
            });
            match backend.load(&path) {
                Ok(duration) => {
                    *current_path = Some(path);
                    meter.activate(accounting_id, duration);
                    update_snapshot(snapshot, events, |state| {
                        state.state = PlaybackState::Ready;
                        state.position = Duration::ZERO;
                        state.duration = duration;
                        state.last_error = None;
                    });
                }
                Err(error) => {
                    let _ = backend.stop();
                    *current_path = None;
                    meter.activate(None, None);
                    set_error(snapshot, events, error.clone());
                    return Err(error);
                }
            }
            if matches!(prior_state, PlaybackState::Playing | PlaybackState::Empty) {
                if let Err(error) = backend.play() {
                    set_error(snapshot, events, error.clone());
                    return Err(error);
                }
                set_state(snapshot, events, PlaybackState::Playing);
            } else if prior_state == PlaybackState::Paused {
                set_state(snapshot, events, PlaybackState::Paused);
            }
        }
        Command::Play => {
            let Some(path) = current_path.as_deref() else {
                let error = AudioError::NoTrackLoaded;
                set_error(snapshot, events, error.clone());
                return Err(error);
            };
            let state = read_snapshot(snapshot).state;
            if matches!(state, PlaybackState::Stopped | PlaybackState::Ended) {
                match backend.load(path) {
                    Ok(duration) => update_snapshot(snapshot, events, |state| {
                        state.position = Duration::ZERO;
                        state.duration = duration;
                    }),
                    Err(error) => {
                        set_error(snapshot, events, error.clone());
                        return Err(error);
                    }
                }
            }
            match backend.play() {
                Ok(()) => update_snapshot(snapshot, events, |state| {
                    state.state = PlaybackState::Playing;
                    state.last_error = None;
                }),
                Err(error) => {
                    set_error(snapshot, events, error.clone());
                    return Err(error);
                }
            }
        }
        Command::Pause => {
            let state = read_snapshot(snapshot).state;
            if state == PlaybackState::Playing {
                match backend.pause() {
                    Ok(()) => {
                        set_state(snapshot, events, PlaybackState::Paused);
                        meter.reset_anchor(PlaybackState::Paused, backend.position());
                    }
                    Err(error) => {
                        meter.reset_anchor(state, backend.position());
                        set_error(snapshot, events, error.clone());
                        return Err(error);
                    }
                }
            }
        }
        Command::Stop => match backend.stop() {
            Ok(()) => {
                let state = if current_path.is_some() {
                    PlaybackState::Stopped
                } else {
                    PlaybackState::Empty
                };
                update_snapshot(snapshot, events, |snapshot| {
                    snapshot.state = state;
                    snapshot.position = Duration::ZERO;
                    snapshot.last_error = None;
                });
                meter.reset_anchor(state, Duration::ZERO);
            }
            Err(error) => {
                set_error(snapshot, events, error.clone());
                return Err(error);
            }
        },
        Command::Seek(position) => {
            if current_path.is_none() {
                let error = AudioError::NoTrackLoaded;
                set_error(snapshot, events, error.clone());
                return Err(error);
            }
            let state = read_snapshot(snapshot).state;
            if matches!(state, PlaybackState::Stopped | PlaybackState::Ended) {
                let path = current_path.as_deref().expect("checked above");
                match backend.load(path) {
                    Ok(duration) => update_snapshot(snapshot, events, |snapshot| {
                        snapshot.state = PlaybackState::Paused;
                        snapshot.position = Duration::ZERO;
                        snapshot.duration = duration;
                    }),
                    Err(error) => {
                        set_error(snapshot, events, error.clone());
                        return Err(error);
                    }
                }
            }
            let duration = read_snapshot(snapshot).duration;
            let requested = duration.map_or(position, |duration| position.min(duration));
            match backend.seek(requested) {
                Ok(actual_position) => {
                    update_snapshot(snapshot, events, |snapshot| {
                        snapshot.position = actual_position;
                        snapshot.last_error = None;
                    });
                    meter.reset_anchor(state, actual_position);
                }
                Err(error) => {
                    meter.reset_anchor(state, backend.position());
                    set_error(snapshot, events, error.clone());
                    return Err(error);
                }
            }
        }
        Command::SetVolume(volume) => match backend.set_volume(volume) {
            Ok(()) => update_snapshot(snapshot, events, |snapshot| snapshot.volume = volume),
            Err(error) => {
                set_error(snapshot, events, error.clone());
                return Err(error);
            }
        },
        Command::CheckpointAccounting => meter.request_flush(),
        Command::Shutdown => {}
    }
    Ok(())
}

fn update_position(
    backend: &mut Option<Box<dyn AudioBackend>>,
    snapshot: &Arc<RwLock<PlaybackSnapshot>>,
    events: &SyncSender<AudioEvent>,
    meter: &mut PlaybackMeter,
) {
    let Some(backend) = backend.as_mut() else {
        return;
    };
    if let Some(error) = backend.take_error() {
        set_error(snapshot, events, error);
        return;
    }
    let current = read_snapshot(snapshot);
    if current.state != PlaybackState::Playing {
        meter.reset_anchor(current.state, backend.position());
        return;
    }
    if backend.is_empty() {
        let position = current.duration.unwrap_or_else(|| backend.position());
        meter.observe(current.state, position, current.duration, Instant::now());
        update_snapshot(snapshot, events, |state| {
            state.position = position;
            state.state = PlaybackState::Ended;
        });
        meter.request_flush();
        meter.reset_anchor(PlaybackState::Ended, position);
    } else {
        let position = backend.position();
        meter.observe(current.state, position, current.duration, Instant::now());
        update_snapshot(snapshot, events, |state| {
            state.position = state
                .duration
                .map_or(position, |duration| position.min(duration));
        });
    }
}

fn sample_before_command(
    meter: &mut PlaybackMeter,
    backend: &mut Option<Box<dyn AudioBackend>>,
    snapshot: &Arc<RwLock<PlaybackSnapshot>>,
) {
    let Some(backend) = backend.as_mut() else {
        return;
    };
    let current = read_snapshot(snapshot);
    let position = if backend.is_empty() {
        current.duration.unwrap_or_else(|| backend.position())
    } else {
        backend.position()
    };
    meter.observe(current.state, position, current.duration, Instant::now());
}

struct PlaybackMeter {
    shared: Arc<PlaybackAccountingShared>,
    active_id: Option<PlaybackAccountingId>,
    segment: Option<(Instant, Duration, u128)>,
}

impl PlaybackMeter {
    fn new(shared: Arc<PlaybackAccountingShared>) -> Self {
        Self {
            shared,
            active_id: None,
            segment: None,
        }
    }

    fn activate(&mut self, id: Option<PlaybackAccountingId>, duration: Option<Duration>) {
        self.active_id = id;
        self.segment = None;
        if let Some(id) = id {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let counter = state.counters.entry(id).or_default();
            if let Some(duration) = duration {
                let duration_ms = Some(duration.as_millis().min(u128::from(u64::MAX)) as u64);
                if counter.duration_ms != duration_ms {
                    counter.duration_ms = duration_ms;
                    state.dirty.insert(id);
                }
            }
        }
    }

    fn observe(
        &mut self,
        state: PlaybackState,
        position: Duration,
        duration: Option<Duration>,
        now: Instant,
    ) {
        if state != PlaybackState::Playing {
            self.segment = None;
            return;
        }
        let Some(id) = self.active_id else {
            self.segment = None;
            return;
        };
        let Some((started_at, started_position, mut credited_nanos)) = self.segment else {
            self.segment = Some((now, position, 0));
            return;
        };
        if position < started_position {
            // A backend position discontinuity is treated like a seek, never
            // as negative or fabricated listening time.
            self.segment = Some((now, position, 0));
            return;
        }
        let elapsed = now.saturating_duration_since(started_at);
        let natural_advance = position.saturating_sub(started_position);
        let allowed_total = elapsed.min(natural_advance).as_nanos();
        let newly_credited = allowed_total.saturating_sub(credited_nanos);
        if newly_credited > 0 {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let duration_changed = {
                let counter = state.counters.entry(id).or_default();
                counter.played_nanos = counter.played_nanos.saturating_add(newly_credited);
                duration
                    .map(|duration| {
                        let duration_ms =
                            Some(duration.as_millis().min(u128::from(u64::MAX)) as u64);
                        if counter.duration_ms != duration_ms {
                            counter.duration_ms = duration_ms;
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false)
            };
            state.dirty.insert(id);
            if duration_changed {
                state.dirty.insert(id);
            }
        }
        credited_nanos = credited_nanos.max(allowed_total);
        self.segment = Some((started_at, started_position, credited_nanos));
    }

    fn reset_anchor(&mut self, state: PlaybackState, position: Duration) {
        self.reset_anchor_at(state, position, Instant::now());
    }

    fn reset_anchor_at(&mut self, state: PlaybackState, position: Duration, now: Instant) {
        self.segment = (state == PlaybackState::Playing).then_some((now, position, 0));
    }

    fn request_flush(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.flush_requested = true;
        self.shared.changed.notify_one();
    }
}

fn acknowledge_accounting(
    shared: &PlaybackAccountingShared,
    id: PlaybackAccountingId,
    played_ms: u64,
    duration_ms: Option<u64>,
) {
    let mut state = shared
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fully_acknowledged = if let Some(counter) = state.counters.get_mut(&id) {
        counter.acknowledged_ms = counter.acknowledged_ms.max(played_ms);
        if counter.duration_ms == duration_ms {
            counter.acknowledged_duration_ms = duration_ms;
        }
        let current_ms = (counter.played_nanos / 1_000_000).min(u128::from(u64::MAX)) as u64;
        current_ms <= counter.acknowledged_ms
            && counter.duration_ms == counter.acknowledged_duration_ms
    } else {
        false
    };
    if fully_acknowledged {
        state.dirty.remove(&id);
    }
}

fn accounting_checkpoints(shared: &PlaybackAccountingShared) -> Vec<PlaybackAccountingCheckpoint> {
    let state = shared
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state
        .dirty
        .iter()
        .filter_map(|id| {
            state
                .counters
                .get(id)
                .map(|counter| PlaybackAccountingCheckpoint {
                    id: *id,
                    played_ms: (counter.played_nanos / 1_000_000).min(u128::from(u64::MAX)) as u64,
                    duration_ms: counter.duration_ms,
                })
        })
        .collect()
}

fn read_snapshot(snapshot: &RwLock<PlaybackSnapshot>) -> PlaybackSnapshot {
    snapshot
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn set_state(
    snapshot: &Arc<RwLock<PlaybackSnapshot>>,
    events: &SyncSender<AudioEvent>,
    state: PlaybackState,
) {
    update_snapshot(snapshot, events, |snapshot| snapshot.state = state);
}

fn set_error(
    snapshot: &Arc<RwLock<PlaybackSnapshot>>,
    events: &SyncSender<AudioEvent>,
    error: AudioError,
) {
    update_snapshot(snapshot, events, |snapshot| {
        snapshot.state = PlaybackState::Error;
        snapshot.last_error = Some(error.clone());
    });
    let _ = events.try_send(AudioEvent::Error(error));
}

fn update_snapshot(
    snapshot: &Arc<RwLock<PlaybackSnapshot>>,
    events: &SyncSender<AudioEvent>,
    update: impl FnOnce(&mut PlaybackSnapshot),
) {
    let (previous_state, current_state) = {
        let mut state = snapshot
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous_state = state.state;
        update(&mut state);
        (previous_state, state.state)
    };
    if current_state != previous_state {
        let _ = events.try_send(AudioEvent::StateChanged(current_state));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    #[derive(Default)]
    struct FakeBackend {
        loaded: bool,
        position: Duration,
    }

    impl AudioBackend for FakeBackend {
        fn load(&mut self, _path: &Path) -> Result<Option<Duration>, AudioError> {
            self.loaded = true;
            self.position = Duration::ZERO;
            Ok(Some(Duration::from_secs(5)))
        }

        fn play(&mut self) -> Result<(), AudioError> {
            if !self.loaded {
                return Err(AudioError::NoTrackLoaded);
            }
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
            if !self.loaded {
                return Err(AudioError::NoTrackLoaded);
            }
            self.position = position.min(Duration::from_secs(5));
            Ok(self.position)
        }

        fn set_volume(&mut self, _volume: f32) -> Result<(), AudioError> {
            Ok(())
        }

        fn position(&self) -> Duration {
            self.position
        }

        fn is_empty(&self) -> bool {
            !self.loaded
        }
    }

    fn player() -> PlayerHandle {
        PlayerHandle::with_backend_factory(|| Ok(Box::<FakeBackend>::default()))
            .expect("worker should start")
    }

    #[test]
    fn process_start_probe_distinguishes_current_and_nonexistent_process() {
        let current = process_started_utc_ms(std::process::id())
            .expect("query current process identity")
            .expect("current process should exist");
        assert!(current > 0);
        assert_eq!(
            process_started_utc_ms(u32::MAX).expect("query nonexistent PID"),
            None
        );
    }

    fn wait_for(player: &PlayerHandle, expected: PlaybackState) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if player.snapshot().state == expected {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!(
            "timed out waiting for {expected:?}; snapshot: {:?}",
            player.snapshot()
        );
    }

    #[test]
    fn listening_meter_requires_playing_state_and_natural_position_advance() {
        let shared = Arc::new(PlaybackAccountingShared::default());
        let mut meter = PlaybackMeter::new(Arc::clone(&shared));
        let id = PlaybackAccountingId::new(7);
        let start = Instant::now();
        meter.activate(Some(id), Some(Duration::from_secs(30)));
        meter.observe(
            PlaybackState::Ready,
            Duration::ZERO,
            Some(Duration::from_secs(30)),
            start,
        );
        meter.observe(
            PlaybackState::Playing,
            Duration::ZERO,
            Some(Duration::from_secs(30)),
            start + Duration::from_secs(1),
        );
        meter.observe(
            PlaybackState::Playing,
            Duration::from_millis(750),
            Some(Duration::from_secs(30)),
            start + Duration::from_secs(2),
        );
        meter.observe(
            PlaybackState::Playing,
            Duration::from_millis(750),
            Some(Duration::from_secs(30)),
            start + Duration::from_secs(3),
        );
        meter.observe(
            PlaybackState::Paused,
            Duration::from_millis(750),
            Some(Duration::from_secs(30)),
            start + Duration::from_secs(4),
        );

        let state = shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(state.counters[&id].played_nanos, 750_000_000);
        assert_eq!(state.counters[&id].duration_ms, Some(30_000));
    }

    #[test]
    fn listening_meter_resets_on_seek_and_keeps_new_progress_dirty_during_ack() {
        let shared = Arc::new(PlaybackAccountingShared::default());
        let mut meter = PlaybackMeter::new(Arc::clone(&shared));
        let id = PlaybackAccountingId::new(9);
        let start = Instant::now();
        meter.activate(Some(id), Some(Duration::from_secs(60)));
        meter.observe(
            PlaybackState::Playing,
            Duration::ZERO,
            Some(Duration::from_secs(60)),
            start,
        );
        meter.observe(
            PlaybackState::Playing,
            Duration::from_millis(100),
            Some(Duration::from_secs(60)),
            start + Duration::from_millis(100),
        );

        let checkpoint = PlaybackAccountingCheckpoint {
            id,
            played_ms: 100,
            duration_ms: Some(60_000),
        };
        meter.reset_anchor_at(
            PlaybackState::Playing,
            Duration::from_secs(20),
            start + Duration::from_millis(100),
        );
        meter.observe(
            PlaybackState::Playing,
            Duration::from_millis(20_050),
            Some(Duration::from_secs(60)),
            start + Duration::from_millis(150),
        );

        let current = shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .counters[&id]
            .played_nanos;
        assert_eq!(current, 150_000_000, "seek jump itself adds no time");

        acknowledge_accounting(&shared, id, checkpoint.played_ms, checkpoint.duration_ms);
        let pending = accounting_checkpoints(&shared);
        assert_eq!(pending.len(), 1);
        assert!(pending[0].played_ms > checkpoint.played_ms);
    }

    #[test]
    fn listening_meter_handles_quantized_positions_and_dense_non_state_commands() {
        let shared = Arc::new(PlaybackAccountingShared::default());
        let mut meter = PlaybackMeter::new(Arc::clone(&shared));
        let id = PlaybackAccountingId::new(11);
        let start = Instant::now();
        meter.activate(Some(id), Some(Duration::from_secs(10)));
        meter.observe(
            PlaybackState::Playing,
            Duration::ZERO,
            Some(Duration::from_secs(10)),
            start,
        );
        // These represent frequent volume/poll samples around a decoder whose
        // reported position advances in coarse 40 ms steps.
        for (elapsed_ms, position_ms) in [(30, 0), (40, 40), (45, 40), (80, 80)] {
            meter.observe(
                PlaybackState::Playing,
                Duration::from_millis(position_ms),
                Some(Duration::from_secs(10)),
                start + Duration::from_millis(elapsed_ms),
            );
        }
        let state = shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(state.counters[&id].played_nanos, 80_000_000);
    }

    #[test]
    fn lifecycle_commands_update_snapshot_and_clamp_seek() {
        let player = player();
        wait_for(&player, PlaybackState::Empty);
        player.load(PathBuf::from("fixture.wav")).unwrap();
        wait_for(&player, PlaybackState::Ready);
        player.seek(Duration::from_secs(9)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && player.snapshot().position != Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(player.snapshot().position, Duration::from_secs(5));
        player.set_volume(0.25).unwrap();
        player.play().unwrap();
        wait_for(&player, PlaybackState::Playing);
        player.pause().unwrap();
        wait_for(&player, PlaybackState::Paused);
        player.stop().unwrap();
        wait_for(&player, PlaybackState::Stopped);
        assert_eq!(player.snapshot().position, Duration::ZERO);
        assert_eq!(player.snapshot().volume, 0.25);
        player.pause().unwrap();
        thread::sleep(Duration::from_millis(20));
        assert_eq!(player.snapshot().state, PlaybackState::Stopped);
        player.play().unwrap();
        wait_for(&player, PlaybackState::Playing);
    }

    #[test]
    fn seek_updates_worker_snapshot_while_playing_and_paused() {
        let player = player();
        wait_for(&player, PlaybackState::Empty);
        player.load(PathBuf::from("fixture.wav")).unwrap();
        wait_for(&player, PlaybackState::Ready);

        player.play().unwrap();
        wait_for(&player, PlaybackState::Playing);
        player.seek(Duration::from_secs(2)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let snapshot = player.snapshot();
            if snapshot.state == PlaybackState::Playing
                && snapshot.position == Duration::from_secs(2)
            {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(player.snapshot().state, PlaybackState::Playing);
        assert_eq!(player.snapshot().position, Duration::from_secs(2));

        player.pause().unwrap();
        wait_for(&player, PlaybackState::Paused);
        player.seek(Duration::from_secs(4)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let snapshot = player.snapshot();
            if snapshot.state == PlaybackState::Paused
                && snapshot.position == Duration::from_secs(4)
            {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(player.snapshot().state, PlaybackState::Paused);
        assert_eq!(player.snapshot().position, Duration::from_secs(4));
    }

    #[test]
    fn acknowledged_commands_return_post_command_snapshots() {
        let player = player();
        wait_for(&player, PlaybackState::Empty);
        player.load(PathBuf::from("fixture.wav")).unwrap();
        wait_for(&player, PlaybackState::Ready);

        let playing = player
            .request_play()
            .unwrap()
            .wait(Duration::from_secs(1))
            .unwrap();
        assert_eq!(playing.state, PlaybackState::Playing);

        let sought = player
            .request_seek(Duration::from_secs(3))
            .unwrap()
            .wait(Duration::from_secs(1))
            .unwrap();
        assert_eq!(sought.state, PlaybackState::Playing);
        assert_eq!(sought.position, Duration::from_secs(3));

        let paused = player
            .request_pause()
            .unwrap()
            .wait(Duration::from_secs(1))
            .unwrap();
        assert_eq!(paused.state, PlaybackState::Paused);

        let volume = player
            .request_set_volume(0.4)
            .unwrap()
            .wait(Duration::from_secs(1))
            .unwrap();
        assert_eq!(volume.volume, 0.4);
        assert_eq!(volume.state, PlaybackState::Paused);
    }

    #[test]
    fn selected_track_load_uses_worker_time_playback_intent() {
        let handle = player();
        wait_for(&handle, PlaybackState::Empty);

        let first = handle
            .request_load_preserving_playback("first.wav")
            .unwrap()
            .wait(Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            first.state,
            PlaybackState::Playing,
            "first selection starts playback"
        );

        // Queue Pause and then selection without reading a Tauri-side snapshot.
        // The second command must observe the completed Pause on the worker.
        let paused = handle.request_pause().unwrap();
        let selected = handle
            .request_load_preserving_playback("second.wav")
            .unwrap();
        assert_eq!(
            paused.wait(Duration::from_secs(1)).unwrap().state,
            PlaybackState::Paused
        );
        assert_eq!(
            selected.wait(Duration::from_secs(1)).unwrap().state,
            PlaybackState::Paused
        );

        handle
            .request_play()
            .unwrap()
            .wait(Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            handle
                .request_load_preserving_playback("third.wav")
                .unwrap()
                .wait(Duration::from_secs(1))
                .unwrap()
                .state,
            PlaybackState::Playing,
            "selection while playing keeps playback active"
        );

        let restored = player();
        wait_for(&restored, PlaybackState::Empty);
        restored.load(PathBuf::from("restored.wav")).unwrap();
        wait_for(&restored, PlaybackState::Ready);
        assert_eq!(
            restored
                .request_load_preserving_playback("replacement.wav")
                .unwrap()
                .wait(Duration::from_secs(1))
                .unwrap()
                .state,
            PlaybackState::Ready,
            "restored Ready state remains nonplaying"
        );
    }

    #[test]
    fn command_ticket_timeout_does_not_block_submission_or_hide_later_state() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let player = PlayerHandle::with_backend_factory(move || {
            Ok(Box::new(BlockingPlayBackend {
                started: started_tx,
                release: release_rx,
                loaded: false,
            }) as Box<dyn AudioBackend>)
        })
        .unwrap();
        wait_for(&player, PlaybackState::Empty);
        player.load(PathBuf::from("fixture.wav")).unwrap();
        wait_for(&player, PlaybackState::Ready);

        let started = Instant::now();
        let ticket = player.request_play().unwrap();
        assert!(started.elapsed() < Duration::from_millis(100));
        let started_result = started_rx.recv_timeout(Duration::from_secs(1));
        let timeout_result = ticket.wait(Duration::from_millis(10));
        release_tx.send(()).unwrap();
        started_result.expect("worker should begin playback");
        assert_eq!(timeout_result, Err(AudioError::CommandTimeout));
        let paused = player
            .request_pause()
            .unwrap()
            .wait(Duration::from_secs(1))
            .unwrap();
        assert_eq!(paused.state, PlaybackState::Paused);
    }

    struct BlockingPlayBackend {
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        loaded: bool,
    }

    impl AudioBackend for BlockingPlayBackend {
        fn load(&mut self, _path: &Path) -> Result<Option<Duration>, AudioError> {
            self.loaded = true;
            Ok(Some(Duration::from_secs(5)))
        }
        fn play(&mut self) -> Result<(), AudioError> {
            self.started.send(()).unwrap();
            self.release.recv().unwrap();
            Ok(())
        }
        fn pause(&mut self) -> Result<(), AudioError> {
            Ok(())
        }
        fn stop(&mut self) -> Result<(), AudioError> {
            Ok(())
        }
        fn seek(&mut self, position: Duration) -> Result<Duration, AudioError> {
            Ok(position)
        }
        fn set_volume(&mut self, _volume: f32) -> Result<(), AudioError> {
            Ok(())
        }
        fn position(&self) -> Duration {
            Duration::ZERO
        }
        fn is_empty(&self) -> bool {
            !self.loaded
        }
    }

    #[test]
    fn invalid_volume_is_rejected_before_queueing() {
        let player = player();
        assert_eq!(player.set_volume(f32::NAN), Err(AudioError::InvalidVolume));
        assert_eq!(player.set_volume(-0.1), Err(AudioError::InvalidVolume));
        assert_eq!(player.set_volume(1.1), Err(AudioError::InvalidVolume));
    }

    #[test]
    fn command_queue_is_bounded_and_reports_full_without_waiting() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let player = PlayerHandle::with_backend_factory(move || {
            Ok(Box::new(BlockingLoadBackend {
                started: started_tx,
                release: release_rx,
            }) as Box<dyn AudioBackend>)
        })
        .unwrap();
        player.load(PathBuf::from("fixture.wav")).unwrap();
        started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker should begin loading");

        for _ in 0..COMMAND_CAPACITY {
            player.pause().unwrap();
        }
        assert_eq!(player.play(), Err(AudioError::CommandQueueFull));

        release_tx.send(()).unwrap();
        wait_for(&player, PlaybackState::Ready);
    }

    #[test]
    fn player_handle_is_safe_for_tauri_managed_state() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PlayerHandle>();
    }

    #[test]
    fn failed_backend_initialization_is_visible_in_snapshot_and_events() {
        let player = PlayerHandle::with_backend_factory(|| {
            Err(AudioError::OutputDevice("test failure".to_owned()))
        })
        .unwrap();
        wait_for(&player, PlaybackState::Error);
        assert_eq!(
            player.snapshot().last_error,
            Some(AudioError::OutputDevice("test failure".to_owned()))
        );
        assert!(matches!(
            player.try_recv_event(),
            Some(AudioEvent::StateChanged(_))
        ));
        assert_eq!(
            player.try_recv_event(),
            Some(AudioEvent::Error(AudioError::OutputDevice(
                "test failure".to_owned()
            )))
        );
    }

    #[test]
    fn decoder_errors_are_reported_and_a_failed_load_is_not_playable() {
        let player = PlayerHandle::with_backend_factory(|| {
            Ok(Box::new(FailingLoadBackend) as Box<dyn AudioBackend>)
        })
        .unwrap();
        wait_for(&player, PlaybackState::Empty);
        player.load(PathBuf::from("unsupported.file")).unwrap();
        wait_for(&player, PlaybackState::Error);
        assert!(matches!(
            player.snapshot().last_error,
            Some(AudioError::UnsupportedFormat { .. })
        ));
        player.play().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline
            && player.snapshot().last_error != Some(AudioError::NoTrackLoaded)
        {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            player.snapshot().last_error,
            Some(AudioError::NoTrackLoaded)
        );
        assert_eq!(
            player.request_play().unwrap().wait(Duration::from_secs(1)),
            Err(AudioError::NoTrackLoaded)
        );
    }

    struct FailingLoadBackend;

    struct BlockingLoadBackend {
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }

    impl AudioBackend for BlockingLoadBackend {
        fn load(&mut self, _path: &Path) -> Result<Option<Duration>, AudioError> {
            self.started.send(()).unwrap();
            self.release.recv().unwrap();
            Ok(Some(Duration::from_secs(1)))
        }
        fn play(&mut self) -> Result<(), AudioError> {
            Ok(())
        }
        fn pause(&mut self) -> Result<(), AudioError> {
            Ok(())
        }
        fn stop(&mut self) -> Result<(), AudioError> {
            Ok(())
        }
        fn seek(&mut self, position: Duration) -> Result<Duration, AudioError> {
            Ok(position)
        }
        fn set_volume(&mut self, _volume: f32) -> Result<(), AudioError> {
            Ok(())
        }
        fn position(&self) -> Duration {
            Duration::ZERO
        }
        fn is_empty(&self) -> bool {
            false
        }
    }

    impl AudioBackend for FailingLoadBackend {
        fn load(&mut self, path: &Path) -> Result<Option<Duration>, AudioError> {
            Err(AudioError::UnsupportedFormat {
                path: path.to_path_buf(),
            })
        }
        fn play(&mut self) -> Result<(), AudioError> {
            Err(AudioError::NoTrackLoaded)
        }
        fn pause(&mut self) -> Result<(), AudioError> {
            Ok(())
        }
        fn stop(&mut self) -> Result<(), AudioError> {
            Ok(())
        }
        fn seek(&mut self, position: Duration) -> Result<Duration, AudioError> {
            Ok(position)
        }
        fn set_volume(&mut self, _volume: f32) -> Result<(), AudioError> {
            Ok(())
        }
        fn position(&self) -> Duration {
            Duration::ZERO
        }
        fn is_empty(&self) -> bool {
            true
        }
    }

    #[test]
    fn final_handle_drop_joins_worker() {
        let worker_exited = Arc::new(AtomicBool::new(false));
        let exited_from_worker = Arc::clone(&worker_exited);
        let player = PlayerHandle::with_backend_factory(move || {
            Ok(Box::new(DropObservedBackend {
                exited: exited_from_worker,
            }))
        })
        .unwrap();
        wait_for(&player, PlaybackState::Empty);
        drop(player);
        assert!(worker_exited.load(Ordering::SeqCst));
    }

    struct DropObservedBackend {
        exited: Arc<AtomicBool>,
    }

    impl Drop for DropObservedBackend {
        fn drop(&mut self) {
            self.exited.store(true, Ordering::SeqCst);
        }
    }

    impl AudioBackend for DropObservedBackend {
        fn load(&mut self, _path: &Path) -> Result<Option<Duration>, AudioError> {
            Ok(None)
        }
        fn play(&mut self) -> Result<(), AudioError> {
            Ok(())
        }
        fn pause(&mut self) -> Result<(), AudioError> {
            Ok(())
        }
        fn stop(&mut self) -> Result<(), AudioError> {
            Ok(())
        }
        fn seek(&mut self, position: Duration) -> Result<Duration, AudioError> {
            Ok(position)
        }
        fn set_volume(&mut self, _volume: f32) -> Result<(), AudioError> {
            Ok(())
        }
        fn position(&self) -> Duration {
            Duration::ZERO
        }
        fn is_empty(&self) -> bool {
            true
        }
    }
}
