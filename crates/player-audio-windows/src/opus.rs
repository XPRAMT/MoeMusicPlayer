//! The rodio source for Ogg Opus playback (ADR 0017) over `player-opus`.

use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use player_opus::{OpusError, OpusReader, OPUS_RATE};
use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};

use crate::AudioError;

pub(crate) struct OpusSource<R: Read + Seek + Send> {
    reader: OpusReader<R>,
    channels: u16,
    total_frames: Option<u64>,
    /// Interleaved frames of the current packet.
    buffer: Vec<Sample>,
    position: usize,
}

impl<R: Read + Seek + Send> OpusSource<R> {
    pub(crate) fn new(reader: OpusReader<R>) -> Self {
        let info = reader.info();
        Self {
            reader,
            channels: u16::from(info.channels),
            total_frames: info.total_frames,
            buffer: Vec::new(),
            position: 0,
        }
    }
}

impl<R: Read + Seek + Send> Iterator for OpusSource<R> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.position >= self.buffer.len() {
            self.buffer.clear();
            self.position = 0;
            match self.reader.read_frames(&mut self.buffer) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(error) => {
                    eprintln!("[audio] Opus decode failed, ending the track: {error}");
                    return None;
                }
            }
        }
        let sample = self.buffer[self.position];
        self.position += 1;
        Some(sample)
    }
}

impl<R: Read + Seek + Send> Source for OpusSource<R> {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.channels).expect("Opus channel count is 1 or 2")
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(OPUS_RATE).expect("Opus rate")
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_frames
            .map(|frames| Duration::from_secs_f64(frames as f64 / f64::from(OPUS_RATE)))
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        let frame = (position.as_secs_f64() * f64::from(OPUS_RATE)).round() as u64;
        self.buffer.clear();
        self.position = 0;
        self.reader
            .seek_to_frame(frame)
            .map_err(|error| SeekError::Other(Arc::new(error)))
    }
}

/// The concrete Opus source the playback chain plays.
pub(crate) type OpusFileSource = OpusSource<File>;

/// Open an Ogg Opus file for playback.
pub(crate) fn open_opus(path: &Path) -> Result<OpusFileSource, AudioError> {
    let file = File::open(path).map_err(|error| AudioError::FileOpen {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let reader = OpusReader::open(file).map_err(|error: OpusError| AudioError::Decode {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    Ok(OpusSource::new(reader))
}
