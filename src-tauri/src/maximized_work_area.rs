//! Keep a borderless maximized window inside the monitor work area.
//!
//! Windows maximizes an undecorated window to the full monitor, including the
//! taskbar. The client area is then clipped back to the work area, and the
//! leftover strip paints black over the taskbar. `WM_GETMINMAXINFO` is the
//! size Windows uses for that maximize, so report the work area there.

use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GetWindowLongPtrW, SetWindowLongPtrW, GWLP_WNDPROC, MINMAXINFO,
    WM_GETMINMAXINFO, WNDPROC,
};

static PREVIOUS: Mutex<Option<(isize, isize)>> = Mutex::new(None);

pub(crate) fn install(hwnd: HWND) {
    unsafe {
        let previous = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
        if previous == 0 {
            return;
        }
        let mut slot = PREVIOUS.lock().unwrap_or_else(|error| error.into_inner());
        if slot.is_some() {
            return;
        }
        SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wndproc as *const () as isize);
        *slot = Some((hwnd.0 as isize, previous));
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let previous = PREVIOUS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .filter(|(installed, _)| *installed == hwnd.0 as isize)
        .map(|(_, previous)| previous);
    let Some(previous) = previous else {
        return LRESULT(0);
    };
    let previous = unsafe { std::mem::transmute::<isize, WNDPROC>(previous) };
    let result = unsafe { CallWindowProcW(previous, hwnd, msg, wparam, lparam) };
    if msg == WM_GETMINMAXINFO {
        constrain_maximized_size_to_work_area(hwnd, lparam);
    }
    result
}

fn constrain_maximized_size_to_work_area(hwnd: HWND, lparam: LPARAM) {
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if monitor.is_invalid() {
            return;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let work = info.rcWork;
        let screen = info.rcMonitor;
        let limits = &mut *(lparam.0 as *mut MINMAXINFO);
        limits.ptMaxPosition.x = work.left - screen.left;
        limits.ptMaxPosition.y = work.top - screen.top;
        limits.ptMaxSize.x = work.right - work.left;
        limits.ptMaxSize.y = work.bottom - work.top;
    }
}
