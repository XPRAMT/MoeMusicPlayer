//! USB Audio Class output straight over Linux usbfs, for the Android USB exclusive test (ADR 0046, P0).
//!
//! The app takes a USB DAC away from the Android audio system (kernel driver detached from its audio interfaces),
//! sets its clock, streams 16-bit PCM over isochronous transfers paced by the DAC's explicit feedback, and gives the
//! DAC back to the kernel driver at the end, whatever the end is. Pure Rust, no libusb: the file descriptor comes from
//! Android's `UsbDeviceConnection`.
//!
//! Everything that decides something is a pure function, compiled and tested on every host: the descriptor plan
//! (`descriptors`), the clock requests (`uac`), feedback (`feedback`), packet sizing (`packets`), the PCM (`pcm`) and
//! the usbfs numbers (`ioctl`). Only `usbfs` makes system calls; it exists on Android and Linux.

pub mod descriptors;
pub mod feedback;
pub mod ioctl;
pub mod packets;
pub mod pcm;
pub mod uac;
#[cfg(any(target_os = "android", target_os = "linux"))]
pub mod usbfs;

use feedback::FeedbackStats;

/// How a test ended.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// The PCM (or the time limit) was played to the end.
    Finished,
    /// The user pressed stop.
    Stopped,
    /// It could not start or broke off; `error` says why.
    #[default]
    Failed,
}

/// What one exclusive test did, shown on the page and logged.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub outcome: Outcome,
    pub error: Option<String>,
    pub file_rate: Option<u32>,
    pub rate_set: Option<u32>,
    pub rate_read_back: Option<u32>,
    pub interface: Option<u8>,
    pub alt: Option<u8>,
    pub high_speed: Option<bool>,
    pub feedback: FeedbackStats,
    pub frames_total: u64,
    pub frames_sent: u64,
    /// Times every data URB had completed before the next was queued (the bus went without data).
    pub underruns: u64,
    /// URBs that could not be submitted or came back with an error.
    pub urb_errors: u64,
    /// Isochronous packets that came back with a non-zero status.
    pub packet_errors: u64,
    pub data_urbs: usize,
    pub packets_per_urb: usize,
    /// `None` when the kernel driver was never detached; else whether the DAC went back to it cleanly.
    pub handed_back: Option<bool>,
    pub hand_back_errors: Vec<String>,
    pub elapsed_ms: u64,
    /// Each usbfs step and its result, for the log.
    pub steps: Vec<String>,
}

impl Report {
    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            error: Some(error.into()),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_is_camel_case_json() {
        let json = serde_json::to_string(&Report::failed("x")).unwrap();
        assert!(json.contains("\"outcome\":\"failed\""), "{json}");
        assert!(json.contains("\"rateReadBack\":null"), "{json}");
        assert!(json.contains("\"handBackErrors\":[]"), "{json}");
        assert!(json.contains("\"feedback\":{\"min\":null"), "{json}");
    }
}
