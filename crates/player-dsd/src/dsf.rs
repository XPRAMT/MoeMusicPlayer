//! DSF (DSD Stream File) reader.
//!
//! Layout (all little endian): a 28-byte `DSD ` chunk (magic, chunk size 28, total file size, offset of the
//! trailing ID3v2 tag or 0), a 52-byte `fmt ` chunk (version, format id 0 = DSD raw, channel type, channel
//! count, sampling frequency, bits per sample, samples per channel, block size per channel, reserved) and a
//! `data` chunk (12-byte header, then blocks: for each block index, `block_size` bytes of channel 0, then of
//! channel 1, ...; the last block is zero-padded). "Bits per sample" 1 means the bits of each byte run
//! least-significant first, 8 means most-significant first; this reader always hands out MSB-first bytes.

use std::io::{Read, Seek, SeekFrom};

use crate::stream::{DsdError, DsdInfo, DsdStream};

pub(crate) const DSD_CHUNK_SIZE: u64 = 28;
pub(crate) const FMT_CHUNK_SIZE: u64 = 52;
pub(crate) const DATA_HEADER_SIZE: u64 = 12;
pub(crate) const DATA_OFFSET: u64 = DSD_CHUNK_SIZE + FMT_CHUNK_SIZE + DATA_HEADER_SIZE;
const MAX_CHANNELS: u32 = 6;
const MAX_BLOCK_SIZE: u32 = 1 << 20;

pub struct DsfReader<R: Read + Seek + Send> {
    reader: R,
    info: DsdInfo,
    block_size: usize,
    lsb_first: bool,
    /// Offset of the ID3v2 tag at the end of the file, when present.
    id3v2_offset: Option<u64>,
    /// Per-channel byte index of the next read.
    position: u64,
    /// Interleaved cache of block `cached_block` (`channels * block_size` bytes).
    cache: Vec<u8>,
    cached_block: Option<u64>,
}

impl<R: Read + Seek + Send> DsfReader<R> {
    pub fn open(mut reader: R) -> Result<Self, DsdError> {
        let file_len = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        let mut header = [0_u8; DATA_OFFSET as usize];
        reader
            .read_exact(&mut header)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::UnexpectedEof => DsdError::InvalidFormat("檔案太短".into()),
                _ => DsdError::from(error),
            })?;

        if &header[0..4] != b"DSD " {
            return Err(DsdError::InvalidFormat(
                "不是 DSF 檔案（缺少 DSD 標記）".into(),
            ));
        }
        if le_u64(&header[4..12]) != DSD_CHUNK_SIZE {
            return Err(DsdError::InvalidFormat("DSD 區塊大小不是 28".into()));
        }
        let metadata_offset = le_u64(&header[20..28]);

        let fmt = &header[28..80];
        if &fmt[0..4] != b"fmt " {
            return Err(DsdError::InvalidFormat("缺少 fmt 區塊".into()));
        }
        if le_u64(&fmt[4..12]) != FMT_CHUNK_SIZE {
            return Err(DsdError::InvalidFormat("fmt 區塊大小不是 52".into()));
        }
        let format_version = le_u32(&fmt[12..16]);
        let format_id = le_u32(&fmt[16..20]);
        let channels = le_u32(&fmt[24..28]);
        let sample_rate = le_u32(&fmt[28..32]);
        let bits_per_sample = le_u32(&fmt[32..36]);
        let declared_samples = le_u64(&fmt[36..44]);
        let block_size = le_u32(&fmt[44..48]);
        if format_version != 1 {
            return Err(DsdError::Unsupported(format!("DSF 版本 {format_version}")));
        }
        if format_id != 0 {
            return Err(DsdError::Unsupported("非 DSD raw 編碼".into()));
        }
        if channels == 0 || channels > MAX_CHANNELS {
            return Err(DsdError::InvalidFormat(format!(
                "聲道數 {channels} 超出範圍"
            )));
        }
        if sample_rate == 0 {
            return Err(DsdError::InvalidFormat("取樣率為 0".into()));
        }
        let lsb_first = match bits_per_sample {
            1 => true,
            8 => false,
            other => {
                return Err(DsdError::InvalidFormat(format!(
                    "每取樣位元數 {other} 無效"
                )))
            }
        };
        if block_size == 0 || block_size > MAX_BLOCK_SIZE {
            return Err(DsdError::InvalidFormat(format!(
                "區塊大小 {block_size} 無效"
            )));
        }

        let data = &header[80..92];
        if &data[0..4] != b"data" {
            return Err(DsdError::InvalidFormat("缺少 data 區塊".into()));
        }
        let data_chunk_size = le_u64(&data[4..12]);
        if data_chunk_size < DATA_HEADER_SIZE {
            return Err(DsdError::InvalidFormat("data 區塊大小無效".into()));
        }

        // Real audio bytes: what the chunk declares, limited by what the file actually holds. Only whole blocks
        // are usable, so a truncated file shortens the stream instead of returning garbage.
        let declared_bytes = data_chunk_size - DATA_HEADER_SIZE;
        let available = file_len.saturating_sub(DATA_OFFSET).min(declared_bytes);
        let stride = u64::from(channels) * u64::from(block_size);
        let usable_per_channel = (available / stride) * u64::from(block_size);
        let samples_per_channel = declared_samples.min(usable_per_channel.saturating_mul(8));

        let id3v2_offset =
            (metadata_offset != 0 && metadata_offset < file_len).then_some(metadata_offset);
        Ok(Self {
            reader,
            info: DsdInfo {
                channels: channels as u16,
                sample_rate,
                samples_per_channel,
            },
            block_size: block_size as usize,
            lsb_first,
            id3v2_offset,
            position: 0,
            cache: vec![0; stride as usize],
            cached_block: None,
        })
    }

    /// Offset of the ID3v2 tag at the end of the file, when the header points at one.
    pub fn id3v2_offset(&self) -> Option<u64> {
        self.id3v2_offset
    }

    fn load_block(&mut self, block: u64) -> Result<(), DsdError> {
        if self.cached_block == Some(block) {
            return Ok(());
        }
        let stride = self.cache.len() as u64;
        self.reader
            .seek(SeekFrom::Start(DATA_OFFSET + block * stride))?;
        self.reader.read_exact(&mut self.cache)?;
        self.cached_block = Some(block);
        Ok(())
    }
}

impl<R: Read + Seek + Send> DsdStream for DsfReader<R> {
    fn info(&self) -> DsdInfo {
        self.info
    }

    fn read(&mut self, max_bytes: usize, out: &mut [Vec<u8>]) -> Result<usize, DsdError> {
        let channels = usize::from(self.info.channels);
        debug_assert_eq!(out.len(), channels);
        for buffer in out.iter_mut() {
            buffer.clear();
        }
        let total = self.info.bytes_per_channel();
        if self.position >= total || max_bytes == 0 {
            return Ok(0);
        }
        let block_size = self.block_size as u64;
        let block = self.position / block_size;
        let offset = (self.position % block_size) as usize;
        let count = max_bytes
            .min(self.block_size - offset)
            .min((total - self.position) as usize);
        self.load_block(block)?;
        for (channel, buffer) in out.iter_mut().enumerate() {
            let start = channel * self.block_size + offset;
            let source = &self.cache[start..start + count];
            if self.lsb_first {
                buffer.extend(source.iter().map(|byte| byte.reverse_bits()));
            } else {
                buffer.extend_from_slice(source);
            }
        }
        self.position += count as u64;
        Ok(count)
    }

    fn seek_to_byte(&mut self, byte_index: u64) -> Result<(), DsdError> {
        self.position = byte_index.min(self.info.bytes_per_channel());
        Ok(())
    }

    fn position_bytes(&self) -> u64 {
        self.position
    }
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().expect("4 bytes"))
}

fn le_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes[..8].try_into().expect("8 bytes"))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::testing::build_dsf;

    fn payload(channel: u8, bytes: usize) -> Vec<u8> {
        (0..bytes)
            .map(|index| (index as u8).wrapping_mul(37).wrapping_add(channel * 91))
            .collect()
    }

    fn read_all(reader: &mut DsfReader<Cursor<Vec<u8>>>, chunk: usize) -> Vec<Vec<u8>> {
        let channels = usize::from(reader.info().channels);
        let mut all = vec![Vec::new(); channels];
        let mut buffers = vec![Vec::new(); channels];
        loop {
            let count = reader.read(chunk, &mut buffers).unwrap();
            if count == 0 {
                break;
            }
            for (target, buffer) in all.iter_mut().zip(&buffers) {
                assert_eq!(buffer.len(), count);
                target.extend_from_slice(buffer);
            }
        }
        all
    }

    #[test]
    fn parses_info_and_reads_msb_first_bytes_from_an_lsb_first_file() {
        let left = payload(0, 10_000);
        let right = payload(1, 10_000);
        let file = build_dsf(
            2_822_400,
            80_000,
            4096,
            true,
            &[left.clone(), right.clone()],
        );
        let mut reader = DsfReader::open(Cursor::new(file)).unwrap();
        let info = reader.info();
        assert_eq!(info.channels, 2);
        assert_eq!(info.sample_rate, 2_822_400);
        assert_eq!(info.samples_per_channel, 80_000);
        assert_eq!(info.bytes_per_channel(), 10_000);
        assert_eq!(info.rate_name(), "DSD64");
        let all = read_all(&mut reader, 4096);
        let expected_left: Vec<u8> = left.iter().map(|byte| byte.reverse_bits()).collect();
        let expected_right: Vec<u8> = right.iter().map(|byte| byte.reverse_bits()).collect();
        assert_eq!(all[0], expected_left);
        assert_eq!(all[1], expected_right);
    }

    #[test]
    fn msb_first_files_are_passed_through_unchanged() {
        let left = payload(0, 5000);
        let file = build_dsf(5_644_800, 40_000, 4096, false, std::slice::from_ref(&left));
        let mut reader = DsfReader::open(Cursor::new(file)).unwrap();
        assert_eq!(reader.info().channels, 1);
        assert_eq!(reader.info().rate_name(), "DSD128");
        assert_eq!(read_all(&mut reader, 1000)[0], left);
    }

    #[test]
    fn small_reads_cross_block_boundaries_and_stop_at_the_declared_length() {
        let left = payload(0, 9000);
        let right = payload(1, 9000);
        // 71 997 samples = 8 999.625 bytes -> 9000 bytes, the last holding 5 valid bits.
        let file = build_dsf(
            2_822_400,
            71_997,
            4096,
            false,
            &[left.clone(), right.clone()],
        );
        let mut reader = DsfReader::open(Cursor::new(file)).unwrap();
        assert_eq!(reader.info().bytes_per_channel(), 9000);
        let all = read_all(&mut reader, 777);
        assert_eq!(all[0], left);
        assert_eq!(all[1], right);
    }

    #[test]
    fn seek_repositions_reads_to_any_byte() {
        let left = payload(0, 12_000);
        let right = payload(1, 12_000);
        let file = build_dsf(
            2_822_400,
            96_000,
            4096,
            false,
            &[left.clone(), right.clone()],
        );
        let mut reader = DsfReader::open(Cursor::new(file)).unwrap();
        let mut buffers = vec![Vec::new(); 2];
        for &target in &[0_u64, 4095, 4096, 8191, 11_900, 3] {
            reader.seek_to_byte(target).unwrap();
            assert_eq!(reader.position_bytes(), target);
            let count = reader.read(64, &mut buffers).unwrap();
            assert!(count > 0);
            assert_eq!(buffers[0], left[target as usize..target as usize + count]);
            assert_eq!(buffers[1], right[target as usize..target as usize + count]);
        }
        reader.seek_to_byte(1_000_000).unwrap();
        assert_eq!(reader.position_bytes(), 12_000);
        assert_eq!(reader.read(64, &mut buffers).unwrap(), 0);
    }

    #[test]
    fn a_truncated_file_is_shortened_to_whole_blocks() {
        let left = payload(0, 12_000);
        let right = payload(1, 12_000);
        let mut file = build_dsf(2_822_400, 96_000, 4096, false, &[left, right]);
        // Keep header + 1.5 blocks worth of data: only the first whole block (4096 bytes per channel) survives.
        file.truncate(DATA_OFFSET as usize + 8192 + 4096);
        let reader = DsfReader::open(Cursor::new(file)).unwrap();
        assert_eq!(reader.info().bytes_per_channel(), 4096);
    }

    #[test]
    fn invalid_files_are_rejected_with_clear_errors() {
        assert!(matches!(
            DsfReader::open(Cursor::new(b"RIFF".to_vec())),
            Err(DsdError::InvalidFormat(_))
        ));
        let mut file = build_dsf(2_822_400, 8000, 4096, true, &[payload(0, 1000)]);
        file[0] = b'X';
        assert!(matches!(
            DsfReader::open(Cursor::new(file)),
            Err(DsdError::InvalidFormat(_))
        ));
        let mut file = build_dsf(2_822_400, 8000, 4096, true, &[payload(0, 1000)]);
        file[28 + 32..28 + 36].copy_from_slice(&3_u32.to_le_bytes());
        assert!(matches!(
            DsfReader::open(Cursor::new(file)),
            Err(DsdError::InvalidFormat(_))
        ));
        let mut file = build_dsf(2_822_400, 8000, 4096, true, &[payload(0, 1000)]);
        file[28 + 12..28 + 16].copy_from_slice(&2_u32.to_le_bytes());
        assert!(matches!(
            DsfReader::open(Cursor::new(file)),
            Err(DsdError::Unsupported(_))
        ));
    }

    #[test]
    fn metadata_pointer_is_exposed_only_when_it_points_inside_the_file() {
        let mut file = build_dsf(2_822_400, 8000, 4096, true, &[payload(0, 1000)]);
        let len = file.len() as u64;
        assert_eq!(
            DsfReader::open(Cursor::new(file.clone()))
                .unwrap()
                .id3v2_offset(),
            None
        );
        file[20..28].copy_from_slice(&(len - 16).to_le_bytes());
        assert_eq!(
            DsfReader::open(Cursor::new(file.clone()))
                .unwrap()
                .id3v2_offset(),
            Some(len - 16)
        );
        file[20..28].copy_from_slice(&(len + 100).to_le_bytes());
        assert_eq!(
            DsfReader::open(Cursor::new(file)).unwrap().id3v2_offset(),
            None
        );
    }
}
