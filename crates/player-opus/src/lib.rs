//! Ogg Opus decoding (ADR 0017): an Ogg page index and packet reader of our own over libopus.
//!
//! [`OpusReader`] yields interleaved `f32` at 48 kHz, with the header output gain applied, the pre-skip dropped and
//! the end trimmed to the final granule position, so its length is exact. Seeking is exact too: the page index gives
//! the position of every page, decoding resumes about 200 ms before the target and the frames before it are dropped.
//! Mono and stereo (channel mapping family 0) only.

mod pages;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

use std::fmt;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use pages::{scan_pages, PacketReader, PageEntry};

/// Opus always decodes at 48 kHz.
pub const OPUS_RATE: u32 = 48_000;
/// Longest Opus packet: 120 ms at 48 kHz.
const MAX_PACKET_FRAMES: usize = 5760;
/// Frames decoded before a seek target so the decoder state has converged. RFC 7845 suggests 80 ms; on a pure tone
/// the residual after 80 ms was still about 3% of the level (decaying with a ~30 ms time constant), so 200 ms is used.
const SEEK_PREROLL_FRAMES: u64 = 9600;

#[derive(Debug)]
pub enum OpusError {
    Io(String),
    /// Not a valid Ogg Opus stream.
    InvalidFormat(String),
    /// A valid stream this version cannot play (channel mapping).
    Unsupported(String),
    /// libopus rejected a packet.
    Decode(String),
}

impl fmt::Display for OpusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "讀取 Opus 檔案失敗：{message}"),
            Self::InvalidFormat(message) => write!(formatter, "Opus 檔案格式無效：{message}"),
            Self::Unsupported(message) => write!(formatter, "不支援的 Opus 檔案：{message}"),
            Self::Decode(message) => write!(formatter, "Opus 解碼失敗：{message}"),
        }
    }
}

impl std::error::Error for OpusError {}

impl From<std::io::Error> for OpusError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// Static description of an Opus stream.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpusInfo {
    /// 1 or 2.
    pub channels: u8,
    /// Frames (at 48 kHz) the decoder produces before the audio starts.
    pub pre_skip: u16,
    /// Sample rate of the original input, informational only (decoding is always 48 kHz).
    pub input_rate: u32,
    /// Output gain from the header, in dB.
    pub gain_db: f32,
    /// Frames of audio (48 kHz) after pre-skip, when the last page carries a granule position.
    pub total_frames: Option<u64>,
}

struct OpusHead {
    channels: u8,
    pre_skip: u16,
    input_rate: u32,
    gain_q8: i16,
}

fn parse_head(packet: &[u8]) -> Result<OpusHead, OpusError> {
    if packet.len() < 19 || &packet[..8] != b"OpusHead" {
        return Err(OpusError::InvalidFormat("缺少 OpusHead".into()));
    }
    if packet[8] >> 4 != 0 {
        return Err(OpusError::Unsupported(format!("OpusHead 版本 {}", packet[8])));
    }
    let channels = packet[9];
    let family = packet[18];
    if family != 0 {
        return Err(OpusError::Unsupported(format!("聲道對應族 {family}（多聲道）")));
    }
    if channels == 0 || channels > 2 {
        return Err(OpusError::Unsupported(format!("{channels} 聲道（只支援單聲道與雙聲道）")));
    }
    Ok(OpusHead {
        channels,
        pre_skip: u16::from_le_bytes([packet[10], packet[11]]),
        input_rate: u32::from_le_bytes(packet[12..16].try_into().expect("4 bytes")),
        gain_q8: i16::from_le_bytes([packet[16], packet[17]]),
    })
}

/// Whether the file starts with an Ogg page whose first packet is `OpusHead`.
pub fn sniff_opus_file(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    sniff_opus(file)
}

/// As [`sniff_opus_file`], for any stream (a remote file).
pub fn sniff_opus(mut stream: impl Read) -> bool {
    let mut buffer = [0_u8; 512];
    let Ok(read) = stream.read(&mut buffer) else {
        return false;
    };
    let buffer = &buffer[..read];
    if buffer.len() < 28 || &buffer[..4] != b"OggS" {
        return false;
    }
    let segments = usize::from(buffer[26]);
    let body = 27 + segments;
    buffer.len() >= body + 8 && &buffer[body..body + 8] == b"OpusHead"
}

pub struct OpusReader<R: Read + Seek + Send> {
    packets: PacketReader<R>,
    decoder: opus::Decoder,
    info: OpusInfo,
    /// First page that carries audio (the headers come before it).
    audio_start: usize,
    pages: Vec<PageEntry>,
    gain: f32,
    channels: usize,
    /// Granule position of the start of the next packet to decode.
    position: u64,
    /// Frames before this granule position are decoded but not returned.
    skip_until: u64,
    /// Frames at or after this granule position are not returned.
    end_granule: u64,
    /// Position (in output frames, pre-skip removed) after the last returned frame.
    delivered: u64,
    buffer: Vec<f32>,
}

impl<R: Read + Seek + Send> OpusReader<R> {
    pub fn open(mut reader: R) -> Result<Self, OpusError> {
        let pages = scan_pages(&mut reader)?;
        reader.seek(SeekFrom::Start(0))?;
        let mut packets = PacketReader::new(reader);
        let head_packet = packets
            .next_packet()?
            .ok_or_else(|| OpusError::InvalidFormat("檔案是空的".into()))?;
        let head = parse_head(&head_packet)?;
        let tags = packets
            .next_packet()?
            .ok_or_else(|| OpusError::InvalidFormat("缺少 OpusTags".into()))?;
        if tags.len() < 8 || &tags[..8] != b"OpusTags" {
            return Err(OpusError::InvalidFormat("缺少 OpusTags".into()));
        }
        // Audio starts on the page after the one that completed OpusTags.
        let audio_offset = packets.next_page_offset();
        let audio_start = pages
            .iter()
            .position(|page| page.offset >= audio_offset)
            .ok_or_else(|| OpusError::InvalidFormat("沒有音訊頁面".into()))?;
        let total_granule = pages
            .iter()
            .rev()
            .map(|page| page.granule)
            .find(|granule| *granule >= 0);
        let total_frames = total_granule.map(|granule| (granule as u64).saturating_sub(u64::from(head.pre_skip)));
        let channels_enum = if head.channels == 1 {
            opus::Channels::Mono
        } else {
            opus::Channels::Stereo
        };
        let decoder = opus::Decoder::new(OPUS_RATE, channels_enum).map_err(|error| OpusError::Decode(error.to_string()))?;
        let mut reader = Self {
            packets,
            decoder,
            info: OpusInfo {
                channels: head.channels,
                pre_skip: head.pre_skip,
                input_rate: head.input_rate,
                gain_db: f32::from(head.gain_q8) / 256.0,
                total_frames,
            },
            audio_start,
            pages,
            gain: 10_f32.powf(f32::from(head.gain_q8) / 256.0 / 20.0),
            channels: usize::from(head.channels),
            position: 0,
            skip_until: u64::from(head.pre_skip),
            end_granule: total_granule.map_or(u64::MAX, |granule| granule as u64),
            delivered: 0,
            buffer: vec![0.0; MAX_PACKET_FRAMES * usize::from(head.channels)],
        };
        reader.seek_to_page_index(reader.audio_start, 0)?;
        Ok(reader)
    }

    pub fn info(&self) -> OpusInfo {
        self.info
    }

    /// Output frames returned so far (the position of the next frame).
    pub fn position_frames(&self) -> u64 {
        self.delivered
    }

    fn seek_to_page_index(&mut self, index: usize, start_position: u64) -> Result<(), OpusError> {
        self.packets.seek_to_page(self.pages[index].offset)?;
        self.decoder
            .reset_state()
            .map_err(|error| OpusError::Decode(error.to_string()))?;
        self.position = start_position;
        Ok(())
    }

    /// Decode packets until some audio is available and append it (interleaved, gain applied) to `out`. Returns the
    /// number of frames appended; 0 is the end of the stream.
    pub fn read_frames(&mut self, out: &mut Vec<f32>) -> Result<usize, OpusError> {
        loop {
            if self.position >= self.end_granule {
                return Ok(0);
            }
            let Some(packet) = self.packets.next_packet()? else {
                return Ok(0);
            };
            let frames = match self.decoder.decode_float(&packet, &mut self.buffer, false) {
                Ok(frames) => frames as u64,
                Err(error) => return Err(OpusError::Decode(error.to_string())),
            };
            let start = self.position;
            let end = start + frames;
            self.position = end;
            let from = start.max(self.skip_until);
            let to = end.min(self.end_granule);
            if from >= to {
                continue;
            }
            let first = (from - start) as usize * self.channels;
            let last = (to - start) as usize * self.channels;
            out.extend(self.buffer[first..last].iter().map(|sample| sample * self.gain));
            self.delivered = to - u64::from(self.info.pre_skip);
            return Ok((to - from) as usize);
        }
    }

    /// Position the next returned frame at output frame `frame` (clamped to the end).
    pub fn seek_to_frame(&mut self, frame: u64) -> Result<(), OpusError> {
        let pre_skip = u64::from(self.info.pre_skip);
        let target = self.info.total_frames.map_or(frame, |total| frame.min(total));
        let target_granule = pre_skip + target;
        let wanted_start = target_granule.saturating_sub(SEEK_PREROLL_FRAMES);
        // The last page that is not a continuation and whose start (the previous page's granule) is early enough.
        let mut chosen = (self.audio_start, 0_u64);
        for index in self.audio_start + 1..self.pages.len() {
            let page = self.pages[index];
            let previous = self.pages[index - 1].granule;
            if page.continued || previous < 0 {
                continue;
            }
            if previous as u64 > wanted_start {
                break;
            }
            chosen = (index, previous as u64);
        }
        self.seek_to_page_index(chosen.0, chosen.1)?;
        self.skip_until = target_granule;
        self.delivered = target;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;
    use std::io::Cursor;

    use ogg::writing::{PacketWriteEndInfo, PacketWriter};

    use super::*;

    use crate::testing::write_opus;

    const SERIAL: u32 = 0x4d4f_4531;

    fn read_all(reader: &mut OpusReader<Cursor<Vec<u8>>>) -> Vec<f32> {
        let mut all = Vec::new();
        while reader.read_frames(&mut all).unwrap() > 0 {}
        all
    }

    /// Amplitude of `frequency` in `channel` of `samples` (frames starting at frame `first`), by least squares.
    fn tone_amplitude(samples: &[f32], channels: usize, channel: usize, first: usize, frequency: f64) -> f64 {
        let omega = 2.0 * PI * frequency / f64::from(OPUS_RATE);
        let (mut cc, mut ss, mut cs, mut yc, mut ys) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (index, frame) in samples.chunks(channels).enumerate() {
            let (sin, cos) = (omega * (first + index) as f64).sin_cos();
            let value = f64::from(frame[channel]);
            cc += cos * cos;
            ss += sin * sin;
            cs += cos * sin;
            yc += value * cos;
            ys += value * sin;
        }
        let determinant = cc * ss - cs * cs;
        let a = (yc * ss - ys * cs) / determinant;
        let b = (ys * cc - yc * cs) / determinant;
        a.hypot(b)
    }

    #[test]
    fn info_and_exact_length() {
        let file = write_opus(48_000, 2, 1_000.0, 0, 0);
        let mut reader = OpusReader::open(Cursor::new(file)).unwrap();
        let info = reader.info();
        assert_eq!(info.channels, 2);
        assert_eq!(info.total_frames, Some(48_000));
        assert!(info.pre_skip > 0);
        let all = read_all(&mut reader);
        assert_eq!(all.len(), 48_000 * 2, "pre-skip dropped and end trimmed to the final granule");
        assert_eq!(reader.position_frames(), 48_000);
    }

    #[test]
    fn decoded_tones_have_the_encoded_level_in_both_channels() {
        let file = write_opus(48_000, 2, 1_000.0, 0, 0);
        let all = read_all(&mut OpusReader::open(Cursor::new(file)).unwrap());
        let window = &all[4_800 * 2..(48_000 - 4_800) * 2];
        let left = tone_amplitude(window, 2, 0, 4_800, 1_000.0);
        let right = tone_amplitude(window, 2, 1, 4_800, 2_000.0);
        for (name, level) in [("left", left), ("right", right)] {
            let error_db = 20.0 * (level / 0.4).log10();
            assert!(error_db.abs() < 0.3, "{name}: {error_db} dB");
        }
    }

    #[test]
    fn mono_streams_decode_to_one_channel() {
        let file = write_opus(24_000, 1, 440.0, 0, 0);
        let mut reader = OpusReader::open(Cursor::new(file)).unwrap();
        assert_eq!(reader.info().channels, 1);
        assert_eq!(read_all(&mut reader).len(), 24_000);
    }

    #[test]
    fn header_output_gain_is_applied() {
        // +6.0 dB in Q7.8.
        let file = write_opus(24_000, 2, 1_000.0, 6 * 256, 0);
        let mut reader = OpusReader::open(Cursor::new(file.clone())).unwrap();
        assert!((reader.info().gain_db - 6.0).abs() < 1e-6);
        let loud = read_all(&mut reader);
        let plain = read_all(&mut OpusReader::open(Cursor::new(write_opus(24_000, 2, 1_000.0, 0, 0))).unwrap());
        let ratio = tone_amplitude(&loud[4_800..20_000 * 2], 2, 0, 2_400, 1_000.0)
            / tone_amplitude(&plain[4_800..20_000 * 2], 2, 0, 2_400, 1_000.0);
        assert!((20.0 * ratio.log10() - 6.0).abs() < 0.1, "{}", 20.0 * ratio.log10());
    }

    #[test]
    fn seeking_matches_sequential_decoding_after_the_preroll() {
        let file = write_opus(96_000, 2, 1_000.0, 0, 0);
        let sequential = read_all(&mut OpusReader::open(Cursor::new(file.clone())).unwrap());
        let mut reader = OpusReader::open(Cursor::new(file)).unwrap();
        for &frame in &[0_u64, 1, 960, 5_000, 20_001, 47_999, 70_000, 95_000] {
            reader.seek_to_frame(frame).unwrap();
            assert_eq!(reader.position_frames(), frame);
            let mut got = Vec::new();
            while got.len() < 4_000 * 2 && reader.read_frames(&mut got).unwrap() > 0 {}
            let want = &sequential[frame as usize * 2..];
            let count = got.len().min(4_000 * 2).min(want.len());
            assert!(count > 0 || frame >= 95_000, "frame {frame}: nothing returned");
            let worst = got[..count]
                .iter()
                .zip(&want[..count])
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f32, f32::max);
            assert!(worst < 0.002, "frame {frame}: worst difference {worst}");
        }
    }

    #[test]
    #[ignore = "prints seek convergence"]
    fn debug_seek_convergence() {
        let file = write_opus(96_000, 2, 1_000.0, 0, 0);
        let sequential = read_all(&mut OpusReader::open(Cursor::new(file.clone())).unwrap());
        let mut reader = OpusReader::open(Cursor::new(file)).unwrap();
        for &frame in &[20_001_u64, 47_999, 70_000] {
            reader.seek_to_frame(frame).unwrap();
            let mut got = Vec::new();
            while got.len() < 6_000 * 2 && reader.read_frames(&mut got).unwrap() > 0 {}
            let want = &sequential[frame as usize * 2..];
            let chunks: Vec<String> = got
                .chunks(480 * 2)
                .zip(want.chunks(480 * 2))
                .take(12)
                .map(|(a, b)| format!("{:.4}", a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0_f32, f32::max)))
                .collect();
            println!("frame {frame}: per-10ms worst diff {chunks:?}");
        }
    }

    #[test]
    fn seeking_past_the_end_returns_nothing_and_seeking_back_works() {
        let file = write_opus(24_000, 2, 1_000.0, 0, 0);
        let mut reader = OpusReader::open(Cursor::new(file)).unwrap();
        reader.seek_to_frame(1_000_000).unwrap();
        let mut out = Vec::new();
        assert_eq!(reader.read_frames(&mut out).unwrap(), 0);
        reader.seek_to_frame(0).unwrap();
        assert!(reader.read_frames(&mut out).unwrap() > 0);
    }

    #[test]
    fn a_tags_packet_spanning_many_pages_is_handled() {
        // 300 KB of padding in OpusTags: the packet continues across a dozen pages, and the audio still starts at
        // the right page for decoding and for seeking.
        let file = write_opus(48_000, 2, 1_000.0, 0, 300_000);
        let sequential = read_all(&mut OpusReader::open(Cursor::new(file.clone())).unwrap());
        assert_eq!(sequential.len(), 48_000 * 2);
        let mut reader = OpusReader::open(Cursor::new(file)).unwrap();
        reader.seek_to_frame(30_000).unwrap();
        let mut got = Vec::new();
        reader.read_frames(&mut got).unwrap();
        assert!((got[0] - sequential[30_000 * 2]).abs() < 0.01);
    }

    #[test]
    fn invalid_and_unsupported_streams_are_refused() {
        assert!(matches!(
            OpusReader::open(Cursor::new(b"not ogg at all".to_vec())),
            Err(OpusError::InvalidFormat(_)) | Err(OpusError::Io(_))
        ));
        // A Vorbis-style first packet.
        let mut bytes = Vec::new();
        {
            let mut writer = PacketWriter::new(Cursor::new(&mut bytes));
            writer.write_packet(b"\x01vorbis....".to_vec(), SERIAL, PacketWriteEndInfo::EndStream, 0).unwrap();
        }
        assert!(matches!(
            OpusReader::open(Cursor::new(bytes)),
            Err(OpusError::InvalidFormat(_))
        ));
        // Channel mapping family 1 (surround).
        let mut bytes = Vec::new();
        {
            let mut writer = PacketWriter::new(Cursor::new(&mut bytes));
            let mut head = b"OpusHead".to_vec();
            head.extend_from_slice(&[1, 6, 0, 0]);
            head.extend_from_slice(&OPUS_RATE.to_le_bytes());
            head.extend_from_slice(&[0, 0, 1]);
            writer.write_packet(head, SERIAL, PacketWriteEndInfo::EndStream, 0).unwrap();
        }
        assert!(matches!(
            OpusReader::open(Cursor::new(bytes)),
            Err(OpusError::Unsupported(_))
        ));
    }

    #[test]
    fn sniffing_recognises_opus_in_ogg_files() {
        let directory = std::env::temp_dir().join(format!("moe-opus-sniff-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let opus = directory.join("a.ogg");
        std::fs::write(&opus, write_opus(4_800, 2, 1_000.0, 0, 0)).unwrap();
        assert!(sniff_opus_file(&opus));
        let vorbis = directory.join("b.ogg");
        let mut bytes = Vec::new();
        {
            let mut writer = PacketWriter::new(Cursor::new(&mut bytes));
            writer.write_packet(b"\x01vorbis....".to_vec(), SERIAL, PacketWriteEndInfo::EndStream, 0).unwrap();
        }
        std::fs::write(&vorbis, bytes).unwrap();
        assert!(!sniff_opus_file(&vorbis));
        assert!(!sniff_opus_file(&directory.join("missing.ogg")));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn the_page_index_records_continuation_pages() {
        let file = write_opus(24_000, 2, 1_000.0, 0, 300_000);
        let pages = scan_pages(Cursor::new(file)).unwrap();
        assert!(pages.iter().filter(|page| page.continued).count() >= 3);
        assert!(pages.last().unwrap().granule > 0);
        // Offsets are strictly increasing.
        assert!(pages.windows(2).all(|pair| pair[0].offset < pair[1].offset));
    }

}
