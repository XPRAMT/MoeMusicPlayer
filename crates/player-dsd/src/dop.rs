//! DoP, DSD over PCM (standard 1.1; ADR 0020): the raw DSD bits of a [`DsdStream`] packed into 24-bit PCM words that a
//! DAC recognizes as DSD.
//!
//! One DoP frame holds 16 consecutive DSD bits of every channel. A 24-bit word is `marker << 16 | bits`: the marker
//! byte alternates `0x05`, `0xFA` from frame to frame (both channels carry the same one), and the 16 bits are two
//! DSD bytes, the earlier byte in the more significant half (the stream's bytes are already MSB-first). The PCM rate
//! is the DSD rate divided by 16: DSD64 → 176.4 kHz, DSD128 → 352.8 kHz, DSD256 → 705.6 kHz.
//!
//! Silence is the DSD idle pattern ([`DSD_IDLE_BYTE`], `0x69`) inside the same marker framing, never PCM zeros.

use crate::stream::{DsdError, DsdInfo, DsdStream, DSD_IDLE_BYTE};

/// The two marker bytes, in the order they alternate.
pub const DOP_MARKERS: [u8; 2] = [0x05, 0xFA];

/// The 16 idle bits of one DoP frame.
pub const DOP_IDLE_BITS: u16 = u16::from_be_bytes([DSD_IDLE_BYTE, DSD_IDLE_BYTE]);

/// DSD bits per channel in one DoP frame.
pub const DOP_BITS_PER_FRAME: u32 = 16;

/// The multiples of 44.1 kHz offered as DoP (DSD64, DSD128, DSD256); DSD512 would need a 1.4112 MHz PCM rate.
const SUPPORTED_MULTIPLES: [u32; 3] = [64, 128, 256];

/// The PCM rate that carries this stream as DoP, or `None` when it cannot (not a 44.1 kHz-family multiple of DSD64 to
/// DSD256, or not mono/stereo; mono is not offered because a DoP stream is always two channels).
pub fn dop_transport_rate(info: &DsdInfo) -> Option<u32> {
    let multiple = info.rate_multiple()?;
    (info.channels == 2 && SUPPORTED_MULTIPLES.contains(&multiple))
        .then_some(info.sample_rate / DOP_BITS_PER_FRAME)
}

/// The 24-bit DoP word (in the low bits of a `u32`) for a marker (index 0 or 1) and 16 DSD bits.
pub fn dop_word(marker_index: usize, bits: u16) -> u32 {
    (u32::from(DOP_MARKERS[marker_index & 1]) << 16) | u32::from(bits)
}

/// Packs DSD bytes into DoP words, carrying the marker phase from call to call.
#[derive(Clone, Debug, Default)]
pub struct DopPacker {
    phase: usize,
}

impl DopPacker {
    pub fn new() -> Self {
        Self::default()
    }

    /// The marker index (0 or 1) the next frame gets.
    pub fn phase(&self) -> usize {
        self.phase
    }

    /// Pack planar bytes (one buffer per channel, all the same even length) and append the interleaved words to
    /// `out`, one frame per two bytes.
    pub fn pack(&mut self, planar: &[Vec<u8>], out: &mut Vec<u32>) {
        let Some(first) = planar.first() else {
            return;
        };
        let frames = first.len() / 2;
        out.reserve(frames * planar.len());
        for frame in 0..frames {
            for channel in planar {
                let bits = u16::from_be_bytes([channel[frame * 2], channel[frame * 2 + 1]]);
                out.push(dop_word(self.phase, bits));
            }
            self.phase ^= 1;
        }
    }

    /// Append `frames` idle frames for `channels` channels.
    pub fn idle(&mut self, channels: usize, frames: usize, out: &mut Vec<u32>) {
        for _ in 0..frames {
            for _ in 0..channels {
                out.push(dop_word(self.phase, DOP_IDLE_BITS));
            }
            self.phase ^= 1;
        }
    }
}

/// A seekable DoP source over a stereo [`DsdStream`]: whole frames of interleaved words. A trailing odd byte at the
/// end of the stream is completed with the idle pattern.
pub struct DopStream<S: DsdStream> {
    stream: S,
    info: DsdInfo,
    packer: DopPacker,
    planar: Vec<Vec<u8>>,
    /// Per-channel bytes read but not yet packed (a read may end in the middle of a frame).
    carry: Vec<Vec<u8>>,
    frames_emitted: u64,
    ended: bool,
}

/// Bytes per channel read from the stream per refill.
const READ_BYTES: usize = 8192;

impl<S: DsdStream> DopStream<S> {
    pub fn new(stream: S) -> Result<Self, DsdError> {
        let info = stream.info();
        if dop_transport_rate(&info).is_none() {
            return Err(DsdError::Unsupported(format!(
                "{}（{} 聲道）無法以 DoP 輸出",
                info.rate_name(),
                info.channels
            )));
        }
        let channels = usize::from(info.channels);
        let mut dop = Self {
            stream,
            info,
            packer: DopPacker::new(),
            planar: vec![Vec::new(); channels],
            carry: vec![Vec::new(); channels],
            frames_emitted: 0,
            ended: false,
        };
        dop.seek_to_frame(0)?;
        Ok(dop)
    }

    pub fn dsd_info(&self) -> DsdInfo {
        self.info
    }

    /// The PCM rate of the words.
    pub fn transport_rate(&self) -> u32 {
        self.info.sample_rate / DOP_BITS_PER_FRAME
    }

    pub fn channels(&self) -> usize {
        usize::from(self.info.channels)
    }

    /// Frames in the whole track (a trailing odd byte counts as a frame).
    pub fn total_frames(&self) -> u64 {
        self.info.bytes_per_channel().div_ceil(2)
    }

    /// Index of the next frame `read_frames` produces.
    pub fn position_frames(&self) -> u64 {
        self.frames_emitted
    }

    /// Move to this frame (clamped to the end). DSD has no filter state, so this is exact.
    pub fn seek_to_frame(&mut self, frame: u64) -> Result<(), DsdError> {
        let frame = frame.min(self.total_frames());
        self.stream.seek_to_byte(frame * 2)?;
        for buffer in &mut self.carry {
            buffer.clear();
        }
        self.frames_emitted = frame;
        self.ended = frame >= self.total_frames();
        Ok(())
    }

    /// Append up to `max_frames` frames of interleaved words to `out`; returns the number of frames (0 at the end).
    /// The marker phase continues from the frame index, so the same position always gets the same markers.
    pub fn read_frames(
        &mut self,
        max_frames: usize,
        out: &mut Vec<u32>,
    ) -> Result<usize, DsdError> {
        self.packer.phase = (self.frames_emitted & 1) as usize;
        let mut produced = 0;
        while produced < max_frames && !self.ended {
            let want = ((max_frames - produced) * 2).min(READ_BYTES);
            let got = self.stream.read(want, &mut self.planar)?;
            if got == 0 {
                // The stream is over: complete an odd trailing byte, if any.
                self.ended = true;
                if !self.carry[0].is_empty() {
                    for buffer in &mut self.carry {
                        buffer.push(DSD_IDLE_BYTE);
                    }
                    self.packer.pack(&self.carry, out);
                    for buffer in &mut self.carry {
                        buffer.clear();
                    }
                    produced += 1;
                }
                break;
            }
            for (carry, planar) in self.carry.iter_mut().zip(&self.planar) {
                carry.extend_from_slice(planar);
            }
            let whole = self.carry[0].len() / 2 * 2;
            if whole == 0 {
                continue;
            }
            let packed: Vec<Vec<u8>> = self
                .carry
                .iter_mut()
                .map(|carry| carry.drain(..whole).collect())
                .collect();
            self.packer.pack(&packed, out);
            produced += whole / 2;
        }
        self.frames_emitted += produced as u64;
        Ok(produced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsf::DsfReader;
    use crate::testing::{build_dsf, modulate, tone};
    use std::io::Cursor;

    fn info(rate: u32, channels: u16) -> DsdInfo {
        DsdInfo {
            channels,
            sample_rate: rate,
            samples_per_channel: 1_000_000,
        }
    }

    /// A stereo DSD64 DSF with the given per-channel bytes (MSB-first, so `lsb_first` is false).
    fn dsf(rate: u32, left: Vec<u8>, right: Vec<u8>) -> Vec<u8> {
        let samples = left.len() as u64 * 8;
        build_dsf(rate, samples, 4096, false, &[left, right])
    }

    fn unpack(words: &[u32], channels: usize) -> Vec<Vec<u8>> {
        let mut planar = vec![Vec::new(); channels];
        for frame in words.chunks(channels) {
            for (channel, word) in frame.iter().enumerate() {
                planar[channel].push((word >> 8) as u8);
                planar[channel].push(*word as u8);
            }
        }
        planar
    }

    #[test]
    fn the_transport_rate_is_the_dsd_rate_over_sixteen_for_stereo_dsd64_to_dsd256() {
        assert_eq!(dop_transport_rate(&info(2_822_400, 2)), Some(176_400));
        assert_eq!(dop_transport_rate(&info(5_644_800, 2)), Some(352_800));
        assert_eq!(dop_transport_rate(&info(11_289_600, 2)), Some(705_600));
        assert_eq!(
            dop_transport_rate(&info(22_579_200, 2)),
            None,
            "DSD512 is not offered"
        );
        assert_eq!(
            dop_transport_rate(&info(2_822_400, 1)),
            None,
            "mono is not offered"
        );
        assert_eq!(
            dop_transport_rate(&info(3_072_000, 2)),
            None,
            "48 kHz-family DSD is not offered"
        );
    }

    #[test]
    fn markers_alternate_and_the_earlier_byte_is_the_more_significant() {
        let mut packer = DopPacker::new();
        let mut words = Vec::new();
        packer.pack(
            &[
                vec![0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC],
                vec![0xFF, 0x00, 0x0F, 0xF0, 0xAA, 0x55],
            ],
            &mut words,
        );
        assert_eq!(
            words,
            [
                0x05_1234, 0x05_FF00, // frame 0, marker 0x05
                0xFA_5678, 0xFA_0FF0, // frame 1, marker 0xFA
                0x05_9ABC, 0x05_AA55, // frame 2
            ]
        );
        assert_eq!(packer.phase(), 1, "the next frame is 0xFA");
    }

    #[test]
    fn idle_frames_carry_the_idle_pattern_and_keep_the_marker_alternating() {
        let mut packer = DopPacker::new();
        let mut words = Vec::new();
        packer.idle(2, 3, &mut words);
        assert_eq!(
            words,
            [0x05_6969, 0x05_6969, 0xFA_6969, 0xFA_6969, 0x05_6969, 0x05_6969]
        );
    }

    #[test]
    fn a_whole_file_unpacks_to_exactly_its_dsd_bits() {
        let samples = 2_822_400 / 4;
        let left = modulate(samples, tone(2_822_400, 1000.0, 0.5));
        let right = modulate(samples, tone(2_822_400, 1500.0, 0.4));
        let image = dsf(2_822_400, left.clone(), right.clone());
        let mut stream = DopStream::new(DsfReader::open(Cursor::new(image)).unwrap()).unwrap();
        assert_eq!(stream.transport_rate(), 176_400);
        let mut words = Vec::new();
        // Odd request sizes cross read boundaries on purpose.
        while stream.read_frames(1001, &mut words).unwrap() > 0 {}
        assert_eq!(words.len() as u64, stream.total_frames() * 2);
        for (index, frame) in words.chunks(2).enumerate() {
            assert_eq!(
                (frame[0] >> 16) as u8,
                DOP_MARKERS[index & 1],
                "marker at frame {index}"
            );
            assert_eq!(
                frame[0] >> 16,
                frame[1] >> 16,
                "both channels carry the same marker"
            );
        }
        let planar = unpack(&words, 2);
        assert_eq!(&planar[0][..left.len()], &left[..]);
        assert_eq!(&planar[1][..right.len()], &right[..]);
    }

    #[test]
    fn seeking_gives_the_same_words_and_markers_as_playing_through() {
        let samples = 2_822_400 / 8;
        let left = modulate(samples, tone(2_822_400, 800.0, 0.5));
        let right = modulate(samples, tone(2_822_400, 1200.0, 0.5));
        let image = dsf(2_822_400, left, right);
        let mut through =
            DopStream::new(DsfReader::open(Cursor::new(image.clone())).unwrap()).unwrap();
        let mut all = Vec::new();
        while through.read_frames(4096, &mut all).unwrap() > 0 {}

        let mut seeked = DopStream::new(DsfReader::open(Cursor::new(image)).unwrap()).unwrap();
        for target in [0_u64, 1, 4097, 12_345] {
            seeked.seek_to_frame(target).unwrap();
            assert_eq!(seeked.position_frames(), target);
            let mut part = Vec::new();
            seeked.read_frames(2000, &mut part).unwrap();
            let start = target as usize * 2;
            assert_eq!(part, all[start..start + part.len()], "from frame {target}");
        }
        seeked.seek_to_frame(u64::MAX).unwrap();
        let mut none = Vec::new();
        assert_eq!(seeked.read_frames(100, &mut none).unwrap(), 0);
        assert_eq!(seeked.position_frames(), through.total_frames());
    }

    #[test]
    fn an_odd_trailing_byte_is_completed_with_the_idle_pattern() {
        // 5 bytes per channel = 2 whole frames plus a half one.
        let left = vec![0x11, 0x22, 0x33, 0x44, 0x55];
        let right = vec![0xA1, 0xA2, 0xA3, 0xA4, 0xA5];
        let image = dsf(2_822_400, left, right);
        let mut stream = DopStream::new(DsfReader::open(Cursor::new(image)).unwrap()).unwrap();
        let mut words = Vec::new();
        let frames = stream.read_frames(100, &mut words).unwrap();
        assert_eq!(frames, 3);
        assert_eq!(words[4], 0x05_5569, "left: 0x55 then idle");
        assert_eq!(words[5], 0x05_A569, "right: 0xA5 then idle");
    }
}
