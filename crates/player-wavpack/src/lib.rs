//! WavPack decoding (ADR 0025): a safe reader over the decoder half of libwavpack 5.6.0 (vendored, BSD-3-Clause).
//!
//! The library reads through callbacks into a Rust `Read + Seek`, so Windows paths of any kind work and the same
//! reader serves playback, the spectrogram and the Android stream bridge. Output is interleaved `f32` in ±1.0:
//! integer samples are divided by `2^(bits - 1)` (exact up to 24 bits), floating point files are normalized by the
//! library, and DSD-in-WavPack files are decimated by the library to 24-bit PCM (rate ÷ 8).

pub mod dsd;
pub mod wav;

use std::ffi::{c_char, c_int, c_void};
use std::fmt;
use std::io::{Read, Seek, SeekFrom};

mod ffi {
    use super::*;

    #[repr(C)]
    pub struct WavpackContext {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct StreamReader64 {
        pub read_bytes: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> i32,
        pub write_bytes: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> i32,
        pub get_pos: unsafe extern "C" fn(*mut c_void) -> i64,
        pub set_pos_abs: unsafe extern "C" fn(*mut c_void, i64) -> c_int,
        pub set_pos_rel: unsafe extern "C" fn(*mut c_void, i64, c_int) -> c_int,
        pub push_back_byte: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
        pub get_length: unsafe extern "C" fn(*mut c_void) -> i64,
        pub can_seek: unsafe extern "C" fn(*mut c_void) -> c_int,
        pub truncate_here: unsafe extern "C" fn(*mut c_void) -> c_int,
        pub close: unsafe extern "C" fn(*mut c_void) -> c_int,
    }

    pub const OPEN_WVC: c_int = 0x1;
    pub const OPEN_NORMALIZE: c_int = 0x10;
    pub const OPEN_DSD_NATIVE: c_int = 0x100;
    pub const OPEN_DSD_AS_PCM: c_int = 0x200;
    pub const OPEN_ALT_TYPES: c_int = 0x400;

    pub const MODE_WVC: c_int = 0x1;
    pub const MODE_LOSSLESS: c_int = 0x2;
    pub const MODE_HYBRID: c_int = 0x4;
    pub const MODE_FLOAT: c_int = 0x8;
    pub const QMODE_DSD_AUDIO: c_int = 0x10 | 0x20;

    extern "C" {
        pub fn WavpackOpenFileInputEx64(
            reader: *mut StreamReader64,
            wv_id: *mut c_void,
            wvc_id: *mut c_void,
            error: *mut c_char,
            flags: c_int,
            norm_offset: c_int,
        ) -> *mut WavpackContext;
        pub fn WavpackCloseFile(wpc: *mut WavpackContext) -> *mut WavpackContext;
        pub fn WavpackGetMode(wpc: *mut WavpackContext) -> c_int;
        pub fn WavpackGetQualifyMode(wpc: *mut WavpackContext) -> c_int;
        pub fn WavpackGetNativeSampleRate(wpc: *mut WavpackContext) -> u32;
        pub fn WavpackGetVersion(wpc: *mut WavpackContext) -> c_int;
        pub fn WavpackUnpackSamples(
            wpc: *mut WavpackContext,
            buffer: *mut i32,
            samples: u32,
        ) -> u32;
        pub fn WavpackGetNumSamples64(wpc: *mut WavpackContext) -> i64;
        pub fn WavpackGetNumErrors(wpc: *mut WavpackContext) -> c_int;
        pub fn WavpackSeekSample64(wpc: *mut WavpackContext, sample: i64) -> c_int;
        pub fn WavpackGetSampleRate(wpc: *mut WavpackContext) -> u32;
        pub fn WavpackGetBitsPerSample(wpc: *mut WavpackContext) -> c_int;
        pub fn WavpackGetNumChannels(wpc: *mut WavpackContext) -> c_int;
        pub fn WavpackGetAverageBitrate(wpc: *mut WavpackContext, count_wvc: c_int) -> f64;
    }
}

/// A stream the library reads from: the `.wv` file or its `.wvc` correction file.
pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

#[derive(Debug)]
pub enum WavPackError {
    /// The library could not open the stream (the text is its own message).
    Open(String),
    /// Decoding failed (damaged blocks).
    Decode(String),
    Seek(u64),
}

impl fmt::Display for WavPackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(message) => write!(f, "無法開啟 WavPack：{message}"),
            Self::Decode(message) => write!(f, "WavPack 解碼失敗：{message}"),
            Self::Seek(frame) => write!(f, "WavPack 無法跳到第 {frame} 個樣本"),
        }
    }
}

impl std::error::Error for WavPackError {}

/// What the file says about itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WavPackInfo {
    pub sample_rate: u32,
    pub channels: u16,
    /// Bits of the encoded samples (DSD files report the 24 bits of the decimated PCM).
    pub bits_per_sample: u32,
    /// Frames per channel; `None` when the file does not say (a streamed, truncated file).
    pub total_frames: Option<u64>,
    pub float: bool,
    /// The file holds DSD (1-bit) audio; the reader hands out the library's 24-bit PCM decimation of it.
    pub dsd: bool,
    /// The sample rate of the source: the 1-bit rate (for example 2 822 400 Hz) of a DSD file, else `sample_rate`.
    pub native_sample_rate: u32,
    /// Lossless (also true for a hybrid file played with its correction file).
    pub lossless: bool,
    pub hybrid: bool,
    /// The `.wvc` correction file is in use.
    pub correction_file: bool,
    pub version: u16,
    /// Average bitrate of the main file in bits per second (0 when unknown).
    pub average_bitrate: u32,
}

/// One open stream plus the single byte the library may push back.
struct StreamState {
    stream: Box<dyn ReadSeek>,
    length: i64,
}

unsafe extern "C" fn read_bytes(id: *mut c_void, data: *mut c_void, count: i32) -> i32 {
    let state = unsafe { &mut *(id as *mut StreamState) };
    let buffer = unsafe { std::slice::from_raw_parts_mut(data as *mut u8, count.max(0) as usize) };
    let mut total = 0;
    while total < buffer.len() {
        match state.stream.read(&mut buffer[total..]) {
            Ok(0) => break,
            Ok(read) => total += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    total as i32
}

unsafe extern "C" fn write_bytes(_: *mut c_void, _: *mut c_void, _: i32) -> i32 {
    0
}

unsafe extern "C" fn get_pos(id: *mut c_void) -> i64 {
    let state = unsafe { &mut *(id as *mut StreamState) };
    state
        .stream
        .stream_position()
        .map(|p| p as i64)
        .unwrap_or(-1)
}

unsafe extern "C" fn set_pos_abs(id: *mut c_void, pos: i64) -> c_int {
    let state = unsafe { &mut *(id as *mut StreamState) };
    match u64::try_from(pos) {
        Ok(pos) if state.stream.seek(SeekFrom::Start(pos)).is_ok() => 0,
        _ => -1,
    }
}

unsafe extern "C" fn set_pos_rel(id: *mut c_void, delta: i64, mode: c_int) -> c_int {
    let state = unsafe { &mut *(id as *mut StreamState) };
    let target = match mode {
        0 => match u64::try_from(delta) {
            Ok(pos) => SeekFrom::Start(pos),
            Err(_) => return -1,
        },
        1 => SeekFrom::Current(delta),
        2 => SeekFrom::End(delta),
        _ => return -1,
    };
    if state.stream.seek(target).is_ok() {
        0
    } else {
        -1
    }
}

/// The library pushes back the byte it has just read, so stepping back one byte restores it.
unsafe extern "C" fn push_back_byte(id: *mut c_void, byte: c_int) -> c_int {
    let state = unsafe { &mut *(id as *mut StreamState) };
    if state.stream.seek(SeekFrom::Current(-1)).is_ok() {
        byte
    } else {
        -1
    }
}

unsafe extern "C" fn get_length(id: *mut c_void) -> i64 {
    unsafe { &*(id as *mut StreamState) }.length
}

unsafe extern "C" fn can_seek(_: *mut c_void) -> c_int {
    1
}

unsafe extern "C" fn truncate_here(_: *mut c_void) -> c_int {
    -1
}

unsafe extern "C" fn close(_: *mut c_void) -> c_int {
    0
}

fn stream_state(mut stream: Box<dyn ReadSeek>) -> Result<Box<StreamState>, WavPackError> {
    let length = stream
        .seek(SeekFrom::End(0))
        .and_then(|length| stream.seek(SeekFrom::Start(0)).map(|_| length))
        .map_err(|error| WavPackError::Open(error.to_string()))?;
    Ok(Box::new(StreamState {
        stream,
        length: length as i64,
    }))
}

/// The library keeps a pointer to this table for as long as a file is open.
static READER: ffi::StreamReader64 = ffi::StreamReader64 {
    read_bytes,
    write_bytes,
    get_pos,
    set_pos_abs,
    set_pos_rel,
    push_back_byte,
    get_length,
    can_seek,
    truncate_here,
    close,
};

const CHUNK_FRAMES: usize = 4096;

pub struct WavPackReader {
    context: *mut ffi::WavpackContext,
    /// Kept alive for the callbacks; the library holds raw pointers into these boxes.
    _main: Box<StreamState>,
    _correction: Option<Box<StreamState>>,
    info: WavPackInfo,
    scale: f64,
    int_samples: Vec<i32>,
    position: u64,
    /// A seek to (or past) the last frame: the library refuses it, so reads simply report the end.
    at_end: bool,
}

// The context and the boxed streams are used from one thread at a time (the reader is moved, never shared).
unsafe impl Send for WavPackReader {}

impl WavPackReader {
    /// Open a `.wv` file on disk, with the `.wvc` correction file next to it when there is one (the pair is lossless).
    pub fn open_path(path: &std::path::Path) -> Result<Self, WavPackError> {
        Self::open_path_with(path, false)
    }

    fn open_path_with(path: &std::path::Path, native_dsd: bool) -> Result<Self, WavPackError> {
        let file =
            std::fs::File::open(path).map_err(|error| WavPackError::Open(error.to_string()))?;
        let mut correction_name = path.as_os_str().to_owned();
        correction_name.push("c");
        let correction = std::fs::File::open(std::path::PathBuf::from(correction_name))
            .ok()
            .map(|file| Box::new(std::io::BufReader::new(file)) as Box<dyn ReadSeek>);
        Self::open_with(
            std::io::BufReader::with_capacity(256 * 1024, file),
            correction,
            native_dsd,
        )
    }

    /// Open a `.wv` stream, with its `.wvc` correction stream if there is one.
    pub fn open(
        main: impl ReadSeek + 'static,
        correction: Option<Box<dyn ReadSeek>>,
    ) -> Result<Self, WavPackError> {
        Self::open_with(main, correction, false)
    }

    /// As [`open`](Self::open); with `native_dsd` a DSD file is read as its 1-bit stream (one byte per channel per
    /// "sample", see [`dsd::WavPackDsdStream`]) instead of the library's 24-bit PCM decimation.
    fn open_with(
        main: impl ReadSeek + 'static,
        correction: Option<Box<dyn ReadSeek>>,
        native_dsd: bool,
    ) -> Result<Self, WavPackError> {
        let mut main = stream_state(Box::new(main))?;
        let mut correction = match correction {
            Some(stream) => Some(stream_state(stream)?),
            None => None,
        };
        let dsd_flag = if native_dsd {
            ffi::OPEN_DSD_NATIVE
        } else {
            ffi::OPEN_DSD_AS_PCM
        };
        let mut flags = ffi::OPEN_NORMALIZE | dsd_flag | ffi::OPEN_ALT_TYPES;
        if correction.is_some() {
            flags |= ffi::OPEN_WVC;
        }
        let mut error = [0 as c_char; 80];
        let context = unsafe {
            ffi::WavpackOpenFileInputEx64(
                &READER as *const ffi::StreamReader64 as *mut ffi::StreamReader64,
                (&mut *main) as *mut StreamState as *mut c_void,
                correction.as_mut().map_or(std::ptr::null_mut(), |c| {
                    (&mut **c) as *mut StreamState as *mut c_void
                }),
                error.as_mut_ptr(),
                flags,
                0,
            )
        };
        if context.is_null() {
            let message = error
                .iter()
                .take_while(|byte| **byte != 0)
                .map(|byte| *byte as u8 as char)
                .collect::<String>();
            return Err(WavPackError::Open(message));
        }
        let (mode, qualify, native_rate, channels, sample_rate, bits, frames, version, bitrate) = unsafe {
            (
                ffi::WavpackGetMode(context),
                ffi::WavpackGetQualifyMode(context),
                ffi::WavpackGetNativeSampleRate(context),
                ffi::WavpackGetNumChannels(context),
                ffi::WavpackGetSampleRate(context),
                ffi::WavpackGetBitsPerSample(context),
                ffi::WavpackGetNumSamples64(context),
                ffi::WavpackGetVersion(context),
                ffi::WavpackGetAverageBitrate(context, 0),
            )
        };
        let float = mode & ffi::MODE_FLOAT != 0;
        let info = WavPackInfo {
            sample_rate,
            channels: u16::try_from(channels).unwrap_or(0),
            bits_per_sample: u32::try_from(bits).unwrap_or(0),
            total_frames: u64::try_from(frames).ok(),
            float,
            dsd: qualify & ffi::QMODE_DSD_AUDIO != 0,
            native_sample_rate: if native_rate > 0 {
                native_rate
            } else {
                sample_rate
            },
            lossless: mode & ffi::MODE_LOSSLESS != 0,
            hybrid: mode & ffi::MODE_HYBRID != 0,
            correction_file: mode & ffi::MODE_WVC != 0,
            version: u16::try_from(version).unwrap_or(0),
            average_bitrate: bitrate.max(0.0) as u32,
        };
        if info.channels == 0 || info.sample_rate == 0 || (!float && !(1..=32).contains(&bits)) {
            unsafe { ffi::WavpackCloseFile(context) };
            return Err(WavPackError::Open(
                "不支援的聲道數、取樣率或位元深度".to_owned(),
            ));
        }
        Ok(Self {
            context,
            _main: main,
            _correction: correction,
            info,
            scale: if float {
                1.0
            } else {
                1.0 / (2.0f64).powi(bits - 1)
            },
            int_samples: Vec::new(),
            position: 0,
            at_end: false,
        })
    }

    pub fn info(&self) -> WavPackInfo {
        self.info
    }

    /// Frame (per-channel sample) the next read starts at.
    pub fn position_frames(&self) -> u64 {
        self.position
    }

    /// Append up to a chunk of interleaved frames to `out`; returns the frames added (0 at the end of the file).
    pub fn read_frames(&mut self, out: &mut Vec<f32>) -> Result<usize, WavPackError> {
        if self.at_end {
            return Ok(0);
        }
        let channels = usize::from(self.info.channels);
        self.int_samples.resize(CHUNK_FRAMES * channels, 0);
        let frames = unsafe {
            ffi::WavpackUnpackSamples(
                self.context,
                self.int_samples.as_mut_ptr(),
                CHUNK_FRAMES as u32,
            )
        } as usize;
        if frames == 0 {
            let errors = unsafe { ffi::WavpackGetNumErrors(self.context) };
            return if errors > 0 {
                Err(WavPackError::Decode(format!("{errors} 個損毀的區塊")))
            } else {
                Ok(0)
            };
        }
        let samples = &self.int_samples[..frames * channels];
        if self.info.float {
            out.extend(samples.iter().map(|&value| f32::from_bits(value as u32)));
        } else {
            out.extend(
                samples
                    .iter()
                    .map(|&value| (f64::from(value) * self.scale) as f32),
            );
        }
        self.position += frames as u64;
        Ok(frames)
    }

    pub fn seek_to_frame(&mut self, frame: u64) -> Result<(), WavPackError> {
        let target = match self.info.total_frames {
            Some(total) => frame.min(total),
            None => frame,
        };
        if self.info.total_frames == Some(target) {
            self.at_end = true;
            self.position = target;
            return Ok(());
        }
        let status = unsafe { ffi::WavpackSeekSample64(self.context, target as i64) };
        if status == 0 {
            return Err(WavPackError::Seek(frame));
        }
        self.at_end = false;
        self.position = target;
        Ok(())
    }
}

impl Drop for WavPackReader {
    fn drop(&mut self) {
        unsafe { ffi::WavpackCloseFile(self.context) };
    }
}

/// What the library list shows of a WavPack file (the codec labels are short on purpose: `WVLL` lossless, `WVL`
/// lossy, `WVDSD` DSD).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavPackSummary {
    pub codec: &'static str,
    /// The source sample rate (the 1-bit rate of a DSD file).
    pub sample_rate_hz: u32,
    /// The bit depth of the encoded samples (32 for floating point, 1 for DSD), only for lossless files.
    pub bit_depth: Option<u8>,
}

/// The codec label, source sample rate and bit depth of a WavPack file on disk (a `.wvc` next to it is used, which
/// makes a hybrid file lossless). A hybrid file without its correction file is lossy and has no meaningful depth.
pub fn summarize(path: &std::path::Path) -> Option<WavPackSummary> {
    let info = WavPackReader::open_path(path).ok()?.info();
    if info.dsd {
        return Some(WavPackSummary {
            codec: "WVDSD",
            sample_rate_hz: info.native_sample_rate,
            bit_depth: Some(1),
        });
    }
    let depth = if info.float { 32 } else { info.bits_per_sample };
    let depth = u8::try_from(depth)
        .ok()
        .filter(|depth| info.lossless && (8..=32).contains(depth));
    Some(WavPackSummary {
        codec: if info.lossless { "WVLL" } else { "WVL" },
        sample_rate_hz: info.sample_rate,
        bit_depth: depth,
    })
}

/// Whether the WavPack file on disk holds DSD audio.
pub fn is_dsd(path: &std::path::Path) -> bool {
    WavPackReader::open_path(path).is_ok_and(|reader| reader.info().dsd)
}

/// Whether `bytes` (the start of a file) is a WavPack block: `wvpk`.
pub fn is_wavpack_header(bytes: &[u8]) -> bool {
    bytes.starts_with(b"wvpk")
}

#[cfg(test)]
mod tests;
