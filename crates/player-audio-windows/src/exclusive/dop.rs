//! The DoP source of the exclusive backend (ADR 0020 P2): a DSF file or DSD inside WavPack as a stream of DoP words, carried as `f32` through
//! the same sample ring PCM uses (a 24-bit integer is exact in an `f32`, and the render thread writes it back as the
//! same integer without touching it).

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use player_dsd::dop::DopStream;
use player_dsd::dsf::DsfReader;
use player_dsd::stream::{DsdError, DsdStream};

use super::format::dop_word_to_sample;
use crate::AudioError;

/// Frames converted per refill.
const REFILL_FRAMES: usize = 4096;

type FileStream = DopStream<Box<dyn DsdStream>>;

/// What a DoP track looks like before it is played.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DopTrackInfo {
    /// The 1-bit rate of the file (2 822 400 for DSD64).
    pub(crate) dsd_rate: u32,
    /// The PCM rate that carries it.
    pub(crate) transport_rate: u32,
    pub(crate) total_frames: u64,
}

impl DopTrackInfo {
    pub(crate) fn duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(
            self.total_frames as f64 / f64::from(self.transport_rate),
        )
    }

    /// The frame that holds this time (rounded down), clamped to the track.
    pub(crate) fn frame_at(&self, position: std::time::Duration) -> u64 {
        ((position.as_secs_f64() * f64::from(self.transport_rate)) as u64).min(self.total_frames)
    }

    pub(crate) fn time_of(&self, frame: u64) -> std::time::Duration {
        std::time::Duration::from_secs_f64(frame as f64 / f64::from(self.transport_rate))
    }
}

/// Why a file cannot be played as DoP.
pub(crate) enum DopOpenError {
    /// The file is fine but DoP cannot carry it (rate, channels); the reason says what.
    Unsupported(String),
    /// The file could not be read.
    Audio(AudioError),
}

pub(crate) struct DopSource {
    stream: FileStream,
    words: Vec<u32>,
    index: usize,
}

impl DopSource {
    /// Open `path` and start at `frame`.
    pub(crate) fn open(path: &Path, frame: u64) -> Result<(Self, DopTrackInfo), DopOpenError> {
        let decode_error = |error: DsdError| {
            DopOpenError::Audio(AudioError::Decode {
                path: path.to_path_buf(),
                message: error.to_string(),
            })
        };
        let reader: Box<dyn DsdStream> = if crate::dsd::is_dsf_path(path) {
            let file = File::open(path).map_err(|error| {
                DopOpenError::Audio(AudioError::FileOpen {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                })
            })?;
            Box::new(
                DsfReader::open(BufReader::with_capacity(256 * 1024, file))
                    .map_err(decode_error)?,
            )
        } else {
            // DSD inside WavPack (ADR 0025): the same 1-bit stream, so the same DoP words.
            Box::new(
                player_wavpack::dsd::WavPackDsdStream::open_path(path).map_err(|error| {
                    DopOpenError::Audio(AudioError::Decode {
                        path: path.to_path_buf(),
                        message: error.to_string(),
                    })
                })?,
            )
        };
        let mut stream = match DopStream::new(reader) {
            Ok(stream) => stream,
            Err(DsdError::Unsupported(reason)) => return Err(DopOpenError::Unsupported(reason)),
            Err(error) => return Err(decode_error(error)),
        };
        stream.seek_to_frame(frame).map_err(decode_error)?;
        let info = DopTrackInfo {
            dsd_rate: stream.dsd_info().sample_rate,
            transport_rate: stream.transport_rate(),
            total_frames: stream.total_frames(),
        };
        Ok((
            Self {
                stream,
                words: Vec::new(),
                index: 0,
            },
            info,
        ))
    }
}

impl DopSource {
    /// Continue from this frame instead.
    pub(crate) fn seek_to_frame(&mut self, frame: u64) -> Result<(), AudioError> {
        self.words.clear();
        self.index = 0;
        self.stream
            .seek_to_frame(frame)
            .map_err(|error| AudioError::Seek(error.to_string()))
    }
}

impl Iterator for DopSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.index >= self.words.len() {
            self.words.clear();
            self.index = 0;
            // A read error ends the track like the end of the file would.
            match self.stream.read_frames(REFILL_FRAMES, &mut self.words) {
                Ok(0) | Err(_) => return None,
                Ok(_) => {}
            }
        }
        let word = self.words[self.index];
        self.index += 1;
        Some(dop_word_to_sample(word))
    }
}
