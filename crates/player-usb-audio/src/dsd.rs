//! Native DSD over USB (ADR 0049): the numbers and rules that differ from PCM streaming.
//!
//! The alternate setting is RAW_DATA, 4-byte subslots of 32 bits, stereo; one frame is the left slot and the right
//! slot (`player_dsd::raw`), the clock rate is the DSD bit rate ÷ 32. Nothing here changes the data: there is no gain,
//! no resampling and no channel mixing. What differs from PCM is the fill: silence is the DSD idle pattern, never
//! zeros (a zero word is a full-scale step to a DSD DAC), and the start and the end of a stream are padded with it.

/// DSD "silence": `01101001` per byte, no DC offset. Every byte the engine makes up in DSD mode is this.
pub const IDLE_BYTE: u8 = 0x69;
/// Bytes of one frame on the bus: two 4-byte slots.
pub const FRAME_BYTES: usize = 8;
/// The frame rates DSD64, DSD128 and DSD256 need (DSD bit rate ÷ 32). DSD512 would need 705.6 kHz.
pub const FRAME_RATES: [u32; 3] = [88_200, 176_400, 352_800];

/// Idle fill sent after the alt and the clock are set and before the first music of a new ring.
pub const LEAD_IN_MS: u32 = 150;
/// Idle fill sent before the alt goes back to 0 or the device is handed back.
pub const LEAD_OUT_MS: u32 = 150;
/// Media3 ramps the volume up over the first 20 ms of every new output's data (it scales the samples themselves, which
/// are DSD words here); the output drops this much of the start of every new ring and sends idle fill instead.
pub const RAMP_DISCARD_MS: u32 = 25;

/// Whole frames covering `ms` at `rate` (rounded up).
pub fn frames_for_ms(rate: u32, ms: u32) -> usize {
    (u64::from(rate) * u64::from(ms)).div_ceil(1000) as usize
}

pub fn lead_in_frames(rate: u32) -> usize {
    frames_for_ms(rate, LEAD_IN_MS)
}

pub fn lead_out_frames(rate: u32) -> usize {
    frames_for_ms(rate, LEAD_OUT_MS)
}

/// Frames dropped from the start of a new ring. A frame is two whole slots, so the cut never splits a slot.
pub fn discard_frames(rate: u32) -> usize {
    frames_for_ms(rate, RAMP_DISCARD_MS)
}

/// The pacing of a native-DSD stream's start and end: idle fill for the lead-in, then the music, then idle fill again
/// for the lead-out (`silence_only`).
#[derive(Clone, Debug)]
pub struct Gate {
    lead_in_left: usize,
    silence_only: bool,
}

impl Gate {
    /// A stream that has just been started at `rate` frames per second.
    pub fn new(rate: u32) -> Self {
        Self {
            lead_in_left: lead_in_frames(rate),
            silence_only: false,
        }
    }

    /// The lead-out begins: idle fill only from now on.
    pub fn start_lead_out(&mut self) {
        self.silence_only = true;
    }

    /// Account for a packet of `frames`; whether it is idle fill (the lead-in, or the lead-out) and must not read the
    /// ring. A packet that starts inside the lead-in is entirely idle.
    pub fn next_packet_silent(&mut self, frames: usize) -> bool {
        let silent = self.silence_only || self.lead_in_left > 0;
        self.lead_in_left = self.lead_in_left.saturating_sub(frames);
        silent
    }
}

/// One packet of native DSD: `out` is a whole number of frames; it gets the ring's bytes unchanged (no gain, no
/// repacking) unless `silent`, and whatever the ring did not have is the idle pattern, never zeros. Returns the bytes
/// taken from the ring (what the packet's credit is).
pub fn fill_packet(
    cursor: &mut crate::ring::Cursor,
    ring: &crate::ring::Ring,
    silent: bool,
    out: &mut [u8],
) -> usize {
    let got = if silent {
        0
    } else {
        cursor.take(ring, out.len(), out)
    };
    out[got..].fill(IDLE_BYTE);
    got
}

/// The byte a gap is filled with: the idle pattern for DSD, zero for PCM.
pub fn fill_byte(dsd: bool) -> u8 {
    if dsd {
        IDLE_BYTE
    } else {
        0
    }
}

/// Whether `rate` is a frame rate native DSD can use.
pub fn is_frame_rate(rate: u32) -> bool {
    FRAME_RATES.contains(&rate)
}

/// `88200` as `88.2`, `352800` as `352.8` (the card's wording).
pub fn khz_text(rate: u32) -> String {
    format!("{}", f64::from(rate) / 1000.0)
}

/// The DSD name of a frame rate (`88200` → `DSD64`).
pub fn dsd_name(rate: u32) -> Option<&'static str> {
    match rate {
        88_200 => Some("DSD64"),
        176_400 => Some("DSD128"),
        352_800 => Some("DSD256"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_idle_pattern_is_the_one_player_dsd_pads_with() {
        assert_eq!(IDLE_BYTE, player_dsd::stream::DSD_IDLE_BYTE);
        assert_eq!(fill_byte(true), 0x69);
        assert_eq!(fill_byte(false), 0);
    }

    #[test]
    fn the_frame_rates_are_the_dsd_bit_rates_over_32() {
        assert_eq!(
            FRAME_RATES,
            [2_822_400 / 32, 5_644_800 / 32, 11_289_600 / 32]
        );
        assert!(is_frame_rate(88_200) && is_frame_rate(176_400) && is_frame_rate(352_800));
        assert!(!is_frame_rate(705_600) && !is_frame_rate(44_100));
        assert_eq!(dsd_name(176_400), Some("DSD128"));
        assert_eq!(dsd_name(96_000), None);
        assert_eq!(khz_text(88_200), "88.2");
        assert_eq!(khz_text(176_400), "176.4");
        assert_eq!(khz_text(352_800), "352.8");
    }

    #[test]
    fn lead_in_and_out_are_between_100_and_200_ms_and_the_discard_covers_the_ramp() {
        for rate in FRAME_RATES {
            for frames in [lead_in_frames(rate), lead_out_frames(rate)] {
                let ms = frames as f64 * 1000.0 / f64::from(rate);
                assert!((100.0..=200.0).contains(&ms), "{rate}: {ms}");
            }
            // The ramp is 20 ms of frames; the discard is longer and a whole number of slots.
            let ramp = frames_for_ms(rate, 20);
            let discard = discard_frames(rate);
            assert!(discard > ramp, "{rate}");
            assert_eq!(discard * FRAME_BYTES % 4, 0, "whole slots");
            let ms = discard as f64 * 1000.0 / f64::from(rate);
            assert!((25.0..25.1).contains(&ms), "{rate}: {ms}");
        }
        assert_eq!(discard_frames(88_200), 2_205);
        assert_eq!(lead_in_frames(88_200), 13_230);
    }

    use crate::ring::{Cursor, Ring};

    /// Run `packets` packets of `frames` frames through a gate and a ring the way the stream does.
    fn run(
        gate: &mut Gate,
        ring: &Ring,
        cursor: &mut Cursor,
        packets: usize,
        frames: usize,
    ) -> Vec<(bool, Vec<u8>)> {
        (0..packets)
            .map(|_| {
                let silent = gate.next_packet_silent(frames);
                let mut out = vec![0_u8; frames * FRAME_BYTES];
                let got = fill_packet(cursor, ring, silent, &mut out);
                Cursor::complete(ring, got);
                (silent, out)
            })
            .collect()
    }

    #[test]
    fn a_new_stream_sends_150_ms_of_idle_before_the_first_music() {
        let rate = 88_200;
        let ring = Ring::new_dsd(50_000, rate);
        // Music already waiting (past the discard) must still wait for the lead-in.
        let discard = discard_frames(rate) * FRAME_BYTES;
        assert_eq!(ring.push(&vec![0x33; discard + 80_000]), discard + 80_000);
        let mut cursor = Cursor::at(&ring);
        let mut gate = Gate::new(rate);
        // 11 frames per packet, as 88.2 kHz at 8000 packets/s averages out (11.025).
        let packets = run(&mut gate, &ring, &mut cursor, 1_500, 11);
        let first_music = packets.iter().position(|(silent, _)| !silent).unwrap();
        let idle_frames = first_music * 11;
        let ms = idle_frames as f64 * 1000.0 / f64::from(rate);
        assert!((150.0..151.0).contains(&ms), "{ms} ms");
        for (index, (silent, bytes)) in packets.iter().enumerate() {
            assert!(!bytes.contains(&0), "packet {index} has a zero byte");
            if index < first_music {
                assert!(*silent && bytes.iter().all(|&b| b == IDLE_BYTE));
            }
        }
        // After the lead-in come the discarded 25 ms (idle again), then the music itself.
        let flat: Vec<u8> = packets[first_music..]
            .iter()
            .flat_map(|(_, bytes)| bytes.iter().copied())
            .collect();
        assert!(flat[..discard].iter().all(|&b| b == IDLE_BYTE));
        assert_eq!(flat[discard], 0x33);
    }

    #[test]
    fn an_empty_ring_and_the_lead_out_are_idle_never_zero() {
        let rate = 176_400;
        let ring = Ring::new_dsd(1_000, rate);
        let mut cursor = Cursor::at(&ring);
        let mut gate = Gate::new(rate);
        // Underrun / nothing written yet: idle in every packet, also past the lead-in.
        let packets = run(&mut gate, &ring, &mut cursor, 3_000, 22);
        assert!(packets
            .iter()
            .all(|(_, bytes)| bytes.iter().all(|&b| b == IDLE_BYTE)));
        // Music, then the lead-out: the ring is not read any more.
        let discard = discard_frames(rate) * FRAME_BYTES;
        ring.push(&vec![0x44; discard + 800]);
        let before = run(&mut gate, &ring, &mut cursor, 2, 22);
        assert!(before.iter().all(|(silent, _)| !silent));
        gate.start_lead_out();
        let after = run(&mut gate, &ring, &mut cursor, 50, 22);
        assert!(after
            .iter()
            .all(|(silent, bytes)| *silent && bytes.iter().all(|&b| b == IDLE_BYTE)));
        assert!(
            cursor.unsent(&ring) > 0,
            "the music not yet sent stays in the ring"
        );
    }

    #[test]
    fn a_partial_packet_is_music_then_idle() {
        let rate = 88_200;
        let ring = Ring::new_dsd(5_000, rate);
        let mut cursor = Cursor::at(&ring);
        // Skip the discard with a first write, then 3 frames of music for an 11-frame packet.
        let discard = discard_frames(rate) * FRAME_BYTES;
        ring.push(&vec![0x11; discard]);
        let mut sink = vec![0_u8; discard];
        cursor.take(&ring, discard, &mut sink);
        Cursor::complete(&ring, discard);
        ring.push(&[0x77; 3 * FRAME_BYTES]);
        let mut out = vec![0_u8; 11 * FRAME_BYTES];
        let got = fill_packet(&mut cursor, &ring, false, &mut out);
        assert_eq!(got, 3 * FRAME_BYTES);
        assert!(out[..got].iter().all(|&b| b == 0x77));
        assert!(out[got..].iter().all(|&b| b == IDLE_BYTE));
    }
}
