//! Class-specific control requests for the sampling frequency (ADR 0046), as plain values the usbfs layer sends.

/// One control transfer on endpoint 0.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlRequest {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    /// OUT: the bytes sent. IN: zeros of the length asked for.
    pub data: Vec<u8>,
}

impl ControlRequest {
    pub fn is_in(&self) -> bool {
        self.request_type & 0x80 != 0
    }
}

/// Host-to-device, class, interface recipient.
const OUT_INTERFACE: u8 = 0x21;
/// Device-to-host, class, interface recipient.
const IN_INTERFACE: u8 = 0xA1;
/// UAC1: host-to-device, class, endpoint recipient.
const OUT_ENDPOINT: u8 = 0x22;
const IN_ENDPOINT: u8 = 0xA2;

/// UAC2 CUR / RANGE; UAC1 SET_CUR (0x01) and GET_CUR (0x81).
const CUR: u8 = 0x01;
const RANGE: u8 = 0x02;
const UAC1_GET_CUR: u8 = 0x81;

/// CS_SAM_FREQ_CONTROL (UAC2) and SAMPLING_FREQ_CONTROL (UAC1) are both 0x01, channel number 0.
const SAM_FREQ_VALUE: u16 = 0x01 << 8;

fn clock_index(clock_id: u8, ac_interface: u8) -> u16 {
    (u16::from(clock_id) << 8) | u16::from(ac_interface)
}

/// UAC2 SET CUR of a clock source's frequency (4 bytes, little endian).
pub fn uac2_set_rate(clock_id: u8, ac_interface: u8, rate: u32) -> ControlRequest {
    ControlRequest {
        request_type: OUT_INTERFACE,
        request: CUR,
        value: SAM_FREQ_VALUE,
        index: clock_index(clock_id, ac_interface),
        data: rate.to_le_bytes().to_vec(),
    }
}

/// UAC2 GET CUR of a clock source's frequency.
pub fn uac2_get_rate(clock_id: u8, ac_interface: u8) -> ControlRequest {
    ControlRequest {
        request_type: IN_INTERFACE,
        request: CUR,
        value: SAM_FREQ_VALUE,
        index: clock_index(clock_id, ac_interface),
        data: vec![0; 4],
    }
}

/// UAC2 GET RANGE of a clock source's frequency, `length` bytes (first 2 for the count, then 2 + 12 × count).
pub fn uac2_get_rate_range(clock_id: u8, ac_interface: u8, length: u16) -> ControlRequest {
    ControlRequest {
        request_type: IN_INTERFACE,
        request: RANGE,
        value: SAM_FREQ_VALUE,
        index: clock_index(clock_id, ac_interface),
        data: vec![0; usize::from(length)],
    }
}

/// The length of the whole RANGE answer from its first two bytes (wNumSubRanges).
pub fn uac2_range_length(head: &[u8]) -> Option<u16> {
    let count = u16::from_le_bytes([*head.first()?, *head.get(1)?]);
    count.checked_mul(12).and_then(|bytes| bytes.checked_add(2))
}

/// A RANGE answer's sub-ranges as (min, max, resolution).
pub fn parse_uac2_rate_ranges(answer: &[u8]) -> Vec<(u32, u32, u32)> {
    let Some(count) = answer
        .get(..2)
        .map(|head| u16::from_le_bytes([head[0], head[1]]))
    else {
        return Vec::new();
    };
    answer[2..]
        .chunks_exact(12)
        .take(usize::from(count))
        .map(|chunk| {
            let word = |at: usize| {
                u32::from_le_bytes([chunk[at], chunk[at + 1], chunk[at + 2], chunk[at + 3]])
            };
            (word(0), word(4), word(8))
        })
        .collect()
}

/// Whether one of the sub-ranges holds `rate` (on its resolution grid; a resolution of 0 is a single value or any).
pub fn rate_in_ranges(ranges: &[(u32, u32, u32)], rate: u32) -> bool {
    ranges.iter().any(|&(min, max, resolution)| {
        (min..=max).contains(&rate) && (resolution == 0 || (rate - min).is_multiple_of(resolution))
    })
}

/// UAC1 SET_CUR of the sampling frequency on the data endpoint (3 bytes, little endian).
pub fn uac1_set_rate(endpoint: u8, rate: u32) -> ControlRequest {
    ControlRequest {
        request_type: OUT_ENDPOINT,
        request: CUR,
        value: SAM_FREQ_VALUE,
        index: u16::from(endpoint),
        data: rate.to_le_bytes()[..3].to_vec(),
    }
}

/// UAC1 GET_CUR of the sampling frequency on the data endpoint.
pub fn uac1_get_rate(endpoint: u8) -> ControlRequest {
    ControlRequest {
        request_type: IN_ENDPOINT,
        request: UAC1_GET_CUR,
        value: SAM_FREQ_VALUE,
        index: u16::from(endpoint),
        data: vec![0; 3],
    }
}

/// The rate in a GET CUR answer (3 or 4 bytes, little endian).
pub fn parse_rate(answer: &[u8]) -> Option<u32> {
    match answer.len() {
        3 => Some(u32::from_le_bytes([answer[0], answer[1], answer[2], 0])),
        4.. => Some(u32::from_le_bytes([
            answer[0], answer[1], answer[2], answer[3],
        ])),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uac2_set_cur_44100_on_the_sony_clock_is_encoded_as_the_spec_says() {
        let request = uac2_set_rate(0x29, 0, 44_100);
        assert_eq!(
            (
                request.request_type,
                request.request,
                request.value,
                request.index
            ),
            (0x21, 0x01, 0x0100, 0x2900)
        );
        assert_eq!(request.data, vec![0x44, 0xAC, 0x00, 0x00]);
        assert!(!request.is_in());
        let read = uac2_get_rate(0x29, 0);
        assert_eq!(
            (read.request_type, read.request, read.value, read.index),
            (0xA1, 0x01, 0x0100, 0x2900)
        );
        assert_eq!(read.data.len(), 4);
        assert!(read.is_in());
        assert_eq!(parse_rate(&[0x44, 0xAC, 0, 0]), Some(44_100));
        assert_eq!(parse_rate(&[0x80, 0xBB, 0x00]), Some(48_000));
        assert_eq!(parse_rate(&[1, 2]), None);
    }

    #[test]
    fn the_range_answer_is_read_and_checked_on_its_grid() {
        let range = uac2_get_rate_range(0x29, 0, 2);
        assert_eq!(
            (
                range.request_type,
                range.request,
                range.index,
                range.data.len()
            ),
            (0xA1, 0x02, 0x2900, 2)
        );
        let mut answer = vec![3, 0];
        for (min, max, res) in [
            (44_100_u32, 44_100_u32, 0_u32),
            (48_000, 48_000, 0),
            (88_200, 352_800, 88_200),
        ] {
            answer.extend(min.to_le_bytes());
            answer.extend(max.to_le_bytes());
            answer.extend(res.to_le_bytes());
        }
        assert_eq!(uac2_range_length(&answer[..2]), Some(38));
        let ranges = parse_uac2_rate_ranges(&answer);
        assert_eq!(ranges.len(), 3);
        assert!(rate_in_ranges(&ranges, 44_100));
        assert!(rate_in_ranges(&ranges, 176_400));
        assert!(!rate_in_ranges(&ranges, 96_000));
        assert!(!rate_in_ranges(&ranges, 32_000));
        // A short answer gives what it holds.
        assert_eq!(parse_uac2_rate_ranges(&answer[..20]).len(), 1);
        assert!(parse_uac2_rate_ranges(&[1]).is_empty());
    }

    #[test]
    fn uac1_requests_go_to_the_endpoint_with_three_bytes() {
        let set = uac1_set_rate(0x01, 48_000);
        assert_eq!(
            (set.request_type, set.request, set.value, set.index),
            (0x22, 0x01, 0x0100, 0x0001)
        );
        assert_eq!(set.data, vec![0x80, 0xBB, 0x00]);
        let get = uac1_get_rate(0x01);
        assert_eq!(
            (get.request_type, get.request, get.data.len()),
            (0xA2, 0x81, 3)
        );
    }
}
