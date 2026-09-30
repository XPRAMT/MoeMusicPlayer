#![cfg(windows)]

use std::path::Path;
use std::sync::{mpsc, Arc};
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
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionMediaProperties, MediaPropertiesChangedEventArgs,
};
use windows::Media::{MediaPlaybackStatus, SystemMediaTransportControls};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
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

fn rgb_png(rgb: [u8; 3]) -> Vec<u8> {
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

fn cover_a() -> Vec<u8> {
    rgb_png([255, 0, 0])
}

fn cover_b() -> Vec<u8> {
    rgb_png([0, 0, 255])
}

fn read_thumbnail(controls: &SystemMediaTransportControls) -> Vec<u8> {
    let updater = controls.DisplayUpdater().expect("get display updater");
    let reference = updater
        .Thumbnail()
        .expect("read thumbnail reference")
        .into();
    read_thumbnail_reference(&reference)
}

fn read_thumbnail_reference(reference: &IRandomAccessStreamReference) -> Vec<u8> {
    let opening = reference.OpenReadAsync().expect("open thumbnail stream");
    let deadline = Instant::now() + Duration::from_secs(5);
    while opening.Status().expect("read thumbnail open status").0 == 0 {
        assert!(Instant::now() < deadline, "thumbnail open timed out");
        thread::sleep(Duration::from_millis(1));
    }
    let stream = opening.GetResults().expect("open thumbnail stream result");
    let size = usize::try_from(stream.Size().expect("read thumbnail size"))
        .expect("thumbnail fits address space");
    let reader = DataReader::CreateDataReader(
        &stream
            .GetInputStreamAt(0)
            .expect("get thumbnail input stream"),
    )
    .expect("create thumbnail reader");
    let loading = reader.LoadAsync(size as u32).expect("load thumbnail bytes");
    let deadline = Instant::now() + Duration::from_secs(5);
    while loading.Status().expect("read thumbnail load status").0 == 0 {
        assert!(Instant::now() < deadline, "thumbnail read timed out");
        thread::sleep(Duration::from_millis(1));
    }
    loading.GetResults().expect("finish thumbnail read");
    let mut bytes = vec![0; size];
    reader.ReadBytes(&mut bytes).expect("copy thumbnail bytes");
    reader.Close().expect("close thumbnail data reader");
    stream
        .Close()
        .expect("simulate media consumer closing thumbnail stream");
    bytes
}

fn read_thumbnail_from_system_session(expected_title: &str) -> Vec<u8> {
    let session = find_system_session(expected_title);
    let properties = session_properties(&session);
    let thumbnail = properties
        .Thumbnail()
        .expect("read system session thumbnail reference");
    read_thumbnail_reference(&thumbnail)
}

fn system_session_manager() -> GlobalSystemMediaTransportControlsSessionManager {
    let deadline = Instant::now() + Duration::from_secs(10);
    let manager_request = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .expect("request system media session manager");
    while manager_request
        .Status()
        .expect("read manager request status")
        .0
        == 0
    {
        assert!(
            Instant::now() < deadline,
            "session manager request timed out"
        );
        thread::sleep(Duration::from_millis(5));
    }
    manager_request
        .GetResults()
        .expect("get system media session manager")
}

fn session_properties(
    session: &GlobalSystemMediaTransportControlsSession,
) -> GlobalSystemMediaTransportControlsSessionMediaProperties {
    let deadline = Instant::now() + Duration::from_secs(10);
    let properties_request = session
        .TryGetMediaPropertiesAsync()
        .expect("request system media properties");
    while properties_request
        .Status()
        .expect("read media properties request status")
        .0
        == 0
    {
        assert!(
            Instant::now() < deadline,
            "media properties request timed out"
        );
        thread::sleep(Duration::from_millis(5));
    }
    properties_request
        .GetResults()
        .expect("get system media properties")
}

fn find_system_session(expected_title: &str) -> GlobalSystemMediaTransportControlsSession {
    let manager = system_session_manager();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let sessions = manager
            .GetSessions()
            .expect("enumerate system media sessions");
        for index in 0..sessions.Size().expect("read session count") {
            let session = sessions.GetAt(index).expect("read system media session");
            let properties = session_properties(&session);
            if properties.Title().expect("read session title") == expected_title {
                return session;
            }
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for system session title {expected_title:?}"
        );
        thread::sleep(Duration::from_millis(25));
    }
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
fn hidden_window_updates_artwork_across_tracks_and_clears_missing_art() {
    let window = HiddenWindow::create();
    let controller =
        SystemMediaController::attach(window.0 .0 as isize).expect("spawn isolated SMTC worker");
    wait_for_event(&controller, |event| {
        matches!(event, SystemMediaEvent::Ready)
    });
    let cover_a_bytes = cover_a();
    let cover_a_arc: Arc<[u8]> = Arc::from(cover_a_bytes.clone());

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
                thumbnail: Some(Arc::clone(&cover_a_arc)),
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
            let has_thumbnail = updater.Thumbnail().is_ok();
            let status = controls.PlaybackStatus().expect("read playback status");
            let pause_enabled = controls.IsPauseEnabled().expect("read pause capability");
            let next_enabled = controls.IsNextEnabled().expect("read next capability");
            let previous_enabled = controls
                .IsPreviousEnabled()
                .expect("read previous capability");
            if title == "SMTC smoke track"
                && artist == "Local test artist"
                && album == "Local test album"
                && has_thumbnail
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
                    has_thumbnail,
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
    let (observed_controls, title, artist, album, has_thumbnail, status, pause, next, previous) =
        inspect
            .join()
            .expect("inspect SMTC from an independent MTA");
    assert_eq!(title, "SMTC smoke track");
    assert_eq!(artist, "Local test artist");
    assert_eq!(album, "Local test album");
    assert!(has_thumbnail);
    assert_eq!(read_thumbnail(&observed_controls), cover_a_bytes);
    assert_eq!(
        read_thumbnail_from_system_session("SMTC smoke track"),
        cover_a_bytes,
        "GSMTC session consumer should receive the published first-track artwork"
    );
    assert_eq!(status, MediaPlaybackStatus::Playing);
    assert!(pause);
    assert!(!next);
    assert!(!previous);

    for position_secs in [20, 21, 22] {
        controller
            .update(MediaControlUpdate {
                snapshot: PlaybackSnapshot {
                    state: PlaybackState::Playing,
                    position: Duration::from_secs(position_secs),
                    duration: Some(Duration::from_secs(90)),
                    volume: 0.7,
                    last_error: None,
                },
                metadata: Some(MediaControlMetadata {
                    title: Some("SMTC smoke track".to_owned()),
                    artist: Some("Local test artist".to_owned()),
                    album: Some("Local test album".to_owned()),
                    thumbnail: Some(Arc::clone(&cover_a_arc)),
                }),
                capabilities: MediaControlCapabilities::default(),
            })
            .expect("queue same-track position update with the same artwork Arc");
        thread::sleep(Duration::from_millis(750));
        assert_eq!(
            read_thumbnail(&observed_controls),
            cover_a_bytes,
            "position-only metadata updates must leave the existing stream readable"
        );
        assert_eq!(
            read_thumbnail_from_system_session("SMTC smoke track"),
            cover_a_bytes,
            "GSMTC session consumer should still read the first cover after position updates"
        );
    }

    let cover_b_bytes = cover_b();
    assert_ne!(cover_a_bytes, cover_b_bytes);
    controller
        .update(MediaControlUpdate {
            snapshot: PlaybackSnapshot {
                state: PlaybackState::Playing,
                position: Duration::from_secs(2),
                duration: Some(Duration::from_secs(75)),
                volume: 0.7,
                last_error: None,
            },
            metadata: Some(MediaControlMetadata {
                title: Some("SMTC second track".to_owned()),
                artist: Some("Second artist".to_owned()),
                album: Some("Second album".to_owned()),
                thumbnail: Some(Arc::from(cover_b_bytes.clone())),
            }),
            capabilities: MediaControlCapabilities::default(),
        })
        .expect("queue second track with different artwork");
    let hwnd = (window.0).0 as isize;
    let expected_cover_b = cover_b_bytes.clone();
    let second_cover = thread::spawn(move || {
        let _apartment = Apartment::mta();
        let controls = controls_for_window(HWND(hwnd as *mut _))
            .expect("get SMTC for hidden top-level window");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let updater = controls.DisplayUpdater().expect("get display updater");
            let music = updater.MusicProperties().expect("get music properties");
            if music.Title().expect("read second title") == "SMTC second track" {
                assert_eq!(
                    music.Artist().expect("read second artist").to_string(),
                    "Second artist"
                );
                assert_eq!(
                    music.AlbumTitle().expect("read second album").to_string(),
                    "Second album"
                );
                assert_eq!(read_thumbnail(&controls), expected_cover_b);
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for second track metadata"
            );
            thread::sleep(Duration::from_millis(10));
        }
    });
    second_cover
        .join()
        .expect("verify second track thumbnail and metadata");
    assert_eq!(
        read_thumbnail_from_system_session("SMTC second track"),
        cover_b_bytes,
        "GSMTC session consumer should receive the second-track artwork"
    );

    controller
        .update(MediaControlUpdate {
            snapshot: PlaybackSnapshot {
                state: PlaybackState::Playing,
                position: Duration::from_secs(13),
                duration: Some(Duration::from_secs(90)),
                volume: 0.7,
                last_error: None,
            },
            metadata: Some(MediaControlMetadata {
                title: Some("SMTC third track without art".to_owned()),
                artist: Some("Local test artist".to_owned()),
                album: Some("Local test album".to_owned()),
                thumbnail: None,
            }),
            capabilities: MediaControlCapabilities::default(),
        })
        .expect("queue metadata update that clears the previous thumbnail");
    let hwnd = (window.0).0 as isize;
    let cleared = thread::spawn(move || {
        let _apartment = Apartment::mta();
        let controls = controls_for_window(HWND(hwnd as *mut _))
            .expect("get SMTC for hidden top-level window");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let updater = controls.DisplayUpdater().expect("get display updater");
            let music = updater.MusicProperties().expect("get music properties");
            let title = music.Title().expect("read title").to_string();
            if title == "SMTC third track without art" {
                assert_eq!(
                    music.Artist().expect("read no-art artist").to_string(),
                    "Local test artist"
                );
                assert_eq!(
                    music.AlbumTitle().expect("read no-art album").to_string(),
                    "Local test album"
                );
                assert!(updater.Thumbnail().is_err(), "previous artwork must clear");
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for thumbnail clear"
            );
            thread::sleep(Duration::from_millis(10));
        }
    });
    cleared.join().expect("verify thumbnail clearing");

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
fn gsmtc_media_properties_events_observe_two_phase_track_cover_publication() {
    let window = HiddenWindow::create();
    let controller =
        SystemMediaController::attach(window.0 .0 as isize).expect("spawn isolated SMTC worker");
    wait_for_event(&controller, |event| {
        matches!(event, SystemMediaEvent::Ready)
    });

    let publish = |title: &str, thumbnail: Option<Arc<[u8]>>| {
        controller
            .update(MediaControlUpdate {
                snapshot: PlaybackSnapshot {
                    state: PlaybackState::Playing,
                    position: Duration::from_secs(1),
                    duration: Some(Duration::from_secs(80)),
                    volume: 0.7,
                    last_error: None,
                },
                metadata: Some(MediaControlMetadata {
                    title: Some(title.to_owned()),
                    artist: Some("Event test artist".to_owned()),
                    album: Some("Event test album".to_owned()),
                    thumbnail,
                }),
                capabilities: MediaControlCapabilities::default(),
            })
            .expect("queue SMTC metadata update");
    };

    let first_bytes = cover_a();
    publish("SMTC event first track", Some(Arc::from(first_bytes)));
    let session = find_system_session("SMTC event first track");
    let (event_tx, event_rx) = mpsc::sync_channel(8);
    let handler = TypedEventHandler::<
        GlobalSystemMediaTransportControlsSession,
        MediaPropertiesChangedEventArgs,
    >::new(move |_, _| {
        let _ = event_tx.try_send(());
        Ok(())
    });
    let token = session
        .MediaPropertiesChanged(&handler)
        .expect("subscribe to GSMTC media properties changes");

    publish("SMTC event second track", None);
    event_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("receive new-track metadata event before artwork is ready");
    let early_properties = session_properties(&session);
    let early_title = early_properties
        .Title()
        .expect("read early-event title")
        .to_string();
    let early_thumbnail = early_properties.Thumbnail();
    println!(
        "GSMTC MediaPropertiesChanged phase=early title={early_title:?} thumbnail_present={}",
        early_thumbnail.is_ok()
    );
    assert_eq!(early_title, "SMTC event second track");
    assert!(
        early_thumbnail.is_err(),
        "the first new-track event currently exposes title without artwork"
    );

    let second_bytes = cover_b();
    publish(
        "SMTC event second track",
        Some(Arc::from(second_bytes.clone())),
    );
    event_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("receive the later artwork metadata event");
    let complete_properties = session_properties(&session);
    let complete_title = complete_properties
        .Title()
        .expect("read complete-event title")
        .to_string();
    let complete_thumbnail = complete_properties
        .Thumbnail()
        .expect("read complete-event thumbnail");
    let complete_bytes = read_thumbnail_reference(&complete_thumbnail);
    println!(
        "GSMTC MediaPropertiesChanged phase=complete title={complete_title:?} thumbnail_bytes={}",
        complete_bytes.len()
    );
    assert_eq!(complete_title, "SMTC event second track");
    assert_eq!(complete_bytes, second_bytes);

    session
        .RemoveMediaPropertiesChanged(token)
        .expect("unsubscribe from GSMTC media properties changes");
    drop(handler);
    drop(controller);
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
