//! DSD inside WavPack as a [`DsdStream`] (ADR 0025): the 1-bit stream goes to the project's own DSD→PCM converter
//! (`player-dsd`, ADR 0016) instead of the 24-bit decimation libwavpack offers, so it sounds, converts and gains
//! exactly like a DSF file. libwavpack hands out one byte of DSD per channel per "sample" (eight 1-bit samples).

use std::path::Path;

use player_dsd::stream::{DsdError, DsdInfo, DsdStream};

use crate::{ffi, WavPackError, WavPackReader, CHUNK_FRAMES};

pub struct WavPackDsdStream {
    reader: WavPackReader,
    info: DsdInfo,
    scratch: Vec<i32>,
}

impl WavPackDsdStream {
    /// Open a WavPack file that holds DSD (a PCM file is refused).
    pub fn open_path(path: &Path) -> Result<Self, WavPackError> {
        Self::from_reader(WavPackReader::open_path_with(path, true)?)
    }

    /// As [`open_path`](Self::open_path), for a stream that is not a file on disk (a remote file), with its `.wvc`
    /// correction stream if there is one.
    pub fn open(
        main: impl crate::ReadSeek + 'static,
        correction: Option<Box<dyn crate::ReadSeek>>,
    ) -> Result<Self, WavPackError> {
        Self::from_reader(WavPackReader::open_with(main, correction, true)?)
    }

    fn from_reader(reader: WavPackReader) -> Result<Self, WavPackError> {
        let info = reader.info();
        if !info.dsd {
            return Err(WavPackError::Open("這不是 DSD 的 WavPack 檔案".to_owned()));
        }
        let total_bytes = info
            .total_frames
            .ok_or_else(|| WavPackError::Open("檔案沒有記錄長度".to_owned()))?;
        Ok(Self {
            reader,
            info: DsdInfo {
                channels: info.channels,
                sample_rate: info.native_sample_rate,
                samples_per_channel: total_bytes * 8,
            },
            scratch: Vec::new(),
        })
    }
}

impl DsdStream for WavPackDsdStream {
    fn info(&self) -> DsdInfo {
        self.info
    }

    fn read(&mut self, max_bytes: usize, out: &mut [Vec<u8>]) -> Result<usize, DsdError> {
        let channels = usize::from(self.info.channels);
        for buffer in out.iter_mut() {
            buffer.clear();
        }
        if self.reader.at_end {
            return Ok(0);
        }
        let wanted = max_bytes.min(CHUNK_FRAMES);
        self.scratch.resize(wanted * channels, 0);
        let bytes = unsafe {
            ffi::WavpackUnpackSamples(
                self.reader.context,
                self.scratch.as_mut_ptr(),
                wanted as u32,
            )
        } as usize;
        if bytes == 0 {
            let errors = unsafe { ffi::WavpackGetNumErrors(self.reader.context) };
            return if errors > 0 {
                Err(DsdError::Io(format!("{errors} 個損毀的區塊")))
            } else {
                Ok(0)
            };
        }
        for (channel, buffer) in out.iter_mut().enumerate().take(channels) {
            buffer.extend(
                self.scratch[..bytes * channels]
                    .chunks_exact(channels)
                    .map(|frame| frame[channel] as u8),
            );
        }
        self.reader.position += bytes as u64;
        Ok(bytes)
    }

    fn seek_to_byte(&mut self, byte_index: u64) -> Result<(), DsdError> {
        self.reader
            .seek_to_frame(byte_index)
            .map_err(|error| DsdError::Io(error.to_string()))
    }

    fn position_bytes(&self) -> u64 {
        self.reader.position
    }
}
