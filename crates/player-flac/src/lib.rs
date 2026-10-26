//! FLAC decoding to interleaved `f32` for the Android data source (ADR 0042).
//!
//! Some Android devices expose only a vendor FLAC `MediaCodec` that cannot decode 24-bit files, so on Android the
//! player decodes FLAC itself and hands Media3 a float WAV stream (`wav::FlacWavStream`), the way DSD and WavPack
//! are served. The decoder is Symphonia's pure-Rust FLAC decoder (MPL-2.0, used unmodified).
//!
//! Output is interleaved `f32` in ±1.0: Symphonia left-aligns every sample in an `i32`, which is divided by `2^31`,
//! so 8- to 24-bit samples are exact. Seeking is sample accurate: the demuxer finds the frame that holds the wanted
//! sample (through the SEEKTABLE when there is one, else a binary search over frame headers), and the samples of that
//! frame before the wanted one are dropped.

pub mod wav;

use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::default::codecs::FlacDecoder;
use symphonia::default::formats::FlacReader as FlacDemuxer;

/// A stream the reader decodes from: a local file or a remote stream.
pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

#[derive(Debug)]
pub enum FlacError {
    /// Reading the file failed; the message is the I/O error's own (a remote "link lost" message stays intact).
    Io(io::Error),
    /// The file is not a FLAC stream this reader can play.
    Open(String),
    /// The stream is damaged in a way the reader cannot step over.
    Decode(String),
}

impl fmt::Display for FlacError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Open(message) | Self::Decode(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for FlacError {}

impl From<io::Error> for FlacError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn decode_error(error: SymphoniaError) -> FlacError {
    match error {
        SymphoniaError::IoError(error) => FlacError::Io(error),
        other => FlacError::Decode(format!("FLAC 解碼失敗：{other}")),
    }
}

/// What STREAMINFO says about the stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlacInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: Option<u32>,
    /// `None` when STREAMINFO does not record the length.
    pub total_frames: Option<u64>,
}

/// The source as Symphonia's `MediaSource`, starting at `base` (after a leading ID3v2 tag, which the FLAC demuxer
/// does not skip itself).
struct Source {
    inner: Mutex<Box<dyn ReadSeek>>,
    base: u64,
    len: u64,
}

impl Source {
    fn inner(&mut self) -> &mut Box<dyn ReadSeek> {
        self.inner.get_mut().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Read for Source {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner().read(buf)
    }
}

impl Seek for Source {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let base = self.base;
        let absolute = match position {
            SeekFrom::Start(offset) => self.inner().seek(SeekFrom::Start(base + offset))?,
            SeekFrom::End(delta) => {
                let target = i128::from(self.len) + i128::from(delta);
                let target = u64::try_from(target).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "seek before the start")
                })?;
                self.inner().seek(SeekFrom::Start(base + target))?
            }
            SeekFrom::Current(delta) => self.inner().seek(SeekFrom::Current(delta))?,
        };
        absolute
            .checked_sub(base)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before the start"))
    }
}

impl MediaSource for Source {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

/// Length of an ID3v2 tag at the start of `source` (0 when there is none); the position afterwards is unspecified.
fn id3v2_len(source: &mut dyn ReadSeek) -> io::Result<u64> {
    let mut header = [0_u8; 10];
    let mut filled = 0;
    while filled < header.len() {
        let read = source.read(&mut header[filled..])?;
        if read == 0 {
            return Ok(0);
        }
        filled += read;
    }
    if &header[0..3] != b"ID3" || header[6..10].iter().any(|byte| byte & 0x80 != 0) {
        return Ok(0);
    }
    let size = header[6..10]
        .iter()
        .fold(0_u64, |size, byte| (size << 7) | u64::from(*byte));
    let footer = if header[5] & 0x10 != 0 { 10 } else { 0 };
    Ok(10 + size + footer)
}

/// A FLAC stream decoded to interleaved `f32`, with sample-accurate seeking.
pub struct FlacReader {
    /// `None` only after reopening the stream failed (every later call then fails too).
    demuxer: Option<Box<FlacDemuxer>>,
    decoder: FlacDecoder,
    track_id: u32,
    info: FlacInfo,
    /// Frame index of the next frame `read_frames` returns.
    position: u64,
    /// Frames still to drop from the next decoded packets (a seek lands at the start of the FLAC frame).
    skip: u64,
    ended: bool,
    /// End (frame index) of the last packet the demuxer parsed since it was opened.
    parsed_until: Option<u64>,
    samples: Option<SampleBuffer<f32>>,
}

impl FlacReader {
    pub fn open_path(path: &Path) -> Result<Self, FlacError> {
        let file = File::open(path)?;
        Self::open(BufReader::with_capacity(256 * 1024, file))
    }

    pub fn open<R: ReadSeek + 'static>(mut source: R) -> Result<Self, FlacError> {
        let end = source.seek(SeekFrom::End(0))?;
        source.seek(SeekFrom::Start(0))?;
        let base = id3v2_len(&mut source)?;
        if base >= end {
            return Err(FlacError::Open("不是 FLAC 檔案".to_owned()));
        }
        let mut source = Source {
            inner: Mutex::new(Box::new(source)),
            base,
            len: end - base,
        };
        source.seek(SeekFrom::Start(0))?;
        let stream = MediaSourceStream::new(Box::new(source), MediaSourceStreamOptions::default());
        let demuxer =
            FlacDemuxer::try_new(stream, &FormatOptions::default()).map_err(
                |error| match error {
                    SymphoniaError::IoError(error) => FlacError::Io(error),
                    other => FlacError::Open(format!("不是可播放的 FLAC 檔案：{other}")),
                },
            )?;
        let track = demuxer
            .default_track()
            .ok_or_else(|| FlacError::Open("FLAC 檔案沒有音軌".to_owned()))?;
        let params = track.codec_params.clone();
        let track_id = track.id;
        let sample_rate = params
            .sample_rate
            .filter(|rate| *rate > 0)
            .ok_or_else(|| FlacError::Open("FLAC 檔案沒有取樣率".to_owned()))?;
        let channels = params
            .channels
            .map(|channels| channels.count())
            .filter(|count| *count > 0)
            .ok_or_else(|| FlacError::Open("FLAC 檔案沒有聲道資訊".to_owned()))?;
        let decoder = FlacDecoder::try_new(&params, &DecoderOptions::default())
            .map_err(|error| FlacError::Open(format!("不是可播放的 FLAC 檔案：{error}")))?;
        Ok(Self {
            demuxer: Some(Box::new(demuxer)),
            decoder,
            track_id,
            info: FlacInfo {
                sample_rate,
                channels: channels as u16,
                bits_per_sample: params.bits_per_sample,
                total_frames: params.n_frames.filter(|frames| *frames > 0),
            },
            position: 0,
            skip: 0,
            ended: false,
            parsed_until: None,
            samples: None,
        })
    }

    pub fn info(&self) -> &FlacInfo {
        &self.info
    }

    /// Frame index of the next frame `read_frames` returns.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Continue at `frame` (clamped to the end of the stream); the next `read_frames` starts exactly there.
    pub fn seek_to_frame(&mut self, frame: u64) -> Result<(), FlacError> {
        let target = self
            .info
            .total_frames
            .map_or(frame, |total| frame.min(total));
        self.skip = 0;
        self.ended = false;
        if self.info.total_frames == Some(target) {
            self.position = target;
            self.ended = true;
            return Ok(());
        }
        // Symphonia's packet parser keeps the header of the last frame it parsed and, when a seek lands exactly on a
        // frame boundary, rejects the frames after an earlier one as out of sequence. Going back therefore starts
        // from a freshly opened demuxer.
        if self.parsed_until.is_some_and(|end| target < end) {
            self.reopen()?;
        }
        let seeked = self.seek_demuxer(target)?;
        let seeked = if seeked > target {
            // The search landed after the wanted sample (a damaged frame header): decode from the start instead. A
            // freshly opened demuxer stands at the first frame.
            self.reopen()?;
            0
        } else {
            seeked
        };
        self.decoder.reset();
        self.skip = target - seeked;
        self.position = target;
        Ok(())
    }

    fn demuxer(&mut self) -> Result<&mut FlacDemuxer, FlacError> {
        self.demuxer
            .as_deref_mut()
            .ok_or_else(|| FlacError::Decode("FLAC 串流已無法使用".to_owned()))
    }

    /// Open the demuxer again on the same source, standing at the first frame.
    fn reopen(&mut self) -> Result<(), FlacError> {
        let old = self
            .demuxer
            .take()
            .ok_or_else(|| FlacError::Decode("FLAC 串流已無法使用".to_owned()))?;
        let mut stream = old.into_inner();
        stream.seek(SeekFrom::Start(0))?;
        let demuxer =
            FlacDemuxer::try_new(stream, &FormatOptions::default()).map_err(decode_error)?;
        self.demuxer = Some(Box::new(demuxer));
        self.parsed_until = None;
        Ok(())
    }

    fn seek_demuxer(&mut self, ts: u64) -> Result<u64, FlacError> {
        let track_id = self.track_id;
        self.demuxer()?
            .seek(SeekMode::Accurate, SeekTo::TimeStamp { ts, track_id })
            .map(|seeked| seeked.actual_ts)
            .map_err(decode_error)
    }

    /// Append the next decoded frames (interleaved) to `out`; returns how many frames were added, 0 at the end.
    pub fn read_frames(&mut self, out: &mut Vec<f32>) -> Result<usize, FlacError> {
        let channels = usize::from(self.info.channels);
        loop {
            if self.ended
                || self
                    .info
                    .total_frames
                    .is_some_and(|total| self.position >= total)
            {
                self.ended = true;
                return Ok(0);
            }
            let packet = match self.demuxer()?.next_packet() {
                Ok(packet) => {
                    self.parsed_until = Some(packet.ts() + packet.dur());
                    packet
                }
                Err(SymphoniaError::IoError(error))
                    if error.kind() == io::ErrorKind::UnexpectedEof =>
                {
                    self.ended = true;
                    return Ok(0);
                }
                Err(error) => return Err(decode_error(error)),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded_frames = match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let frames = decoded.frames();
                    let wanted = decoded.capacity() * channels;
                    if self
                        .samples
                        .as_ref()
                        .is_none_or(|buffer| buffer.capacity() < wanted)
                    {
                        self.samples = Some(SampleBuffer::new(
                            decoded.capacity() as u64,
                            *decoded.spec(),
                        ));
                    }
                    if let Some(buffer) = self.samples.as_mut() {
                        buffer.copy_interleaved_ref(decoded);
                    }
                    Some(frames)
                }
                // A damaged frame plays as silence of its length, so the timeline (and the WAV length) stays right.
                Err(SymphoniaError::DecodeError(_)) => None,
                Err(error) => return Err(decode_error(error)),
            };
            let frames = decoded_frames.unwrap_or(packet.dur as usize);
            let dropped = (self.skip.min(frames as u64)) as usize;
            self.skip -= dropped as u64;
            let mut kept = frames - dropped;
            if let Some(total) = self.info.total_frames {
                kept = kept.min((total - self.position) as usize);
            }
            if kept == 0 {
                continue;
            }
            match (decoded_frames, self.samples.as_ref()) {
                (Some(_), Some(buffer)) => out.extend_from_slice(
                    &buffer.samples()[dropped * channels..(dropped + kept) * channels],
                ),
                _ => out.resize(out.len() + kept * channels, 0.0),
            }
            self.position += kept as u64;
            return Ok(kept);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn an_id3v2_tag_is_measured_with_its_footer() {
        let mut tag = b"ID3\x04\x00\x00\x00\x00\x01\x00".to_vec();
        tag.extend(std::iter::repeat_n(0, 128));
        assert_eq!(id3v2_len(&mut Cursor::new(tag.clone())).unwrap(), 138);
        tag[5] = 0x10;
        assert_eq!(id3v2_len(&mut Cursor::new(tag)).unwrap(), 148);
        assert_eq!(id3v2_len(&mut Cursor::new(b"fLaC".to_vec())).unwrap(), 0);
    }

    #[test]
    fn something_that_is_not_flac_is_refused() {
        let error = FlacReader::open(Cursor::new(vec![0_u8; 4096]))
            .err()
            .expect("refused");
        assert!(matches!(error, FlacError::Open(_)), "{error:?}");
    }
}
