//! DSD and WavPack playback on Android (ADR 0016, ADR 0025).
//!
//! Media3 has neither a DSD nor a WavPack decoder, so a `.dsf` track is loaded with a `moedsd:///path/to/file.dsf`
//! URI and a `.wv` track with `moewv:///path/to/file.wv` (the same data source serves both; the WavPack file is
//! decoded at its own sample rate, with its `.wvc` correction file when there is one).
//! The text below describes the DSD case; the WavPack case is the same except for the converter.
//!
//! A DSD URI is served like this. The Kotlin
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
use player_wavpack::wav::WavPackWavStream;

/// URI scheme of a DSD track handed to the Kotlin data source.
pub(crate) const DSD_URI_SCHEME: &str = "moedsd";
/// URI scheme of a WavPack track handed to the Kotlin data source.
pub(crate) const WAVPACK_URI_SCHEME: &str = "moewv";
/// URI scheme of a remote track (ADR 0026): `moeremote://<source id>/<path>`, the text form of its locator.
pub(crate) const REMOTE_URI_SCHEME: &str = "moeremote";
/// `AudioTrack` tops out at 192 kHz (a PCM WavPack file above that cannot be played; DSD is converted to 176.4 kHz).
const ANDROID_MAX_SAMPLE_RATE: u32 = 192_000;
/// The only PCM rate DSD is converted to on Android.
pub(crate) const ANDROID_DSD_OUTPUT_RATE: u32 = 176_400;

/// A file served to Media3 as a float WAV, or, for a remote file Media3 can decode itself, as the file's own bytes.
enum Served {
    Dsd(Box<DsdWavStream<DsfReader<BufReader<File>>>>),
    /// A remote DSF file, converted like a local one.
    DsdRemote(Box<DsdWavStream<DsfReader<BufReader<Box<dyn player_wavpack::ReadSeek>>>>>),
    /// A remote file passed through unchanged (FLAC, MP3, AAC, Opus…): Media3 reads and seeks it like a local file.
    Raw(RawRemote),
    /// DSD inside WavPack, converted like a DSF file.
    DsdWavPack(Box<DsdWavStream<Box<dyn player_dsd::stream::DsdStream>>>),
    WavPack(Box<WavPackWavStream>),
}

/// A remote file served byte for byte.
struct RawRemote {
    reader: Box<dyn player_core::RemoteStream>,
    len: u64,
}

impl RawRemote {
    fn read_at(&mut self, position: u64, out: &mut [u8]) -> Result<usize, String> {
        use std::io::{Read, Seek, SeekFrom};
        if position >= self.len || out.is_empty() {
            return Ok(0);
        }
        self.reader
            .seek(SeekFrom::Start(position))
            .map_err(|error| error.to_string())?;
        let wanted = ((self.len - position).min(out.len() as u64)) as usize;
        let mut done = 0;
        while done < wanted {
            let read = self
                .reader
                .read(&mut out[done..wanted])
                .map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            done += read;
        }
        Ok(done)
    }
}

impl Served {
    fn total_len(&self) -> u64 {
        match self {
            Self::Dsd(stream) => stream.total_len(),
            Self::DsdRemote(stream) => stream.total_len(),
            Self::Raw(file) => file.len,
            Self::DsdWavPack(stream) => stream.total_len(),
            Self::WavPack(stream) => stream.total_len(),
        }
    }

    fn read_at(&mut self, position: u64, out: &mut [u8]) -> Result<usize, String> {
        match self {
            Self::Dsd(stream) => stream.read_at(position, out).map_err(|error| error.to_string()),
            Self::DsdRemote(stream) => stream.read_at(position, out).map_err(|error| error.to_string()),
            Self::Raw(file) => file.read_at(position, out),
            Self::DsdWavPack(stream) => stream.read_at(position, out).map_err(|error| error.to_string()),
            Self::WavPack(stream) => stream.read_at(position, out).map_err(|error| error.to_string()),
        }
    }
}

type DsdStreamHandle = Served;

/// `file:///x/y.dsf` becomes `moedsd:///x/y.dsf` and `file:///x/y.wv` becomes `moewv:///x/y.wv`; every other URI is
/// returned unchanged.
pub(crate) fn to_dsd_uri(uri: &str) -> String {
    let Some(rest) = uri.strip_prefix("file://") else {
        return uri.to_owned();
    };
    let lower = rest.to_ascii_lowercase();
    if lower.ends_with(".dsf") {
        format!("{DSD_URI_SCHEME}://{rest}")
    } else if lower.ends_with(".wv") {
        format!("{WAVPACK_URI_SCHEME}://{rest}")
    } else {
        uri.to_owned()
    }
}

/// The URI the player loads for a track: its file URI, as a `moedsd` / `moewv` URI when it is a DSF or WavPack file.
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

fn open_dsf(path: &Path) -> Result<Served, String> {
    let file = File::open(path).map_err(|error| format!("無法開啟 DSD 檔案：{error}"))?;
    let dsf = DsfReader::open(BufReader::with_capacity(256 * 1024, file))
        .map_err(|error| error.to_string())?;
    let reader = DsdPcmReader::new(dsf, ANDROID_DSD_OUTPUT_RATE, Arc::clone(gain()))
        .map_err(|error| error.to_string())?;
    Ok(Served::Dsd(Box::new(DsdWavStream::new(reader))))
}

fn open_wavpack(path: &Path) -> Result<Served, String> {
    // DSD inside WavPack goes through the DSD converter at 176.4 kHz, like a DSF file (ADR 0025).
    if player_wavpack::is_dsd(path) {
        let stream = player_wavpack::dsd::WavPackDsdStream::open_path(path)
            .map_err(|error| error.to_string())?;
        let reader = DsdPcmReader::new(
            Box::new(stream) as Box<dyn player_dsd::stream::DsdStream>,
            ANDROID_DSD_OUTPUT_RATE,
            Arc::clone(gain()),
        )
        .map_err(|error| error.to_string())?;
        return Ok(Served::DsdWavPack(Box::new(DsdWavStream::new(reader))));
    }
    // The `.wvc` correction file next to a hybrid `.wv` is used too, as on Windows.
    let reader = player_wavpack::WavPackReader::open_path(path).map_err(|error| error.to_string())?;
    let rate = reader.info().sample_rate;
    if rate > ANDROID_MAX_SAMPLE_RATE {
        return Err(format!("取樣率 {rate} Hz 超過 Android 的上限 {ANDROID_MAX_SAMPLE_RATE} Hz"));
    }
    let stream = WavPackWavStream::new(reader).map_err(|error| error.to_string())?;
    Ok(Served::WavPack(Box::new(stream)))
}

/// Open a remote file through the shared provider (ADR 0026). DSF and WavPack are converted like local files; any
/// other format is passed through for Media3 to decode.
fn open_remote(uri: &str) -> Result<Served, String> {
    let Some(player_core::MediaLocator::Remote { source_id, path }) =
        player_core::parse_remote_locator_uri(uri)
    else {
        return Err("不是遠端曲目的位置".to_owned());
    };
    let content = player_core::remote_content().ok_or_else(|| "遠端播放尚未就緒".to_owned())?;
    let file = content
        .open_streaming(source_id, &path)
        .map_err(|error| format!("無法開啟遠端檔案：{error}"))?;
    let name = Path::new(&path);
    match player_formats::decode_engine(name) {
        player_formats::DecodeEngine::Dsd => {
            let dsf = DsfReader::open(BufReader::with_capacity(
                256 * 1024,
                Box::new(RemoteStreamAdapter(file.reader)) as Box<dyn player_wavpack::ReadSeek>,
            ))
            .map_err(|error| error.to_string())?;
            let reader = DsdPcmReader::new(dsf, ANDROID_DSD_OUTPUT_RATE, Arc::clone(gain()))
                .map_err(|error| error.to_string())?;
            Ok(Served::DsdRemote(Box::new(DsdWavStream::new(reader))))
        }
        player_formats::DecodeEngine::WavPack => {
            // A hybrid file is lossless with its `.wvc` correction file next to it.
            let correction = content
                .open_streaming(source_id, &format!("{path}c"))
                .ok()
                .map(|handle| Box::new(RemoteStreamAdapter(handle.reader)) as Box<dyn player_wavpack::ReadSeek>);
            let main = RemoteStreamAdapter(file.reader);
            let probe_main = content.open(source_id, &path).map_err(|error| error.to_string())?;
            if player_wavpack::is_dsd_from(RemoteStreamAdapter(probe_main.reader)) {
                let stream = player_wavpack::dsd::WavPackDsdStream::open(main, correction)
                    .map_err(|error| error.to_string())?;
                let reader = DsdPcmReader::new(
                    Box::new(stream) as Box<dyn player_dsd::stream::DsdStream>,
                    ANDROID_DSD_OUTPUT_RATE,
                    Arc::clone(gain()),
                )
                .map_err(|error| error.to_string())?;
                return Ok(Served::DsdWavPack(Box::new(DsdWavStream::new(reader))));
            }
            let reader = player_wavpack::WavPackReader::open(main, correction).map_err(|error| error.to_string())?;
            let rate = reader.info().sample_rate;
            if rate > ANDROID_MAX_SAMPLE_RATE {
                return Err(format!("取樣率 {rate} Hz 超過 Android 的上限 {ANDROID_MAX_SAMPLE_RATE} Hz"));
            }
            let stream = WavPackWavStream::new(reader).map_err(|error| error.to_string())?;
            Ok(Served::WavPack(Box::new(stream)))
        }
        _ => Ok(Served::Raw(RawRemote {
            reader: file.reader,
            len: file.len,
        })),
    }
}

/// A provider stream as the `Read + Seek + Send` the converters take.
struct RemoteStreamAdapter(Box<dyn player_core::RemoteStream>);

impl std::io::Read for RemoteStreamAdapter {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl std::io::Seek for RemoteStreamAdapter {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(position)
    }
}

/// Open a DSF or WavPack file (or a remote track); the handle (never 0) is used with the other functions until `close`.
pub(crate) fn open(path: &Path) -> Result<i64, String> {
    if path.to_str().is_some_and(|text| text.starts_with("moeremote://")) {
        let started = std::time::Instant::now();
        let served = open_remote(path.to_str().unwrap_or_default());
        eprintln!("[remote] open {:?} took {} ms (ok: {})", path.file_name(), started.elapsed().as_millis(), served.is_ok());
        return Ok(register(served?));
    }
    let served = match player_formats::decode_engine(path) {
        player_formats::DecodeEngine::Dsd => open_dsf(path)?,
        player_formats::DecodeEngine::WavPack => open_wavpack(path)?,
        _ => return Err(format!("不是 DSD 或 WavPack 檔案：{}", path.display())),
    };
    Ok(register(served))
}

fn register(served: Served) -> i64 {
    let mut registry = registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    registry.next += 1;
    let handle = registry.next;
    registry
        .streams
        .insert(handle, Arc::new(Mutex::new(served)));
    handle
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
    let started = std::time::Instant::now();
    let result = stream.read_at(position, out);
    if started.elapsed().as_millis() >= 150 {
        eprintln!("[remote] slow read at {position} ({} bytes) took {} ms", out.len(), started.elapsed().as_millis());
    }
    result
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

    /// The cover of a remote track (`moeremote://…`) as the file's own picture bytes, or null. Blocks on the server, so
    /// the service calls it from a worker thread after the track has started.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_DsdBridge_nativeRemoteCover<'local>(
        mut env: JNIEnv<'local>,
        _class: JClass<'local>,
        uri: JString<'local>,
    ) -> jni::sys::jbyteArray {
        let Ok(uri) = env.get_string(&uri) else {
            return std::ptr::null_mut();
        };
        let uri: String = uri.into();
        let Some(player_core::MediaLocator::Remote { source_id, path }) =
            player_core::parse_remote_locator_uri(&uri)
        else {
            return std::ptr::null_mut();
        };
        let started = std::time::Instant::now();
        let bytes = crate::remote_media::remote_cover_bytes(source_id, &path);
        eprintln!(
            "[remote] cover lookup took {} ms (found: {})",
            started.elapsed().as_millis(),
            bytes.is_some()
        );
        match bytes.and_then(|bytes| env.byte_array_from_slice(&bytes).ok()) {
            Some(array) => array.into_raw(),
            None => std::ptr::null_mut(),
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
                // The remote stream retried and gave up (ADR 0026, "link lost"): -2 tells Kotlin to stop, not retry.
                if player_core::is_remote_link_lost(&error) {
                    return -2;
                }
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
    fn wavpack_file_uris_become_wavpack_uris() {
        assert_eq!(
            to_dsd_uri("file:///storage/emulated/0/Music/a.wv"),
            "moewv:///storage/emulated/0/Music/a.wv"
        );
        assert_eq!(to_dsd_uri("file:///x/A%20B.WV"), "moewv:///x/A%20B.WV");
        assert_eq!(to_dsd_uri("file:///x/a.wvc"), "file:///x/a.wvc");
        assert_eq!(to_dsd_uri("file:///x/a.wav"), "file:///x/a.wav");
    }

    fn wavpack_reference(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/player-wavpack/tests/reference")
            .join(name)
    }

    #[test]
    fn a_wavpack_file_is_served_as_a_float_wav_at_its_own_rate_with_random_access() {
        let handle = open(&wavpack_reference("int24_stereo.wv")).unwrap();
        assert_ne!(handle, 0);
        let total = length(handle).unwrap();
        let mut header = [0_u8; 44];
        assert_eq!(read(handle, 0, &mut header).unwrap(), 44);
        assert_eq!(&header[0..4], b"RIFF");
        assert_eq!(u16::from_le_bytes(header[20..22].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(header[24..28].try_into().unwrap()), 48_000);
        // 0.1 s of stereo float: 4800 frames x 8 bytes.
        assert_eq!(total, 44 + 4_800 * 8);
        let mut buffer = vec![0_u8; 1_024];
        let middle = 44 + 2_400 * 8;
        assert_eq!(read(handle, middle, &mut buffer).unwrap(), 1_024);
        assert!(buffer
            .chunks_exact(4)
            .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 0.01));
        assert_eq!(read(handle, 100, &mut buffer).unwrap(), 1_024);
        assert_eq!(read(handle, total, &mut buffer).unwrap(), 0);
        close(handle);
        assert!(length(handle).is_none());
    }

    #[test]
    fn dsd_inside_wavpack_is_served_at_176_4_khz_by_the_dsd_converter() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/player-wavpack/tests/fixtures/dsd64.wv");
        let handle = open(&path).unwrap();
        let total = length(handle).unwrap();
        let mut header = [0_u8; 44];
        assert_eq!(read(handle, 0, &mut header).unwrap(), 44);
        assert_eq!(u32::from_le_bytes(header[24..28].try_into().unwrap()), 176_400);
        // 0.2 s of DSD64 at 176.4 kHz: 35 280 frames of 8 bytes.
        assert_eq!(total, 44 + 35_280 * 8);
        let mut buffer = vec![0_u8; 8_192];
        assert_eq!(read(handle, 44 + 10_000 * 8, &mut buffer).unwrap(), 8_192);
        assert!(buffer
            .chunks_exact(4)
            .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 0.05));
        close(handle);
    }

    #[test]
    fn files_that_are_neither_dsd_nor_wavpack_are_refused() {
        assert!(open(&wavpack_reference("int24_stereo.ss.txt")).is_err());
        assert!(open(&wavpack_reference("missing.wv")).is_err());
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
