//! The parts of the USB Audio Class 1.0 / 2.0 descriptors that the exclusive output needs (ADR 0046), written from
//! the class specifications. Read only and total: a truncated or unknown descriptor is skipped, never panicked on.
//!
//! The input is what Android's `UsbDeviceConnection.getRawDescriptors()` returns: the device descriptor followed by
//! the configuration descriptors. Only the first configuration is read (the one Android activates).

const DT_DEVICE: u8 = 0x01;
const DT_CONFIG: u8 = 0x02;
const DT_INTERFACE: u8 = 0x04;
const DT_ENDPOINT: u8 = 0x05;
const DT_CS_INTERFACE: u8 = 0x24;

const CLASS_AUDIO: u8 = 0x01;
const SUBCLASS_AUDIO_CONTROL: u8 = 0x01;
const SUBCLASS_AUDIO_STREAMING: u8 = 0x02;

/// One endpoint descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Endpoint {
    pub address: u8,
    pub attributes: u8,
    /// wMaxPacketSize as sent (bits 0–10 size, bits 11–12 extra high-bandwidth transactions).
    pub max_packet_raw: u16,
    pub interval: u8,
    /// UAC1 audio endpoints (9 bytes): the address of the synch (feedback) endpoint, 0 when none.
    pub synch_address: u8,
}

/// Endpoint synchronisation type (bmAttributes bits 2–3).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncType {
    None,
    Async,
    Adaptive,
    Sync,
}

/// Endpoint usage type (bmAttributes bits 4–5).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Usage {
    Data,
    Feedback,
    ImplicitFeedback,
    Reserved,
}

impl Endpoint {
    pub fn is_in(&self) -> bool {
        self.address & 0x80 != 0
    }

    pub fn is_isochronous(&self) -> bool {
        self.attributes & 0x03 == 0x01
    }

    pub fn sync(&self) -> SyncType {
        match (self.attributes >> 2) & 0x03 {
            1 => SyncType::Async,
            2 => SyncType::Adaptive,
            3 => SyncType::Sync,
            _ => SyncType::None,
        }
    }

    pub fn usage(&self) -> Usage {
        match (self.attributes >> 4) & 0x03 {
            0 => Usage::Data,
            1 => Usage::Feedback,
            2 => Usage::ImplicitFeedback,
            _ => Usage::Reserved,
        }
    }

    /// Bytes per service interval: the packet size times (1 + extra high-bandwidth transactions).
    pub fn max_bytes_per_interval(&self) -> usize {
        let size = usize::from(self.max_packet_raw & 0x07FF);
        let extra = usize::from((self.max_packet_raw >> 11) & 0x03);
        size * (1 + extra.min(2))
    }
}

/// One interface descriptor (one alternate setting) with what follows it up to the next interface descriptor.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AltSetting {
    pub interface: u8,
    pub alt: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    /// Class-specific interface descriptors (CS_INTERFACE), whole, in order.
    pub class_specific: Vec<Vec<u8>>,
    pub endpoints: Vec<Endpoint>,
}

impl AltSetting {
    fn is_audio(&self, subclass: u8) -> bool {
        self.class == CLASS_AUDIO && self.subclass == subclass
    }
}

/// The device descriptor's version and the first configuration, split into alternate settings.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Configuration {
    /// bcdUSB of the device descriptor (0x0200 for USB 2.0), 0 when absent.
    pub bcd_usb: u16,
    pub vendor_id: u16,
    pub product_id: u16,
    pub alts: Vec<AltSetting>,
}

/// Walk the raw descriptors by bLength.
pub fn parse_configuration(raw: &[u8]) -> Configuration {
    let mut config = Configuration::default();
    let mut seen_config = false;
    let mut offset = 0;
    while offset + 2 <= raw.len() {
        let length = usize::from(raw[offset]);
        if length < 2 {
            break;
        }
        let end = (offset + length).min(raw.len());
        let descriptor = &raw[offset..end];
        offset += length;
        let complete = descriptor.len() == length;
        match descriptor[1] {
            DT_DEVICE if complete && length >= 18 => {
                config.bcd_usb = u16::from_le_bytes([descriptor[2], descriptor[3]]);
                config.vendor_id = u16::from_le_bytes([descriptor[8], descriptor[9]]);
                config.product_id = u16::from_le_bytes([descriptor[10], descriptor[11]]);
            }
            DT_CONFIG => {
                if seen_config {
                    break;
                }
                seen_config = true;
            }
            DT_INTERFACE if complete && length >= 9 => config.alts.push(AltSetting {
                interface: descriptor[2],
                alt: descriptor[3],
                class: descriptor[5],
                subclass: descriptor[6],
                protocol: descriptor[7],
                ..AltSetting::default()
            }),
            DT_CS_INTERFACE if complete && length >= 3 => {
                if let Some(alt) = config.alts.last_mut() {
                    alt.class_specific.push(descriptor.to_vec());
                }
            }
            DT_ENDPOINT if complete && length >= 7 => {
                if let Some(alt) = config.alts.last_mut() {
                    alt.endpoints.push(Endpoint {
                        address: descriptor[2],
                        attributes: descriptor[3],
                        max_packet_raw: u16::from_le_bytes([descriptor[4], descriptor[5]]),
                        interval: descriptor[6],
                        synch_address: if length >= 9 { descriptor[8] } else { 0 },
                    });
                }
            }
            _ => {}
        }
    }
    config
}

/// The PCM format the caller wants to send.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcmFormat {
    /// Bytes each sample takes on the bus (UAC2 bSubslotSize, UAC1 bSubframeSize).
    pub subslot_bytes: u8,
    pub bits: u8,
    pub channels: u8,
    pub rate: u32,
}

/// How to drive the device for one stream, chosen from the descriptors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamPlan {
    pub uac_version: u8,
    pub ac_interface: u8,
    pub streaming_interface: u8,
    pub alt: u8,
    /// UAC2: the clock source whose frequency is set (a clock selector or multiplier followed to its source).
    pub clock_id: Option<u8>,
    pub data_endpoint: Endpoint,
    pub feedback_endpoint: Option<Endpoint>,
    pub bytes_per_frame: usize,
    /// The alternate setting carries RAW_DATA (native DSD, ADR 0049) instead of PCM.
    pub raw: bool,
    /// UAC1: the rates the alternate setting lists (empty: continuous, see `uac1_rate_range`).
    pub uac1_rates: Vec<u32>,
    pub uac1_rate_range: Option<(u32, u32)>,
    /// Every audio interface of the configuration (the kernel driver is detached from and reattached to all of them).
    pub audio_interfaces: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanError {
    NoAudioControl,
    UnsupportedUacVersion(u8),
    NoMatchingFormat(PcmFormat),
    NoDataEndpoint,
    /// The data endpoint is asynchronous with no explicit feedback endpoint (implicit feedback is not supported).
    NoFeedbackEndpoint,
    NoClockSource,
    /// UAC1: the alternate setting does not list the rate.
    RateNotListed(u32),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAudioControl => f.write_str("找不到 USB 音訊控制介面"),
            Self::UnsupportedUacVersion(version) => write!(f, "不支援 UAC {version}"),
            Self::NoMatchingFormat(format) => write!(
                f,
                "DAC 沒有 {} bit／{} byte／{} 聲道的 PCM 設定",
                format.bits, format.subslot_bytes, format.channels
            ),
            Self::NoDataEndpoint => f.write_str("找不到等時輸出端點"),
            Self::NoFeedbackEndpoint => f.write_str("非同步端點沒有回饋端點（不支援隱式回饋）"),
            Self::NoClockSource => f.write_str("找不到時脈來源"),
            Self::RateNotListed(rate) => write!(f, "DAC 不支援 {rate} Hz"),
        }
    }
}

impl std::error::Error for PlanError {}

/// The UAC version of the Audio Control interface: protocol 0x20 is UAC2, 0x30 UAC3, else the header's bcdADC.
fn uac_version(ac: &AltSetting) -> u8 {
    match ac.protocol {
        0x20 => 2,
        0x30 => 3,
        _ => ac
            .class_specific
            .iter()
            .find(|d| d.len() >= 5 && d[2] == 0x01)
            .map_or(1, |header| if header[4] >= 0x02 { 2 } else { 1 }),
    }
}

/// Follow a UAC2 clock entity to its clock source (selector: its first input; multiplier: its source).
fn resolve_clock(ac: &AltSetting, id: u8) -> Option<u8> {
    let mut id = id;
    for _ in 0..8 {
        let entity = ac
            .class_specific
            .iter()
            .find(|d| d.len() >= 4 && matches!(d[2], 0x0A..=0x0C) && d[3] == id)?;
        match entity[2] {
            0x0A => return Some(id),
            0x0B => {
                id = *entity
                    .get(5)
                    .filter(|_| entity.get(4).is_some_and(|n| *n >= 1))?
            }
            _ => id = *entity.get(4)?,
        }
    }
    None
}

/// UAC2: the clock entity of the terminal an AS interface links to (an input terminal for playback).
fn terminal_clock(ac: &AltSetting, terminal: u8) -> Option<u8> {
    ac.class_specific
        .iter()
        .find(|d| d.len() >= 8 && matches!(d[2], 0x02 | 0x03) && d[3] == terminal)
        .map(|d| {
            if d[2] == 0x02 {
                d[7]
            } else {
                d.get(8).copied().unwrap_or(0)
            }
        })
}

struct Uac2Format {
    terminal: u8,
    pcm: bool,
    /// Type I RAW_DATA (bmFormats bit 31): native DSD slots.
    raw: bool,
    channels: u8,
    subslot: u8,
    bits: u8,
}

fn uac2_format(alt: &AltSetting) -> Option<Uac2Format> {
    let general = alt
        .class_specific
        .iter()
        .find(|d| d.len() >= 11 && d[2] == 0x01)?;
    let format = alt
        .class_specific
        .iter()
        .find(|d| d.len() >= 6 && d[2] == 0x02)?;
    let bm_formats = u32::from_le_bytes([general[6], general[7], general[8], general[9]]);
    Some(Uac2Format {
        terminal: general[3],
        // Type I (bFormatType 1), PCM (bit 0), not raw data (bit 31).
        pcm: general[5] == 0x01
            && format[3] == 0x01
            && bm_formats & 1 != 0
            && bm_formats & (1 << 31) == 0,
        raw: general[5] == 0x01 && format[3] == 0x01 && bm_formats & (1 << 31) != 0,
        channels: general[10],
        subslot: format[4],
        bits: format[5],
    })
}

struct Uac1Format {
    pcm: bool,
    channels: u8,
    subframe: u8,
    bits: u8,
    rates: Vec<u32>,
    range: Option<(u32, u32)>,
}

fn rate24(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0])
}

fn uac1_format(alt: &AltSetting) -> Option<Uac1Format> {
    let general = alt
        .class_specific
        .iter()
        .find(|d| d.len() >= 7 && d[2] == 0x01)?;
    let format = alt
        .class_specific
        .iter()
        .find(|d| d.len() >= 8 && d[2] == 0x02)?;
    let tag = u16::from_le_bytes([general[5], general[6]]);
    let count = usize::from(format[7]);
    let table = &format[8..];
    let (rates, range) = if count == 0 {
        (
            Vec::new(),
            (table.len() >= 6).then(|| (rate24(&table[0..3]), rate24(&table[3..6]))),
        )
    } else {
        (
            table.chunks_exact(3).take(count).map(rate24).collect(),
            None,
        )
    };
    Some(Uac1Format {
        pcm: tag == 0x0001 && format[3] == 0x01,
        channels: format[4],
        subframe: format[5],
        bits: format[6],
        rates,
        range,
    })
}

/// Choose the alternate setting, endpoints and clock for `format`.
pub fn plan_stream(config: &Configuration, format: PcmFormat) -> Result<StreamPlan, PlanError> {
    plan_stream_of(config, format, false)
}

/// The RAW_DATA alternate setting that carries native DSD as 32-bit slots, stereo, at `rate` frames per second (the DSD
/// bit rate ÷ 32, ADR 0049). UAC2 only.
pub fn plan_raw_stream(config: &Configuration, rate: u32) -> Result<StreamPlan, PlanError> {
    plan_stream_of(
        config,
        PcmFormat {
            subslot_bytes: 4,
            bits: 32,
            channels: 2,
            rate,
        },
        true,
    )
}

fn plan_stream_of(
    config: &Configuration,
    format: PcmFormat,
    raw: bool,
) -> Result<StreamPlan, PlanError> {
    let ac = config
        .alts
        .iter()
        .find(|alt| alt.is_audio(SUBCLASS_AUDIO_CONTROL))
        .ok_or(PlanError::NoAudioControl)?;
    let version = uac_version(ac);
    if !matches!(version, 1 | 2) {
        return Err(PlanError::UnsupportedUacVersion(version));
    }
    let mut audio_interfaces: Vec<u8> = config
        .alts
        .iter()
        .filter(|alt| alt.class == CLASS_AUDIO)
        .map(|alt| alt.interface)
        .collect();
    audio_interfaces.dedup();
    audio_interfaces.sort_unstable();
    audio_interfaces.dedup();

    let mut unlisted_rate = false;
    for alt in config
        .alts
        .iter()
        .filter(|alt| alt.is_audio(SUBCLASS_AUDIO_STREAMING))
    {
        let Some(data) = alt
            .endpoints
            .iter()
            .find(|ep| ep.is_isochronous() && !ep.is_in() && ep.usage() == Usage::Data)
            .copied()
        else {
            continue;
        };
        let (matches, clock_id, uac1_rates, uac1_rate_range) = if version == 2 {
            let Some(found) = uac2_format(alt) else {
                continue;
            };
            let matches = (if raw { found.raw } else { found.pcm })
                && found.channels == format.channels
                && found.subslot == format.subslot_bytes
                && found.bits == format.bits;
            let clock = terminal_clock(ac, found.terminal).and_then(|id| resolve_clock(ac, id));
            (matches, clock, Vec::new(), None)
        } else {
            let Some(found) = uac1_format(alt) else {
                continue;
            };
            let matches = !raw
                && found.pcm
                && found.channels == format.channels
                && found.subframe == format.subslot_bytes
                && found.bits == format.bits;
            let listed = found.rates.contains(&format.rate)
                || found
                    .range
                    .is_some_and(|(low, high)| (low..=high).contains(&format.rate));
            if matches && !listed {
                unlisted_rate = true;
                continue;
            }
            (matches, None, found.rates, found.range)
        };
        if !matches {
            continue;
        }
        if version == 2 && clock_id.is_none() {
            return Err(PlanError::NoClockSource);
        }
        let feedback = match data.sync() {
            SyncType::Async => {
                let explicit = alt
                    .endpoints
                    .iter()
                    .find(|ep| {
                        ep.is_isochronous()
                            && ep.is_in()
                            && (ep.usage() == Usage::Feedback
                                || (data.synch_address != 0 && ep.address == data.synch_address))
                    })
                    .copied();
                Some(explicit.ok_or(PlanError::NoFeedbackEndpoint)?)
            }
            _ => None,
        };
        return Ok(StreamPlan {
            uac_version: version,
            ac_interface: ac.interface,
            streaming_interface: alt.interface,
            alt: alt.alt,
            clock_id,
            data_endpoint: data,
            feedback_endpoint: feedback,
            bytes_per_frame: usize::from(format.subslot_bytes) * usize::from(format.channels),
            raw,
            uac1_rates,
            uac1_rate_range,
            audio_interfaces,
        });
    }
    if unlisted_rate {
        return Err(PlanError::RateNotListed(format.rate));
    }
    Err(PlanError::NoMatchingFormat(format))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sony() -> Vec<u8> {
        let text = include_str!("../tests/fixtures/sony-054c-0b8c.txt");
        text.lines()
            .filter(|line| !line.starts_with('#'))
            .flat_map(|line| line.split_whitespace().skip(1))
            .map(|byte| u8::from_str_radix(byte, 16).unwrap())
            .collect()
    }

    const S16: PcmFormat = PcmFormat {
        subslot_bytes: 2,
        bits: 16,
        channels: 2,
        rate: 44_100,
    };

    #[test]
    fn the_fixture_holds_the_whole_configuration() {
        let raw = sony();
        assert_eq!(raw.len(), 18 + 0x01DE);
    }

    #[test]
    fn the_sony_dac_plans_alt_6_on_interface_1_with_clock_0x29_and_feedback_0x81() {
        let config = parse_configuration(&sony());
        assert_eq!(config.bcd_usb, 0x0200);
        assert_eq!((config.vendor_id, config.product_id), (0x054C, 0x0B8C));
        let plan = plan_stream(&config, S16).unwrap();
        assert_eq!(plan.uac_version, 2);
        assert_eq!(plan.ac_interface, 0);
        assert_eq!(plan.streaming_interface, 1);
        assert_eq!(plan.alt, 6);
        // The input terminal's clock is selector 0x28, whose only input is source 0x29.
        assert_eq!(plan.clock_id, Some(0x29));
        assert_eq!(plan.data_endpoint.address, 0x01);
        assert_eq!(plan.data_endpoint.sync(), SyncType::Async);
        assert_eq!(plan.data_endpoint.max_bytes_per_interval(), 1024);
        assert_eq!(plan.data_endpoint.interval, 1);
        let feedback = plan.feedback_endpoint.unwrap();
        assert_eq!(
            (
                feedback.address,
                feedback.max_bytes_per_interval(),
                feedback.interval
            ),
            (0x81, 4, 4)
        );
        assert_eq!(plan.bytes_per_frame, 4);
        assert_eq!(plan.audio_interfaces, vec![0, 1]);
    }

    #[test]
    fn every_pcm_setting_of_the_sony_dac_is_found_and_dsd_is_not_pcm() {
        let config = parse_configuration(&sony());
        let cases = [
            (4, 32, 1),
            (4, 24, 2),
            (4, 16, 3),
            (3, 24, 4),
            (3, 16, 5),
            (2, 16, 6),
        ];
        for (subslot, bits, alt) in cases {
            let format = PcmFormat {
                subslot_bytes: subslot,
                bits,
                ..S16
            };
            assert_eq!(
                plan_stream(&config, format).unwrap().alt,
                alt,
                "{subslot}/{bits}"
            );
        }
        let mono = PcmFormat { channels: 1, ..S16 };
        assert_eq!(
            plan_stream(&config, mono),
            Err(PlanError::NoMatchingFormat(mono))
        );
        // Alt 7 is 4/32 raw data (DSD); 4/32 PCM is alt 1, never alt 7.
        let alt7 = config
            .alts
            .iter()
            .find(|alt| alt.interface == 1 && alt.alt == 7)
            .unwrap();
        assert!(!uac2_format(alt7).unwrap().pcm);
    }

    #[test]
    fn the_sony_dac_plans_native_dsd_on_alt_7_with_the_same_clock_and_endpoints() {
        let config = parse_configuration(&sony());
        for rate in [88_200, 176_400, 352_800] {
            let plan = plan_raw_stream(&config, rate).unwrap();
            assert!(plan.raw, "{rate}");
            assert_eq!((plan.streaming_interface, plan.alt), (1, 7), "{rate}");
            assert_eq!(plan.clock_id, Some(0x29));
            assert_eq!(plan.data_endpoint.address, 0x01);
            assert_eq!(plan.feedback_endpoint.unwrap().address, 0x81);
            // Two 4-byte slots per frame.
            assert_eq!(plan.bytes_per_frame, 8);
        }
        // A PCM plan is never the raw alt, and a raw plan is never a PCM alt.
        let pcm = plan_stream(
            &config,
            PcmFormat {
                subslot_bytes: 4,
                bits: 32,
                channels: 2,
                rate: 88_200,
            },
        )
        .unwrap();
        assert_eq!((pcm.alt, pcm.raw), (1, false));
    }

    #[test]
    fn a_device_without_a_raw_data_alt_has_no_native_dsd_plan() {
        // The UAC1 DAC has PCM alts only.
        let config = parse_configuration(&uac1());
        assert_eq!(
            plan_raw_stream(&config, 88_200),
            Err(PlanError::NoMatchingFormat(PcmFormat {
                subslot_bytes: 4,
                bits: 32,
                channels: 2,
                rate: 88_200
            }))
        );
    }

    #[test]
    fn a_truncated_or_random_input_never_panics() {
        let raw = sony();
        for end in 0..raw.len() {
            let config = parse_configuration(&raw[..end]);
            let _ = plan_stream(&config, S16);
        }
        let mut state = 0x1234_5678_u32;
        for _ in 0..2000 {
            let bytes: Vec<u8> = (0..64)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (state >> 24) as u8
                })
                .collect();
            let _ = plan_stream(&parse_configuration(&bytes), S16);
        }
    }

    /// A UAC1 adaptive DAC: AC interface 0 (header bcdADC 1.00), AS interface 1 alt 1, 16-bit stereo at 44.1/48 kHz.
    fn uac1() -> Vec<u8> {
        let mut raw = vec![
            0x12, 0x01, 0x10, 0x01, 0, 0, 0, 0x40, 0x34, 0x12, 0x78, 0x56, 0, 1, 1, 2, 3, 1,
        ];
        raw.extend([0x09, 0x02, 0, 0, 2, 1, 0, 0x80, 0x32]);
        raw.extend([0x09, 0x04, 0, 0, 0, 0x01, 0x01, 0x00, 0]);
        raw.extend([0x09, 0x24, 0x01, 0x00, 0x01, 0x1E, 0x00, 0x01, 0x01]);
        raw.extend([0x09, 0x04, 1, 0, 0, 0x01, 0x02, 0x00, 0]);
        raw.extend([0x09, 0x04, 1, 1, 1, 0x01, 0x02, 0x00, 0]);
        raw.extend([0x07, 0x24, 0x01, 0x01, 0x01, 0x01, 0x00]);
        raw.extend([
            0x0E, 0x24, 0x02, 0x01, 0x02, 0x02, 0x10, 0x02, 0x44, 0xAC, 0x00, 0x80, 0xBB, 0x00,
        ]);
        raw.extend([0x09, 0x05, 0x01, 0x09, 0xC8, 0x00, 0x01, 0x00, 0x00]);
        raw
    }

    #[test]
    fn a_uac1_adaptive_dac_needs_no_feedback_and_lists_its_rates() {
        let config = parse_configuration(&uac1());
        let plan = plan_stream(&config, S16).unwrap();
        assert_eq!(
            (plan.uac_version, plan.streaming_interface, plan.alt),
            (1, 1, 1)
        );
        assert_eq!(plan.clock_id, None);
        assert_eq!(plan.data_endpoint.sync(), SyncType::Adaptive);
        assert_eq!(plan.feedback_endpoint, None);
        assert_eq!(plan.uac1_rates, vec![44_100, 48_000]);
        let rate = PcmFormat {
            rate: 96_000,
            ..S16
        };
        assert_eq!(
            plan_stream(&config, rate),
            Err(PlanError::RateNotListed(96_000))
        );
    }

    #[test]
    fn an_async_endpoint_without_feedback_is_refused() {
        let mut raw = uac1();
        let last = raw.len() - 9;
        raw[last + 3] = 0x05; // asynchronous, no synch address and no feedback endpoint
        assert_eq!(
            plan_stream(&parse_configuration(&raw), S16),
            Err(PlanError::NoFeedbackEndpoint)
        );
    }

    #[test]
    fn high_bandwidth_packet_sizes_count_the_extra_transactions() {
        let ep = Endpoint {
            address: 1,
            attributes: 5,
            max_packet_raw: 0x1400,
            interval: 1,
            synch_address: 0,
        };
        assert_eq!(ep.max_bytes_per_interval(), 1024 * 3);
    }
}
