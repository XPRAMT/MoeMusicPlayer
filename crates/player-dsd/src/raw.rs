//! Native DSD over USB (ADR 0049): a DSF track as the raw 32-bit slots a UAC2 RAW_DATA alternate setting carries.
//!
//! One slot holds 32 consecutive DSD bits of one channel (four DSD bytes); a frame is the left slot followed by the
//! right slot, and the clock runs at the DSD bit rate ÷ 32: DSD64 (2 822 400 Hz) → 88 200 Hz, DSD128 → 176 400 Hz,
//! DSD256 → 352 800 Hz. The bits are most-significant-first on the wire, which is what every [`DsdStream`] hands out.
//! The order of the four bytes inside a slot is [`SlotByteOrder`].
//!
//! Media3 only knows PCM, so the track is served to it as a WAV file whose data is these frames. The header describes
//! 16-bit PCM with four channels at the frame rate (8 bytes per frame, the same as two 32-bit slots): Media3's
//! `DefaultAudioSink` converts 24- and 32-bit integer PCM to float before the output sees it, which would destroy the
//! words, while 16-bit PCM reaches the output untouched. The header rate is therefore a fake PCM rate and the data is
//! not audio to anything but the USB output, which the load marks (ADR 0049).
//!
//! Silence is the DSD idle pattern ([`DSD_IDLE_BYTE`]) everywhere: the tail padding of the last slot and the invalid
//! low bits of the last DSD byte. A word of zeros is a full-scale DC step to a DSD DAC.

use crate::stream::{DsdError, DsdInfo, DsdStream, DSD_IDLE_BYTE};

/// DSD bytes (per channel) in one slot.
pub const SLOT_BYTES: usize = 4;
/// Bytes of one stereo frame: the left slot and the right slot.
pub const FRAME_BYTES: usize = 2 * SLOT_BYTES;
/// DSD bits per slot, the divisor from the DSD bit rate to the frame rate.
pub const SLOT_BITS: u32 = 32;
pub const WAV_HEADER_LEN: u64 = 44;
/// The WAV carrier: 16-bit PCM, this many channels (8 bytes per frame).
pub const CARRIER_CHANNELS: u16 = 4;
pub const CARRIER_BITS: u16 = 16;
/// A WAV file cannot describe more than 4 GiB; a longer track is cut at the last whole frame that fits.
const MAX_DATA_BYTES: u64 = (u32::MAX as u64 - 36) / FRAME_BYTES as u64 * FRAME_BYTES as u64;
/// Bytes per channel read from the stream per refill.
const READ_BYTES: usize = 4096;

/// The multiples of 44.1 kHz offered on the bus (DSD64, DSD128, DSD256); DSD512 would need a 705.6 kHz clock.
const SUPPORTED_MULTIPLES: [u32; 3] = [64, 128, 256];

/// How the four DSD bytes of a slot are laid out in the 32-bit word the device reads.
///
/// `OldestFirst` (setting `"a"`, the default) sends the bytes in time order, the earliest first (Linux `DSD_U32_BE`
/// memory order). `NewestFirst` (`"b"`) sends the word reversed (`DSD_U32_LE`). Which one a device wants is not
/// verified; the setting `dsd.nativeByteOrder` chooses (ADR 0049).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SlotByteOrder {
    #[default]
    OldestFirst,
    NewestFirst,
}

impl SlotByteOrder {
    /// `"b"` is [`NewestFirst`](Self::NewestFirst); anything else is the default.
    pub fn from_setting(text: &str) -> Self {
        if text.trim().eq_ignore_ascii_case("b") {
            Self::NewestFirst
        } else {
            Self::OldestFirst
        }
    }

    pub fn setting(self) -> &'static str {
        match self {
            Self::OldestFirst => "a",
            Self::NewestFirst => "b",
        }
    }
}

/// The frame rate that carries this stream natively: stereo DSD64, DSD128 or DSD256 of the 44.1 kHz family.
pub fn raw_transport_rate(info: &DsdInfo) -> Option<u32> {
    let multiple = info.rate_multiple()?;
    (info.channels == 2 && SUPPORTED_MULTIPLES.contains(&multiple))
        .then_some(info.sample_rate / SLOT_BITS)
}

/// Why [`raw_transport_rate`] is `None`, in words for the output card (`None` when the stream is accepted).
pub fn raw_unsupported_reason(info: &DsdInfo) -> Option<String> {
    if raw_transport_rate(info).is_some() {
        return None;
    }
    Some(if info.channels != 2 {
        "不是雙聲道".to_owned()
    } else {
        match info.rate_multiple() {
            None => "不是 44.1 kHz 系列的 DSD".to_owned(),
            Some(multiple) if multiple > 256 => {
                format!("DSD{multiple} 超出 USB 音訊的取樣率上限")
            }
            Some(multiple) => format!("DSD{multiple} 不支援直通"),
        }
    })
}

/// Append the slots of `left` and `right` (equal length, MSB-first DSD bytes) to `out`: for every four bytes the left
/// slot, then the right slot, each in `order`. A tail shorter than four bytes is padded with the idle pattern.
pub fn pack_raw_words(left: &[u8], right: &[u8], order: SlotByteOrder, out: &mut Vec<u8>) {
    debug_assert_eq!(left.len(), right.len());
    let length = left.len().min(right.len());
    out.reserve(length.div_ceil(SLOT_BYTES) * FRAME_BYTES);
    let mut index = 0;
    while index < length {
        push_slot(&left[index..length], order, out);
        push_slot(&right[index..length], order, out);
        index += SLOT_BYTES;
    }
}

/// One slot from the first (up to) four bytes of `bytes`.
fn push_slot(bytes: &[u8], order: SlotByteOrder, out: &mut Vec<u8>) {
    let mut slot = [DSD_IDLE_BYTE; SLOT_BYTES];
    let count = bytes.len().min(SLOT_BYTES);
    slot[..count].copy_from_slice(&bytes[..count]);
    if order == SlotByteOrder::NewestFirst {
        slot.reverse();
    }
    out.extend_from_slice(&slot);
}

/// `byte` with only its first `valid_bits` bits kept (MSB-first); the rest are the idle pattern's bits.
pub fn idle_padded_byte(byte: u8, valid_bits: u32) -> u8 {
    if valid_bits >= 8 {
        return byte;
    }
    let keep = 0xFF_u8.checked_shl(8 - valid_bits).unwrap_or(0);
    (byte & keep) | (DSD_IDLE_BYTE & !keep)
}

/// The 44-byte WAV header of the carrier for `frame_rate` and `data_bytes` of frames.
pub fn raw_wav_header(frame_rate: u32, data_bytes: u64) -> [u8; WAV_HEADER_LEN as usize] {
    let data = data_bytes.min(MAX_DATA_BYTES) as u32;
    let block = CARRIER_CHANNELS * (CARRIER_BITS / 8);
    let mut header = [0_u8; WAV_HEADER_LEN as usize];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(36 + data).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16_u32.to_le_bytes());
    header[20..22].copy_from_slice(&1_u16.to_le_bytes());
    header[22..24].copy_from_slice(&CARRIER_CHANNELS.to_le_bytes());
    header[24..28].copy_from_slice(&frame_rate.to_le_bytes());
    header[28..32].copy_from_slice(&(frame_rate * u32::from(block)).to_le_bytes());
    header[32..34].copy_from_slice(&block.to_le_bytes());
    header[34..36].copy_from_slice(&CARRIER_BITS.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data.to_le_bytes());
    header
}

/// A DSD track as a byte-addressable WAV file of raw slots (see the module documentation).
pub struct RawDsdWavStream<S: DsdStream> {
    stream: S,
    info: DsdInfo,
    order: SlotByteOrder,
    frame_rate: u32,
    header: [u8; WAV_HEADER_LEN as usize],
    total_frames: u64,
    data_bytes: u64,
    /// DSD bytes per channel read ahead but not yet a whole slot (sequential reads only).
    pending: [Vec<u8>; 2],
    /// The stream is at its end and the tail was padded.
    finished: bool,
    /// Offset in the data section of the first byte of `carry`.
    carry_start: u64,
    carry: Vec<u8>,
    planar: Vec<Vec<u8>>,
}

impl<S: DsdStream> RawDsdWavStream<S> {
    /// Fails with the reason when the stream cannot be carried natively ([`raw_unsupported_reason`]).
    pub fn new(stream: S, order: SlotByteOrder) -> Result<Self, DsdError> {
        let info = stream.info();
        let frame_rate = raw_transport_rate(&info).ok_or_else(|| {
            DsdError::Unsupported(
                raw_unsupported_reason(&info).unwrap_or_else(|| "不支援直通".to_owned()),
            )
        })?;
        let total_frames = info.bytes_per_channel().div_ceil(SLOT_BYTES as u64);
        let data_bytes = (total_frames * FRAME_BYTES as u64).min(MAX_DATA_BYTES);
        Ok(Self {
            stream,
            info,
            order,
            frame_rate,
            header: raw_wav_header(frame_rate, data_bytes),
            total_frames: data_bytes / FRAME_BYTES as u64,
            data_bytes,
            pending: [Vec::new(), Vec::new()],
            finished: false,
            carry_start: 0,
            carry: Vec::new(),
            planar: vec![Vec::new(); 2],
        })
    }

    /// Length of the whole virtual file.
    pub fn total_len(&self) -> u64 {
        WAV_HEADER_LEN + self.data_bytes
    }

    /// Frames per second on the bus (the header's sample rate).
    pub fn frame_rate(&self) -> u32 {
        self.frame_rate
    }

    pub fn total_frames(&self) -> u64 {
        self.total_frames
    }

    pub fn dsd_info(&self) -> DsdInfo {
        self.info
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

    /// Refill `carry` with the next packed frames so that it covers data offset `offset`.
    fn fill_carry(&mut self, offset: u64) -> Result<(), DsdError> {
        let carry_end = self.carry_start + self.carry.len() as u64;
        let sequential = !self.carry.is_empty() && offset == carry_end;
        let chunk_start = if sequential {
            carry_end
        } else {
            // A new position starts on a frame boundary: four DSD bytes per channel per frame.
            let frame = offset / FRAME_BYTES as u64;
            self.stream.seek_to_byte(frame * SLOT_BYTES as u64)?;
            self.pending[0].clear();
            self.pending[1].clear();
            self.finished = false;
            frame * FRAME_BYTES as u64
        };
        self.carry.clear();
        self.carry_start = chunk_start;
        while self.carry.is_empty() && !self.finished {
            let count = self.stream.read(READ_BYTES, &mut self.planar)?;
            if count == 0 {
                self.finished = true;
            } else {
                self.fix_last_byte();
                for channel in 0..2 {
                    self.pending[channel].extend_from_slice(&self.planar[channel]);
                }
            }
            let ready = if self.finished {
                self.pending[0].len()
            } else {
                self.pending[0].len() / SLOT_BYTES * SLOT_BYTES
            };
            if ready > 0 {
                pack_raw_words(
                    &self.pending[0][..ready],
                    &self.pending[1][..ready],
                    self.order,
                    &mut self.carry,
                );
                self.pending[0].drain(..ready);
                self.pending[1].drain(..ready);
            }
        }
        Ok(())
    }

    /// The last DSD byte of the stream may hold fewer than 8 valid bits: the others become idle-pattern bits.
    fn fix_last_byte(&mut self) {
        let valid = (self.info.samples_per_channel % 8) as u32;
        if valid == 0 || self.stream.position_bytes() < self.info.bytes_per_channel() {
            return;
        }
        for buffer in &mut self.planar {
            if let Some(last) = buffer.last_mut() {
                *last = idle_padded_byte(*last, valid);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::dsf::DsfReader;
    use crate::testing::build_dsf;

    fn info(channels: u16, multiple: u32) -> DsdInfo {
        DsdInfo {
            channels,
            sample_rate: 44_100 * multiple,
            samples_per_channel: 1_000_000,
        }
    }

    #[test]
    fn only_stereo_dsd64_128_256_are_carried_and_the_rate_is_the_bit_rate_over_32() {
        assert_eq!(raw_transport_rate(&info(2, 64)), Some(88_200));
        assert_eq!(raw_transport_rate(&info(2, 128)), Some(176_400));
        assert_eq!(raw_transport_rate(&info(2, 256)), Some(352_800));
        assert_eq!(
            raw_transport_rate(&info(2, 512)),
            None,
            "705.6 kHz is out of reach"
        );
        assert_eq!(raw_transport_rate(&info(1, 64)), None, "mono");
        assert_eq!(raw_transport_rate(&info(6, 64)), None, "multichannel");
        let odd = DsdInfo {
            channels: 2,
            sample_rate: 48_000 * 64,
            samples_per_channel: 10,
        };
        assert_eq!(raw_transport_rate(&odd), None, "the 48 kHz family");
        assert_eq!(raw_unsupported_reason(&info(2, 64)), None);
        assert!(raw_unsupported_reason(&info(1, 64))
            .unwrap()
            .contains("雙聲道"));
        assert!(raw_unsupported_reason(&info(2, 512))
            .unwrap()
            .contains("DSD512"));
        assert!(raw_unsupported_reason(&odd).unwrap().contains("44.1"));
    }

    #[test]
    fn a_slot_is_four_bytes_left_then_right_in_the_chosen_order() {
        let left = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let right = [0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18];
        let mut a = Vec::new();
        pack_raw_words(&left, &right, SlotByteOrder::OldestFirst, &mut a);
        assert_eq!(
            a,
            [
                0x01, 0x02, 0x03, 0x04, 0x11, 0x12, 0x13, 0x14, // frame 0: L slot, R slot
                0x05, 0x06, 0x07, 0x08, 0x15, 0x16, 0x17, 0x18,
            ]
        );
        let mut b = Vec::new();
        pack_raw_words(&left, &right, SlotByteOrder::NewestFirst, &mut b);
        assert_eq!(
            b,
            [
                0x04, 0x03, 0x02, 0x01, 0x14, 0x13, 0x12, 0x11, //
                0x08, 0x07, 0x06, 0x05, 0x18, 0x17, 0x16, 0x15,
            ]
        );
    }

    #[test]
    fn a_tail_shorter_than_a_slot_is_padded_with_the_idle_pattern_never_zero() {
        for order in [SlotByteOrder::OldestFirst, SlotByteOrder::NewestFirst] {
            for tail in 1..SLOT_BYTES {
                let left: Vec<u8> = (0..tail as u8).map(|n| 0xA0 + n).collect();
                let right: Vec<u8> = (0..tail as u8).map(|n| 0xB0 + n).collect();
                let mut out = Vec::new();
                pack_raw_words(&left, &right, order, &mut out);
                assert_eq!(out.len(), FRAME_BYTES, "{order:?} {tail}");
                let idle = out.iter().filter(|&&b| b == DSD_IDLE_BYTE).count();
                assert_eq!(idle, 2 * (SLOT_BYTES - tail), "{order:?} {tail}");
                assert!(!out.contains(&0), "no zero byte in the fill");
            }
        }
        let mut empty = Vec::new();
        pack_raw_words(&[], &[], SlotByteOrder::OldestFirst, &mut empty);
        assert!(empty.is_empty());
    }

    #[test]
    fn the_last_byte_keeps_only_its_valid_bits() {
        assert_eq!(idle_padded_byte(0xFF, 8), 0xFF);
        // 0x69 = 0110 1001: with 3 valid bits the rest are 0 1001 → 111 0 1001.
        assert_eq!(idle_padded_byte(0xFF, 3), 0b1110_1001);
        assert_eq!(idle_padded_byte(0x00, 3), 0b0000_1001);
        assert_eq!(idle_padded_byte(0xAB, 0), DSD_IDLE_BYTE);
    }

    #[test]
    fn the_setting_text_maps_to_the_order() {
        assert_eq!(SlotByteOrder::from_setting("a"), SlotByteOrder::OldestFirst);
        assert_eq!(SlotByteOrder::from_setting("B"), SlotByteOrder::NewestFirst);
        assert_eq!(
            SlotByteOrder::from_setting(" b "),
            SlotByteOrder::NewestFirst
        );
        assert_eq!(SlotByteOrder::from_setting("x"), SlotByteOrder::OldestFirst);
        assert_eq!(SlotByteOrder::from_setting(""), SlotByteOrder::OldestFirst);
        assert_eq!(SlotByteOrder::default().setting(), "a");
        assert_eq!(SlotByteOrder::NewestFirst.setting(), "b");
    }

    fn payload(channel: u8, bytes: usize) -> Vec<u8> {
        (0..bytes)
            .map(|index| (index as u8).wrapping_mul(37).wrapping_add(channel * 91) | 1)
            .collect()
    }

    fn stream_of(
        left: &[u8],
        right: &[u8],
        samples: u64,
        order: SlotByteOrder,
    ) -> RawDsdWavStream<DsfReader<Cursor<Vec<u8>>>> {
        let file = build_dsf(
            2_822_400,
            samples,
            4096,
            false,
            &[left.to_vec(), right.to_vec()],
        );
        RawDsdWavStream::new(DsfReader::open(Cursor::new(file)).unwrap(), order).unwrap()
    }

    fn read_all<S: DsdStream>(stream: &mut RawDsdWavStream<S>, chunk: usize) -> Vec<u8> {
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
    fn the_header_describes_sixteen_bit_pcm_with_four_channels_at_the_frame_rate() {
        let bytes = 10_000;
        let (left, right) = (payload(0, bytes), payload(1, bytes));
        let mut stream = stream_of(&left, &right, bytes as u64 * 8, SlotByteOrder::OldestFirst);
        assert_eq!(stream.frame_rate(), 88_200);
        assert_eq!(stream.total_frames(), 2_500);
        assert_eq!(stream.total_len(), 44 + 2_500 * 8);
        let all = read_all(&mut stream, 4096);
        assert_eq!(all.len() as u64, stream.total_len());
        assert_eq!(&all[0..4], b"RIFF");
        assert_eq!(&all[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([all[20], all[21]]), 1, "integer PCM");
        assert_eq!(
            u16::from_le_bytes([all[22], all[23]]),
            4,
            "carrier channels"
        );
        assert_eq!(u32::from_le_bytes(all[24..28].try_into().unwrap()), 88_200);
        assert_eq!(
            u32::from_le_bytes(all[28..32].try_into().unwrap()),
            88_200 * 8
        );
        assert_eq!(u16::from_le_bytes([all[32], all[33]]), 8, "block align");
        assert_eq!(u16::from_le_bytes([all[34], all[35]]), 16);
        assert_eq!(&all[36..40], b"data");
        assert_eq!(
            u32::from_le_bytes(all[40..44].try_into().unwrap()) as u64,
            2_500 * 8
        );
        assert_eq!(
            u32::from_le_bytes(all[4..8].try_into().unwrap()) as u64,
            36 + 2_500 * 8
        );
    }

    #[test]
    fn the_data_is_the_dsf_bytes_in_slots_and_does_not_depend_on_the_read_size() {
        let bytes = 9_000;
        let (left, right) = (payload(0, bytes), payload(1, bytes));
        let samples = bytes as u64 * 8;
        // build_dsf with lsb_first = false stores MSB-first bytes: they come out unchanged.
        let mut expected = Vec::new();
        pack_raw_words(&left, &right, SlotByteOrder::OldestFirst, &mut expected);
        let reference = read_all(
            &mut stream_of(&left, &right, samples, SlotByteOrder::OldestFirst),
            1 << 16,
        );
        assert_eq!(&reference[44..], &expected[..]);
        for chunk in [1, 7, 8, 4096, 33_333] {
            let got = read_all(
                &mut stream_of(&left, &right, samples, SlotByteOrder::OldestFirst),
                chunk,
            );
            assert_eq!(got, reference, "chunk {chunk}");
        }
        // The other order reverses every slot.
        let swapped = read_all(
            &mut stream_of(&left, &right, samples, SlotByteOrder::NewestFirst),
            4096,
        );
        for (a, b) in reference[44..]
            .chunks_exact(4)
            .zip(swapped[44..].chunks_exact(4))
        {
            let mut reversed = a.to_vec();
            reversed.reverse();
            assert_eq!(reversed, b);
        }
    }

    #[test]
    fn an_lsb_first_file_is_normalised_to_msb_first_slots() {
        let bytes = 64;
        let (left, right) = (payload(0, bytes), payload(1, bytes));
        let file = build_dsf(
            2_822_400,
            bytes as u64 * 8,
            4096,
            true,
            &[left.clone(), right.clone()],
        );
        let mut stream = RawDsdWavStream::new(
            DsfReader::open(Cursor::new(file)).unwrap(),
            SlotByteOrder::OldestFirst,
        )
        .unwrap();
        let data = read_all(&mut stream, 4096);
        // The file's LSB-first byte 0x01 is the MSB-first 0x80 on the wire.
        assert_eq!(data[44], left[0].reverse_bits());
        assert_eq!(data[44 + 4], right[0].reverse_bits());
    }

    #[test]
    fn a_track_that_is_not_a_whole_number_of_slots_ends_in_idle_fill_not_zeros() {
        // 4 slots and 3 bytes (and the last byte has only 5 valid bits).
        let bytes = 4 * SLOT_BYTES + 3;
        let samples = (bytes as u64 - 1) * 8 + 5;
        let left = vec![0xFF; bytes];
        let right = vec![0xFF; bytes];
        let mut stream = stream_of(&left, &right, samples, SlotByteOrder::OldestFirst);
        assert_eq!(stream.total_frames(), 5);
        let data = read_all(&mut stream, 4096);
        let frames = &data[44..];
        assert_eq!(frames.len(), 5 * FRAME_BYTES);
        let last = &frames[4 * FRAME_BYTES..];
        // Left slot: FF FF [FF with 5 valid bits] 69.
        assert_eq!(last[0], 0xFF);
        assert_eq!(last[1], 0xFF);
        assert_eq!(last[2], idle_padded_byte(0xFF, 5));
        assert_eq!(last[3], DSD_IDLE_BYTE);
        assert_eq!(
            &last[4..8],
            &[0xFF, 0xFF, idle_padded_byte(0xFF, 5), DSD_IDLE_BYTE]
        );
        assert!(!last.contains(&0));
    }

    #[test]
    fn random_access_reads_match_the_sequential_bytes() {
        let bytes = 20_000;
        let (left, right) = (payload(0, bytes), payload(1, bytes));
        let samples = bytes as u64 * 8;
        let sequential = read_all(
            &mut stream_of(&left, &right, samples, SlotByteOrder::OldestFirst),
            1 << 16,
        );
        let mut stream = stream_of(&left, &right, samples, SlotByteOrder::OldestFirst);
        let total = stream.total_len();
        let mut buffer = vec![0_u8; 3_001];
        for &position in &[
            0_u64,
            20,
            44,
            45,
            30_003,
            44,
            20_000,
            total - 4_000,
            9_999,
            total - 7,
        ] {
            let count = stream.read_at(position, &mut buffer).unwrap();
            assert_eq!(
                count,
                buffer.len().min((total - position) as usize),
                "position {position}"
            );
            assert_eq!(
                &buffer[..count],
                &sequential[position as usize..position as usize + count],
                "position {position}"
            );
        }
        assert_eq!(stream.read_at(total, &mut buffer).unwrap(), 0);
        assert_eq!(stream.read_at(total + 100, &mut buffer).unwrap(), 0);
    }

    #[test]
    fn a_mono_or_dsd512_file_is_refused_with_its_reason() {
        let mono = build_dsf(2_822_400, 800, 4096, false, &[vec![0x55; 100]]);
        let error = RawDsdWavStream::new(
            DsfReader::open(Cursor::new(mono)).unwrap(),
            SlotByteOrder::OldestFirst,
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("雙聲道"), "{error}");
        let wide = build_dsf(
            22_579_200,
            800,
            4096,
            false,
            &[vec![0x55; 100], vec![0x55; 100]],
        );
        let error = RawDsdWavStream::new(
            DsfReader::open(Cursor::new(wide)).unwrap(),
            SlotByteOrder::OldestFirst,
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("DSD512"), "{error}");
    }

    #[test]
    fn the_header_is_capped_at_the_wav_limit() {
        let header = raw_wav_header(88_200, u64::MAX);
        let data = u32::from_le_bytes(header[40..44].try_into().unwrap()) as u64;
        assert!(data <= u32::MAX as u64 - 36);
        assert_eq!(data % FRAME_BYTES as u64, 0);
    }
}
