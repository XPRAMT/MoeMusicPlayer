//! The rodio source for WavPack playback (ADR 0025) over `player-wavpack`.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use player_wavpack::{ReadSeek, WavPackError, WavPackReader};
use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};

use crate::AudioError;

pub(crate) struct WavPackSource {
    reader: WavPackReader,
    channels: u16,
    sample_rate: u32,
    total_frames: Option<u64>,
    /// Interleaved frames of the last decoded chunk.
    buffer: Vec<Sample>,
    position: usize,
}

impl WavPackSource {
    pub(crate) fn new(reader: WavPackReader) -> Self {
        let info = reader.info();
        Self {
            reader,
            channels: info.channels,
            sample_rate: info.sample_rate,
            total_frames: info.total_frames,
            buffer: Vec::new(),
            position: 0,
        }
    }
}

impl Iterator for WavPackSource {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.position >= self.buffer.len() {
            self.buffer.clear();
            self.position = 0;
            match self.reader.read_frames(&mut self.buffer) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(error) => {
                    eprintln!("[audio] WavPack decode failed, ending the track: {error}");
                    return None;
                }
            }
        }
        let sample = self.buffer[self.position];
        self.position += 1;
        Some(sample)
    }
}

impl Source for WavPackSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.channels).expect("WavPack channel count is at least 1")
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.sample_rate).expect("WavPack sample rate is at least 1")
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_frames
            .map(|frames| Duration::from_secs_f64(frames as f64 / f64::from(self.sample_rate)))
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        let frame = (position.as_secs_f64() * f64::from(self.sample_rate)).round() as u64;
        self.buffer.clear();
        self.position = 0;
        self.reader
            .seek_to_frame(frame)
            .map_err(|error| SeekError::Other(Arc::new(error)))
    }
}

/// The `.wvc` correction file next to a hybrid `.wv`, when there is one.
fn correction_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push("c");
    PathBuf::from(name)
}

/// Open a WavPack file for playback (with its `.wvc` correction file if present).
pub(crate) fn open_wavpack(path: &Path) -> Result<WavPackSource, AudioError> {
    if crate::remote_input::is_remote(path) {
        let input = crate::remote_input::open(path)?;
        let reader = WavPackReader::open(input.main, input.correction).map_err(
            |error: WavPackError| AudioError::Decode {
                path: path.to_path_buf(),
                message: error.to_string(),
            },
        )?;
        return Ok(WavPackSource::new(reader));
    }
    let file = File::open(path).map_err(|error| AudioError::FileOpen {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let correction: Option<Box<dyn ReadSeek>> = File::open(correction_path(path))
        .ok()
        .map(|file| Box::new(BufReader::new(file)) as Box<dyn ReadSeek>);
    let reader = WavPackReader::open(BufReader::with_capacity(256 * 1024, file), correction)
        .map_err(|error: WavPackError| AudioError::Decode {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    Ok(WavPackSource::new(reader))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../player-wavpack/tests/fixtures")
            .join(name)
    }

    #[test]
    fn plays_through_the_rodio_source_and_seeks() {
        let mut source = open_wavpack(&fixture("s16.wv")).unwrap();
        assert_eq!(source.sample_rate().get(), 44_100);
        assert_eq!(source.channels().get(), 2);
        assert_eq!(source.total_duration(), Some(Duration::from_millis(300)));
        let all: Vec<f32> = source.by_ref().collect();
        assert_eq!(all.len(), 13_230 * 2);
        source.try_seek(Duration::from_millis(100)).unwrap();
        let tail: Vec<f32> = source.collect();
        assert_eq!(tail[..], all[4410 * 2..]);
    }
}
