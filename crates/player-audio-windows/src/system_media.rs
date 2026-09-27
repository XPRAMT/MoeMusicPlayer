//! Windows System Media Transport Controls (SMTC) bridge.
//!
//! The controller owns a WinRT worker thread and only publishes the audio
//! engine's latest snapshot. Its callbacks enqueue commands for the caller;
//! they never call the audio backend or Tauri directly.

use crate::{PlaybackSnapshot, PlaybackState};
use std::error::Error;
use std::fmt;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::core::{factory, HSTRING};
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
    SystemMediaTransportControls, SystemMediaTransportControlsButton,
    SystemMediaTransportControlsButtonPressedEventArgs,
    SystemMediaTransportControlsTimelineProperties,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::{
    ISystemMediaTransportControlsInterop, RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED,
};

const COMMAND_CAPACITY: usize = 8;
const EVENT_CAPACITY: usize = 32;

/// Metadata shown by Windows for the current track. File paths and app track
/// identifiers are intentionally not part of this projection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaControlMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

/// Queue capabilities advertised to Windows. They default to disabled until
/// a real queue implementation can honor these requests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MediaControlCapabilities {
    pub can_next: bool,
    pub can_previous: bool,
}

/// Authoritative audio state to project into Windows media controls.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaControlUpdate {
    pub snapshot: PlaybackSnapshot,
    pub metadata: Option<MediaControlMetadata>,
    pub capabilities: MediaControlCapabilities,
}

/// Requests received from Windows. Seek positions are measured from the start
/// of the current track and are already clamped to its known duration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SystemMediaEvent {
    Ready,
    PlayRequested,
    PauseRequested,
    StopRequested,
    NextRequested,
    PreviousRequested,
    SeekRequested(Duration),
    Error(SystemMediaError),
}

/// Failures in the optional SMTC bridge. These errors do not affect playback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SystemMediaError {
    InvalidWindowHandle,
    ThreadStart(String),
    CommandQueueFull,
    WorkerStopped,
    EventReceiverPoisoned,
    WindowsRuntime(String),
}

impl fmt::Display for SystemMediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWindowHandle => f.write_str("the SMTC window handle is null"),
            Self::ThreadStart(message) => {
                write!(f, "could not start the SMTC worker: {message}")
            }
            Self::CommandQueueFull => f.write_str("the SMTC command queue is full"),
            Self::WorkerStopped => f.write_str("the SMTC worker has stopped"),
            Self::EventReceiverPoisoned => f.write_str("the SMTC event receiver is unavailable"),
            Self::WindowsRuntime(message) => write!(f, "Windows media controls failed: {message}"),
        }
    }
}

impl Error for SystemMediaError {}

/// Cloneable handle for a window-bound SMTC worker.
///
/// `hwnd` must be the top-level HWND for this process and remain alive until
/// the final controller handle is dropped. Callers should create this optional
/// controller independently from `PlayerHandle`; a WinRT failure is reported
/// as an SMTC event and does not stop local playback.
#[derive(Clone)]
pub struct SystemMediaController {
    inner: Arc<ControllerInner>,
}

struct ControllerInner {
    commands: SyncSender<WorkerCommand>,
    events: Mutex<Receiver<SystemMediaEvent>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

enum WorkerCommand {
    Update(MediaControlUpdate),
    Shutdown,
}

impl SystemMediaController {
    /// Start the WinRT worker for a Tauri top-level window.
    ///
    /// The HWND is passed as its pointer-sized integer representation so the
    /// handle can be transferred to the worker thread. A null handle is rejected
    /// immediately; WinRT registration errors arrive as `SystemMediaEvent::Error`.
    pub fn attach(hwnd: isize) -> Result<Self, SystemMediaError> {
        if hwnd == 0 {
            return Err(SystemMediaError::InvalidWindowHandle);
        }

        let (command_tx, command_rx) = mpsc::sync_channel(COMMAND_CAPACITY);
        let (event_tx, event_rx) = mpsc::sync_channel(EVENT_CAPACITY);
        let worker = thread::Builder::new()
            .name("moemusicplayer-smtc".to_owned())
            .spawn(move || run_worker(hwnd, command_rx, event_tx))
            .map_err(|error| SystemMediaError::ThreadStart(error.to_string()))?;

        Ok(Self {
            inner: Arc::new(ControllerInner {
                commands: command_tx,
                events: Mutex::new(event_rx),
                worker: Mutex::new(Some(worker)),
            }),
        })
    }

    /// Queue the latest playback state for the dedicated WinRT worker.
    ///
    /// This method is non-blocking. If the bounded queue is full, callers can
    /// retry with the next authoritative snapshot.
    pub fn update(&self, update: MediaControlUpdate) -> Result<(), SystemMediaError> {
        match self.inner.commands.try_send(WorkerCommand::Update(update)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(SystemMediaError::CommandQueueFull),
            Err(TrySendError::Disconnected(_)) => Err(SystemMediaError::WorkerStopped),
        }
    }

    /// Read one pending Windows request without waiting.
    pub fn try_recv_event(&self) -> Result<Option<SystemMediaEvent>, SystemMediaError> {
        let receiver = self
            .inner
            .events
            .lock()
            .map_err(|_| SystemMediaError::EventReceiverPoisoned)?;
        match receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => Ok(None),
        }
    }
}

impl Drop for ControllerInner {
    fn drop(&mut self) {
        // The worker owns all WinRT interfaces and event tokens. Waking and
        // joining it here guarantees those tokens are revoked before teardown.
        let _ = self.commands.send(WorkerCommand::Shutdown);
        if let Ok(worker) = self.worker.get_mut() {
            if let Some(worker) = worker.take() {
                let _ = worker.join();
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Availability {
    can_play: bool,
    can_pause: bool,
    can_stop: bool,
    can_seek: bool,
    can_next: bool,
    can_previous: bool,
    duration: Option<Duration>,
}

fn availability_for(update: &MediaControlUpdate) -> Availability {
    let state = update.snapshot.state;
    let has_track = matches!(
        state,
        PlaybackState::Ready
            | PlaybackState::Playing
            | PlaybackState::Paused
            | PlaybackState::Stopped
            | PlaybackState::Ended
    );
    let duration = update
        .snapshot
        .duration
        .filter(|duration| !duration.is_zero());
    Availability {
        can_play: has_track && state != PlaybackState::Playing,
        can_pause: has_track && state == PlaybackState::Playing,
        can_stop: matches!(
            state,
            PlaybackState::Ready | PlaybackState::Playing | PlaybackState::Paused
        ),
        can_seek: has_track && duration.is_some(),
        can_next: has_track && update.capabilities.can_next,
        can_previous: has_track && update.capabilities.can_previous,
        duration,
    }
}

fn event_for_button(
    button: SystemMediaTransportControlsButton,
    availability: Availability,
) -> Option<SystemMediaEvent> {
    match button {
        SystemMediaTransportControlsButton::Play if availability.can_play => {
            Some(SystemMediaEvent::PlayRequested)
        }
        SystemMediaTransportControlsButton::Pause if availability.can_pause => {
            Some(SystemMediaEvent::PauseRequested)
        }
        SystemMediaTransportControlsButton::Stop if availability.can_stop => {
            Some(SystemMediaEvent::StopRequested)
        }
        SystemMediaTransportControlsButton::Next if availability.can_next => {
            Some(SystemMediaEvent::NextRequested)
        }
        SystemMediaTransportControlsButton::Previous if availability.can_previous => {
            Some(SystemMediaEvent::PreviousRequested)
        }
        _ => None,
    }
}

fn event_for_seek(requested: TimeSpan, availability: Availability) -> Option<SystemMediaEvent> {
    if !availability.can_seek {
        return None;
    }
    let requested = duration_from_timespan(requested)?;
    let position = availability
        .duration
        .map(|duration| requested.min(duration))
        .unwrap_or(requested);
    Some(SystemMediaEvent::SeekRequested(position))
}

struct NativeControls {
    controls: SystemMediaTransportControls,
    button_token: i64,
    seek_token: i64,
    availability: Arc<RwLock<Availability>>,
}

impl NativeControls {
    fn new(hwnd: isize, events: &SyncSender<SystemMediaEvent>) -> Result<Self, SystemMediaError> {
        let interop =
            factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()
                .map_err(winrt_error)?;
        // SAFETY: the public API documents that the caller supplies a live,
        // top-level HWND owned by this process. The controller lifetime is
        // documented to be no longer than that window's lifetime.
        let controls =
            unsafe { interop.GetForWindow::<SystemMediaTransportControls>(HWND(hwnd as *mut _)) }
                .map_err(winrt_error)?;

        let availability = Arc::new(RwLock::new(Availability::default()));
        let button_events = events.clone();
        let button_availability = Arc::clone(&availability);
        let button_handler = TypedEventHandler::<
            SystemMediaTransportControls,
            SystemMediaTransportControlsButtonPressedEventArgs,
        >::new(move |_, args| {
            if let Some(args) = args.as_ref() {
                if let Ok(button) = args.Button() {
                    let enabled = button_availability
                        .read()
                        .map(|availability| *availability)
                        .unwrap_or_default();
                    if let Some(event) = event_for_button(button, enabled) {
                        let _ = button_events.try_send(event);
                    }
                }
            }
            Ok(())
        });
        let button_token = controls
            .ButtonPressed(&button_handler)
            .map_err(winrt_error)?;

        let seek_events = events.clone();
        let seek_availability = Arc::clone(&availability);
        let seek_handler = TypedEventHandler::<
            SystemMediaTransportControls,
            PlaybackPositionChangeRequestedEventArgs,
        >::new(move |_, args| {
            if let Some(args) = args.as_ref() {
                if let Ok(requested) = args.RequestedPlaybackPosition() {
                    let current = seek_availability
                        .read()
                        .map(|availability| *availability)
                        .unwrap_or_default();
                    if let Some(event) = event_for_seek(requested, current) {
                        let _ = seek_events.try_send(event);
                    }
                }
            }
            Ok(())
        });
        let seek_token = match controls.PlaybackPositionChangeRequested(&seek_handler) {
            Ok(token) => token,
            Err(error) => {
                let _ = controls.RemoveButtonPressed(button_token);
                return Err(winrt_error(error));
            }
        };

        let native = Self {
            controls,
            button_token,
            seek_token,
            availability,
        };
        native.initialize()?;
        Ok(native)
    }

    fn initialize(&self) -> Result<(), SystemMediaError> {
        self.controls.SetIsEnabled(true).map_err(winrt_error)?;
        self.controls
            .SetPlaybackStatus(MediaPlaybackStatus::Closed)
            .map_err(winrt_error)?;
        set_button_availability(&self.controls, Availability::default())?;
        Ok(())
    }

    fn update(&self, update: &MediaControlUpdate) -> Result<(), SystemMediaError> {
        let availability = availability_for(update);
        update_metadata(&self.controls, update.metadata.as_ref())?;
        update_timeline(&self.controls, &update.snapshot)?;
        self.controls
            .SetPlaybackStatus(playback_status(update.snapshot.state))
            .map_err(winrt_error)?;
        set_button_availability(&self.controls, availability)?;
        if let Ok(mut current) = self.availability.write() {
            *current = availability;
        }
        Ok(())
    }
}

impl Drop for NativeControls {
    fn drop(&mut self) {
        let _ = self
            .controls
            .RemovePlaybackPositionChangeRequested(self.seek_token);
        let _ = self.controls.RemoveButtonPressed(self.button_token);
        let _ = self.controls.SetPlaybackStatus(MediaPlaybackStatus::Closed);
        let _ = self.controls.SetIsEnabled(false);
        if let Ok(updater) = self.controls.DisplayUpdater() {
            let _ = updater.ClearAll();
            let _ = updater.Update();
        }
    }
}

fn run_worker(
    hwnd: isize,
    commands: Receiver<WorkerCommand>,
    events: SyncSender<SystemMediaEvent>,
) {
    // Each WinRT interface, event token, and handler is created and released
    // on this worker's MTA apartment.
    if let Err(error) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        let _ = events.try_send(SystemMediaEvent::Error(winrt_error(error)));
        return;
    }
    let _apartment = ApartmentGuard;

    let native = match NativeControls::new(hwnd, &events) {
        Ok(native) => native,
        Err(error) => {
            let _ = events.try_send(SystemMediaEvent::Error(error));
            return;
        }
    };
    let _ = events.try_send(SystemMediaEvent::Ready);

    while let Ok(command) = commands.recv() {
        match command {
            WorkerCommand::Update(update) => {
                if let Err(error) = native.update(&update) {
                    let _ = events.try_send(SystemMediaEvent::Error(error));
                    break;
                }
            }
            WorkerCommand::Shutdown => break,
        }
    }
    drop(native);
}

struct ApartmentGuard;

impl Drop for ApartmentGuard {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

fn set_button_availability(
    controls: &SystemMediaTransportControls,
    availability: Availability,
) -> Result<(), SystemMediaError> {
    controls
        .SetIsPlayEnabled(availability.can_play)
        .map_err(winrt_error)?;
    controls
        .SetIsPauseEnabled(availability.can_pause)
        .map_err(winrt_error)?;
    controls
        .SetIsStopEnabled(availability.can_stop)
        .map_err(winrt_error)?;
    controls
        .SetIsNextEnabled(availability.can_next)
        .map_err(winrt_error)?;
    controls
        .SetIsPreviousEnabled(availability.can_previous)
        .map_err(winrt_error)?;
    Ok(())
}

fn update_metadata(
    controls: &SystemMediaTransportControls,
    metadata: Option<&MediaControlMetadata>,
) -> Result<(), SystemMediaError> {
    let updater = controls.DisplayUpdater().map_err(winrt_error)?;
    if let Some(metadata) = metadata {
        updater
            .SetType(MediaPlaybackType::Music)
            .map_err(winrt_error)?;
        let properties = updater.MusicProperties().map_err(winrt_error)?;
        properties
            .SetTitle(&HSTRING::from(
                metadata.title.as_deref().unwrap_or_default(),
            ))
            .map_err(winrt_error)?;
        properties
            .SetArtist(&HSTRING::from(
                metadata.artist.as_deref().unwrap_or_default(),
            ))
            .map_err(winrt_error)?;
        properties
            .SetAlbumTitle(&HSTRING::from(
                metadata.album.as_deref().unwrap_or_default(),
            ))
            .map_err(winrt_error)?;
    } else {
        updater.ClearAll().map_err(winrt_error)?;
    }
    updater.Update().map_err(winrt_error)
}

fn update_timeline(
    controls: &SystemMediaTransportControls,
    snapshot: &PlaybackSnapshot,
) -> Result<(), SystemMediaError> {
    let duration = snapshot.duration.filter(|duration| !duration.is_zero());
    let duration = duration.unwrap_or(Duration::ZERO);
    let position = snapshot.position.min(duration);
    let timeline = SystemMediaTransportControlsTimelineProperties::new().map_err(winrt_error)?;
    timeline
        .SetStartTime(TimeSpan { Duration: 0 })
        .map_err(winrt_error)?;
    timeline
        .SetMinSeekTime(TimeSpan { Duration: 0 })
        .map_err(winrt_error)?;
    timeline
        .SetPosition(timespan_from_duration(position))
        .map_err(winrt_error)?;
    timeline
        .SetMaxSeekTime(timespan_from_duration(duration))
        .map_err(winrt_error)?;
    timeline
        .SetEndTime(timespan_from_duration(duration))
        .map_err(winrt_error)?;
    controls
        .UpdateTimelineProperties(&timeline)
        .map_err(winrt_error)
}

fn playback_status(state: PlaybackState) -> MediaPlaybackStatus {
    match state {
        PlaybackState::Playing => MediaPlaybackStatus::Playing,
        PlaybackState::Paused | PlaybackState::Ready => MediaPlaybackStatus::Paused,
        PlaybackState::Stopped | PlaybackState::Ended => MediaPlaybackStatus::Stopped,
        PlaybackState::Initializing | PlaybackState::Loading => MediaPlaybackStatus::Changing,
        PlaybackState::Empty | PlaybackState::Error => MediaPlaybackStatus::Closed,
    }
}

fn timespan_from_duration(duration: Duration) -> TimeSpan {
    let ticks = duration.as_nanos() / 100;
    TimeSpan {
        Duration: i64::try_from(ticks).unwrap_or(i64::MAX),
    }
}

fn duration_from_timespan(timespan: TimeSpan) -> Option<Duration> {
    if timespan.Duration < 0 {
        return None;
    }
    let nanoseconds = u64::try_from(timespan.Duration).ok()?.checked_mul(100)?;
    Some(Duration::from_nanos(nanoseconds))
}

fn winrt_error(error: windows::core::Error) -> SystemMediaError {
    SystemMediaError::WindowsRuntime(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(state: PlaybackState) -> MediaControlUpdate {
        MediaControlUpdate {
            snapshot: PlaybackSnapshot {
                state,
                position: Duration::from_secs(12),
                duration: Some(Duration::from_secs(90)),
                volume: 0.8,
                last_error: None,
            },
            metadata: None,
            capabilities: MediaControlCapabilities::default(),
        }
    }

    #[test]
    fn queue_controls_stay_disabled_without_capabilities() {
        let available = availability_for(&update(PlaybackState::Playing));

        assert!(!available.can_next);
        assert!(!available.can_previous);
        assert_eq!(
            event_for_button(SystemMediaTransportControlsButton::Next, available),
            None
        );
        assert_eq!(
            event_for_button(SystemMediaTransportControlsButton::Previous, available),
            None
        );
    }

    #[test]
    fn queue_buttons_only_emit_when_a_queue_capability_is_advertised() {
        let mut update = update(PlaybackState::Playing);
        update.capabilities = MediaControlCapabilities {
            can_next: true,
            can_previous: true,
        };
        let available = availability_for(&update);

        assert_eq!(
            event_for_button(SystemMediaTransportControlsButton::Next, available),
            Some(SystemMediaEvent::NextRequested)
        );
        assert_eq!(
            event_for_button(SystemMediaTransportControlsButton::Previous, available),
            Some(SystemMediaEvent::PreviousRequested)
        );
    }

    #[test]
    fn playback_buttons_follow_authoritative_snapshot() {
        let playing = availability_for(&update(PlaybackState::Playing));
        assert_eq!(
            event_for_button(SystemMediaTransportControlsButton::Pause, playing),
            Some(SystemMediaEvent::PauseRequested)
        );
        assert_eq!(
            event_for_button(SystemMediaTransportControlsButton::Play, playing),
            None
        );

        let paused = availability_for(&update(PlaybackState::Paused));
        assert_eq!(
            event_for_button(SystemMediaTransportControlsButton::Play, paused),
            Some(SystemMediaEvent::PlayRequested)
        );
        assert_eq!(
            event_for_button(SystemMediaTransportControlsButton::Pause, paused),
            None
        );
    }

    #[test]
    fn seek_is_unavailable_without_known_positive_duration() {
        let mut no_duration = update(PlaybackState::Playing);
        no_duration.snapshot.duration = None;
        assert!(!availability_for(&no_duration).can_seek);

        no_duration.snapshot.duration = Some(Duration::ZERO);
        assert!(!availability_for(&no_duration).can_seek);
    }

    #[test]
    fn seek_time_conversion_uses_winrt_hundred_nanosecond_ticks() {
        let original = Duration::from_millis(12_345);
        assert_eq!(
            duration_from_timespan(timespan_from_duration(original)),
            Some(original)
        );
        assert_eq!(duration_from_timespan(TimeSpan { Duration: -1 }), None);
    }

    #[test]
    fn seek_events_use_absolute_positions_and_clamp_to_track_end() {
        let available = availability_for(&update(PlaybackState::Playing));
        assert_eq!(
            event_for_seek(timespan_from_duration(Duration::from_secs(120)), available),
            Some(SystemMediaEvent::SeekRequested(Duration::from_secs(90)))
        );
        assert_eq!(
            event_for_seek(timespan_from_duration(Duration::from_secs(20)), available),
            Some(SystemMediaEvent::SeekRequested(Duration::from_secs(20)))
        );
    }

    #[test]
    fn errors_are_safe_to_move_between_shell_and_worker_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SystemMediaError>();
        assert_send_sync::<SystemMediaEvent>();
    }
}
