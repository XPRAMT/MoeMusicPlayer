#![cfg(windows)]

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use player_audio_windows::system_media::{
    MediaControlCapabilities, MediaControlMetadata, MediaControlUpdate, SystemMediaController,
    SystemMediaError, SystemMediaEvent,
};
use player_audio_windows::{
    AudioBackend, AudioError, PlaybackSnapshot, PlaybackState, PlayerHandle,
};
use windows::core::{factory, w};
use windows::Media::{MediaPlaybackStatus, SystemMediaTransportControls};
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::System::WinRT::{
    ISystemMediaTransportControlsInterop, RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
};

struct HiddenWindow(HWND);

impl HiddenWindow {
    fn create() -> Self {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("MoeMusicPlayer SMTC smoke test"),
                WINDOW_STYLE(0),
                0,
                0,
                1,
                1,
                None,
                None,
                Some(HINSTANCE::default()),
                None,
            )
        }
        .expect("create a hidden top-level window for the WinRT interop test");
        Self(hwnd)
    }
}

impl Drop for HiddenWindow {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.0) }.expect("destroy hidden test window");
    }
}

struct Apartment;

impl Apartment {
    fn mta() -> Self {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.expect("initialize test MTA");
        Self
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

fn controls_for_window(hwnd: HWND) -> Result<SystemMediaTransportControls, windows::core::Error> {
    let interop = factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
    unsafe { interop.GetForWindow::<SystemMediaTransportControls>(hwnd) }
}

fn wait_for_event(
    controller: &SystemMediaController,
    expected: fn(&SystemMediaEvent) -> bool,
) -> SystemMediaEvent {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event) = controller.try_recv_event().expect("read SMTC event") {
            if matches!(event, SystemMediaEvent::Error(_)) {
                panic!("SMTC worker reported an error before expected event: {event:?}");
            }
            if expected(&event) {
                return event;
            }
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for SMTC event"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn hidden_window_receives_metadata_and_disables_unimplemented_queue_controls() {
    let window = HiddenWindow::create();
    let controller =
        SystemMediaController::attach(window.0 .0 as isize).expect("spawn isolated SMTC worker");
    wait_for_event(&controller, |event| {
        matches!(event, SystemMediaEvent::Ready)
    });

    controller
        .update(MediaControlUpdate {
            snapshot: PlaybackSnapshot {
                state: PlaybackState::Playing,
                position: Duration::from_secs(12),
                duration: Some(Duration::from_secs(90)),
                volume: 0.7,
                last_error: None,
            },
            metadata: Some(MediaControlMetadata {
                title: Some("SMTC smoke track".to_owned()),
                artist: Some("Local test artist".to_owned()),
                album: Some("Local test album".to_owned()),
            }),
            capabilities: MediaControlCapabilities::default(),
        })
        .expect("queue metadata and playback update");

    let hwnd = (window.0).0 as isize;
    let inspect = thread::spawn(move || {
        let _apartment = Apartment::mta();
        let hwnd = HWND(hwnd as *mut _);
        let controls = controls_for_window(hwnd).expect("get SMTC for hidden top-level window");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let updater = controls.DisplayUpdater().expect("get display updater");
            let music = updater.MusicProperties().expect("get music properties");
            let title = music.Title().expect("read displayed title").to_string();
            let artist = music.Artist().expect("read displayed artist").to_string();
            let album = music
                .AlbumTitle()
                .expect("read displayed album")
                .to_string();
            let status = controls.PlaybackStatus().expect("read playback status");
            let pause_enabled = controls.IsPauseEnabled().expect("read pause capability");
            let next_enabled = controls.IsNextEnabled().expect("read next capability");
            let previous_enabled = controls
                .IsPreviousEnabled()
                .expect("read previous capability");
            if title == "SMTC smoke track"
                && artist == "Local test artist"
                && album == "Local test album"
                && status == MediaPlaybackStatus::Playing
                && pause_enabled
                && !next_enabled
                && !previous_enabled
            {
                return (
                    controls,
                    title,
                    artist,
                    album,
                    status,
                    pause_enabled,
                    next_enabled,
                    previous_enabled,
                );
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for SMTC update"
            );
            thread::sleep(Duration::from_millis(10));
        }
    });
    let (observed_controls, title, artist, album, status, pause, next, previous) = inspect
        .join()
        .expect("inspect SMTC from an independent MTA");
    assert_eq!(title, "SMTC smoke track");
    assert_eq!(artist, "Local test artist");
    assert_eq!(album, "Local test album");
    assert_eq!(status, MediaPlaybackStatus::Playing);
    assert!(pause);
    assert!(!next);
    assert!(!previous);

    // Dropping the controller joins its worker after event-token revocation.
    drop(controller);
    assert_eq!(
        observed_controls
            .PlaybackStatus()
            .expect("read status after teardown"),
        MediaPlaybackStatus::Closed
    );
    assert!(!observed_controls
        .IsEnabled()
        .expect("read enabled state after teardown"));
    drop(observed_controls);
    drop(window);
}

#[test]
fn registration_failure_does_not_stop_the_independent_audio_worker() {
    let controller = SystemMediaController::attach(1).expect("spawn SMTC worker");
    let player =
        PlayerHandle::with_backend_factory(|| Ok(Box::new(QuietBackend) as Box<dyn AudioBackend>))
            .expect("start independent audio worker");
    player.load("unused-test-file.wav").expect("load fixture");
    player.play().expect("queue audio play");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_registration_error = false;
    let mut saw_playing = false;
    while Instant::now() < deadline {
        if let Some(SystemMediaEvent::Error(SystemMediaError::WindowsRuntime(_))) =
            controller.try_recv_event().expect("read SMTC event")
        {
            saw_registration_error = true;
        }
        if player.snapshot().state == PlaybackState::Playing {
            saw_playing = true;
        }
        if saw_registration_error && saw_playing {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(
        saw_registration_error,
        "invalid HWND should fail SMTC setup"
    );
    assert!(saw_playing, "audio worker should play while SMTC fails");
    drop(controller);
    drop(player);
}

struct QuietBackend;

impl AudioBackend for QuietBackend {
    fn load(&mut self, _path: &Path) -> Result<Option<Duration>, AudioError> {
        Ok(Some(Duration::from_secs(60)))
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
