//! Linux usbfs structures and ioctl numbers (`linux/usbdevice_fs.h`), laid out as the kernel's C structs. Compiled on
//! every host so the numbers are tested; only the Android/Linux `usbfs` module uses them.

use std::os::raw::{c_int, c_uint, c_void};

#[repr(C)]
pub struct CtrlTransfer {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub length: u16,
    /// Milliseconds.
    pub timeout: u32,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct SetInterface {
    pub interface: c_uint,
    pub alt_setting: c_uint,
}

#[repr(C)]
pub struct IoctlRequest {
    pub interface: c_int,
    pub ioctl_code: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IsoPacketDesc {
    pub length: c_uint,
    pub actual_length: c_uint,
    pub status: c_uint,
}

/// `struct usbdevfs_urb` without its flexible `iso_frame_desc[]`, which follows it in memory.
#[repr(C)]
pub struct Urb {
    pub kind: u8,
    pub endpoint: u8,
    pub status: c_int,
    pub flags: c_uint,
    pub buffer: *mut c_void,
    pub buffer_length: c_int,
    pub actual_length: c_int,
    pub start_frame: c_int,
    pub number_of_packets: c_int,
    pub error_count: c_int,
    pub signal: c_uint,
    pub user_context: *mut c_void,
}

pub const URB_TYPE_ISO: u8 = 0;
pub const URB_ISO_ASAP: c_uint = 0x02;

/// `enum usb_device_speed` values of `USBDEVFS_GET_SPEED`.
pub const SPEED_HIGH: c_int = 3;

const NONE: u32 = 0;
const WRITE: u32 = 1;
const READ: u32 = 2;

const fn ioc(direction: u32, number: u32, size: usize) -> u32 {
    (direction << 30) | ((size as u32) << 16) | ((b'U' as u32) << 8) | number
}

pub const CONTROL: u32 = ioc(READ | WRITE, 0, std::mem::size_of::<CtrlTransfer>());
pub const SET_INTERFACE: u32 = ioc(READ, 4, std::mem::size_of::<SetInterface>());
pub const SUBMIT_URB: u32 = ioc(READ, 10, std::mem::size_of::<Urb>());
pub const DISCARD_URB: u32 = ioc(NONE, 11, 0);
pub const REAP_URB: u32 = ioc(WRITE, 12, std::mem::size_of::<*mut c_void>());
pub const REAP_URB_NDELAY: u32 = ioc(WRITE, 13, std::mem::size_of::<*mut c_void>());
pub const CLAIM_INTERFACE: u32 = ioc(READ, 15, std::mem::size_of::<c_uint>());
pub const RELEASE_INTERFACE: u32 = ioc(READ, 16, std::mem::size_of::<c_uint>());
pub const IOCTL: u32 = ioc(READ | WRITE, 18, std::mem::size_of::<IoctlRequest>());
/// Sub-codes of `IOCTL`: detach / let the kernel driver bind again.
pub const DISCONNECT: u32 = ioc(NONE, 22, 0);
pub const CONNECT: u32 = ioc(NONE, 23, 0);
pub const GET_SPEED: u32 = ioc(NONE, 31, 0);

#[cfg(all(test, target_pointer_width = "64"))]
mod tests {
    use super::*;

    #[test]
    fn the_numbers_match_the_64_bit_kernel_headers() {
        assert_eq!(std::mem::size_of::<Urb>(), 56);
        assert_eq!(std::mem::size_of::<CtrlTransfer>(), 24);
        assert_eq!(CONTROL, 0xC018_5500);
        assert_eq!(SET_INTERFACE, 0x8008_5504);
        assert_eq!(SUBMIT_URB, 0x8038_550A);
        assert_eq!(DISCARD_URB, 0x0000_550B);
        assert_eq!(REAP_URB, 0x4008_550C);
        assert_eq!(REAP_URB_NDELAY, 0x4008_550D);
        assert_eq!(CLAIM_INTERFACE, 0x8004_550F);
        assert_eq!(RELEASE_INTERFACE, 0x8004_5510);
        assert_eq!(IOCTL, 0xC010_5512);
        assert_eq!(DISCONNECT, 0x0000_5516);
        assert_eq!(CONNECT, 0x0000_5517);
        assert_eq!(GET_SPEED, 0x0000_551F);
    }
}
