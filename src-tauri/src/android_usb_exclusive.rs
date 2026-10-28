//! The USB exclusive test (ADR 0046, P0, development only): plays the current local 16-bit stereo FLAC for at most
//! [`TEST_SECONDS`] straight to a USB Audio Class DAC over usbfs, bypassing the Android audio system, then hands the
//! DAC back to the kernel driver.
//!
//! Flow: the Tauri command checks the guards (nothing playing, the current track is a local 16-bit stereo FLAC) and
//! asks the plugin (`usbExclusiveStart`), which gets USB permission, opens the device and calls
//! `UsbExclusiveBridge.nativeRun` on its own thread with the connection's file descriptor and raw descriptors. The run
//! (`player_usb_audio::usbfs::run_exclusive`) blocks until the end; its report is kept here for the page and returned to
//! Kotlin, which logs it and only then closes the connection.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use player_core::MediaLocator;
use player_usb_audio::Report;

/// The test plays at most this long from the start of the track.
pub(crate) const TEST_SECONDS: u32 = 30;

struct TestState {
    running: bool,
    last: Option<Report>,
}

static STATE: Mutex<TestState> = Mutex::new(TestState {
    running: false,
    last: None,
});

/// Set by 「停止」; cleared when a test begins.
static STOP: AtomicBool = AtomicBool::new(false);

fn state() -> std::sync::MutexGuard<'static, TestState> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `{ running, report }` for the page (`report`: the last finished test, or null).
pub(crate) fn status() -> serde_json::Value {
    let state = state();
    serde_json::json!({ "running": state.running, "report": state.last })
}

/// Media3 must not be using the DAC: refuse while the player plays or buffers.
pub(crate) fn refuse_while_playing(playback_state: &str) -> Result<(), String> {
    if matches!(playback_state, "playing" | "buffering") {
        Err("目前正在播放，請先暫停再試播".to_owned())
    } else {
        Ok(())
    }
}

/// The track's local file; a remote track gets its own reason.
pub(crate) fn local_file(
    locators: impl IntoIterator<Item = MediaLocator>,
    is_file: impl Fn(&Path) -> bool,
) -> Result<PathBuf, String> {
    let mut remote = false;
    for locator in locators {
        match locator {
            MediaLocator::FileSystem(path) if is_file(&path) => return Ok(path),
            MediaLocator::Remote { .. } => remote = true,
            _ => {}
        }
    }
    Err(if remote {
        "這首是遠端曲目，試播只支援本機 FLAC".to_owned()
    } else {
        "只支援本機 FLAC 檔".to_owned()
    })
}

/// The track must be a local FLAC of 16-bit stereo; answers its sample rate.
pub(crate) fn check_track(path: &Path) -> Result<u32, String> {
    let is_flac = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("flac"));
    if !is_flac || !path.is_file() {
        return Err("只支援本機 FLAC 檔".to_owned());
    }
    let reader = player_flac::FlacReader::open_path(path).map_err(|error| error.to_string())?;
    player_usb_audio::pcm::check_format(reader.info()).map_err(|error| error.to_string())?;
    Ok(reader.info().sample_rate)
}

/// Mark a test as starting; refused while one runs.
pub(crate) fn begin() -> Result<(), String> {
    let mut state = state();
    if state.running {
        return Err("USB 獨佔試播正在進行".to_owned());
    }
    state.running = true;
    STOP.store(false, Ordering::Relaxed);
    Ok(())
}

/// The plugin could not start the run (no device, no permission, cannot open it).
pub(crate) fn fail_start(reason: String) {
    let mut state = state();
    state.running = false;
    state.last = Some(Report::failed(reason));
}

pub(crate) fn request_stop() {
    STOP.store(true, Ordering::Relaxed);
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn finish(report: Report) -> String {
    let json = serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_owned());
    let mut state = state();
    state.running = false;
    state.last = Some(report);
    json
}

/// Decode, play exclusively, hand back; the report as JSON. Called on the plugin's thread.
#[cfg(target_os = "android")]
fn run(fd: i32, raw: &[u8], path: &Path) -> String {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match player_usb_audio::pcm::decode_flac_s16(path, TEST_SECONDS) {
            Ok(pcm) => player_usb_audio::usbfs::run_exclusive(
                fd,
                raw,
                &pcm,
                &STOP,
                std::time::Duration::from_secs(u64::from(TEST_SECONDS)),
            ),
            Err(error) => Report::failed(error.to_string()),
        }
    }));
    finish(outcome.unwrap_or_else(|_| Report::failed("USB 獨佔試播內部錯誤")))
}

#[cfg(target_os = "android")]
mod jni_exports {
    use jni::objects::{JByteArray, JClass, JString};
    use jni::sys::{jint, jstring};
    use jni::JNIEnv;

    /// `UsbExclusiveBridge.nativeRun(fd, rawDescriptors, path)`: blocks until the test ended and the DAC was handed
    /// back; answers the report as JSON.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveBridge_nativeRun(
        mut env: JNIEnv,
        _class: JClass,
        fd: jint,
        raw: JByteArray,
        path: JString,
    ) -> jstring {
        let raw = env.convert_byte_array(&raw).unwrap_or_default();
        let path: String = env.get_string(&path).map(Into::into).unwrap_or_default();
        let json = super::run(fd, &raw, std::path::Path::new(&path));
        env.new_string(json)
            .map(|value| value.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/player-flac/tests/fixtures")
            .join(name)
    }

    #[test]
    fn only_a_paused_or_stopped_player_lets_the_test_take_the_dac() {
        assert!(refuse_while_playing("playing").is_err());
        assert!(refuse_while_playing("buffering").is_err());
        for state in ["paused", "stopped", "ended", "idle"] {
            assert!(refuse_while_playing(state).is_ok(), "{state}");
        }
    }

    #[test]
    fn only_a_local_16_bit_stereo_flac_is_taken() {
        assert_eq!(check_track(&fixture("s16_stereo_44100.flac")), Ok(44_100));
        assert!(check_track(&fixture("s24_stereo_96000.flac"))
            .unwrap_err()
            .contains("16 bit"));
        assert!(check_track(&fixture("s16_6ch_48000.flac")).is_err());
        assert_eq!(
            check_track(Path::new("missing.flac")),
            Err("只支援本機 FLAC 檔".to_owned())
        );
        assert_eq!(
            check_track(Path::new("song.mp3")),
            Err("只支援本機 FLAC 檔".to_owned())
        );
    }

    #[test]
    fn a_remote_track_is_refused_with_its_own_reason() {
        let remote = || MediaLocator::Remote {
            source_id: player_core::SourceId::new(),
            path: "Music/a.flac".to_owned(),
        };
        let local = || MediaLocator::FileSystem(PathBuf::from("/sdcard/Music/a.flac"));
        assert_eq!(
            local_file([remote()], |_| true),
            Err("這首是遠端曲目，試播只支援本機 FLAC".to_owned())
        );
        assert_eq!(
            local_file([remote(), local()], |_| true),
            Ok(PathBuf::from("/sdcard/Music/a.flac"))
        );
        assert_eq!(
            local_file([local()], |_| false),
            Err("只支援本機 FLAC 檔".to_owned())
        );
        assert_eq!(
            local_file([MediaLocator::ContentUri("content://x".to_owned())], |_| {
                true
            }),
            Err("只支援本機 FLAC 檔".to_owned())
        );
        assert_eq!(
            local_file([], |_| true),
            Err("只支援本機 FLAC 檔".to_owned())
        );
    }

    #[test]
    fn one_test_at_a_time_and_a_failed_start_is_reported() {
        assert!(begin().is_ok());
        assert!(begin().is_err());
        assert_eq!(status()["running"], true);
        fail_start("沒有找到 USB 音訊裝置".to_owned());
        let status = status();
        assert_eq!(status["running"], false);
        assert_eq!(status["report"]["outcome"], "failed");
        assert_eq!(status["report"]["error"], "沒有找到 USB 音訊裝置");
        assert!(begin().is_ok());
        request_stop();
        let json = finish(Report::default());
        assert!(json.contains("\"outcome\":\"failed\""));
        assert_eq!(super::status()["running"], false);
    }
}
