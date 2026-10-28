//! How many frames go into each isochronous packet (ADR 0046).
//!
//! A fractional accumulator in exact integers: one unit is 1 / (packets per second × 65 536) of a frame, so both the
//! nominal rate (`rate` frames per second) and a Q16.16 feedback value (frames per bus interval) add a whole number
//! of units per packet and nothing is lost to rounding: over any time the frames sent are the rate × time rounded
//! down, never drifting. A packet never exceeds the endpoint's size; what did not fit is carried into the next ones.

#[derive(Clone, Debug)]
pub struct PacketSizer {
    rate: u64,
    packets_per_second: u64,
    bus_intervals_per_packet: u64,
    max_frames: u64,
    accumulator: u64,
}

impl PacketSizer {
    /// `bus_rate`: bus intervals per second (8000 high speed, 1000 full speed); `endpoint_interval`: the data
    /// endpoint's bInterval (a packet every 2^(bInterval−1) bus intervals); `max_frames`: frames that fit a packet.
    pub fn new(rate: u32, bus_rate: u32, endpoint_interval: u8, max_frames: usize) -> Self {
        let exponent = u32::from(endpoint_interval.clamp(1, 16) - 1);
        let bus_intervals_per_packet = 1_u64 << exponent;
        Self {
            rate: u64::from(rate),
            packets_per_second: (u64::from(bus_rate) / bus_intervals_per_packet).max(1),
            bus_intervals_per_packet,
            max_frames: max_frames as u64,
            accumulator: 0,
        }
    }

    pub fn packets_per_second(&self) -> u64 {
        self.packets_per_second
    }

    fn unit(&self) -> u64 {
        self.packets_per_second * 65_536
    }

    /// Frames for the next packet, at the feedback rate (Q16.16 frames per bus interval) when there is one, else at
    /// the nominal rate.
    pub fn next_frames(&mut self, feedback_q16: Option<u32>) -> usize {
        let increment = match feedback_q16 {
            Some(value) => {
                u64::from(value) * self.bus_intervals_per_packet * self.packets_per_second
            }
            None => self.rate * 65_536,
        };
        self.accumulator += increment;
        let unit = self.unit();
        let frames = (self.accumulator / unit).min(self.max_frames);
        self.accumulator -= frames * unit;
        // Never carry more than one full packet: a device asking for more than fits is not followed further.
        self.accumulator = self.accumulator.min(self.max_frames * unit);
        frames as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_44100_high_speed_the_frames_match_the_rate_exactly_over_ten_minutes() {
        let mut sizer = PacketSizer::new(44_100, 8000, 1, 256);
        assert_eq!(sizer.packets_per_second(), 8000);
        let mut total = 0_u64;
        for second in 1..=600_u64 {
            for _ in 0..8000 {
                let frames = sizer.next_frames(None);
                assert!(frames == 5 || frames == 6, "{frames}");
                total += frames as u64;
            }
            assert_eq!(total, 44_100 * second);
        }
    }

    #[test]
    fn every_listed_rate_stays_exact_at_high_and_full_speed() {
        for rate in [
            32_000_u32, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000,
        ] {
            for (bus, interval) in [(8000, 1), (8000, 4), (1000, 1)] {
                let mut sizer = PacketSizer::new(rate, bus, interval, 4096);
                let pps = sizer.packets_per_second();
                let total: u64 = (0..pps * 7).map(|_| sizer.next_frames(None) as u64).sum();
                assert_eq!(total, u64::from(rate) * 7, "{rate} {bus} {interval}");
            }
        }
    }

    #[test]
    fn a_feedback_rate_is_followed_within_one_frame() {
        let feedback = (5.5126_f64 * 65_536.0).round() as u32;
        let mut sizer = PacketSizer::new(44_100, 8000, 1, 256);
        let packets = 8000 * 120_u64;
        let total: u64 = (0..packets)
            .map(|_| sizer.next_frames(Some(feedback)) as u64)
            .sum();
        let expected = packets as f64 * f64::from(feedback) / 65_536.0;
        assert!(
            (total as f64 - expected).abs() <= 1.0,
            "{total} vs {expected}"
        );
    }

    #[test]
    fn a_packet_every_four_microframes_carries_four_microframes_of_feedback() {
        let feedback = 6 * 65_536; // 48 kHz
        let mut sizer = PacketSizer::new(48_000, 8000, 3, 256);
        assert_eq!(sizer.packets_per_second(), 2000);
        assert_eq!(sizer.next_frames(Some(feedback)), 24);
        assert_eq!(sizer.next_frames(None), 24);
    }

    #[test]
    fn a_packet_never_exceeds_the_endpoint() {
        let mut sizer = PacketSizer::new(384_000, 8000, 1, 40);
        for _ in 0..100 {
            assert!(sizer.next_frames(None) <= 40);
        }
        let mut small = PacketSizer::new(44_100, 8000, 1, 5);
        let frames: Vec<usize> = (0..10)
            .map(|_| small.next_frames(Some(u32::MAX / 1024)))
            .collect();
        assert!(frames.iter().all(|f| *f <= 5));
    }
}
