//! Lyrics on the Windows taskbar (ADR 0018): a transparent, click-through, always-on-top window that a watcher thread
//! keeps over the free stretch of the taskbar. The window's page (`?view=taskbar-lyrics`) owns the lyric state.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use player_platform_windows::taskbar::{
    pick_free_area, taskbar_uses_light_theme, AreaChoice, Rect, TaskbarProber,
};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, HWND_TOPMOST, SWP_HIDEWINDOW,
    SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_SHOWWINDOW, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

use crate::settings::{TaskbarLyricsArea, TaskbarLyricsSettings};

pub const WINDOW_LABEL: &str = "taskbar-lyrics";
/// Emitted with the saved `TaskbarLyricsSettings` whenever they change.
pub const SETTINGS_EVENT: &str = "taskbar-lyrics-settings";
/// Emitted with `true` when the taskbar switches to the light theme and `false` for dark.
pub const THEME_EVENT: &str = "taskbar-lyrics-theme";

const POLL_STEP: Duration = Duration::from_millis(100);
const POLL_STEPS: u32 = 4;
/// Space kept free beside the neighbouring buttons, and the narrowest stretch worth using (CSS px at 96 DPI).
const MARGIN_CSS_PX: i32 = 8;
const MIN_WIDTH_CSS_PX: i32 = 120;

static STOP: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);
static AREA: AtomicU8 = AtomicU8::new(0);

fn area_code(area: TaskbarLyricsArea) -> u8 {
    match area {
        TaskbarLyricsArea::Auto => 0,
        TaskbarLyricsArea::Left => 1,
        TaskbarLyricsArea::Right => 2,
    }
}

fn area_choice() -> AreaChoice {
    match AREA.load(Ordering::Relaxed) {
        1 => AreaChoice::Leftmost,
        2 => AreaChoice::Rightmost,
        _ => AreaChoice::Widest,
    }
}

pub fn system_uses_light_theme() -> bool {
    taskbar_uses_light_theme()
}

/// Make the running state match `settings`: open the window and start the watcher, or stop both.
pub async fn apply(app: &AppHandle, settings: &TaskbarLyricsSettings) -> Result<(), String> {
    AREA.store(area_code(settings.area), Ordering::Relaxed);
    if settings.enabled {
        ensure_window(app)?;
        start_watcher(app);
        let _ = app.emit(SETTINGS_EVENT, settings);
    } else {
        shutdown(app);
    }
    Ok(())
}

/// Stop the watcher and close the window (also when the main window closes, so it cannot keep the process alive).
pub fn shutdown(app: &AppHandle) {
    if let Some(stop) = STOP.lock().ok().and_then(|mut guard| guard.take()) {
        stop.store(true, Ordering::Relaxed);
    }
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.destroy();
    }
}

fn ensure_window(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window(WINDOW_LABEL).is_some() {
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(
        app,
        WINDOW_LABEL,
        WebviewUrl::App("index.html?view=taskbar-lyrics".into()),
    )
    .title("工作列歌詞")
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .skip_taskbar(true)
    .always_on_top(true)
    .resizable(false)
    .focusable(false)
    .visible(false)
    .inner_size(400.0, 40.0)
    .build()
    .map_err(|error| format!("無法開啟工作列歌詞視窗：{error}"))?;
    window
        .set_ignore_cursor_events(true)
        .map_err(|error| error.to_string())?;
    if let Ok(hwnd) = window.hwnd() {
        let hwnd = HWND(hwnd.0);
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let style = (style | WS_EX_TOOLWINDOW.0 as isize | WS_EX_NOACTIVATE.0 as isize)
                & !(WS_EX_APPWINDOW.0 as isize);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
        }
    }
    Ok(())
}

fn start_watcher(app: &AppHandle) {
    let Ok(mut guard) = STOP.lock() else {
        return;
    };
    if guard.is_some() {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    *guard = Some(Arc::clone(&stop));
    let app = app.clone();
    std::thread::Builder::new()
        .name("taskbar-lyrics".into())
        .spawn(move || watch(app, stop))
        .ok();
}

fn watch(app: AppHandle, stop: Arc<AtomicBool>) {
    let prober = match TaskbarProber::new() {
        Ok(prober) => prober,
        Err(error) => {
            eprintln!("工作列歌詞無法啟動：{error}");
            return;
        }
    };
    let mut last_light = None;
    let mut visible = false;
    while !stop.load(Ordering::Relaxed) {
        let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
            break;
        };
        if let Ok(hwnd) = window.hwnd() {
            let hwnd = HWND(hwnd.0);
            let area = target_area(&prober);
            match area {
                Some(area) => {
                    // Re-asserted every pass: clicking the taskbar raises it over other topmost windows.
                    let _ = unsafe {
                        SetWindowPos(
                            hwnd,
                            Some(HWND_TOPMOST),
                            area.left,
                            area.top,
                            area.width(),
                            area.height(),
                            SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
                        )
                    };
                    visible = true;
                }
                None if visible => {
                    let _ = unsafe {
                        SetWindowPos(
                            hwnd,
                            None,
                            0,
                            0,
                            0,
                            0,
                            windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                                | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                                | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                                | SWP_NOACTIVATE
                                | SWP_HIDEWINDOW,
                        )
                    };
                    visible = false;
                }
                None => {}
            }
        }
        let light = taskbar_uses_light_theme();
        if last_light != Some(light) {
            last_light = Some(light);
            let _ = app.emit_to(WINDOW_LABEL, THEME_EVENT, light);
        }
        for _ in 0..POLL_STEPS {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(POLL_STEP);
        }
    }
}

/// The free stretch lyrics may use right now, or `None` when they should be hidden.
fn target_area(prober: &TaskbarProber) -> Option<Rect> {
    let probe = prober.probe().ok()?;
    if probe.covered || probe.auto_hide || probe.vertical {
        return None;
    }
    let scale = |css_px: i32| css_px * probe.dpi.max(96) as i32 / 96;
    pick_free_area(
        probe.bounds,
        &probe.occupied,
        area_choice(),
        scale(MARGIN_CSS_PX),
        scale(MIN_WIDTH_CSS_PX),
    )
}
