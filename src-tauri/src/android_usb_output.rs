//! The Android USB exclusive output (ADR 0047): the JNI side of `player-usb-audio` for the playback service.
//!
//! Kotlin's `UsbAudioOutputProvider` hands Media3 a `UsbAudioOutput` when the setting is on and the chosen USB audio
//! device can take the track. Its PCM goes into a [`Ring`] (created here, written from Media3's playback thread with a
//! direct `ByteBuffer`), and one device thread (`UsbExclusiveEngine`'s Kotlin thread at URGENT_AUDIO priority, running
//! [`player_usb_audio::usbfs::run`] through `nativeLoop`) owns the device for one claim: requests (attach, play, pause,
//! detach, release) are sent to it and answered once carried out. When the claim ends (idle, released, an error), the
//! loop returns and Kotlin closes the `UsbDeviceConnection`; the next output claims again.

#[cfg(target_os = "android")]
use std::sync::Arc;

use player_usb_audio::format::Source;
#[cfg(target_os = "android")]
use player_usb_audio::ring::Ring;

/// The track as the library knows it: the bit depth Rust sent with the load (`metadata.bitsPerSample`, 0 = unknown)
/// whether it is DSD, and whether the load chose native passthrough (the data is then the DSD slots, ADR 0049); without
/// passthrough a DSD track is played through the DSD-to-PCM converter.
pub(crate) fn source_of(bits: i32, dsd: bool, native_dsd: bool) -> Source {
    Source {
        bits: u8::try_from(bits)
            .ok()
            .filter(|bits| (8..=32).contains(bits)),
        dsd: dsd || native_dsd,
        native_dsd,
    }
}

/// How long the playback thread waits for the device thread to carry out a request (a claim with its clock settle is
/// well under this; a device that stopped answering must not hang Media3).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// The answer of an attach as the Kotlin output reads it.
pub(crate) fn attach_json(answer: Result<player_usb_audio::AttachReply, String>) -> String {
    match answer {
        Ok(reply) => serde_json::json!({
            "ok": true,
            "rate": reply.rate,
            "channels": reply.channels,
            "alt": reply.alt,
            "subslotBytes": reply.container.bytes,
            "bits": reply.container.bits,
        }),
        Err(reason) => serde_json::json!({ "ok": false, "reason": reason }),
    }
    .to_string()
}

#[cfg(target_os = "android")]
mod engine {
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Mutex, PoisonError};

    use player_usb_audio::format::Source;
    use player_usb_audio::ring::Ring;
    use player_usb_audio::usbfs::{self, Request, Shared};
    use player_usb_audio::EngineStatus;

    use super::REQUEST_TIMEOUT;

    struct Live {
        requests: Sender<Request>,
        shared: Arc<Shared>,
    }

    struct Pending {
        fd: i32,
        raw: Vec<u8>,
        requests: Receiver<Request>,
        shared: Arc<Shared>,
    }

    static LIVE: Mutex<Option<Live>> = Mutex::new(None);
    static PENDING: Mutex<Option<Pending>> = Mutex::new(None);
    /// The status of the last claim once it ended.
    static LAST: Mutex<Option<EngineStatus>> = Mutex::new(None);
    /// The app volume, kept across claims.
    static GAIN: Mutex<f32> = Mutex::new(1.0);

    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Install a claim for `fd`; its loop runs on the caller's next `run_loop`. False while one is alive.
    pub(super) fn start(fd: i32, raw: Vec<u8>) -> bool {
        let mut live = lock(&LIVE);
        if live.is_some() {
            return false;
        }
        let (sender, receiver) = mpsc::channel();
        let shared = Arc::new(Shared::new());
        shared.set_gain(*lock(&GAIN));
        *live = Some(Live {
            requests: sender,
            shared: Arc::clone(&shared),
        });
        *lock(&PENDING) = Some(Pending {
            fd,
            raw,
            requests: receiver,
            shared,
        });
        true
    }

    /// Run the installed claim until the device is handed back; its last status as JSON.
    pub(super) fn run_loop() -> String {
        let Some(pending) = lock(&PENDING).take() else {
            return "null".to_owned();
        };
        let shared = Arc::clone(&pending.shared);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            usbfs::run(pending.fd, &pending.raw, &pending.requests, &pending.shared)
        }));
        let mut status = shared.status();
        if result.is_err() {
            status
                .error
                .get_or_insert_with(|| "USB 獨佔輸出內部錯誤".to_owned());
        }
        {
            let mut live = lock(&LIVE);
            if live
                .as_ref()
                .is_some_and(|live| Arc::ptr_eq(&live.shared, &shared))
            {
                *live = None;
            }
        }
        // Whatever is still queued finds the receiver gone when `pending` drops.
        drop(pending);
        let json = serde_json::to_string(&status).unwrap_or_else(|_| "null".to_owned());
        *lock(&LAST) = Some(status);
        json
    }

    pub(super) fn alive() -> bool {
        lock(&LIVE).is_some()
    }

    fn send<T>(
        make: impl FnOnce(mpsc::SyncSender<Result<T, String>>) -> Request,
    ) -> Result<T, String> {
        let (reply, answer) = mpsc::sync_channel(1);
        {
            let live = lock(&LIVE);
            let live = live.as_ref().ok_or("USB 獨佔輸出未在執行")?;
            live.requests
                .send(make(reply))
                .map_err(|_| "USB 獨佔輸出已交還".to_owned())?;
        }
        answer
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|_| "USB 音訊裝置沒有回應".to_owned())?
    }

    pub(super) fn attach(
        ring: Arc<Ring>,
        rate: u32,
        source: Source,
    ) -> Result<player_usb_audio::AttachReply, String> {
        let failed = Arc::clone(&ring);
        let answer = send(|reply| Request::Attach {
            ring,
            rate,
            source,
            reply,
        });
        if answer.is_err() {
            failed.fail();
        }
        answer
    }

    pub(super) fn play(ring: usize) -> Result<(), String> {
        send(|reply| Request::Play { ring, reply })
    }

    pub(super) fn pause(ring: usize) -> Result<(), String> {
        send(|reply| Request::Pause { ring, reply })
    }

    pub(super) fn detach(ring: usize) -> Result<(), String> {
        send(|reply| Request::Detach { ring, reply })
    }

    pub(super) fn release(reason: String) -> Result<(), String> {
        if !alive() {
            return Ok(());
        }
        send(|reply| Request::Release { reason, reply })
    }

    pub(super) fn set_gain(gain: f32) {
        *lock(&GAIN) = gain;
        if let Some(live) = lock(&LIVE).as_ref() {
            live.shared.set_gain(gain);
        }
    }

    pub(super) fn status() -> Option<EngineStatus> {
        if let Some(live) = lock(&LIVE).as_ref() {
            return Some(live.shared.status());
        }
        lock(&LAST).clone()
    }
}

/// A ring handle as Kotlin keeps it: a leaked `Box<Arc<Ring>>`.
#[cfg(target_os = "android")]
fn ring_of(handle: i64) -> Option<Arc<Ring>> {
    if handle == 0 {
        return None;
    }
    // SAFETY: `handle` came from `nativeCreateRing` and is destroyed only by `nativeDestroyRing`, after the last use.
    Some(Arc::clone(unsafe { &*(handle as *const Arc<Ring>) }))
}

#[cfg(target_os = "android")]
mod jni_exports {
    use std::sync::Arc;

    use jni::objects::{JByteArray, JByteBuffer, JClass, JString};
    use jni::sys::{jboolean, jfloat, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
    use jni::JNIEnv;
    use player_usb_audio::pcm::InputEncoding;
    use player_usb_audio::ring::Ring;
    use player_usb_audio::usbfs::ring_id;

    use super::{engine, ring_of, source_of};

    fn string(env: &JNIEnv, text: &str) -> jstring {
        env.new_string(text)
            .map(|value| value.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }

    /// Null when it worked, else the reason.
    fn outcome(env: &JNIEnv, result: Result<(), String>) -> jstring {
        match result {
            Ok(()) => std::ptr::null_mut(),
            Err(reason) => string(env, &reason),
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeStart(
        env: JNIEnv,
        _class: JClass,
        fd: jint,
        raw: JByteArray,
    ) -> jboolean {
        let raw = env.convert_byte_array(&raw).unwrap_or_default();
        if engine::start(fd, raw) {
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    }

    /// Blocks on the device thread until the claim ended; the last status as JSON.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeLoop(
        env: JNIEnv,
        _class: JClass,
    ) -> jstring {
        let json = engine::run_loop();
        string(&env, &json)
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeAlive(
        _env: JNIEnv,
        _class: JClass,
    ) -> jboolean {
        if engine::alive() {
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    }

    /// A ring for `capacityFrames` of `encoding` (Android `ENCODING_PCM_*`); 0 for an encoding that is not PCM.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeCreateRing(
        _env: JNIEnv,
        _class: JClass,
        capacity_frames: jint,
        encoding: jint,
        channels: jint,
        dsd_rate: jint,
    ) -> jlong {
        // Native DSD (`dsd_rate` is the frame rate, DSD bit rate / 32): the carrier ring of ADR 0049.
        if dsd_rate > 0 {
            let ring = Arc::new(Ring::new_dsd(
                capacity_frames.max(1) as usize,
                dsd_rate as u32,
            ));
            return Box::into_raw(Box::new(ring)) as jlong;
        }
        let (Some(encoding), Ok(channels)) = (
            InputEncoding::from_android(encoding),
            u8::try_from(channels),
        ) else {
            return 0;
        };
        let ring = Arc::new(Ring::new(
            capacity_frames.max(1) as usize,
            encoding,
            channels,
        ));
        Box::into_raw(Box::new(ring)) as jlong
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeDestroyRing(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) {
        if handle != 0 {
            // SAFETY: the handle came from `nativeCreateRing`; Kotlin destroys it once, after its last use.
            drop(unsafe { Box::from_raw(handle as *mut Arc<Ring>) });
        }
    }

    /// Copy what fits of `buffer[offset, offset + length)` (whole frames); the bytes taken. Never blocks.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeWrite(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
        buffer: JByteBuffer,
        offset: jint,
        length: jint,
    ) -> jint {
        let Some(ring) = ring_of(handle) else {
            return 0;
        };
        let (Ok(address), Ok(capacity)) = (
            env.get_direct_buffer_address(&buffer),
            env.get_direct_buffer_capacity(&buffer),
        ) else {
            return 0;
        };
        let (offset, length) = (offset.max(0) as usize, length.max(0) as usize);
        if offset + length > capacity {
            return 0;
        }
        // SAFETY: a direct buffer of `capacity` bytes at `address`, alive for this call; the range is inside it.
        let data = unsafe { std::slice::from_raw_parts(address.add(offset), length) };
        ring.push(data) as jint
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativePlayedFrames(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jlong {
        ring_of(handle).map_or(0, |ring| ring.played_frames() as jlong)
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeRingEnd(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) {
        if let Some(ring) = ring_of(handle) {
            ring.end();
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeRingFailed(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jboolean {
        if ring_of(handle).is_none_or(|ring| ring.is_failed()) {
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeRingUnderruns(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jlong {
        ring_of(handle).map_or(0, |ring| ring.underruns() as jlong)
    }

    /// Attach the ring to the device for `rate` (claim, alt, clock as needed); JSON `{ ok, … }` or `{ ok: false, reason }`.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeAttach(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
        rate: jint,
        source_bits: jint,
        dsd: jboolean,
        native_dsd: jboolean,
    ) -> jstring {
        let answer = match ring_of(handle) {
            Some(ring) => engine::attach(
                ring,
                rate.max(0) as u32,
                source_of(source_bits, dsd != JNI_FALSE, native_dsd != JNI_FALSE),
            ),
            None => Err("沒有輸出緩衝".to_owned()),
        };
        string(&env, &super::attach_json(answer))
    }

    fn with_ring(
        handle: jlong,
        request: impl FnOnce(usize) -> Result<(), String>,
    ) -> Result<(), String> {
        match ring_of(handle) {
            Some(ring) => request(ring_id(&ring)),
            None => Err("沒有輸出緩衝".to_owned()),
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativePlay(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jstring {
        outcome(&env, with_ring(handle, engine::play))
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativePause(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jstring {
        outcome(&env, with_ring(handle, engine::pause))
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeDetach(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jstring {
        outcome(&env, with_ring(handle, engine::detach))
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeRelease(
        mut env: JNIEnv,
        _class: JClass,
        reason: JString,
    ) -> jstring {
        let reason: String = env.get_string(&reason).map(Into::into).unwrap_or_default();
        outcome(&env, engine::release(reason))
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeSetGain(
        _env: JNIEnv,
        _class: JClass,
        gain: jfloat,
    ) {
        engine::set_gain(gain);
    }

    /// The exclusive output's status as JSON, or a Java null when nothing was claimed in this process.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeStatus(
        env: JNIEnv,
        _class: JClass,
    ) -> jstring {
        match engine::status().and_then(|status| serde_json::to_string(&status).ok()) {
            Some(json) => string(&env, &json),
            None => std::ptr::null_mut(),
        }
    }

    /// Whether the device of `raw` descriptors can take `rate` / `channels` of the track (from the descriptors and the
    /// clock ranges read earlier): a Java null when it can, else the reason.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeSupports(
        env: JNIEnv,
        _class: JClass,
        raw: JByteArray,
        rate: jint,
        channels: jint,
        source_bits: jint,
        dsd: jboolean,
    ) -> jstring {
        let raw = env.convert_byte_array(&raw).unwrap_or_default();
        outcome(
            &env,
            super::supports(
                &raw,
                rate,
                channels,
                source_of(source_bits, dsd != JNI_FALSE, false),
            ),
        )
    }

    /// Whether the device of `raw` descriptors can take native DSD at `rate` frames per second (DSD bit rate / 32): a
    /// Java null when it can, else the reason (ADR 0049).
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_UsbExclusiveNative_nativeSupportsDsd(
        env: JNIEnv,
        _class: JClass,
        raw: JByteArray,
        rate: jint,
    ) -> jstring {
        let raw = env.convert_byte_array(&raw).unwrap_or_default();
        outcome(&env, super::supports_dsd(&raw, rate))
    }
}

/// Whether the device of `raw` descriptors can take the track: the alt for its channels and bit depth, and the rate
/// against the clock ranges read in this process (a rate is otherwise checked when the device is configured).
pub(crate) fn supports(raw: &[u8], rate: i32, channels: i32, source: Source) -> Result<(), String> {
    use player_usb_audio::{descriptors::parse_configuration, format};
    let (Ok(rate), Ok(channels)) = (u32::try_from(rate), u8::try_from(channels)) else {
        return Err("格式不明".to_owned());
    };
    let config = parse_configuration(raw);
    let ranges = format::known_ranges(config.vendor_id, config.product_id);
    format::choose(&config, rate, channels, source, ranges.as_deref())
        .map(|_| ())
        .map_err(|reason| reason.to_string())
}

/// Whether the device of `raw` descriptors has the RAW_DATA alt for native DSD at `rate` frames per second, and, when
/// its clock ranges are known from an earlier claim, lists the rate. Otherwise the rate is checked when the device is
/// configured (and a refusal there makes the track fall back).
pub(crate) fn supports_dsd(raw: &[u8], rate: i32) -> Result<(), String> {
    use player_usb_audio::{descriptors::parse_configuration, format};
    let Ok(rate) = u32::try_from(rate) else {
        return Err("格式不明".to_owned());
    };
    let config = parse_configuration(raw);
    let ranges = format::known_ranges(config.vendor_id, config.product_id);
    format::choose_dsd(&config, rate, ranges.as_deref())
        .map(|_| ())
        .map_err(|reason| reason.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_bit_depth_is_known_only_for_real_depths() {
        assert_eq!(
            source_of(16, false, false),
            Source {
                bits: Some(16),
                dsd: false,
                native_dsd: false
            }
        );
        assert_eq!(source_of(24, false, false).bits, Some(24));
        assert_eq!(source_of(0, false, false).bits, None);
        assert_eq!(source_of(-1, false, false).bits, None);
        assert_eq!(source_of(64, false, false).bits, None);
        assert!(source_of(1, true, false).dsd);
        assert!(!source_of(1, true, false).native_dsd);
    }

    #[test]
    fn native_dsd_is_a_dsd_source() {
        let native = source_of(0, false, true);
        assert!(native.dsd && native.native_dsd);
    }

    #[test]
    fn native_dsd_support_follows_the_raw_alt_of_the_descriptors() {
        let raw = sony();
        for rate in [88_200, 176_400, 352_800] {
            assert_eq!(supports_dsd(&raw, rate), Ok(()), "{rate}");
        }
        // DSD512 needs 705.6 kHz; PCM rates are not frame rates of DSD.
        let refused = supports_dsd(&raw, 705_600).unwrap_err();
        assert!(refused.contains("705.6"), "{refused}");
        assert!(supports_dsd(&raw, 44_100).is_err());
        // Not a USB audio configuration at all, and a bad rate.
        assert!(supports_dsd(&[], 88_200).is_err());
        assert!(supports_dsd(&raw, -1).is_err());
    }

    #[test]
    fn an_attach_answer_is_json_for_the_output() {
        let ok: serde_json::Value =
            serde_json::from_str(&attach_json(Ok(player_usb_audio::AttachReply {
                rate: 44_100,
                channels: 2,
                alt: 6,
                container: player_usb_audio::pcm::Container { bytes: 2, bits: 16 },
            })))
            .unwrap();
        assert_eq!(
            ok,
            serde_json::json!({ "ok": true, "rate": 44100, "channels": 2, "alt": 6, "subslotBytes": 2, "bits": 16 })
        );
        let refused: serde_json::Value =
            serde_json::from_str(&attach_json(Err("USB 音訊裝置不支援 705600 Hz".into()))).unwrap();
        assert_eq!(refused["ok"], false);
        assert_eq!(refused["reason"], "USB 音訊裝置不支援 705600 Hz");
    }

    fn sony() -> Vec<u8> {
        let text = include_str!("../../crates/player-usb-audio/tests/fixtures/sony-054c-0b8c.txt");
        text.lines()
            .filter(|line| !line.starts_with('#'))
            .flat_map(|line| line.split_whitespace().skip(1))
            .map(|byte| u8::from_str_radix(byte, 16).unwrap())
            .collect()
    }

    #[test]
    fn support_follows_the_descriptors() {
        let raw = sony();
        assert_eq!(
            supports(&raw, 44_100, 2, source_of(16, false, false)),
            Ok(())
        );
        assert_eq!(
            supports(&raw, 96_000, 2, source_of(24, false, false)),
            Ok(())
        );
        assert!(supports(&raw, 48_000, 6, source_of(16, false, false))
            .unwrap_err()
            .contains('6'));
        // Not a USB audio configuration at all.
        assert!(supports(&[], 44_100, 2, source_of(16, false, false)).is_err());
        assert!(supports(&[], -1, 2, source_of(16, false, false)).is_err());
    }
}
