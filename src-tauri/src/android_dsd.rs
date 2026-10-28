//! DSD, WavPack and FLAC playback on Android (ADR 0016, ADR 0025, ADR 0043).
//!
//! Media3 has neither a DSD nor a WavPack decoder, and some devices offer only a vendor FLAC decoder that fails on
//! 24-bit files, so these formats are decoded here. Which formats go through a converter is decided by
//! `player_formats::android_converter`: a `.dsf` track is loaded with a `moedsd:///path/to/file.dsf` URI, a `.wv`
//! track with `moewv:///path/to/file.wv` and a `.flac` track with `moeflac:///path/to/file.flac` (the same data
//! source serves all three; WavPack and FLAC are decoded at their own sample rate, a hybrid WavPack file with its
//! `.wvc` correction file when there is one). A remote track keeps its `moeremote://` URI and is converted the same
//! way when its format needs it. The text below describes the DSD case; the others are the same except for the
//! converter.
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
use player_dsd::raw::{RawDsdWavStream, SlotByteOrder};
use player_dsd::reader::DsdPcmReader;
use player_dsd::stream::DsdStream;
use player_dsd::wav::DsdWavStream;
use player_flac::wav::FlacWavStream;
use player_formats::AndroidConverter;
use player_wavpack::wav::WavPackWavStream;

/// URI scheme of a DSD track handed to the Kotlin data source.
pub(crate) const DSD_URI_SCHEME: &str = "moedsd";
/// URI scheme of a local DSF served as the raw 32-bit DSD slots of native passthrough to the USB exclusive output
/// (ADR 0049). Only Kotlin's load-time decision (`DsdPassthrough`) makes it from a `moedsd://` URI (`raw_uri_of`);
/// Rust never produces it. Kotlin lists it in `LOCAL_CONVERTED_SCHEMES`.
pub(crate) const DSD_RAW_URI_SCHEME: &str = "moedsdraw";
/// As [`DSD_RAW_URI_SCHEME`], for a local WavPack file that holds DSD (made from `moewv://`).
pub(crate) const WAVPACK_RAW_URI_SCHEME: &str = "moewvraw";
/// As [`DSD_RAW_URI_SCHEME`], for a remote DSF or WavPack-DSD file (made from `moeremote://`). It is a scheme of its
/// own, not a query on the locator, because `parse_remote_locator_uri` takes a `?` as part of the path.
pub(crate) const REMOTE_RAW_URI_SCHEME: &str = "moeremoteraw";
/// URI scheme of a WavPack track handed to the Kotlin data source.
pub(crate) const WAVPACK_URI_SCHEME: &str = "moewv";
/// URI scheme of a FLAC track handed to the Kotlin data source.
pub(crate) const FLAC_URI_SCHEME: &str = "moeflac";
/// URI scheme of a remote track (ADR 0026): `moeremote://<source id>/<path>`, the text form of its locator.
pub(crate) const REMOTE_URI_SCHEME: &str = "moeremote";
/// `AudioTrack` tops out at 192 kHz (a PCM WavPack or FLAC file above that cannot be played; DSD is converted to
/// 176.4 kHz).
const ANDROID_MAX_SAMPLE_RATE: u32 = 192_000;
/// The only PCM rate DSD is converted to on Android.
pub(crate) const ANDROID_DSD_OUTPUT_RATE: u32 = 176_400;

/// A file served to Media3 as a float WAV, or, for a remote file Media3 can decode itself, as the file's own bytes.
enum Served {
    Dsd(Box<DsdWavStream<DsfReader<BufReader<File>>>>),
    /// A DSF or WavPack-DSD file, local or remote, as raw DSD slots (native passthrough, ADR 0049).
    DsdRaw(Box<RawDsdWavStream<Box<dyn DsdStream>>>),
    /// A remote DSF file, converted like a local one.
    DsdRemote(Box<DsdWavStream<DsfReader<BufReader<Box<dyn player_wavpack::ReadSeek>>>>>),
    /// A remote file passed through unchanged (FLAC, MP3, AAC, Opus…): Media3 reads and seeks it like a local file.
    Raw(RawRemote),
    /// DSD inside WavPack, converted like a DSF file.
    DsdWavPack(Box<DsdWavStream<Box<dyn player_dsd::stream::DsdStream>>>),
    WavPack(Box<WavPackWavStream>),
    /// A local or remote FLAC file.
    Flac(Box<FlacWavStream>),
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
            Self::DsdRaw(stream) => stream.total_len(),
            Self::DsdRemote(stream) => stream.total_len(),
            Self::Raw(file) => file.len,
            Self::DsdWavPack(stream) => stream.total_len(),
            Self::WavPack(stream) => stream.total_len(),
            Self::Flac(stream) => stream.total_len(),
        }
    }

    fn read_at(&mut self, position: u64, out: &mut [u8]) -> Result<usize, String> {
        match self {
            Self::Dsd(stream) => stream
                .read_at(position, out)
                .map_err(|error| error.to_string()),
            Self::DsdRaw(stream) => stream
                .read_at(position, out)
                .map_err(|error| error.to_string()),
            Self::DsdRemote(stream) => stream
                .read_at(position, out)
                .map_err(|error| error.to_string()),
            Self::Raw(file) => file.read_at(position, out),
            Self::DsdWavPack(stream) => stream
                .read_at(position, out)
                .map_err(|error| error.to_string()),
            Self::WavPack(stream) => stream
                .read_at(position, out)
                .map_err(|error| error.to_string()),
            Self::Flac(stream) => stream
                .read_at(position, out)
                .map_err(|error| error.to_string()),
        }
    }
}

type DsdStreamHandle = Served;

fn scheme_of(converter: AndroidConverter) -> &'static str {
    match converter {
        AndroidConverter::Dsd => DSD_URI_SCHEME,
        AndroidConverter::WavPack => WAVPACK_URI_SCHEME,
        AndroidConverter::Flac => FLAC_URI_SCHEME,
    }
}

/// A `file://` URI of a format Android converts gets the converter's scheme (`file:///x/y.dsf` becomes
/// `moedsd:///x/y.dsf`, `.wv` becomes `moewv://`, `.flac` becomes `moeflac://`); every other URI is returned unchanged.
pub(crate) fn to_converted_uri(uri: &str) -> String {
    let Some(rest) = uri.strip_prefix("file://") else {
        return uri.to_owned();
    };
    match player_formats::android_converter(Path::new(rest)) {
        Some(converter) => format!("{}://{rest}", scheme_of(converter)),
        None => uri.to_owned(),
    }
}

/// The native-passthrough form of a load URI (`moedsd://` becomes `moedsdraw://`, `moewv://` becomes `moewvraw://`,
/// `moeremote://` becomes `moeremoteraw://`, same rest), or `None` for anything else. Kotlin does the same swap when it
/// decides on passthrough; this pins the contract.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn raw_uri_of(converted: &str) -> Option<String> {
    [
        (DSD_URI_SCHEME, DSD_RAW_URI_SCHEME),
        (WAVPACK_URI_SCHEME, WAVPACK_RAW_URI_SCHEME),
        (REMOTE_URI_SCHEME, REMOTE_RAW_URI_SCHEME),
    ]
    .into_iter()
    .find_map(|(scheme, raw)| {
        converted
            .strip_prefix(scheme)
            .filter(|rest| rest.starts_with("://"))
            .map(|rest| format!("{raw}{rest}"))
    })
}

/// The `moeremote://` location inside a `moeremoteraw://` one, or `None` for anything else.
fn remote_uri_of_raw(raw: &str) -> Option<String> {
    raw.strip_prefix(REMOTE_RAW_URI_SCHEME)
        .filter(|rest| rest.starts_with("://"))
        .map(|rest| format!("{REMOTE_URI_SCHEME}{rest}"))
}

/// Whether the data source serves this load URI as a converted WAV stream (the service then tells Media3 it is
/// `audio/wav`, since the extension says otherwise): a converter scheme, or a remote track whose format is converted.
/// The raw DSD schemes are a WAV of raw slots, not audio, but they are read the same way.
pub(crate) fn serves_converted_wav(uri: &str) -> bool {
    let has_scheme = |scheme: &str| {
        uri.strip_prefix(scheme)
            .is_some_and(|rest| rest.starts_with("://"))
    };
    if [
        DSD_URI_SCHEME,
        DSD_RAW_URI_SCHEME,
        WAVPACK_URI_SCHEME,
        WAVPACK_RAW_URI_SCHEME,
        FLAC_URI_SCHEME,
        REMOTE_RAW_URI_SCHEME,
    ]
    .into_iter()
    .any(has_scheme)
    {
        return true;
    }
    match player_core::parse_remote_locator_uri(uri) {
        Some(player_core::MediaLocator::Remote { path, .. }) => {
            player_formats::android_converter(Path::new(&path)).is_some()
        }
        _ => false,
    }
}

/// The URI the player loads for a track: its file URI, with a converter scheme when Android converts its format.
pub(crate) fn playable_uri(
    database: &player_db::Database,
    track_id: player_core::TrackId,
) -> Result<String, player_db::DatabaseError> {
    database
        .resolve_playable_content_uri(track_id)
        .map(|uri| to_converted_uri(&uri))
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

static RAW_BYTE_ORDER: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Set the byte order of the slots native passthrough serves (`dsd.nativeByteOrder`, hidden; ADR 0049). It applies to
/// the next file opened.
pub(crate) fn set_native_byte_order(order: crate::settings::DsdNativeByteOrder) {
    RAW_BYTE_ORDER.store(
        u8::from(order == crate::settings::DsdNativeByteOrder::B),
        Ordering::Relaxed,
    );
}

fn native_byte_order() -> SlotByteOrder {
    if RAW_BYTE_ORDER.load(Ordering::Relaxed) == 1 {
        SlotByteOrder::NewestFirst
    } else {
        SlotByteOrder::OldestFirst
    }
}

/// Which reader a raw DSD source gets from the remote provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RawAccess {
    /// Only the header is read (the plan): the provider's cached reader, which keeps the first block for the playback
    /// that follows and does not read ahead.
    Header,
    /// The track plays from start to end: the streaming reader with read-ahead.
    Playback,
}

const NOT_DSD_FILE: &str = "不是 DSF 或 WavPack-DSD 檔案";

/// The DSD bitstream of a DSF or WavPack-DSD file named by `target`: a path, a `moeremote://` location or a
/// `moeremoteraw://` one. Local and remote files go through the same two readers (`DsfReader`, `WavPackDsdStream`).
fn raw_dsd_stream(
    content: Option<&dyn player_core::RemoteContent>,
    target: &str,
    access: RawAccess,
) -> Result<Box<dyn DsdStream>, String> {
    let remote = remote_uri_of_raw(target).or_else(|| {
        target
            .starts_with("moeremote://")
            .then(|| target.to_owned())
    });
    match remote {
        Some(uri) => raw_remote_dsd_stream(content, &uri, access),
        None => raw_local_dsd_stream(Path::new(target)),
    }
}

fn raw_local_dsd_stream(path: &Path) -> Result<Box<dyn DsdStream>, String> {
    match player_formats::android_converter(path) {
        Some(AndroidConverter::Dsd) => {
            let file = File::open(path).map_err(|error| format!("無法開啟 DSD 檔案：{error}"))?;
            let dsf = DsfReader::open(BufReader::with_capacity(256 * 1024, file))
                .map_err(|error| error.to_string())?;
            Ok(Box::new(dsf))
        }
        Some(AndroidConverter::WavPack) => {
            let stream = player_wavpack::dsd::WavPackDsdStream::open_path(path)
                .map_err(|error| error.to_string())?;
            Ok(Box::new(stream))
        }
        _ => Err(NOT_DSD_FILE.to_owned()),
    }
}

fn raw_remote_dsd_stream(
    content: Option<&dyn player_core::RemoteContent>,
    uri: &str,
    access: RawAccess,
) -> Result<Box<dyn DsdStream>, String> {
    let Some(player_core::MediaLocator::Remote { source_id, path }) =
        player_core::parse_remote_locator_uri(uri)
    else {
        return Err("不是遠端曲目的位置".to_owned());
    };
    let content = content.ok_or_else(|| "遠端播放尚未就緒".to_owned())?;
    let converter = player_formats::android_converter(Path::new(&path));
    if !matches!(
        converter,
        Some(AndroidConverter::Dsd | AndroidConverter::WavPack)
    ) {
        return Err(NOT_DSD_FILE.to_owned());
    }
    let opened = match access {
        RawAccess::Header => content.open_cached(source_id, &path),
        RawAccess::Playback => content.open_streaming(source_id, &path),
    };
    let file = opened.map_err(|error| format!("無法開啟遠端檔案：{error}"))?;
    if converter == Some(AndroidConverter::Dsd) {
        let capacity = match access {
            RawAccess::Header => 4096,
            RawAccess::Playback => 256 * 1024,
        };
        let dsf = DsfReader::open(BufReader::with_capacity(
            capacity,
            Box::new(RemoteStreamAdapter(file.reader)) as Box<dyn player_wavpack::ReadSeek>,
        ))
        .map_err(|error| error.to_string())?;
        return Ok(Box::new(dsf));
    }
    // A hybrid file is lossless with its `.wvc` correction file next to it (only a playing track needs it).
    let correction = match access {
        RawAccess::Header => None,
        RawAccess::Playback => content
            .open_streaming(source_id, &format!("{path}c"))
            .ok()
            .map(|handle| {
                Box::new(RemoteStreamAdapter(handle.reader)) as Box<dyn player_wavpack::ReadSeek>
            }),
    };
    let stream =
        player_wavpack::dsd::WavPackDsdStream::open(RemoteStreamAdapter(file.reader), correction)
            .map_err(|error| error.to_string())?;
    Ok(Box::new(stream))
}

fn open_raw_with(
    content: Option<&dyn player_core::RemoteContent>,
    target: &str,
) -> Result<Served, String> {
    let stream = raw_dsd_stream(content, target, RawAccess::Playback)?;
    let stream =
        RawDsdWavStream::new(stream, native_byte_order()).map_err(|error| error.to_string())?;
    Ok(Served::DsdRaw(Box::new(stream)))
}

/// Whether the DSF or WavPack-DSD file `target` (a path or a remote location) can be carried natively and at which
/// frame rate (DSD bit rate / 32): JSON `{ "ok": true, "rate": 88200, "name": "DSD64" }` or
/// `{ "ok": false, "reason": "..." }`. Reads the header only; a remote file goes through the provider's cached reader,
/// so call it off the main thread.
pub(crate) fn raw_plan_json(target: &str) -> String {
    let content = player_core::remote_content();
    raw_plan_json_with(content.as_deref(), target)
}

fn raw_plan_json_with(content: Option<&dyn player_core::RemoteContent>, target: &str) -> String {
    let plan = (|| -> Result<(u32, String), String> {
        let stream = raw_dsd_stream(content, target, RawAccess::Header)?;
        let info = stream.info();
        match player_dsd::raw::raw_transport_rate(&info) {
            Some(rate) => Ok((rate, info.rate_name())),
            None => Err(player_dsd::raw::raw_unsupported_reason(&info)
                .unwrap_or_else(|| "不支援直通".to_owned())),
        }
    })();
    match plan {
        Ok((rate, name)) => serde_json::json!({ "ok": true, "rate": rate, "name": name }),
        Err(reason) => serde_json::json!({ "ok": false, "reason": reason }),
    }
    .to_string()
}

fn flac_stream(reader: player_flac::FlacReader) -> Result<Served, String> {
    let rate = reader.info().sample_rate;
    if rate > ANDROID_MAX_SAMPLE_RATE {
        return Err(format!(
            "取樣率 {rate} Hz 超過 Android 的上限 {ANDROID_MAX_SAMPLE_RATE} Hz"
        ));
    }
    let stream = FlacWavStream::new(reader).map_err(|error| error.to_string())?;
    Ok(Served::Flac(Box::new(stream)))
}

fn open_flac(path: &Path) -> Result<Served, String> {
    let reader = player_flac::FlacReader::open_path(path)
        .map_err(|error| format!("無法開啟 FLAC 檔案：{error}"))?;
    flac_stream(reader)
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
    let reader =
        player_wavpack::WavPackReader::open_path(path).map_err(|error| error.to_string())?;
    let rate = reader.info().sample_rate;
    if rate > ANDROID_MAX_SAMPLE_RATE {
        return Err(format!(
            "取樣率 {rate} Hz 超過 Android 的上限 {ANDROID_MAX_SAMPLE_RATE} Hz"
        ));
    }
    let stream = WavPackWavStream::new(reader).map_err(|error| error.to_string())?;
    Ok(Served::WavPack(Box::new(stream)))
}

/// Open a remote file through the shared provider (ADR 0026). DSF, WavPack and FLAC are converted like local files;
/// any other format is passed through for Media3 to decode.
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
    match player_formats::android_converter(name) {
        Some(AndroidConverter::Dsd) => {
            let dsf = DsfReader::open(BufReader::with_capacity(
                256 * 1024,
                Box::new(RemoteStreamAdapter(file.reader)) as Box<dyn player_wavpack::ReadSeek>,
            ))
            .map_err(|error| error.to_string())?;
            let reader = DsdPcmReader::new(dsf, ANDROID_DSD_OUTPUT_RATE, Arc::clone(gain()))
                .map_err(|error| error.to_string())?;
            Ok(Served::DsdRemote(Box::new(DsdWavStream::new(reader))))
        }
        Some(AndroidConverter::Flac) => {
            let reader = player_flac::FlacReader::open(RemoteStreamAdapter(file.reader))
                .map_err(|error| error.to_string())?;
            flac_stream(reader)
        }
        Some(AndroidConverter::WavPack) => {
            // A hybrid file is lossless with its `.wvc` correction file next to it.
            let correction = content
                .open_streaming(source_id, &format!("{path}c"))
                .ok()
                .map(|handle| {
                    Box::new(RemoteStreamAdapter(handle.reader))
                        as Box<dyn player_wavpack::ReadSeek>
                });
            let main = RemoteStreamAdapter(file.reader);
            let probe_main = content
                .open(source_id, &path)
                .map_err(|error| error.to_string())?;
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
            let reader = player_wavpack::WavPackReader::open(main, correction)
                .map_err(|error| error.to_string())?;
            let rate = reader.info().sample_rate;
            if rate > ANDROID_MAX_SAMPLE_RATE {
                return Err(format!(
                    "取樣率 {rate} Hz 超過 Android 的上限 {ANDROID_MAX_SAMPLE_RATE} Hz"
                ));
            }
            let stream = WavPackWavStream::new(reader).map_err(|error| error.to_string())?;
            Ok(Served::WavPack(Box::new(stream)))
        }
        None => Ok(Served::Raw(RawRemote {
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

/// Open a DSF or WavPack-DSD file, local (a path) or remote (`moeremoteraw://…`), as raw DSD slots (native
/// passthrough, ADR 0049); the handle is used like `open`'s.
pub(crate) fn open_raw(target: &str) -> Result<i64, String> {
    let content = player_core::remote_content();
    Ok(register(open_raw_with(content.as_deref(), target)?))
}

/// Open a DSF, WavPack or FLAC file (or a remote track); the handle (never 0) is used with the other functions until
/// `close`.
pub(crate) fn open(path: &Path) -> Result<i64, String> {
    if path
        .to_str()
        .is_some_and(|text| text.starts_with("moeremote://"))
    {
        let started = std::time::Instant::now();
        let served = open_remote(path.to_str().unwrap_or_default());
        eprintln!(
            "[remote] open {:?} took {} ms (ok: {})",
            path.file_name(),
            started.elapsed().as_millis(),
            served.is_ok()
        );
        return Ok(register(served?));
    }
    let served = match player_formats::android_converter(path) {
        Some(AndroidConverter::Dsd) => open_dsf(path)?,
        Some(AndroidConverter::WavPack) => open_wavpack(path)?,
        Some(AndroidConverter::Flac) => open_flac(path)?,
        None => {
            return Err(format!(
                "不是 DSD、WavPack 或 FLAC 檔案：{}",
                path.display()
            ))
        }
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
        eprintln!(
            "[remote] slow read at {position} ({} bytes) took {} ms",
            out.len(),
            started.elapsed().as_millis()
        );
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
                // The server of a remote track cannot be reached: -2 tells Kotlin to stop at once, not retry.
                if player_core::is_remote_unreachable(&error) {
                    return -2;
                }
                0
            }
        }
    }

    /// Opens a DSF or WavPack-DSD file (a path, or a remote location) as the raw slots of native passthrough (ADR 0049);
    /// 0 when it cannot be carried, -2 an unreachable server.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_DsdBridge_nativeOpenRaw(
        mut env: JNIEnv,
        _class: JClass,
        path: JString,
    ) -> jlong {
        let Ok(path) = env.get_string(&path) else {
            return 0;
        };
        match open_raw(&path.to_string_lossy()) {
            Ok(handle) => handle,
            Err(error) => {
                eprintln!("[dsd] raw open failed: {error}");
                // As `nativeOpen`: an unreachable server stops the load at once.
                if player_core::is_remote_unreachable(&error) {
                    return -2;
                }
                0
            }
        }
    }

    /// Whether the DSF or WavPack-DSD file can be played natively: the JSON of `raw_plan_json`.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_DsdBridge_nativeRawPlan<'local>(
        mut env: JNIEnv<'local>,
        _class: JClass<'local>,
        path: JString<'local>,
    ) -> jni::sys::jstring {
        let Ok(path) = env.get_string(&path) else {
            return std::ptr::null_mut();
        };
        let json = raw_plan_json(&path.to_string_lossy());
        env.new_string(json)
            .map(|value| value.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }

    /// The cover of a remote track (`moeremote://…`) as the file's own picture bytes, or null. Blocks on the server, so
    /// the service calls it from a worker thread after the track has started.
    #[no_mangle]
    pub extern "system" fn Java_com_moemusicplayer_mediaindex_DsdBridge_nativeRemoteCover<
        'local,
    >(
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
    use std::sync::PoisonError;

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
            to_converted_uri("file:///storage/emulated/0/Music/a.dsf"),
            "moedsd:///storage/emulated/0/Music/a.dsf"
        );
        assert_eq!(
            to_converted_uri("file:///x/A%20B.DSF"),
            "moedsd:///x/A%20B.DSF"
        );
        assert_eq!(to_converted_uri("file:///x/a.mp3"), "file:///x/a.mp3");
        assert_eq!(
            to_converted_uri("content://media/external/audio/media/5"),
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
            to_converted_uri("file:///storage/emulated/0/Music/a.wv"),
            "moewv:///storage/emulated/0/Music/a.wv"
        );
        assert_eq!(
            to_converted_uri("file:///x/A%20B.WV"),
            "moewv:///x/A%20B.WV"
        );
        assert_eq!(to_converted_uri("file:///x/a.wvc"), "file:///x/a.wvc");
        assert_eq!(to_converted_uri("file:///x/a.wav"), "file:///x/a.wav");
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
        assert_eq!(
            u32::from_le_bytes(header[24..28].try_into().unwrap()),
            48_000
        );
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
        assert_eq!(
            u32::from_le_bytes(header[24..28].try_into().unwrap()),
            176_400
        );
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
        assert!(open(&flac_fixture("missing.flac")).is_err());
    }

    #[test]
    fn flac_file_uris_become_flac_uris() {
        assert_eq!(
            to_converted_uri("file:///storage/emulated/0/Music/a.flac"),
            "moeflac:///storage/emulated/0/Music/a.flac"
        );
        assert_eq!(
            to_converted_uri("file:///x/A%20B.FLAC"),
            "moeflac:///x/A%20B.FLAC"
        );
        assert_eq!(
            to_converted_uri("file:///x/a.flac.lrc"),
            "file:///x/a.flac.lrc"
        );
    }

    #[test]
    fn converted_load_uris_are_recognised_by_scheme_or_remote_format() {
        let source = player_core::SourceId::new();
        let remote = |path: &str| player_core::remote_locator_uri(source, path);
        for uri in [
            "moedsd:///x/a.dsf".to_owned(),
            "moewv:///x/a.wv".to_owned(),
            "moeflac:///x/a.flac".to_owned(),
            remote("Music/a b.flac"),
            remote("Music/a.DSF"),
            remote("Music/a.wv"),
        ] {
            assert!(serves_converted_wav(&uri), "{uri}");
        }
        for uri in [
            "file:///x/a.flac".to_owned(),
            "content://media/external/audio/media/5".to_owned(),
            "moeflacx:///x/a.flac".to_owned(),
            remote("Music/a.mp3"),
            remote("Music/a.m4a"),
        ] {
            assert!(!serves_converted_wav(&uri), "{uri}");
        }
    }

    fn flac_fixture(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/player-flac/tests/fixtures")
            .join(name)
    }

    #[test]
    fn a_24_bit_flac_file_is_served_as_a_float_wav_at_its_own_rate_with_random_access() {
        let handle = open(&flac_fixture("s24_stereo_96000.flac")).unwrap();
        assert_ne!(handle, 0);
        let total = length(handle).unwrap();
        let mut header = [0_u8; 44];
        assert_eq!(read(handle, 0, &mut header).unwrap(), 44);
        assert_eq!(&header[0..4], b"RIFF");
        assert_eq!(u16::from_le_bytes(header[20..22].try_into().unwrap()), 3);
        assert_eq!(u16::from_le_bytes(header[22..24].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(header[24..28].try_into().unwrap()),
            96_000
        );
        // 0.25 s of stereo float: 24 000 frames x 8 bytes.
        assert_eq!(total, 44 + 24_000 * 8);
        let mut buffer = vec![0_u8; 4_096];
        let middle = 44 + 12_000 * 8;
        assert_eq!(read(handle, middle, &mut buffer).unwrap(), 4_096);
        assert!(buffer
            .chunks_exact(4)
            .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 0.05));
        let first = buffer.clone();
        assert_eq!(read(handle, 100, &mut buffer).unwrap(), 4_096);
        assert_eq!(read(handle, middle, &mut buffer).unwrap(), 4_096);
        assert_eq!(buffer, first, "the same range read again after going back");
        assert_eq!(read(handle, total, &mut buffer).unwrap(), 0);
        close(handle);
        assert!(length(handle).is_none());
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

    #[test]
    fn the_raw_scheme_is_a_converted_wav_and_comes_from_a_dsd_uri_only() {
        assert_eq!(
            raw_uri_of("moedsd:///storage/emulated/0/Music/a.dsf").as_deref(),
            Some("moedsdraw:///storage/emulated/0/Music/a.dsf")
        );
        assert_eq!(
            raw_uri_of("moewv:///x/A%20B.wv").as_deref(),
            Some("moewvraw:///x/A%20B.wv")
        );
        assert_eq!(
            raw_uri_of("moeremote://id/Music/a.dsf").as_deref(),
            Some("moeremoteraw://id/Music/a.dsf")
        );
        assert_eq!(raw_uri_of("moeflac:///x/a.flac"), None);
        assert_eq!(raw_uri_of("file:///x/a.dsf"), None);
        assert_eq!(raw_uri_of("moedsdraw:///x/a.dsf"), None);
        assert_eq!(raw_uri_of("moeremoteraw://id/a.dsf"), None);
        assert_eq!(
            remote_uri_of_raw("moeremoteraw://id/a.dsf").as_deref(),
            Some("moeremote://id/a.dsf")
        );
        assert_eq!(remote_uri_of_raw("moeremote://id/a.dsf"), None);
        assert_eq!(remote_uri_of_raw("moeremoterawx://id/a.dsf"), None);
        for scheme in ["moewvraw:///x/a.wv", "moeremoteraw://id/a.dsf"] {
            assert!(serves_converted_wav(scheme), "{scheme}");
        }
        assert!(!serves_converted_wav("moeremoterawx://id/a.dsf"));
        assert!(serves_converted_wav("moedsdraw:///x/a.dsf"));
        assert!(serves_converted_wav("moedsd:///x/a.dsf"));
        assert!(!serves_converted_wav("moedsdrawx:///x/a.dsf"));
        // Rust never produces the raw scheme on its own.
        assert_eq!(to_converted_uri("file:///x/a.dsf"), "moedsd:///x/a.dsf");
    }

    #[test]
    fn a_dsf_is_served_as_raw_slots_in_the_chosen_byte_order() {
        use crate::settings::DsdNativeByteOrder;
        let path = write_dsf("raw");
        let _order = ORDER_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let path_text = path.to_str().unwrap().to_owned();
        let plan: serde_json::Value = serde_json::from_str(&raw_plan_json(&path_text)).unwrap();
        assert_eq!(plan["ok"], true);
        assert_eq!(plan["rate"], 88_200);
        assert_eq!(plan["name"], "DSD64");

        set_native_byte_order(DsdNativeByteOrder::A);
        let handle = open_raw(&path_text).unwrap();
        let total = length(handle).unwrap();
        let mut header = [0_u8; 44];
        assert_eq!(read(handle, 0, &mut header).unwrap(), 44);
        assert_eq!(&header[0..4], b"RIFF");
        assert_eq!(u16::from_le_bytes(header[20..22].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(header[22..24].try_into().unwrap()), 4);
        assert_eq!(
            u32::from_le_bytes(header[24..28].try_into().unwrap()),
            88_200
        );
        assert_eq!(u16::from_le_bytes(header[34..36].try_into().unwrap()), 16);
        // 0.25 s of DSD64 = 705 600 bits per channel = 22 050 frames of 8 bytes.
        assert_eq!(total, 44 + 22_050 * 8);
        let mut a = vec![0_u8; 4_096];
        let middle = 44 + 8_000 * 8;
        assert_eq!(read(handle, middle, &mut a).unwrap(), 4_096);
        close(handle);

        set_native_byte_order(DsdNativeByteOrder::B);
        let handle = open_raw(&path_text).unwrap();
        let mut b = vec![0_u8; 4_096];
        assert_eq!(read(handle, middle, &mut b).unwrap(), 4_096);
        close(handle);
        set_native_byte_order(DsdNativeByteOrder::A);
        for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            let mut reversed = a.to_vec();
            reversed.reverse();
            assert_eq!(reversed, b);
        }
        let _ = std::fs::remove_file(path);
    }

    static ORDER_LOCK: Mutex<()> = Mutex::new(());

    /// A provider that serves local files as a remote source's, and notes which reader each open used.
    struct FileContent {
        files: HashMap<String, std::path::PathBuf>,
        opens: Mutex<Vec<(String, &'static str)>>,
    }

    impl FileContent {
        fn new(files: &[(&str, &std::path::Path)]) -> Self {
            Self {
                files: files
                    .iter()
                    .map(|(name, path)| ((*name).to_owned(), path.to_path_buf()))
                    .collect(),
                opens: Mutex::new(Vec::new()),
            }
        }

        fn note(&self, path: &str, how: &'static str) {
            self.opens.lock().unwrap().push((path.to_owned(), how));
        }
    }

    impl player_core::RemoteContent for FileContent {
        fn open(
            &self,
            _source_id: player_core::SourceId,
            path: &str,
        ) -> std::io::Result<player_core::RemoteFileHandle> {
            let file = self
                .files
                .get(path)
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
            let bytes = std::fs::read(file)?;
            Ok(player_core::RemoteFileHandle {
                len: bytes.len() as u64,
                reader: Box::new(std::io::Cursor::new(bytes)),
            })
        }

        fn open_cached(
            &self,
            source_id: player_core::SourceId,
            path: &str,
        ) -> std::io::Result<player_core::RemoteFileHandle> {
            self.note(path, "cached");
            self.open(source_id, path)
        }

        fn open_streaming(
            &self,
            source_id: player_core::SourceId,
            path: &str,
        ) -> std::io::Result<player_core::RemoteFileHandle> {
            self.note(path, "streaming");
            self.open(source_id, path)
        }
    }

    fn remote_uri(path: &str) -> String {
        player_core::remote_locator_uri(player_core::SourceId::new(), path)
    }

    fn read_all(served: &mut Served) -> Vec<u8> {
        let mut all = vec![0_u8; served.total_len() as usize];
        let mut done = 0;
        while done < all.len() {
            let count = served.read_at(done as u64, &mut all[done..]).unwrap();
            assert!(count > 0);
            done += count;
        }
        all
    }

    #[test]
    fn a_remote_dsf_is_planned_from_the_cached_header_and_served_like_the_local_one() {
        let _order = ORDER_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        set_native_byte_order(crate::settings::DsdNativeByteOrder::A);
        let path = write_dsf("remote-raw");
        let content = FileContent::new(&[("Music/a.dsf", &path), ("Music/a.mp3", &path)]);
        let uri = remote_uri("Music/a.dsf");

        let plan: serde_json::Value =
            serde_json::from_str(&raw_plan_json_with(Some(&content), &uri)).unwrap();
        assert_eq!(plan["ok"], true, "{plan}");
        assert_eq!(plan["rate"], 88_200);
        assert_eq!(plan["name"], "DSD64");
        // The plan reads through the cached reader, never the streaming one; the raw-scheme form is the same file.
        let raw_form = raw_uri_of(&uri).unwrap();
        assert!(raw_form.starts_with("moeremoteraw://"));
        let again: serde_json::Value =
            serde_json::from_str(&raw_plan_json_with(Some(&content), &raw_form)).unwrap();
        assert_eq!(again["ok"], true);
        assert!(content
            .opens
            .lock()
            .unwrap()
            .iter()
            .all(|(_, how)| *how == "cached"));

        // Playing opens the streaming reader and serves the same bytes a local file does.
        let mut remote = open_raw_with(Some(&content), &raw_form).unwrap();
        assert_eq!(
            content.opens.lock().unwrap().last().unwrap(),
            &("Music/a.dsf".to_owned(), "streaming")
        );
        let mut local = open_raw_with(None, path.to_str().unwrap()).unwrap();
        assert_eq!(remote.total_len(), 44 + 22_050 * 8);
        assert_eq!(remote.total_len(), local.total_len());
        assert_eq!(read_all(&mut remote), read_all(&mut local));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_remote_raw_stream_seeks_exactly() {
        let _order = ORDER_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        set_native_byte_order(crate::settings::DsdNativeByteOrder::A);
        let path = write_dsf("remote-seek");
        let content = FileContent::new(&[("a.dsf", &path)]);
        let mut remote = open_raw_with(Some(&content), &remote_uri("a.dsf")).unwrap();
        let mut local = open_raw_with(None, path.to_str().unwrap()).unwrap();
        for position in [
            44_u64,
            44 + 8_000 * 8,
            44 + 22_000 * 8,
            44,
            1_000,
            44 + 8 * 5_000,
        ] {
            let mut a = vec![0_u8; 2_048];
            let mut b = vec![0_u8; 2_048];
            let left = remote.read_at(position, &mut a).unwrap();
            let right = local.read_at(position, &mut b).unwrap();
            assert_eq!(left, right, "length at {position}");
            assert_eq!(a[..left], b[..right], "bytes at {position}");
        }
        let mut tail = [0_u8; 16];
        assert_eq!(remote.read_at(remote.total_len(), &mut tail).unwrap(), 0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_remote_file_that_is_not_dsd_says_so_and_a_missing_one_is_an_error() {
        let path = write_dsf("remote-other");
        let content = FileContent::new(&[("a.dsf", &path), ("a.mp3", &path)]);
        let plan_of = |name: &str| -> serde_json::Value {
            serde_json::from_str(&raw_plan_json_with(Some(&content), &remote_uri(name))).unwrap()
        };
        for name in ["a.mp3", "missing.dsf"] {
            let plan = plan_of(name);
            assert_eq!(plan["ok"], false, "{name}");
            assert!(!plan["reason"].as_str().unwrap().is_empty());
        }
        assert!(plan_of("a.mp3")["reason"].as_str().unwrap().contains("DSF"));
        // No provider installed yet.
        let none: serde_json::Value =
            serde_json::from_str(&raw_plan_json_with(None, &remote_uri("a.dsf"))).unwrap();
        assert_eq!(none["ok"], false);
        assert!(open_raw_with(Some(&content), &remote_uri("a.mp3")).is_err());
        let _ = std::fs::remove_file(path);
    }

    fn dsd_wavpack_fixture() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/player-wavpack/tests/fixtures/dsd64.wv")
    }

    /// The DSF that `dsd64.wv` was made from (see the `player-wavpack` tests), written to disk.
    fn write_dsf_of_wavpack_fixture(name: &str) -> std::path::PathBuf {
        let rate = 2_822_400_u32;
        let samples = rate as usize / 5 / 16 * 16;
        let lsb = |bytes: Vec<u8>| bytes.into_iter().map(u8::reverse_bits).collect::<Vec<u8>>();
        let left = lsb(modulate(samples, tone(rate, 1_000.0, 0.25)));
        let right = lsb(modulate(samples, tone(rate, 2_000.0, 0.25)));
        let path =
            std::env::temp_dir().join(format!("moe-android-wv-{name}-{}.dsf", std::process::id()));
        std::fs::write(
            &path,
            build_dsf(rate, samples as u64, 4096, true, &[left, right]),
        )
        .unwrap();
        path
    }

    #[test]
    fn dsd_inside_wavpack_is_served_as_the_same_raw_slots_as_its_dsf() {
        let _order = ORDER_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let wavpack = dsd_wavpack_fixture();
        let dsf = write_dsf_of_wavpack_fixture("local");
        let content = FileContent::new(&[("Music/a.wv", &wavpack), ("Music/a.dsf", &dsf)]);
        for order in [
            crate::settings::DsdNativeByteOrder::A,
            crate::settings::DsdNativeByteOrder::B,
        ] {
            set_native_byte_order(order);
            let mut expected = open_raw_with(None, dsf.to_str().unwrap()).unwrap();
            let mut local = open_raw_with(None, wavpack.to_str().unwrap()).unwrap();
            let mut remote = open_raw_with(Some(&content), &remote_uri("Music/a.wv")).unwrap();
            let expected = read_all(&mut expected);
            // 0.2 s of DSD64: 564 480 bits per channel = 17 640 frames of 8 bytes.
            assert_eq!(expected.len(), 44 + 17_640 * 8);
            assert_eq!(read_all(&mut local), expected, "local, order {order:?}");
            assert_eq!(read_all(&mut remote), expected, "remote, order {order:?}");
        }
        set_native_byte_order(crate::settings::DsdNativeByteOrder::A);

        let plan: serde_json::Value =
            serde_json::from_str(&raw_plan_json(wavpack.to_str().unwrap())).unwrap();
        assert_eq!(plan["ok"], true, "{plan}");
        assert_eq!(plan["rate"], 88_200);
        assert_eq!(plan["name"], "DSD64");
        let remote_plan: serde_json::Value = serde_json::from_str(&raw_plan_json_with(
            Some(&content),
            &remote_uri("Music/a.wv"),
        ))
        .unwrap();
        assert_eq!(remote_plan["ok"], true, "{remote_plan}");
        assert_eq!(remote_plan["rate"], 88_200);
        let _ = std::fs::remove_file(dsf);
    }

    #[test]
    fn a_wavpack_file_that_is_not_dsd_is_not_carried_natively() {
        let pcm = wavpack_reference("int24_stereo.wv");
        let plan: serde_json::Value =
            serde_json::from_str(&raw_plan_json(pcm.to_str().unwrap())).unwrap();
        assert_eq!(plan["ok"], false);
        assert!(open_raw(pcm.to_str().unwrap()).is_err());
        let content = FileContent::new(&[("a.wv", &pcm)]);
        let remote: serde_json::Value =
            serde_json::from_str(&raw_plan_json_with(Some(&content), &remote_uri("a.wv"))).unwrap();
        assert_eq!(remote["ok"], false);
        assert!(open_raw_with(Some(&content), &remote_uri("a.wv")).is_err());
    }

    #[test]
    fn a_file_that_cannot_be_carried_natively_says_why() {
        let path =
            std::env::temp_dir().join(format!("moe-android-dsd-mono-{}.dsf", std::process::id()));
        std::fs::write(
            &path,
            build_dsf(2_822_400, 800, 4096, true, &[vec![0x55; 100]]),
        )
        .unwrap();
        let plan: serde_json::Value =
            serde_json::from_str(&raw_plan_json(path.to_str().unwrap())).unwrap();
        assert_eq!(plan["ok"], false);
        assert!(plan["reason"].as_str().unwrap().contains("雙聲道"));
        assert!(open_raw(path.to_str().unwrap()).is_err());
        let missing: serde_json::Value =
            serde_json::from_str(&raw_plan_json("/definitely/not/here.dsf")).unwrap();
        assert_eq!(missing["ok"], false);
        let _ = std::fs::remove_file(path);
    }
}
