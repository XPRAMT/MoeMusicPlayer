//! Explicit feedback of an asynchronous USB audio endpoint (ADR 0046).
//!
//! The device reports how many frames it consumes per bus interval (a microframe at high speed, a frame at full
//! speed): 4 bytes Q16.16 at high speed, 3 bytes Q10.14 at full speed (USB 2.0 §5.12.4.2). Some devices send another
//! scaling than the one their speed implies, so the first non-zero value fixes a power-of-two shift that brings it
//! next to the nominal rate (the same idea as the Linux driver's frequency shift). After that a value is accepted only
//! within ±[`FEEDBACK_TOLERANCE`] of the nominal rate; until the first accepted value the caller uses the nominal rate.

/// How far an accepted value may be from the nominal rate. Real DAC clocks are within a few hundred ppm of nominal;
/// ±2 % keeps every real value and rejects garbage (a wrong scaling is a factor of 2 or more).
pub const FEEDBACK_TOLERANCE: f64 = 0.02;

/// After this many rejected values in a row, the scaling is worked out again from the next value.
const REJECTS_BEFORE_RESCALE: u32 = 8;

#[derive(Clone, Debug)]
pub struct Feedback {
    /// Nominal frames per bus interval in Q16.16.
    nominal_q16: f64,
    shift: Option<i32>,
    current_q16: Option<u32>,
    rejected_in_a_row: u32,
    pub accepted: u64,
    pub rejected: u64,
    min_q16: u32,
    max_q16: u32,
    sum_q16: u128,
}

/// What the feedback did over a stream, in frames per bus interval.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackStats {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub avg: Option<f64>,
    pub accepted: u64,
    pub rejected: u64,
}

impl Feedback {
    /// `bus_rate`: bus intervals per second (8000 at high speed, 1000 at full speed).
    pub fn new(rate: u32, bus_rate: u32) -> Self {
        Self {
            nominal_q16: f64::from(rate) * 65_536.0 / f64::from(bus_rate),
            shift: None,
            current_q16: None,
            rejected_in_a_row: 0,
            accepted: 0,
            rejected: 0,
            min_q16: u32::MAX,
            max_q16: 0,
            sum_q16: 0,
        }
    }

    /// The raw value as Q16.16: 4 bytes as they are, 3 bytes (Q10.14) shifted left by two.
    fn raw_q16(bytes: &[u8]) -> Option<u64> {
        match bytes.len() {
            0..=2 => None,
            3 => Some(u64::from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0])) << 2),
            _ => Some(u64::from(u32::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3],
            ]))),
        }
    }

    fn scale(value: u64, shift: i32) -> u64 {
        if shift >= 0 {
            value << shift
        } else {
            value >> (-shift)
        }
    }

    /// Take one feedback packet; returns whether its value is now the current rate.
    pub fn push(&mut self, bytes: &[u8]) -> bool {
        let Some(raw) = Self::raw_q16(bytes).filter(|raw| *raw > 0) else {
            self.reject();
            return false;
        };
        let shift = *self.shift.get_or_insert_with(|| {
            let mut value = raw;
            let mut shift = 0;
            while (value as f64) < self.nominal_q16 * 0.75 && shift < 8 {
                value <<= 1;
                shift += 1;
            }
            while (value as f64) > self.nominal_q16 * 1.5 && shift > -8 {
                value >>= 1;
                shift -= 1;
            }
            shift
        });
        let value = Self::scale(raw, shift);
        let deviation = (value as f64 - self.nominal_q16).abs() / self.nominal_q16;
        let Ok(value) = u32::try_from(value) else {
            self.reject();
            return false;
        };
        if deviation > FEEDBACK_TOLERANCE {
            self.reject();
            return false;
        }
        self.rejected_in_a_row = 0;
        self.accepted += 1;
        self.current_q16 = Some(value);
        self.min_q16 = self.min_q16.min(value);
        self.max_q16 = self.max_q16.max(value);
        self.sum_q16 += u128::from(value);
        true
    }

    fn reject(&mut self) {
        self.rejected += 1;
        self.rejected_in_a_row += 1;
        if self.rejected_in_a_row >= REJECTS_BEFORE_RESCALE {
            self.shift = None;
            self.rejected_in_a_row = 0;
        }
    }

    /// The latest accepted rate (Q16.16 frames per bus interval), `None` before the first.
    pub fn current_q16(&self) -> Option<u32> {
        self.current_q16
    }

    pub fn stats(&self) -> FeedbackStats {
        let frames = |q16: u32| f64::from(q16) / 65_536.0;
        let any = self.accepted > 0;
        FeedbackStats {
            min: any.then(|| frames(self.min_q16)),
            max: any.then(|| frames(self.max_q16)),
            avg: any.then(|| self.sum_q16 as f64 / self.accepted as f64 / 65_536.0),
            accepted: self.accepted,
            rejected: self.rejected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q16(frames: f64) -> u32 {
        (frames * 65_536.0).round() as u32
    }

    #[test]
    fn high_speed_q16_16_values_near_nominal_are_taken() {
        let mut feedback = Feedback::new(44_100, 8000);
        assert_eq!(feedback.current_q16(), None);
        assert!(feedback.push(&q16(5.5126).to_le_bytes()));
        assert_eq!(feedback.current_q16(), Some(q16(5.5126)));
        assert!(feedback.push(&q16(5.5124).to_le_bytes()));
        let stats = feedback.stats();
        assert!((stats.avg.unwrap() - 5.5125).abs() < 1e-4);
        assert!((stats.min.unwrap() - 5.5124).abs() < 1e-4);
        assert!((stats.max.unwrap() - 5.5126).abs() < 1e-4);
        assert_eq!((stats.accepted, stats.rejected), (2, 0));
    }

    #[test]
    fn full_speed_q10_14_values_are_read_from_three_bytes() {
        let mut feedback = Feedback::new(44_100, 1000);
        let value = (44.1_f64 * 16_384.0).round() as u32;
        assert!(feedback.push(&value.to_le_bytes()[..3]));
        assert!((f64::from(feedback.current_q16().unwrap()) / 65_536.0 - 44.1).abs() < 1e-3);
    }

    #[test]
    fn a_device_that_sends_another_scaling_is_shifted_once() {
        // A high-speed device sending Q12.13 (8 times too small as Q16.16).
        let mut feedback = Feedback::new(48_000, 8000);
        assert!(feedback.push(&(q16(6.0) >> 3).to_le_bytes()));
        assert_eq!(feedback.current_q16(), Some(q16(6.0)));
        assert!(feedback.push(&(q16(6.001) >> 3).to_le_bytes()));
    }

    #[test]
    fn values_out_of_tolerance_zero_and_short_packets_are_rejected_and_keep_the_last_value() {
        let mut feedback = Feedback::new(44_100, 8000);
        assert!(feedback.push(&q16(5.5125).to_le_bytes()));
        assert!(!feedback.push(&q16(5.8).to_le_bytes()));
        assert!(!feedback.push(&0_u32.to_le_bytes()));
        assert!(!feedback.push(&[1, 2]));
        assert_eq!(feedback.current_q16(), Some(q16(5.5125)));
        assert_eq!(feedback.stats().rejected, 3);
    }

    #[test]
    fn rejections_before_any_value_leave_the_nominal_rate_to_the_caller() {
        let mut feedback = Feedback::new(44_100, 8000);
        for _ in 0..20 {
            assert!(!feedback.push(&q16(7.0).to_le_bytes()));
        }
        assert_eq!(feedback.current_q16(), None);
        assert_eq!(feedback.stats().avg, None);
        // After the rescale a good value is still taken.
        assert!(feedback.push(&q16(5.5125).to_le_bytes()));
    }
}
