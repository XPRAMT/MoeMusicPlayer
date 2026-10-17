//! A DSD track as a byte-addressable WAV file of 32-bit float PCM, converted on demand (used by the Android data
//! source: ExoPlayer reads "WAV" ranges and seeks by byte position).
//!
//! Layout: a 44-byte header (`RIFF`, `fmt ` with format tag 3 = IEEE float, `data`) followed by the interleaved
//! little-endian `f32` frames. The total length is known up front (`header + total_frames × channels × 4`), so the
//! player can compute the duration and request any position.

use std::sync::atomic::AtomicU32;
use std::sync::Arc;

use crate::reader::DsdPcmReader;
use crate::stream::{DsdError, DsdStream};

pub const WAV_HEADER_LEN: u64 = 44;
const BYTES_PER_SAMPLE: u64 = 4;
/// A WAV file cannot describe more than 4 GiB; longer tracks are cut at the last whole frame that fits.
const MAX_DATA_BYTES: u64 = u32::MAX as u64 - 36;

pub struct DsdWavStream<S: DsdStream> {
    reader: DsdPcmReader<S>,
    header: [u8; WAV_HEADER_LEN as usize],
    bytes_per_frame: u64,
    data_bytes: u64,
    /// Offset in the data section of the first byte of `carry`.
    carry_start: u64,
    carry: Vec<u8>,
    frames: Vec<f32>,
}

impl<S: DsdStream> DsdWavStream<S> {
    pub fn new(reader: DsdPcmReader<S>) -> Self {
        let channels = reader.channels() as u64;
        let bytes_per_frame = channels * BYTES_PER_SAMPLE;
        let data_bytes = (reader.total_frames() * bytes_per_frame).min(MAX_DATA_BYTES)
            / bytes_per_frame
            * bytes_per_frame;
        let rate = reader.out_rate();
        let mut header = [0_u8; WAV_HEADER_LEN as usize];
        header[0..4].copy_from_slice(b"RIFF");
        header[4..8].copy_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
        header[8..12].copy_from_slice(b"WAVE");
        header[12..16].copy_from_slice(b"fmt ");
        header[16..20].copy_from_slice(&16_u32.to_le_bytes());
        header[20..22].copy_from_slice(&3_u16.to_le_bytes());
        header[22..24].copy_from_slice(&(channels as u16).to_le_bytes());
        header[24..28].copy_from_slice(&rate.to_le_bytes());
        header[28..32].copy_from_slice(&(rate * bytes_per_frame as u32).to_le_bytes());
        header[32..34].copy_from_slice(&(bytes_per_frame as u16).to_le_bytes());
        header[34..36].copy_from_slice(&32_u16.to_le_bytes());
        header[36..40].copy_from_slice(b"data");
        header[40..44].copy_from_slice(&(data_bytes as u32).to_le_bytes());
        Self {
            reader,
            header,
            bytes_per_frame,
            data_bytes,
            carry_start: 0,
            carry: Vec::new(),
            frames: Vec::new(),
        }
    }

    /// Length of the whole virtual file.
    pub fn total_len(&self) -> u64 {
        WAV_HEADER_LEN + self.data_bytes
    }

    pub fn out_rate(&self) -> u32 {
        self.reader.out_rate()
    }

    pub fn channels(&self) -> usize {
        self.reader.channels()
    }

    pub fn total_frames(&self) -> u64 {
        self.data_bytes / self.bytes_per_frame
    }

    /// Copy bytes starting at `position` of the virtual file into `out`; returns the number written (0 at the end).
    pub fn read_at(&mut self, position: u64, out: &mut [u8]) -> Result<usize, DsdError> {
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

    /// Refill `carry` with the next converted chunk so that it covers data offset `offset`.
    fn fill_carry(&mut self, offset: u64) -> Result<(), DsdError> {
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

/// A gain handle for a fixed gain in dB, for callers that do not need to change it while playing.
pub fn fixed_gain(gain_db: f32) -> Arc<AtomicU32> {
    Arc::new(AtomicU32::new(gain_db.to_bits()))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::dsf::DsfReader;
    use crate::testing::{build_dsf, modulate, tone};

    fn stereo_file(seconds: f64) -> Vec<u8> {
        let rate = 2_822_400_u32;
        let samples = (f64::from(rate) * seconds) as usize / 16 * 16;
        let lsb = |bytes: Vec<u8>| bytes.into_iter().map(u8::reverse_bits).collect::<Vec<u8>>();
        let left = lsb(modulate(samples, tone(rate, 1_000.0, 0.25)));
        let right = lsb(modulate(samples, tone(rate, 3_000.0, 0.25)));
        build_dsf(rate, samples as u64, 4096, true, &[left, right])
    }

    fn wav(file: Vec<u8>, gain_db: f32) -> DsdWavStream<DsfReader<Cursor<Vec<u8>>>> {
        let reader = DsdPcmReader::new(
            DsfReader::open(Cursor::new(file)).unwrap(),
            176_400,
            fixed_gain(gain_db),
        )
        .unwrap();
        DsdWavStream::new(reader)
    }

    fn read_all(stream: &mut DsdWavStream<DsfReader<Cursor<Vec<u8>>>>, chunk: usize) -> Vec<u8> {
        let mut all = Vec::new();
        let mut buffer = vec![0_u8; chunk];
        loop {
            let count = stream.read_at(all.len() as u64, &mut buffer).unwrap();
            if count == 0 {
                break;
            }
            all.extend_from_slice(&buffer[..count]);
        }
        all
    }

    #[test]
    fn header_describes_a_float_wav_of_the_exact_length() {
        let mut stream = wav(stereo_file(0.5), 0.0);
        let frames = stream.total_frames();
        assert_eq!(frames, 2_822_400 / 2 / 16);
        assert_eq!(stream.total_len(), 44 + frames * 8);
        let bytes = read_all(&mut stream, 4096);
        assert_eq!(bytes.len() as u64, stream.total_len());
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 3, "IEEE float");
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            176_400
        );
        assert_eq!(
            u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
            176_400 * 8
        );
        assert_eq!(u16::from_le_bytes([bytes[32], bytes[33]]), 8);
        assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 32);
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(
            u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as u64,
            frames * 8
        );
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as u64,
            36 + frames * 8
        );
        // The samples are real audio: finite and not silent.
        let samples: Vec<f32> = bytes[44..]
            .chunks(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert!(samples.iter().all(|s| s.is_finite()));
        assert!(samples.iter().any(|s| s.abs() > 0.1));
    }

    #[test]
    fn the_bytes_do_not_depend_on_the_read_size() {
        let file = stereo_file(0.3);
        let reference = read_all(&mut wav(file.clone(), 3.0), 1 << 16);
        for chunk in [1, 7, 4096, 33_333] {
            assert_eq!(
                read_all(&mut wav(file.clone(), 3.0), chunk),
                reference,
                "chunk {chunk}"
            );
        }
    }

    #[test]
    fn random_access_reads_match_the_sequential_bytes_after_the_preroll() {
        let file = stereo_file(0.5);
        let mut stream = wav(file.clone(), 0.0);
        let sequential = read_all(&mut wav(file, 0.0), 1 << 16);
        let mut buffer = vec![0_u8; 8_000];
        // Positions inside the header, on and off frame boundaries, in the middle and near the end, then backwards.
        let total = stream.total_len();
        for &position in &[
            0_u64,
            20,
            44,
            45,
            47,
            300_003,
            200_001,
            400_000,
            total - 5_000,
            60_001,
            44,
        ] {
            let count = stream.read_at(position, &mut buffer).unwrap();
            assert_eq!(
                count,
                buffer.len().min((total - position) as usize),
                "position {position}"
            );
            let got: Vec<f32> = floats(&buffer[..count], position);
            let want: Vec<f32> = floats(
                &sequential[position as usize..position as usize + count],
                position,
            );
            let worst = got
                .iter()
                .zip(&want)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f32, f32::max);
            assert!(
                worst < 2e-6,
                "position {position}: worst difference {worst}"
            );
        }
        assert_eq!(stream.read_at(total, &mut buffer).unwrap(), 0);
        assert_eq!(stream.read_at(total + 100, &mut buffer).unwrap(), 0);
    }

    /// The f32 values fully contained in `bytes`, which start at virtual-file offset `position`.
    fn floats(bytes: &[u8], position: u64) -> Vec<f32> {
        let skip = if position < WAV_HEADER_LEN {
            (WAV_HEADER_LEN - position) as usize
        } else {
            ((4 - (position - WAV_HEADER_LEN) % 4) % 4) as usize
        };
        bytes[skip.min(bytes.len())..]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect()
    }
}
