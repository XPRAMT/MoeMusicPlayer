//! Background-thread Windows audio playback for local files.
//!
//! The handle exposes a small, backend-independent command surface. Native
//! device access, file opening, decoding, and seeking run on a dedicated worker
//! thread. Callers poll a bounded snapshot and may drain state/error events.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[cfg(windows)]
mod rodio_backend;

#[cfg(windows)]
pub mod system_media;

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
    commands: SyncSender<Command>,
    snapshot: Arc<RwLock<PlaybackSnapshot>>,
    events: Mutex<Receiver<AudioEvent>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

enum Command {
    Load(PathBuf),
    Play,
    Pause,
    Stop,
    Seek(Duration),
    SetVolume(f32),
    Shutdown,
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
        let worker = thread::Builder::new()
            .name("moe-audio-player".to_owned())
            .spawn(move || worker_loop(factory, command_rx, worker_snapshot, event_tx))
            .map_err(|error| AudioError::WorkerStart(error.to_string()))?;

        Ok(Self {
            inner: Arc::new(PlayerInner {
                commands: command_tx,
                snapshot,
                events: Mutex::new(event_rx),
                worker: Mutex::new(Some(worker)),
            }),
        })
    }

    /// Queue a local file for playback. The path is preserved as an OS path;
    /// it is not converted to a lossy UTF-8 string.
    pub fn load(&self, path: impl Into<PathBuf>) -> Result<(), AudioError> {
        self.enqueue(Command::Load(path.into()))
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

    /// Return the most recent playback snapshot. Audio operations never hold
    /// this lock while opening files, decoding, or talking to the device.
    pub fn snapshot(&self) -> PlaybackSnapshot {
        read_snapshot(&self.inner.snapshot)
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
        match self.inner.commands.try_send(command) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(AudioError::CommandQueueFull),
            Err(TrySendError::Disconnected(_)) => Err(AudioError::WorkerStopped),
        }
    }
}

impl Drop for PlayerInner {
    fn drop(&mut self) {
        // Sending shutdown through the bounded queue may wait for the worker to
        // drain earlier commands, but public playback methods remain non-blocking.
        let _ = self.commands.send(Command::Shutdown);
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
    commands: Receiver<Command>,
    snapshot: Arc<RwLock<PlaybackSnapshot>>,
    events: SyncSender<AudioEvent>,
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

    loop {
        match commands.recv_timeout(POSITION_POLL_INTERVAL) {
            Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(command) => {
                handle_command(command, &mut backend, &mut current_path, &snapshot, &events)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        update_position(&mut backend, &snapshot, &events);
    }
}

fn handle_command(
    command: Command,
    backend: &mut Option<Box<dyn AudioBackend>>,
    current_path: &mut Option<PathBuf>,
    snapshot: &Arc<RwLock<PlaybackSnapshot>>,
    events: &SyncSender<AudioEvent>,
) {
    let Some(backend) = backend.as_mut() else {
        set_error(snapshot, events, AudioError::BackendUnavailable);
        return;
    };

    match command {
        Command::Load(path) => {
            update_snapshot(snapshot, events, |state| {
                state.state = PlaybackState::Loading;
                state.position = Duration::ZERO;
                state.duration = None;
                state.last_error = None;
            });
            match backend.load(&path) {
                Ok(duration) => {
                    *current_path = Some(path);
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
                    set_error(snapshot, events, error);
                }
            }
        }
        Command::Play => {
            let Some(path) = current_path.as_deref() else {
                set_error(snapshot, events, AudioError::NoTrackLoaded);
                return;
            };
            let state = read_snapshot(snapshot).state;
            if matches!(state, PlaybackState::Stopped | PlaybackState::Ended) {
                match backend.load(path) {
                    Ok(duration) => update_snapshot(snapshot, events, |state| {
                        state.position = Duration::ZERO;
                        state.duration = duration;
                    }),
                    Err(error) => {
                        set_error(snapshot, events, error);
                        return;
                    }
                }
            }
            match backend.play() {
                Ok(()) => update_snapshot(snapshot, events, |state| {
                    state.state = PlaybackState::Playing;
                    state.last_error = None;
                }),
                Err(error) => set_error(snapshot, events, error),
            }
        }
        Command::Pause => {
            if read_snapshot(snapshot).state == PlaybackState::Playing {
                match backend.pause() {
                    Ok(()) => set_state(snapshot, events, PlaybackState::Paused),
                    Err(error) => set_error(snapshot, events, error),
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
            }
            Err(error) => set_error(snapshot, events, error),
        },
        Command::Seek(position) => {
            if current_path.is_none() {
                set_error(snapshot, events, AudioError::NoTrackLoaded);
                return;
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
                        set_error(snapshot, events, error);
                        return;
                    }
                }
            }
            let duration = read_snapshot(snapshot).duration;
            let requested = duration.map_or(position, |duration| position.min(duration));
            match backend.seek(requested) {
                Ok(actual_position) => update_snapshot(snapshot, events, |snapshot| {
                    snapshot.position = actual_position;
                    snapshot.last_error = None;
                }),
                Err(error) => set_error(snapshot, events, error),
            }
        }
        Command::SetVolume(volume) => match backend.set_volume(volume) {
            Ok(()) => update_snapshot(snapshot, events, |snapshot| snapshot.volume = volume),
            Err(error) => set_error(snapshot, events, error),
        },
        Command::Shutdown => {}
    }
}

fn update_position(
    backend: &mut Option<Box<dyn AudioBackend>>,
    snapshot: &Arc<RwLock<PlaybackSnapshot>>,
    events: &SyncSender<AudioEvent>,
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
        return;
    }
    if backend.is_empty() {
        update_snapshot(snapshot, events, |state| {
            state.position = state.duration.unwrap_or_else(|| backend.position());
            state.state = PlaybackState::Ended;
        });
    } else {
        let position = backend.position();
        update_snapshot(snapshot, events, |state| {
            state.position = state
                .duration
                .map_or(position, |duration| position.min(duration));
        });
    }
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
