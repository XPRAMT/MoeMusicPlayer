//! Which streaming alt plays a track, and whether what reaches the bus is the file's samples (ADR 0047).
//!
//! No resampling and no channel mixing: the track's rate and channel count must be the device's. The alt follows the
//! source's bit depth (taken from the track, because Media3 hands the output float PCM whatever the file was): 16-bit
//! prefers the 2-byte alt, 24-bit the 3-byte packed one and else 4 bytes with 24 valid bits, 32-bit the 4-byte 32-bit
//! one; a smaller source in a larger container is padded with zero low bits, which keeps it exact.

use crate::descriptors::{plan_stream, Configuration, PcmFormat, PlanError, StreamPlan};
use crate::pcm::{Container, InputEncoding};

/// What the track is, as the playback service knows it from the library.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Source {
    /// The file's bits per sample (lossless codecs only; `None`: unknown, e.g. a lossy file).
    pub bits: Option<u8>,
    /// A DSD file, played through the DSD-to-PCM converter (DSD over the bus is not done).
    pub dsd: bool,
}

const C2_16: Container = Container { bytes: 2, bits: 16 };
const C3_16: Container = Container { bytes: 3, bits: 16 };
const C4_16: Container = Container { bytes: 4, bits: 16 };
const C3_24: Container = Container { bytes: 3, bits: 24 };
const C4_24: Container = Container { bytes: 4, bits: 24 };
const C4_32: Container = Container { bytes: 4, bits: 32 };

/// The containers to try for a source, best first.
pub fn container_preference(source: Source) -> &'static [Container] {
    match source.bits {
        _ if source.dsd => &[C3_24, C4_24, C4_32],
        Some(16) => &[C2_16, C3_24, C4_24, C4_32, C3_16, C4_16],
        Some(32) => &[C4_32, C4_24, C3_24],
        // 24-bit, and an unknown depth (a lossy decoder's float output) is given 24 bits.
        _ => &[C3_24, C4_24, C4_32],
    }
}

/// The plan for a track, or why the device cannot take it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Unsupported {
    Plan(PlanError),
    /// No PCM alt for this channel count.
    Channels(u8),
    /// No PCM alt with a container that holds the source.
    Bits(Option<u8>),
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan(error) => error.fmt(f),
            Self::Channels(channels) => write!(f, "USB 音訊裝置沒有 {channels} 聲道的 PCM 格式"),
            Self::Bits(Some(bits)) => write!(f, "USB 音訊裝置沒有可放 {bits} bit 的 PCM 格式"),
            Self::Bits(None) => f.write_str("USB 音訊裝置沒有合適的 PCM 格式"),
        }
    }
}

/// Choose the alt for `rate` and `channels` of `source`. UAC1 alts list their rates (checked here); a UAC2 rate is
/// checked against the clock's RANGE when the device is configured (or `ranges`, when it was read before).
pub fn choose(
    config: &Configuration,
    rate: u32,
    channels: u8,
    source: Source,
    ranges: Option<&[(u32, u32, u32)]>,
) -> Result<(StreamPlan, Container), Unsupported> {
    if let Some(ranges) = ranges.filter(|ranges| !ranges.is_empty()) {
        if !crate::uac::rate_in_ranges(ranges, rate) {
            return Err(Unsupported::Plan(PlanError::RateNotListed(rate)));
        }
    }
    for &container in container_preference(source) {
        match plan_stream(config, pcm_format(container, channels, rate)) {
            Ok(plan) => return Ok((plan, container)),
            Err(PlanError::NoMatchingFormat(_)) => {}
            // Not about the format (no feedback endpoint, a UAC1 rate not listed, …): the same for every container.
            Err(error) => return Err(Unsupported::Plan(error)),
        }
    }
    let any_container = [C2_16, C3_16, C4_16, C3_24, C4_24, C4_32]
        .into_iter()
        .any(|container| plan_stream(config, pcm_format(container, channels, rate)).is_ok());
    Err(if any_container {
        Unsupported::Bits(source.bits)
    } else {
        Unsupported::Channels(channels)
    })
}

fn pcm_format(container: Container, channels: u8, rate: u32) -> PcmFormat {
    PcmFormat {
        subslot_bytes: container.bytes,
        bits: container.bits,
        channels,
        rate,
    }
}

/// The clock RANGEs read from devices in this process, by vendor and product id: once a device was claimed, a rate it
/// does not take is known before it is claimed again (the track then plays through Android at once).
type KnownRanges = Vec<((u16, u16), Vec<(u32, u32, u32)>)>;
static RANGES: std::sync::Mutex<KnownRanges> = std::sync::Mutex::new(Vec::new());

pub fn remember_ranges(vendor_id: u16, product_id: u16, ranges: Vec<(u32, u32, u32)>) {
    let mut known = RANGES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    known.retain(|(key, _)| *key != (vendor_id, product_id));
    known.push(((vendor_id, product_id), ranges));
}

pub fn known_ranges(vendor_id: u16, product_id: u16) -> Option<Vec<(u32, u32, u32)>> {
    RANGES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .find(|(key, _)| *key == (vendor_id, product_id))
        .map(|(_, ranges)| ranges.clone())
}

/// Why the samples on the bus are not the file's.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NotBitPerfect {
    /// The app volume is below 100 %: a gain is applied.
    Volume,
    /// A 32-bit integer source reached the output as float (24-bit significand).
    Float32,
    /// The source depth is not known (a lossy file).
    UnknownBits,
    /// DSD converted to PCM.
    Dsd,
    /// The container has fewer valid bits than the source.
    Truncated,
}

/// Whether the bus carries the file's samples bit for bit, and if not, why.
pub fn bit_perfect(
    source: Source,
    encoding: InputEncoding,
    container: Container,
    gain: f32,
) -> Vec<NotBitPerfect> {
    let mut reasons = Vec::new();
    if source.dsd {
        reasons.push(NotBitPerfect::Dsd);
    } else {
        match source.bits {
            None => reasons.push(NotBitPerfect::UnknownBits),
            Some(bits) if bits > container.bits => reasons.push(NotBitPerfect::Truncated),
            Some(bits) if bits > 24 && encoding == InputEncoding::F32 => {
                reasons.push(NotBitPerfect::Float32)
            }
            Some(_) => {}
        }
    }
    if gain != 1.0 {
        reasons.push(NotBitPerfect::Volume);
    }
    reasons
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::{parse_configuration, tests::sony as sony_raw};

    fn sony() -> Configuration {
        parse_configuration(&sony_raw())
    }

    fn alt_of(source: Source) -> (u8, Container) {
        let (plan, container) = choose(&sony(), 44_100, 2, source, None).unwrap();
        (plan.alt, container)
    }

    #[test]
    fn the_alt_follows_the_source_bit_depth_on_the_sony_device() {
        // Alts: 1 = 4/32, 2 = 4/24, 3 = 4/16, 4 = 3/24, 5 = 3/16, 6 = 2/16.
        let bits = |bits| Source {
            bits: Some(bits),
            dsd: false,
        };
        assert_eq!(alt_of(bits(16)), (6, C2_16));
        assert_eq!(alt_of(bits(24)), (4, C3_24));
        assert_eq!(alt_of(bits(32)), (1, C4_32));
        assert_eq!(alt_of(Source::default()), (4, C3_24));
        assert_eq!(
            alt_of(Source {
                bits: Some(1),
                dsd: true
            }),
            (4, C3_24)
        );
    }

    #[test]
    fn channels_and_known_ranges_decide_support() {
        assert_eq!(
            choose(&sony(), 44_100, 6, Source::default(), None).unwrap_err(),
            Unsupported::Channels(6)
        );
        let ranges = [(32_000, 384_000, 100)];
        assert!(choose(&sony(), 44_100, 2, Source::default(), Some(&ranges)).is_ok());
        assert_eq!(
            choose(
                &sony(),
                44_150,
                2,
                Source::default(),
                Some(&[(44_100, 48_000, 3_900)])
            )
            .unwrap_err(),
            Unsupported::Plan(PlanError::RateNotListed(44_150))
        );
        assert!(Unsupported::Channels(6).to_string().contains('6'));
    }

    #[test]
    fn ranges_read_once_are_remembered_per_device() {
        assert_eq!(known_ranges(0x1234, 0x0001), None);
        remember_ranges(0x1234, 0x0001, vec![(44_100, 48_000, 3_900)]);
        remember_ranges(0x1234, 0x0001, vec![(44_100, 44_100, 0)]);
        assert_eq!(
            known_ranges(0x1234, 0x0001),
            Some(vec![(44_100, 44_100, 0)])
        );
        assert_eq!(known_ranges(0x1234, 0x0002), None);
    }

    #[test]
    fn bit_perfect_needs_a_known_integer_source_that_fits_and_unity_gain() {
        let f = InputEncoding::F32;
        let s = |bits| Source {
            bits: Some(bits),
            dsd: false,
        };
        assert!(bit_perfect(s(16), f, C2_16, 1.0).is_empty());
        assert!(bit_perfect(s(16), f, C3_24, 1.0).is_empty());
        assert!(bit_perfect(s(24), f, C4_24, 1.0).is_empty());
        assert_eq!(
            bit_perfect(s(16), f, C2_16, 0.99),
            vec![NotBitPerfect::Volume]
        );
        assert_eq!(
            bit_perfect(s(32), f, C4_32, 1.0),
            vec![NotBitPerfect::Float32]
        );
        assert!(bit_perfect(s(32), InputEncoding::S32, C4_32, 1.0).is_empty());
        assert_eq!(
            bit_perfect(s(32), f, C3_24, 1.0),
            vec![NotBitPerfect::Truncated]
        );
        assert_eq!(
            bit_perfect(Source::default(), f, C3_24, 0.5),
            vec![NotBitPerfect::UnknownBits, NotBitPerfect::Volume]
        );
        assert_eq!(
            bit_perfect(
                Source {
                    bits: Some(1),
                    dsd: true
                },
                f,
                C3_24,
                1.0
            ),
            vec![NotBitPerfect::Dsd]
        );
    }
}
