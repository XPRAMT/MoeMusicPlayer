//! What the URBs of an exclusive test came back with (ADR 0046): counts kept on the streaming thread, reported as they
//! are so a silent run can be told apart (feedback never answering, packets short, statuses the kernel set).
//!
//! usbfs reports a URB's and an isochronous packet's status as a negative Linux errno (0 = ok). The numbers are the
//! Linux ones on every architecture Android runs on, so the names are a fixed table here and the module is pure.

/// One distinct status and how often it came back.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusCount {
    pub status: i32,
    /// The errno name (e.g. `EXDEV`), `None` for 0 and unknown values.
    pub name: Option<&'static str>,
    pub count: u64,
}

/// Distinct statuses with counts, in the order they were first seen.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(transparent)]
pub struct StatusCounts(pub Vec<StatusCount>);

impl StatusCounts {
    pub fn record(&mut self, status: i32) {
        if let Some(entry) = self.0.iter_mut().find(|entry| entry.status == status) {
            entry.count += 1;
        } else {
            self.0.push(StatusCount {
                status,
                name: errno_name(status),
                count: 1,
            });
        }
    }

    pub fn total(&self) -> u64 {
        self.0.iter().map(|entry| entry.count).sum()
    }

    /// How often a value other than 0 came back.
    pub fn non_zero(&self) -> u64 {
        self.0
            .iter()
            .filter(|entry| entry.status != 0)
            .map(|entry| entry.count)
            .sum()
    }
}

/// The Linux errno name of a usbfs status (either sign), for the statuses USB host controllers set.
pub fn errno_name(status: i32) -> Option<&'static str> {
    Some(match status.unsigned_abs() {
        2 => "ENOENT",
        5 => "EIO",
        11 => "EAGAIN",
        16 => "EBUSY",
        18 => "EXDEV",
        19 => "ENODEV",
        22 => "EINVAL",
        32 => "EPIPE",
        62 => "ETIME",
        63 => "ENOSR",
        70 => "ECOMM",
        71 => "EPROTO",
        75 => "EOVERFLOW",
        84 => "EILSEQ",
        104 => "ECONNRESET",
        108 => "ESHUTDOWN",
        110 => "ETIMEDOUT",
        115 => "EINPROGRESS",
        121 => "EREMOTEIO",
        _ => return None,
    })
}

/// How many raw feedback packets are kept for the report.
pub const FEEDBACK_SAMPLES: usize = 8;

/// Every reaped feedback URB (one isochronous packet each).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackUrbStats {
    pub reaped: u64,
    /// URB and packet status 0 and at least one byte: handed to the feedback parser.
    pub with_data: u64,
    /// URB and packet status 0 but no bytes.
    pub zero_length: u64,
    /// Non-zero packet statuses (URB status 0).
    pub packet_status: StatusCounts,
    /// Non-zero URB statuses.
    pub urb_status: StatusCounts,
    /// The first [`FEEDBACK_SAMPLES`] packets that carried bytes, as hex (e.g. `"9C 58 05 00"`).
    pub first_packets: Vec<String>,
}

impl FeedbackUrbStats {
    /// Record one reaped feedback URB; returns the bytes to give the parser, if any.
    pub fn record<'a>(
        &mut self,
        urb_status: i32,
        packet_status: i32,
        bytes: &'a [u8],
    ) -> Option<&'a [u8]> {
        self.reaped += 1;
        if urb_status != 0 {
            self.urb_status.record(urb_status);
            return None;
        }
        if packet_status != 0 {
            self.packet_status.record(packet_status);
            return None;
        }
        if bytes.is_empty() {
            self.zero_length += 1;
            return None;
        }
        self.with_data += 1;
        if self.first_packets.len() < FEEDBACK_SAMPLES {
            self.first_packets.push(hex(bytes));
        }
        Some(bytes)
    }
}

/// Every reaped data URB and its packets.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataUrbStats {
    pub reaped: u64,
    /// Every URB status, 0 included.
    pub urb_status: StatusCounts,
    /// Every packet status, 0 included.
    pub packet_status: StatusCounts,
    pub packets: u64,
    /// Packets whose `actual_length` is not the `length` asked for.
    pub length_mismatch: u64,
    pub requested_bytes: u64,
    pub actual_bytes: u64,
}

impl DataUrbStats {
    pub fn record_urb(&mut self, status: i32) {
        self.reaped += 1;
        self.urb_status.record(status);
    }

    pub fn record_packet(&mut self, status: i32, length: u32, actual_length: u32) {
        self.packets += 1;
        self.packet_status.record(status);
        if actual_length != length {
            self.length_mismatch += 1;
        }
        self.requested_bytes += u64::from(length);
        self.actual_bytes += u64::from(actual_length);
    }
}

/// Bytes as upper-case hex separated by spaces.
pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_are_counted_once_each_with_their_errno_name() {
        let mut counts = StatusCounts::default();
        for status in [0, -18, 0, -71, -18, 0, -9999] {
            counts.record(status);
        }
        assert_eq!(
            counts.0,
            vec![
                StatusCount {
                    status: 0,
                    name: None,
                    count: 3
                },
                StatusCount {
                    status: -18,
                    name: Some("EXDEV"),
                    count: 2
                },
                StatusCount {
                    status: -71,
                    name: Some("EPROTO"),
                    count: 1
                },
                StatusCount {
                    status: -9999,
                    name: None,
                    count: 1
                },
            ]
        );
        assert_eq!((counts.total(), counts.non_zero()), (7, 4));
    }

    #[test]
    fn every_reaped_feedback_urb_lands_in_exactly_one_bucket() {
        let mut stats = FeedbackUrbStats::default();
        assert_eq!(stats.record(-108, 0, &[1, 2, 3, 4]), None);
        assert_eq!(stats.record(0, -18, &[1, 2, 3, 4]), None);
        assert_eq!(stats.record(0, -18, &[]), None);
        assert_eq!(stats.record(0, 0, &[]), None);
        let packet = [0x9C, 0x58, 0x05, 0x00];
        assert_eq!(stats.record(0, 0, &packet), Some(&packet[..]));
        assert_eq!(stats.reaped, 5);
        assert_eq!(
            stats.with_data
                + stats.zero_length
                + stats.packet_status.total()
                + stats.urb_status.total(),
            stats.reaped
        );
        assert_eq!((stats.with_data, stats.zero_length), (1, 1));
        assert_eq!(stats.packet_status.0[0].count, 2);
        assert_eq!(stats.urb_status.0[0].name, Some("ESHUTDOWN"));
        assert_eq!(stats.first_packets, vec!["9C 58 05 00".to_owned()]);
    }

    #[test]
    fn only_the_first_feedback_packets_are_kept() {
        let mut stats = FeedbackUrbStats::default();
        for value in 0..20_u8 {
            stats.record(0, 0, &[value]);
        }
        assert_eq!(stats.first_packets.len(), FEEDBACK_SAMPLES);
        assert_eq!(stats.first_packets[7], "07");
        assert_eq!(stats.with_data, 20);
    }

    #[test]
    fn data_packets_count_statuses_including_zero_and_short_packets() {
        let mut stats = DataUrbStats::default();
        stats.record_urb(0);
        stats.record_packet(0, 24, 24);
        stats.record_packet(0, 20, 0);
        stats.record_packet(-18, 24, 0);
        assert_eq!(stats.reaped, 1);
        assert_eq!(stats.urb_status.0[0].count, 1);
        assert_eq!(stats.packet_status.total(), 3);
        assert_eq!(stats.packet_status.non_zero(), 1);
        assert_eq!(stats.length_mismatch, 2);
        assert_eq!((stats.requested_bytes, stats.actual_bytes), (68, 24));
    }

    #[test]
    fn the_json_is_camel_case_and_statuses_are_a_list() {
        let mut stats = FeedbackUrbStats::default();
        stats.record(0, -18, &[]);
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"withData\":0"), "{json}");
        assert!(json.contains("\"zeroLength\":0"), "{json}");
        assert!(
            json.contains("\"packetStatus\":[{\"status\":-18,\"name\":\"EXDEV\",\"count\":1}]"),
            "{json}"
        );
        assert!(json.contains("\"firstPackets\":[]"), "{json}");
        let json = serde_json::to_string(&DataUrbStats::default()).unwrap();
        assert!(json.contains("\"lengthMismatch\":0"), "{json}");
        assert!(json.contains("\"urbStatus\":[]"), "{json}");
    }
}
