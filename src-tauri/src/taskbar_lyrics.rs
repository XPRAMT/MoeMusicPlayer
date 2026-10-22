//! Lyrics on the Windows taskbars (ADR 0018, ADR 0031): one transparent, click-through, always-on-top window per
//! taskbar the settings pick (`Shell_TrayWnd` and every `Shell_SecondaryTrayWnd`). One watcher thread lists the
//! taskbars every pass, opens and closes windows to match, and keeps each over the free stretch of its own taskbar.
//! Every window loads the same page (`?view=taskbar-lyrics`); the main window broadcasts lyric data, playback and
//! settings to all of them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use player_platform_windows::taskbar::{
    list_monitors, list_taskbars, lyrics_area, monitor_device_of_window, taskbar_uses_light_theme,
    DisplayMonitor, Rect, TaskbarProber, TaskbarWindow,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, SetWindowPos, GWLP_HWNDPARENT,
    GWL_EXSTYLE, HWND_TOPMOST, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER,
    SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

use crate::settings::{TaskbarLyricsDisplayMode, TaskbarLyricsSettings};

/// Every lyrics window's label starts with this (`taskbar-lyrics-1`, `taskbar-lyrics-2`, ...).
pub const WINDOW_LABEL_PREFIX: &str = "taskbar-lyrics";
/// Emitted with the saved `TaskbarLyricsSettings` whenever they change.
pub const SETTINGS_EVENT: &str = "taskbar-lyrics-settings";
/// Emitted with `true` when the taskbar switches to the light theme and `false` for dark.
pub const THEME_EVENT: &str = "taskbar-lyrics-theme";
/// Emitted with the connected monitors (`Vec<MonitorChoice>`) when they change.
pub const MONITORS_EVENT: &str = "taskbar-lyrics-monitors";

const POLL_STEP: Duration = Duration::from_millis(100);
const POLL_STEPS: u32 = 4;
/// Space kept free beside the neighbouring buttons, and the narrowest stretch worth using (CSS px at 96 DPI).
const MARGIN_CSS_PX: i32 = 8;
const MIN_WIDTH_CSS_PX: i32 = 120;

static STOP: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);
/// The settings the watcher follows (updated by `apply`).
static CURRENT: Mutex<Option<TaskbarLyricsSettings>> = Mutex::new(None);
static NEXT_WINDOW: AtomicU32 = AtomicU32::new(1);

/// A connected monitor as the settings page shows it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorChoice {
    /// Stable id stored in `customMonitors`.
    pub id: String,
    /// 1-based number, as close to Settings → Display as can be read.
    pub number: u32,
    pub name: String,
    pub primary: bool,
    /// Physical pixels.
    pub width: i32,
    pub height: i32,
}

impl From<&DisplayMonitor> for MonitorChoice {
    fn from(monitor: &DisplayMonitor) -> Self {
        Self {
            id: monitor.id.clone(),
            number: monitor.number,
            name: monitor.name.clone(),
            primary: monitor.primary,
            width: monitor.bounds.width(),
            height: monitor.bounds.height(),
        }
    }
}

/// Whether `label` is one of the lyrics windows.
pub fn is_window_label(label: &str) -> bool {
    label == WINDOW_LABEL_PREFIX
        || label
            .strip_prefix(WINDOW_LABEL_PREFIX)
            .is_some_and(|rest| rest.starts_with('-'))
}

pub fn system_uses_light_theme() -> bool {
    taskbar_uses_light_theme()
}

/// The connected monitors, numbered.
pub fn monitors() -> Vec<MonitorChoice> {
    list_monitors().iter().map(MonitorChoice::from).collect()
}

/// Whether the taskbar on one monitor should show lyrics.
///
/// `monitor_id` is the stable id of the taskbar's monitor (`None` when it is not among the listed monitors),
/// `on_player_monitor` whether the main window is on that monitor (`None` when the main window's monitor is unknown,
/// then the primary taskbar stands in).
pub fn wants_taskbar(
    mode: TaskbarLyricsDisplayMode,
    custom_monitors: &[String],
    monitor_id: Option<&str>,
    primary: bool,
    on_player_monitor: Option<bool>,
) -> bool {
    match mode {
        TaskbarLyricsDisplayMode::All => true,
        TaskbarLyricsDisplayMode::PlayerMonitor => on_player_monitor.unwrap_or(primary),
        TaskbarLyricsDisplayMode::Custom => {
            monitor_id.is_some_and(|id| custom_monitors.iter().any(|chosen| chosen == id))
        }
    }
}

/// Make the running state match `settings`: start the watcher (it opens the windows), or stop it and close them.
pub async fn apply(app: &AppHandle, settings: &TaskbarLyricsSettings) -> Result<(), String> {
    if let Ok(mut current) = CURRENT.lock() {
        *current = Some(settings.clone());
    }
    if settings.enabled {
        start_watcher(app);
        let _ = app.emit(SETTINGS_EVENT, settings);
    } else {
        shutdown(app);
    }
    Ok(())
}

/// Stop the watcher and close every lyrics window (also when the main window closes, so none can keep the process
/// alive).
pub fn shutdown(app: &AppHandle) {
    if let Some(stop) = STOP.lock().ok().and_then(|mut guard| guard.take()) {
        stop.store(true, Ordering::Relaxed);
    }
    for (label, window) in app.webview_windows() {
        if is_window_label(&label) {
            let _ = window.destroy();
        }
    }
}

fn open_window(app: &AppHandle, label: &str) -> Result<WebviewWindow, String> {
    let window = WebviewWindowBuilder::new(
        app,
        label,
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
    Ok(window)
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

/// One lyrics window and the taskbar it sits on.
struct Slot {
    label: String,
    visible: bool,
    /// The taskbar's DPI when the window was last placed: a change (the window moved to another monitor, or the
    /// scale changed) places it once more after Windows resized it for the new DPI.
    dpi: u32,
}

fn watch(app: AppHandle, stop: Arc<AtomicBool>) {
    let prober = match TaskbarProber::new() {
        Ok(prober) => prober,
        Err(error) => {
            eprintln!("工作列歌詞無法啟動：{error}");
            clear_stop(&stop);
            return;
        }
    };
    let mut slots: HashMap<isize, Slot> = HashMap::new();
    let mut last_light = None;
    let mut last_monitors: Option<Vec<DisplayMonitor>> = None;
    while !stop.load(Ordering::Relaxed) {
        let settings = CURRENT
            .lock()
            .ok()
            .and_then(|current| current.clone())
            .unwrap_or_default();
        let monitors = list_monitors();
        if last_monitors.as_ref() != Some(&monitors) {
            let choices: Vec<MonitorChoice> = monitors.iter().map(MonitorChoice::from).collect();
            let _ = app.emit(MONITORS_EVENT, choices);
            last_monitors = Some(monitors.clone());
        }
        let player_device = app
            .get_webview_window("main")
            .and_then(|main| main.hwnd().ok())
            .and_then(|hwnd| monitor_device_of_window(hwnd.0 as isize));
        let wanted: Vec<TaskbarWindow> = list_taskbars()
            .into_iter()
            .filter(|taskbar| {
                let monitor_id = monitors
                    .iter()
                    .find(|monitor| monitor.device == taskbar.monitor_device)
                    .map(|monitor| monitor.id.as_str());
                wants_taskbar(
                    settings.display_mode,
                    &settings.custom_monitors,
                    monitor_id,
                    taskbar.primary,
                    player_device
                        .as_ref()
                        .map(|device| *device == taskbar.monitor_device),
                )
            })
            .collect();

        // Taskbars that are gone (a monitor was unplugged, Explorer restarted) or no longer chosen lose their window.
        slots.retain(|tray, slot| {
            let keep = wanted.iter().any(|taskbar| taskbar.hwnd == *tray);
            if !keep {
                if let Some(window) = app.get_webview_window(&slot.label) {
                    let _ = window.destroy();
                }
            }
            keep
        });
        for taskbar in &wanted {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let slot = slots.entry(taskbar.hwnd).or_insert_with(|| Slot {
                label: format!(
                    "{WINDOW_LABEL_PREFIX}-{}",
                    NEXT_WINDOW.fetch_add(1, Ordering::Relaxed)
                ),
                visible: false,
                dpi: 0,
            });
            // Owned by its taskbar, the window dies with it (Explorer restarting): open a new one.
            let window = match app.get_webview_window(&slot.label) {
                Some(window) => window,
                None => match open_window(&app, &slot.label) {
                    Ok(window) => {
                        slot.visible = false;
                        slot.dpi = 0;
                        window
                    }
                    Err(error) => {
                        eprintln!("{error}");
                        continue;
                    }
                },
            };
            place(&window, slot, taskbar, &prober);
        }

        let light = taskbar_uses_light_theme();
        if last_light != Some(light) {
            last_light = Some(light);
            let _ = app.emit(THEME_EVENT, light);
        }
        for _ in 0..POLL_STEPS {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(POLL_STEP);
        }
    }
}

fn clear_stop(stop: &Arc<AtomicBool>) {
    if let Ok(mut guard) = STOP.lock() {
        if guard
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, stop))
        {
            *guard = None;
        }
    }
}

fn window_rect(hwnd: HWND) -> Option<Rect> {
    let mut raw = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut raw) }.ok()?;
    Some(Rect {
        left: raw.left,
        top: raw.top,
        right: raw.right,
        bottom: raw.bottom,
    })
}

fn show_at(hwnd: HWND, area: Rect) {
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
}

/// Keep one window over the free stretch of its own taskbar, or hide it.
fn place(window: &WebviewWindow, slot: &mut Slot, taskbar: &TaskbarWindow, prober: &TaskbarProber) {
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let hwnd = HWND(hwnd.0);
    // Owned by the taskbar, the window keeps its place above it: when the taskbar takes focus (a tray click, the
    // Windows quick settings) Windows lifts it into a higher layer than any ordinary topmost window.
    let tray = HWND(taskbar.hwnd as *mut _);
    if unsafe { GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) } != tray.0 as isize {
        unsafe { SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, tray.0 as isize) };
    }
    let probe = prober.probe_taskbar(taskbar).ok();
    let area = probe
        .as_ref()
        .and_then(|probe| lyrics_area(probe, MARGIN_CSS_PX, MIN_WIDTH_CSS_PX));
    match area {
        Some(area) => {
            show_at(hwnd, area);
            let dpi = probe.as_ref().map_or(0, |probe| probe.dpi);
            // Moving onto a monitor with another DPI makes Windows send WM_DPICHANGED, and the window is resized to
            // the size Windows suggests for the new scale: place it again so it covers exactly the stretch.
            if dpi != slot.dpi || window_rect(hwnd) != Some(area) {
                show_at(hwnd, area);
            }
            slot.dpi = dpi;
            slot.visible = true;
        }
        None if slot.visible => {
            let _ = unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_HIDEWINDOW,
                )
            };
            slot.visible = false;
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_picks_its_taskbars() {
        let all = TaskbarLyricsDisplayMode::All;
        assert!(wants_taskbar(all, &[], None, false, Some(false)));
        assert!(wants_taskbar(all, &[], Some("A"), true, None));

        let player = TaskbarLyricsDisplayMode::PlayerMonitor;
        assert!(
            wants_taskbar(player, &[], Some("B"), false, Some(true)),
            "the player is on this monitor"
        );
        assert!(
            !wants_taskbar(player, &[], Some("A"), true, Some(false)),
            "the player is elsewhere"
        );
        assert!(
            wants_taskbar(player, &[], Some("A"), true, None),
            "unknown: the primary taskbar stands in"
        );
        assert!(!wants_taskbar(player, &[], Some("B"), false, None));

        let custom = TaskbarLyricsDisplayMode::Custom;
        let chosen = vec!["B".to_owned(), "offline".to_owned()];
        assert!(wants_taskbar(custom, &chosen, Some("B"), false, None));
        assert!(!wants_taskbar(custom, &chosen, Some("A"), true, Some(true)));
        assert!(
            !wants_taskbar(custom, &chosen, None, false, None),
            "a monitor that is not listed"
        );
        assert!(
            !wants_taskbar(custom, &[], Some("A"), true, None),
            "nothing chosen shows nothing"
        );
    }

    #[test]
    fn lyrics_window_labels_share_the_prefix() {
        assert!(is_window_label("taskbar-lyrics"));
        assert!(is_window_label("taskbar-lyrics-3"));
        assert!(!is_window_label("taskbar-lyricsX"));
        assert!(!is_window_label("main"));
    }
}
