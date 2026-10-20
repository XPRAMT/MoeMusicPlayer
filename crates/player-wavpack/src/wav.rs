//! A WavPack file as a byte-addressable WAV file of 32-bit float PCM, decoded on demand (used by the Android data
//! source: Media3 has no WavPack decoder, ExoPlayer reads "WAV" ranges and seeks by byte position; ADR 0025).
//!
//! Layout: a 44-byte header (`RIFF`, `fmt ` with format tag 3 = IEEE float, `data`) followed by the interleaved
//! little-endian `f32` frames at the file's own sample rate. The total length is known up front
//! (`header + total_frames × channels × 4`), so the player can compute the duration and request any position.

use crate::{WavPackError, WavPackReader};

pub const WAV_HEADER_LEN: u64 = 44;
const BYTES_PER_SAMPLE: u64 = 4;
/// A WAV file cannot describe more than 4 GiB; longer tracks are cut at the last whole frame that fits.
const MAX_DATA_BYTES: u64 = u32::MAX as u64 - 36;

pub struct WavPackWavStream {
    reader: WavPackReader,
    header: [u8; WAV_HEADER_LEN as usize],
    bytes_per_frame: u64,
    data_bytes: u64,
    /// Offset in the data section of the first byte of `carry`.
    carry_start: u64,
    carry: Vec<u8>,
    frames: Vec<f32>,
}

impl WavPackWavStream {
    /// Serves `reader` as a WAV file; a file that does not say how long it is cannot be served.
    pub fn new(reader: WavPackReader) -> Result<Self, WavPackError> {
        let info = reader.info();
        let total_frames = info.total_frames.ok_or_else(|| {
            WavPackError::Open("檔案沒有記錄長度，無法以位元組位置播放".to_owned())
        })?;
        let channels = u64::from(info.channels);
        let bytes_per_frame = channels * BYTES_PER_SAMPLE;
        let data_bytes = (total_frames * bytes_per_frame).min(MAX_DATA_BYTES) / bytes_per_frame
            * bytes_per_frame;
        let rate = info.sample_rate;
        let mut header = [0_u8; WAV_HEADER_LEN as usize];
        header[0..4].copy_from_slice(b"RIFF");
        header[4..8].copy_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
        header[8..12].copy_from_slice(b"WAVE");
        header[12..16].copy_from_slice(b"fmt ");
        header[16..20].copy_from_slice(&16_u32.to_le_bytes());
        header[20..22].copy_from_slice(&3_u16.to_le_bytes());
        header[22..24].copy_from_slice(&info.channels.to_le_bytes());
        header[24..28].copy_from_slice(&rate.to_le_bytes());
        header[28..32].copy_from_slice(&(rate * bytes_per_frame as u32).to_le_bytes());
        header[32..34].copy_from_slice(&(bytes_per_frame as u16).to_le_bytes());
        header[34..36].copy_from_slice(&32_u16.to_le_bytes());
        header[36..40].copy_from_slice(b"data");
        header[40..44].copy_from_slice(&(data_bytes as u32).to_le_bytes());
        Ok(Self {
            reader,
            header,
            bytes_per_frame,
            data_bytes,
            carry_start: 0,
            carry: Vec::new(),
            frames: Vec::new(),
        })
    }

    /// Length of the whole virtual file.
    pub fn total_len(&self) -> u64 {
        WAV_HEADER_LEN + self.data_bytes
    }

    pub fn sample_rate(&self) -> u32 {
        self.reader.info().sample_rate
    }

    pub fn channels(&self) -> usize {
        usize::from(self.reader.info().channels)
    }

    pub fn total_frames(&self) -> u64 {
        self.data_bytes / self.bytes_per_frame
    }

    /// Copy bytes starting at `position` of the virtual file into `out`; returns the number written (0 at the end).
    pub fn read_at(&mut self, position: u64, out: &mut [u8]) -> Result<usize, WavPackError> {
        let mut written = 0;
        let mut position = position;
        if position < WAV_HEADER_LEN {
            let count = ((WAV_HEADER_LEN - position) as usize).min(out.len());
            out[..count]
                .copy_from_slice(&self.header[position as usize..position as usize + count]);
            written = count;
            position += count as u64;
        }
        while written < out.len() {
            let offset = position - WAV_HEADER_LEN;
            if offset >= self.data_bytes {
                break;
            }
            let carry_end = self.carry_start + self.carry.len() as u64;
            if offset < self.carry_start || offset >= carry_end {
                self.fill_carry(offset)?;
                if self.carry.is_empty() {
                    break;
                }
            }
            let start = (offset - self.carry_start) as usize;
            let available = self.carry.len() - start;
            let wanted = (self.data_bytes - offset).min((out.len() - written) as u64) as usize;
            let count = available.min(wanted);
            out[written..written + count].copy_from_slice(&self.carry[start..start + count]);
            written += count;
            position += count as u64;
        }
        Ok(written)
    }

    /// Refill `carry` with the next decoded chunk so that it covers data offset `offset`.
    fn fill_carry(&mut self, offset: u64) -> Result<(), WavPackError> {
        let frame = offset / self.bytes_per_frame;
        let carry_end = self.carry_start + self.carry.len() as u64;
        // The next chunk follows the previous one without a seek; anything else repositions the reader.
        let (chunk_start, sequential) = if !self.carry.is_empty() && offset == carry_end {
            (carry_end, true)
        } else {
            (frame * self.bytes_per_frame, false)
        };
        if !sequential {
            self.reader.seek_to_frame(frame)?;
        }
        self.frames.clear();
        let produced = self.reader.read_frames(&mut self.frames)?;
        self.carry.clear();
        self.carry_start = chunk_start;
        if produced == 0 {
            return Ok(());
        }
        self.carry
            .reserve(self.frames.len() * BYTES_PER_SAMPLE as usize);
        for sample in &self.frames {
            self.carry.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::BufReader;
    use std::path::PathBuf;

    use super::*;

    fn reference(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/reference")
            .join(name)
    }

    fn reader(name: &str) -> WavPackReader {
        WavPackReader::open(BufReader::new(File::open(reference(name)).unwrap()), None).unwrap()
    }

    fn read_all(stream: &mut WavPackWavStream, chunk: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buffer = vec![0_u8; chunk];
        loop {
            let count = stream.read_at(out.len() as u64, &mut buffer).unwrap();
            if count == 0 {
                return out;
            }
            out.extend_from_slice(&buffer[..count]);
        }
    }

    #[test]
    fn serves_a_float_wav_whose_samples_equal_the_decoder_output() {
        let mut wav = WavPackWavStream::new(reader("int24_stereo.wv")).unwrap();
        let bytes = read_all(&mut wav, 1_000);
        assert_eq!(bytes.len() as u64, wav.total_len());
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 3, "IEEE float");
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            48_000
        );
        let served: Vec<f32> = bytes[44..]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let mut decoder = reader("int24_stereo.wv");
        let mut expected = Vec::new();
        while decoder.read_frames(&mut expected).unwrap() > 0 {}
        assert_eq!(served, expected);
    }

    #[test]
    fn random_access_matches_sequential_reading() {
        let mut wav = WavPackWavStream::new(reader("int16_stereo.wv")).unwrap();
        let whole = read_all(&mut wav, 777);
        for &(position, length) in &[
            (0_u64, 44_usize),
            (30, 100),
            (1_000, 4_096),
            (7_777, 513),
            (19_000, 5_000),
        ] {
            let mut buffer = vec![0_u8; length];
            let count = wav.read_at(position, &mut buffer).unwrap();
            let end = (position as usize + length).min(whole.len());
            assert_eq!(
                buffer[..count],
                whole[position as usize..end],
                "at {position}"
            );
        }
        let mut buffer = [0_u8; 16];
        assert_eq!(wav.read_at(wav.total_len(), &mut buffer).unwrap(), 0);
        assert_eq!(wav.read_at(wav.total_len() + 100, &mut buffer).unwrap(), 0);
    }
}
