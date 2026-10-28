//! USB Audio Class output straight over Linux usbfs: the Android USB exclusive output (ADR 0046 P0, ADR 0047 P1).
//!
//! The app takes a USB audio device away from the Android audio system (kernel driver detached from its audio
//! interfaces), sets its clock, streams the PCM Media3 hands over (`ring`) through isochronous transfers paced by the
//! device's explicit feedback, and gives the device back to the kernel driver at the end, whatever the end is. Pure
//! Rust, no libusb: the file descriptor comes from Android's `UsbDeviceConnection`.
//!
//! Everything that decides something is a pure function, compiled and tested on every host: the descriptor plan
//! (`descriptors`), the alt for a track and the bit-perfect rule (`format`), the life of the output (`engine`), the
//! clock requests (`uac`), feedback (`feedback`), packet sizing (`packets`), sample packing (`pcm`), the buffer between
//! the threads (`ring`) and the usbfs numbers (`ioctl`). Only `usbfs` makes system calls; it exists on Android and Linux.

pub mod descriptors;
pub mod diagnostics;
pub mod engine;
pub mod feedback;
pub mod format;
pub mod ioctl;
pub mod packets;
pub mod pcm;
pub mod ring;
pub mod uac;
#[cfg(any(target_os = "android", target_os = "linux"))]
pub mod usbfs;

use format::NotBitPerfect;
use pcm::Container;

/// What an attach chose.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachReply {
    pub rate: u32,
    pub channels: u8,
    pub alt: u8,
    pub container: Container,
}


/// What the exclusive output is doing, for the settings page (`android_audio_output_status` → `exclusive`) and the log.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    /// `unclaimed`, `claimed`, `streaming`, `paused` or `handedBack` (`engine::state_name`).
    pub state: String,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub high_speed: Option<bool>,
    pub rate: Option<u32>,
    pub rate_read_back: Option<u32>,
    pub channels: Option<u8>,
    pub alt: Option<u8>,
    /// Bytes per sample on the bus (bSubslotSize) and valid bits (bBitResolution).
    pub subslot_bytes: Option<u8>,
    pub bits: Option<u8>,
    /// The track's own bit depth, when the library knows it.
    pub source_bits: Option<u8>,
    pub dsd: bool,
    /// Why the bus does not carry the file's samples bit for bit (empty: bit-perfect), at the current volume.
    pub not_bit_perfect: Vec<NotBitPerfect>,
    pub bit_perfect: bool,
    /// The app volume applied as gain before packing.
    pub gain: f32,
    /// Times the buffer ran dry while a track played, over this claim.
    pub underruns: u64,
    /// The latest accepted feedback value and the average, in frames per bus interval.
    pub feedback_avg: Option<f64>,
    pub urb_errors: u64,
    pub packet_errors: u64,
    /// Why the output stopped (a failed step, the device gone), or `None`.
    pub error: Option<String>,
    /// `None` until the device was given back; then whether every release and reattach succeeded.
    pub handed_back: Option<bool>,
    pub hand_back_errors: Vec<String>,
    /// The last usbfs steps, for the log.
    pub steps: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_is_camel_case_json() {
        let status = EngineStatus {
            state: "streaming".into(),
            not_bit_perfect: vec![NotBitPerfect::Volume, NotBitPerfect::Float32],
            ..EngineStatus::default()
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"state\":\"streaming\""), "{json}");
        assert!(
            json.contains("\"notBitPerfect\":[\"volume\",\"float32\"]"),
            "{json}"
        );
        assert!(json.contains("\"rateReadBack\":null"), "{json}");
        assert!(json.contains("\"subslotBytes\":null"), "{json}");
        assert!(json.contains("\"handBackErrors\":[]"), "{json}");
    }
}
