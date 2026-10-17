//! Filter design for the DSD→PCM converter (ADR 0016): Kaiser-windowed low-pass prototypes, the stage-A lookup
//! tables and the halfband tap sets. Everything is designed once, in `f64`, and stored as `f32`.

use std::f64::consts::PI;

/// Kaiser β for about 110 dB of stopband attenuation.
pub(crate) const KAISER_BETA: f64 = 11.2;
/// Transition width of a Kaiser filter in cycles/sample for `taps` coefficients at [`KAISER_BETA`]:
/// `(A − 7.95) / (14.36 · taps)` with `A ≈ 110.3 dB`.
const KAISER_TRANSITION_NUMERATOR: f64 = 7.13;

/// Highest audio frequency the converter keeps flat (±0.01 dB). Content above it only has to be removed, not
/// preserved, so the filters can be short.
pub(crate) const PASSBAND_EDGE_HZ: f64 = 40_000.0;

/// Taps of the stage-A prototype and its lookup-table groups (8 taps per input byte).
pub(crate) const STAGE_A_GROUPS: usize = 16;
pub(crate) const STAGE_A_TAPS: usize = STAGE_A_GROUPS * 8;

/// Zeroth-order modified Bessel function of the first kind (power series).
pub(crate) fn bessel_i0(x: f64) -> f64 {
    let half = x / 2.0;
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..64 {
        term *= (half / f64::from(k)) * (half / f64::from(k));
        sum += term;
        if term < sum * 1e-18 {
            break;
        }
    }
    sum
}

/// Linear-phase low-pass: windowed sinc with cutoff `cutoff` (cycles/sample, 0 < cutoff < 0.5), Kaiser window,
/// normalised to unity gain at DC.
pub(crate) fn lowpass_kaiser(taps: usize, cutoff: f64, beta: f64) -> Vec<f64> {
    let center = (taps as f64 - 1.0) / 2.0;
    let denominator = bessel_i0(beta);
    let mut coefficients: Vec<f64> = (0..taps)
        .map(|index| {
            let offset = index as f64 - center;
            let ideal = if offset == 0.0 {
                2.0 * cutoff
            } else {
                (2.0 * PI * cutoff * offset).sin() / (PI * offset)
            };
            let ratio = if center == 0.0 { 0.0 } else { offset / center };
            let window = bessel_i0(beta * (1.0 - ratio * ratio).max(0.0).sqrt()) / denominator;
            ideal * window
        })
        .collect();
    let sum: f64 = coefficients.iter().sum();
    for coefficient in &mut coefficients {
        *coefficient /= sum;
    }
    coefficients
}

/// Stage A: ÷8 by byte-wise lookup. `tables[g][byte]` is the contribution of the byte that arrived `g` bytes ago
/// to the current output: the sum of that group's eight taps, each weighted +1 for a set bit and −1 for a clear
/// bit. Bytes are MSB-first, so sample `i` of a byte is bit `7 − i`, and the tap for the sample `8g + 7 − i`
/// samples before the newest one is `h[8g + 7 − i]`.
pub(crate) fn stage_a_tables() -> Vec<[f32; 256]> {
    let prototype = lowpass_kaiser(STAGE_A_TAPS, 1.0 / 16.0, KAISER_BETA);
    (0..STAGE_A_GROUPS)
        .map(|group| {
            let mut table = [0.0_f32; 256];
            for (byte, entry) in table.iter_mut().enumerate() {
                let mut sum = 0.0_f64;
                for sample in 0..8 {
                    let bit = (byte >> (7 - sample)) & 1;
                    let sign = if bit == 1 { 1.0 } else { -1.0 };
                    sum += sign * prototype[group * 8 + 7 - sample];
                }
                *entry = sum as f32;
            }
            table
        })
        .collect()
}

/// Group delay of stage A in DSD samples, measured from the newest sample of the byte it is evaluated at.
pub(crate) const STAGE_A_DELAY_SAMPLES: f64 = (STAGE_A_TAPS as f64 - 1.0) / 2.0;

/// Number of taps (of the form 4k + 3, at least 15) of a halfband stage whose input rate is `input_rate`, so that
/// everything above `input_rate / 2 − PASSBAND_EDGE_HZ` is removed and everything below `PASSBAND_EDGE_HZ` is
/// kept, with 100% margin on the transition width.
pub(crate) fn halfband_tap_count(input_rate: f64) -> usize {
    let required_transition = 0.5 - 2.0 * PASSBAND_EDGE_HZ / input_rate;
    let minimum = (KAISER_TRANSITION_NUMERATOR / required_transition * 2.0).ceil() as usize;
    let mut taps = minimum.max(15);
    while taps % 4 != 3 {
        taps += 1;
    }
    taps
}

/// Halfband prototype: the centre tap and the taps at odd offsets `1, 3, 5, ...` on one side of it (the filter is
/// symmetric and all other taps are zero). DC gain is exactly 1.
#[derive(Clone, Debug)]
pub(crate) struct HalfbandTaps {
    pub(crate) center: f32,
    /// `odd[j]` is the tap at offset `2j + 1` from the centre.
    pub(crate) odd: Vec<f32>,
    /// Taps on each side of the centre (`(N − 1) / 2`).
    pub(crate) half_span: usize,
}

pub(crate) fn halfband_taps(taps: usize) -> HalfbandTaps {
    debug_assert_eq!(taps % 4, 3);
    let prototype = lowpass_kaiser(taps, 0.25, KAISER_BETA);
    let center_index = (taps - 1) / 2;
    let mut center = prototype[center_index];
    let mut odd: Vec<f64> = (0..=center_index / 2)
        .filter_map(|j| prototype.get(center_index + 2 * j + 1).copied())
        .collect();
    // Even offsets are zero in theory; renormalise so DC gain is exactly 1 with them removed.
    let sum = center + 2.0 * odd.iter().sum::<f64>();
    center /= sum;
    for tap in &mut odd {
        *tap /= sum;
    }
    HalfbandTaps {
        center: center as f32,
        odd: odd.into_iter().map(|tap| tap as f32).collect(),
        half_span: center_index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_db(coefficients: &[f64], frequency: f64) -> f64 {
        let (mut real, mut imaginary) = (0.0, 0.0);
        for (index, coefficient) in coefficients.iter().enumerate() {
            let angle = -2.0 * PI * frequency * index as f64;
            real += coefficient * angle.cos();
            imaginary += coefficient * angle.sin();
        }
        20.0 * real.hypot(imaginary).log10()
    }

    #[test]
    fn bessel_matches_known_values() {
        assert!((bessel_i0(0.0) - 1.0).abs() < 1e-15);
        assert!((bessel_i0(1.0) - 1.266_065_877_752_008_4).abs() < 1e-12);
        assert!((bessel_i0(5.0) - 27.239_871_823_604_44).abs() < 1e-9);
    }

    #[test]
    fn prototype_has_unity_dc_gain_and_is_symmetric() {
        let taps = lowpass_kaiser(128, 1.0 / 16.0, KAISER_BETA);
        assert!((taps.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        for index in 0..taps.len() / 2 {
            assert!((taps[index] - taps[taps.len() - 1 - index]).abs() < 1e-15);
        }
    }

    #[test]
    fn stage_a_prototype_meets_passband_and_stopband_for_every_dsd_rate() {
        let taps = lowpass_kaiser(STAGE_A_TAPS, 1.0 / 16.0, KAISER_BETA);
        for rate in [2_822_400.0, 5_644_800.0, 11_289_600.0, 22_579_200.0] {
            let passband_edge = PASSBAND_EDGE_HZ / rate;
            let stop_edge = 1.0 / 8.0 - passband_edge;
            let mut worst_ripple: f64 = 0.0;
            for step in 0..=200 {
                let frequency = passband_edge * f64::from(step) / 200.0;
                worst_ripple = worst_ripple.max(response_db(&taps, frequency).abs());
            }
            assert!(
                worst_ripple < 0.01,
                "{rate}: passband ripple {worst_ripple} dB"
            );
            let mut best_rejection = f64::MAX;
            let mut frequency = stop_edge;
            while frequency <= 0.5 {
                best_rejection = best_rejection.min(-response_db(&taps, frequency));
                frequency += 0.001;
            }
            assert!(
                best_rejection > 105.0,
                "{rate}: stopband {best_rejection} dB"
            );
        }
    }

    #[test]
    fn halfband_tap_counts_shrink_as_the_input_rate_grows() {
        let counts: Vec<usize> = [352_800.0, 705_600.0, 1_411_200.0, 2_822_400.0]
            .iter()
            .map(|rate| halfband_tap_count(*rate))
            .collect();
        for count in &counts {
            assert_eq!(count % 4, 3);
            assert!(*count >= 15);
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] >= pair[1]),
            "{counts:?}"
        );
        assert!(counts[0] < 64, "{counts:?}");
    }

    #[test]
    fn halfband_meets_passband_and_stopband_at_every_stage_rate() {
        for input_rate in [352_800.0, 705_600.0, 1_411_200.0, 2_822_400.0] {
            let count = halfband_tap_count(input_rate);
            let taps = halfband_taps(count);
            // Rebuild the full symmetric impulse response.
            let mut full = vec![0.0_f64; count];
            full[taps.half_span] = f64::from(taps.center);
            for (j, tap) in taps.odd.iter().enumerate() {
                full[taps.half_span + 2 * j + 1] = f64::from(*tap);
                full[taps.half_span - 2 * j - 1] = f64::from(*tap);
            }
            assert!((full.iter().sum::<f64>() - 1.0).abs() < 1e-6);
            let passband_edge = PASSBAND_EDGE_HZ / input_rate;
            let stop_edge = 0.5 - passband_edge;
            let mut worst_ripple: f64 = 0.0;
            for step in 0..=100 {
                worst_ripple = worst_ripple
                    .max(response_db(&full, passband_edge * f64::from(step) / 100.0).abs());
            }
            assert!(
                worst_ripple < 0.01,
                "{input_rate}: ripple {worst_ripple} dB"
            );
            let mut best_rejection = f64::MAX;
            let mut frequency = stop_edge;
            while frequency <= 0.5 {
                best_rejection = best_rejection.min(-response_db(&full, frequency));
                frequency += 0.0005;
            }
            assert!(
                best_rejection > 100.0,
                "{input_rate}: stopband {best_rejection} dB"
            );
        }
    }
}
