//! The Windows taskbar button uses the window's large icon (`ICON_BIG`).
//! tao applies Tauri's window icon only as `ICON_SMALL`, then clears `ICON_BIG`.
//! With no large icon, the shell supplies the icon cached for this exe path, so
//! replacing the file in place leaves the previous picture on the taskbar.
//! Load the icon compiled into this process and assign it before the window is shown.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    LoadImageW, SendMessageW, SetClassLongPtrW, GCLP_HICON, GCLP_HICONSM, ICON_BIG, ICON_SMALL,
    IMAGE_ICON, WM_SETICON,
};

/// `tauri-winres` writes the bundle icon as resource 32512 (`IDI_APPLICATION`).
const IDI_APPLICATION: usize = 32512;

pub(crate) fn apply_embedded_taskbar_icon(hwnd: HWND) {
    unsafe {
        let Ok(module) = GetModuleHandleW(PCWSTR::null()) else {
            return;
        };
        let instance = HINSTANCE(module.0);
        let Some(big) = load_icon(instance, 256) else {
            return;
        };
        let _ = SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(big)),
        );
        let _ = SetClassLongPtrW(hwnd, GCLP_HICON, big);
        if let Some(small) = load_icon(instance, 32) {
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_SMALL as usize)),
                Some(LPARAM(small)),
            );
            let _ = SetClassLongPtrW(hwnd, GCLP_HICONSM, small);
        }
    }
}

unsafe fn load_icon(instance: HINSTANCE, size: i32) -> Option<isize> {
    let name = PCWSTR(IDI_APPLICATION as *const u16);
    LoadImageW(
        Some(instance),
        name,
        IMAGE_ICON,
        size,
        size,
        Default::default(),
    )
    .ok()
    .map(|handle| handle.0 as isize)
    .filter(|handle| *handle != 0)
}
