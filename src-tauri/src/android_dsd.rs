//! DSD playback on Android (ADR 0016).
//!
//! Media3 has no DSD decoder, so a `.dsf` track is loaded with a `moedsd:///path/to/file.dsf` URI. The Kotlin
//! `DsdDataSource` opens that URI through the JNI exports below and reads it as a WAV file of 32-bit float PCM at
//! 176.4 kHz, which `player_dsd` converts on demand (the same converter Windows plays through). ExoPlayer seeks by
//! byte position, and the reader seeks exactly. The gain is shared by every open stream, so changing it in the
//! settings eases in while a track plays. 352.8 kHz is not offered on Android: `AudioTrack` tops out at 192 kHz.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use player_dsd::dsf::DsfReader;
use player_dsd::reader::DsdPcmReader;
use player_dsd::wav::DsdWavStream;

/// URI scheme of a DSD track handed to the Kotlin data source.
pub(crate) const DSD_URI_SCHEME: &str = "moedsd";
/// The only PCM rate DSD is converted to on Android.
pub(crate) const ANDROID_DSD_OUTPUT_RATE: u32 = 176_400;

type DsdStreamHandle = DsdWavStream<DsfReader<BufReader<File>>>;

/// `file:///x/y.dsf` becomes `moedsd:///x/y.dsf`; every other URI is returned unchanged.
pub(crate) fn to_dsd_uri(uri: &str) -> String {
    match uri.strip_prefix("file://") {
        Some(rest) if rest.to_ascii_lowercase().ends_with(".dsf") => {
            format!("{DSD_URI_SCHEME}://{rest}")
        }
        _ => uri.to_owned(),
    }
}

/// The URI the player loads for a track: its file URI, as a `moedsd` URI when it is a DSF file.
pub(crate) fn playable_uri(
    database: &player_db::Database,
    track_id: player_core::TrackId,
) -> Result<String, player_db::DatabaseError> {
    database
        .resolve_playable_content_uri(track_id)
        .map(|uri| to_dsd_uri(&uri))
}

fn gain() -> &'static Arc<AtomicU32> {
    static GAIN: OnceLock<Arc<AtomicU32>> = OnceLock::new();
    GAIN.get_or_init(|| Arc::new(AtomicU32::new(3.0_f32.to_bits())))
}

/// Set the DSD gain (dB) every open stream follows.
pub(crate) fn set_gain_db(gain_db: f32) {
    gain().store(gain_db.to_bits(), Ordering::Relaxed);
}

#[derive(Default)]
struct Registry {
    next: i64,
    streams: HashMap<i64, Arc<Mutex<DsdStreamHandle>>>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

fn stream(handle: i64) -> Option<Arc<Mutex<DsdStreamHandle>>> {
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .streams
        .get(&handle)
        .cloned()
}

/// Open a DSF file; the handle (never 0) is used with the other functions until `close`.
pub(crate) fn open(path: &Path) -> Result<i64, String> {
    let file = File::open(path).map_err(|error| format!("無法開啟 DSD 檔案：{error}"))?;
    let dsf = DsfReader::open(BufReader::with_capacity(256 * 1024, file))
        .map_err(|error| error.to_string())?;
    let reader = DsdPcmReader::new(dsf, ANDROID_DSD_OUTPUT_RATE, Arc::clone(gain()))
        .map_err(|error| error.to_string())?;
    let mut registry = registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    registry.next += 1;
    let handle = registry.next;
    registry
        .streams
        .insert(handle, Arc::new(Mutex::new(DsdWavStream::new(reader))));
    Ok(handle)
}

/// Length in bytes of the virtual WAV file.
pub(crate) fn length(handle: i64) -> Option<u64> {
    let stream = stream(handle)?;
    let stream = stream
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Some(stream.total_len())
}

/// Copy bytes of the virtual WAV file starting at `position` into `out`; 0 is the end.
pub(crate) fn read(handle: i64, position: u64, out: &mut [u8]) -> Result<usize, String> {
    let stream = stream(handle).ok_or_else(|| "DSD 串流已關閉".to_owned())?;
    let mut stream = stream
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    stream
        .read_at(position, out)
        .map_err(|error| error.to_string())
}

pub(crate) fn close(handle: i64) {
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .streams
        .remove(&handle);
}

#[cfg(target_os = "android")]
mod jni_exports {
    use jni::objects::{JByteArray, JClass, JString};
    use jni::sys::{jint, jlong};
    use jni::JNIEnv;

    use super::*;

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_DsdBridge_nativeOpen(
        mut env: JNIEnv,
        _class: JClass,
        path: JString,
    ) -> jlong {
        let Ok(path) = env.get_string(&path) else {
            return 0;
        };
        match open(Path::new(&path.to_string_lossy().into_owned())) {
            Ok(handle) => handle,
            Err(error) => {
                eprintln!("[dsd] open failed: {error}");
                0
            }
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_DsdBridge_nativeLength(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) -> jlong {
        length(handle).map_or(-1, |length| length as jlong)
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_DsdBridge_nativeRead(
        env: JNIEnv,
        _class: JClass,
        handle: jlong,
        position: jlong,
        buffer: JByteArray,
        offset: jint,
        length: jint,
    ) -> jint {
        if position < 0 || offset < 0 || length < 0 {
            return -1;
        }
        let mut bytes = vec![0_u8; length as usize];
        let count = match read(handle, position as u64, &mut bytes) {
            Ok(count) => count,
            Err(error) => {
                eprintln!("[dsd] read failed: {error}");
                return -1;
            }
        };
        // SAFETY: `i8` and `u8` have the same layout.
        let signed = unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<i8>(), count) };
        if env.set_byte_array_region(&buffer, offset, signed).is_err() {
            return -1;
        }
        count as jint
    }

    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_DsdBridge_nativeClose(
        _env: JNIEnv,
        _class: JClass,
        handle: jlong,
    ) {
        close(handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use player_dsd::testing::{build_dsf, modulate, tone};

    fn write_dsf(name: &str) -> std::path::PathBuf {
        let rate = 2_822_400_u32;
        let samples = rate as usize / 4 / 16 * 16;
        let lsb = |bytes: Vec<u8>| bytes.into_iter().map(u8::reverse_bits).collect::<Vec<u8>>();
        let left = lsb(modulate(samples, tone(rate, 1_000.0, 0.25)));
        let right = lsb(modulate(samples, tone(rate, 2_000.0, 0.25)));
        let path =
            std::env::temp_dir().join(format!("moe-android-dsd-{name}-{}.dsf", std::process::id()));
        std::fs::write(
            &path,
            build_dsf(rate, samples as u64, 4096, true, &[left, right]),
        )
        .unwrap();
        path
    }

    #[test]
    fn only_dsf_file_uris_become_dsd_uris() {
        assert_eq!(
            to_dsd_uri("file:///storage/emulated/0/Music/a.dsf"),
            "moedsd:///storage/emulated/0/Music/a.dsf"
        );
        assert_eq!(to_dsd_uri("file:///x/A%20B.DSF"), "moedsd:///x/A%20B.DSF");
        assert_eq!(to_dsd_uri("file:///x/a.flac"), "file:///x/a.flac");
        assert_eq!(
            to_dsd_uri("content://media/external/audio/media/5"),
            "content://media/external/audio/media/5"
        );
    }

    #[test]
    fn a_dsf_is_served_as_a_float_wav_with_random_access() {
        let path = write_dsf("serve");
        let handle = open(&path).unwrap();
        assert_ne!(handle, 0);
        let total = length(handle).unwrap();
        let frames = 2_822_400 / 4 / 16;
        assert_eq!(total, 44 + frames * 8);
        let mut header = [0_u8; 44];
        assert_eq!(read(handle, 0, &mut header).unwrap(), 44);
        assert_eq!(&header[0..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(header[24..28].try_into().unwrap()),
            176_400
        );
        // Read the middle, then an earlier position (what a seek in the player does), then past the end.
        let mut buffer = vec![0_u8; 4_096];
        let middle = 44 + (total - 44) / 16 * 8;
        assert_eq!(read(handle, middle, &mut buffer).unwrap(), 4_096);
        assert!(buffer
            .chunks_exact(4)
            .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 0.05));
        assert_eq!(read(handle, 1_000, &mut buffer).unwrap(), 4_096);
        assert_eq!(read(handle, total, &mut buffer).unwrap(), 0);
        close(handle);
        assert!(length(handle).is_none(), "a closed handle is gone");
        assert!(read(handle, 0, &mut buffer).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_stream_starts_at_the_current_shared_gain() {
        let path = write_dsf("gain");
        let peak_of_new_stream = |gain_db: f32| {
            set_gain_db(gain_db);
            let handle = open(&path).unwrap();
            let total = length(handle).unwrap();
            let mut buffer = vec![0_u8; 64 * 1024];
            // A position on a frame boundary of the data section (8 bytes per stereo frame after the 44-byte header).
            let middle = 44 + (total - 44) / 16 * 8;
            let count = read(handle, middle, &mut buffer).unwrap();
            close(handle);
            buffer[..count]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()).abs())
                .fold(0.0_f32, f32::max)
        };
        let quiet = peak_of_new_stream(0.0);
        let loud = peak_of_new_stream(6.0);
        set_gain_db(3.0);
        assert!(quiet > 0.1, "{quiet}");
        assert!((loud / quiet - 1.995).abs() < 0.05, "{quiet} -> {loud}");
        let _ = std::fs::remove_file(path);
    }
}
