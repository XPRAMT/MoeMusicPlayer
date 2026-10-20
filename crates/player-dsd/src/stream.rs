//! Container-neutral DSD bitstream (ADR 0016).
//!
//! A [`DsdStream`] hands out raw DSD as planar bytes, one buffer per channel, **earliest bit in the most
//! significant bit of each byte**, whatever the container stores. Every renderer (the PCM converter now, DoP later)
//! relies on this one convention.

use std::fmt;
use std::time::Duration;

/// DSD "silence": an alternating pattern with no DC offset (`01101001`). Padding with zero bytes would be a
/// full-scale negative step.
pub const DSD_IDLE_BYTE: u8 = 0x69;

/// 44.1 kHz, the base of the DSD rate family (DSD64 = 64 × 44.1 kHz).
pub const DSD_BASE_RATE: u32 = 44_100;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DsdError {
    Io(String),
    /// The file is not a valid DSD container (bad magic, impossible sizes, ...).
    InvalidFormat(String),
    /// A valid file that this version cannot play (channel layout, rate family, ...).
    Unsupported(String),
}

impl fmt::Display for DsdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "讀取 DSD 檔案失敗：{message}"),
            Self::InvalidFormat(message) => write!(formatter, "DSD 檔案格式無效：{message}"),
            Self::Unsupported(message) => write!(formatter, "不支援的 DSD 檔案：{message}"),
        }
    }
}

impl std::error::Error for DsdError {}

impl From<std::io::Error> for DsdError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// Static description of a DSD stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DsdInfo {
    pub channels: u16,
    /// 1-bit sample rate in Hz (2 822 400 for DSD64).
    pub sample_rate: u32,
    /// 1-bit samples per channel (the last byte may hold fewer than 8 valid bits).
    pub samples_per_channel: u64,
}

impl DsdInfo {
    /// Bytes per channel (the last one possibly partial).
    pub fn bytes_per_channel(&self) -> u64 {
        self.samples_per_channel.div_ceil(8)
    }

    pub fn duration(&self) -> Duration {
        if self.sample_rate == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(self.samples_per_channel as f64 / f64::from(self.sample_rate))
    }

    /// The "DSD64/128/256/512" multiple of 44.1 kHz, when the rate is exactly such a multiple.
    pub fn rate_multiple(&self) -> Option<u32> {
        self.sample_rate
            .is_multiple_of(DSD_BASE_RATE)
            .then_some(self.sample_rate / DSD_BASE_RATE)
    }

    /// Display name such as "DSD64".
    pub fn rate_name(&self) -> String {
        match self.rate_multiple() {
            Some(multiple) => format!("DSD{multiple}"),
            None => format!("DSD {:.4} MHz", f64::from(self.sample_rate) / 1_000_000.0),
        }
    }
}

/// A boxed stream is a stream, so a converter can play any container behind one type.
impl DsdStream for Box<dyn DsdStream> {
    fn info(&self) -> DsdInfo {
        (**self).info()
    }

    fn read(&mut self, max_bytes: usize, out: &mut [Vec<u8>]) -> Result<usize, DsdError> {
        (**self).read(max_bytes, out)
    }

    fn seek_to_byte(&mut self, byte_index: u64) -> Result<(), DsdError> {
        (**self).seek_to_byte(byte_index)
    }

    fn position_bytes(&self) -> u64 {
        (**self).position_bytes()
    }
}

/// A seekable source of raw DSD.
pub trait DsdStream: Send {
    fn info(&self) -> DsdInfo;

    /// Clear every buffer in `out` (one per channel) and refill each with the same number of bytes, at most
    /// `max_bytes`. Returns that count; 0 means the end of the stream.
    fn read(&mut self, max_bytes: usize, out: &mut [Vec<u8>]) -> Result<usize, DsdError>;

    /// Position the next `read` at this per-channel byte index (clamped to the end).
    fn seek_to_byte(&mut self, byte_index: u64) -> Result<(), DsdError>;

    /// The per-channel byte index the next `read` starts at.
    fn position_bytes(&self) -> u64;
}
