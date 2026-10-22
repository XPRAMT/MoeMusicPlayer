//! Floating lyrics on Windows (ADR 0032): one transparent, frameless, always-on-top window (`?view=floating-lyrics`)
//! fed by the same events as the taskbar lyrics. Move mode: the page drags the window and shows a lock button on
//! hover. Fixed mode: the window ignores the mouse (WS_EX_TRANSPARENT through `set_ignore_cursor_events`); a 100 ms
//! watcher polls the cursor and makes only the unlock button's rectangle clickable while the cursor is over it.
//! The position is saved in physical pixels and kept inside the monitors' work areas.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

use crate::settings::{FloatingLyricsPosition, FloatingLyricsSettings};

pub const WINDOW_LABEL: &str = "floating-lyrics";
pub const SETTINGS_EVENT: &str = "floating-lyrics-settings";
/// `true` while the cursor is over the window in fixed mode (the page then shows the unlock button).
pub const HOVER_EVENT: &str = "floating-lyrics-hover";

/// Keep in step with `FLOATING_LYRICS_LAYOUT` in `src/lib/floating-lyrics-settings.js`.
/// Height of one drawn line in em: the text's own height (1.25 em) with a 10% buffer.
pub const LINE_HEIGHT: f64 = 1.375;

const POLL_STEP: Duration = Duration::from_millis(100);
/// Every this many polls the window is checked against the monitors again (unplugged monitor, resolution change).
const PLACEMENT_CHECK_STEPS: u32 = 20;
const SAVE_DELAY: Duration = Duration::from_millis(700);

static CURRENT: Mutex<Option<FloatingLyricsSettings>> = Mutex::new(None);
static STOP: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);
static SAVE_GENERATION: AtomicU64 = AtomicU64::new(0);
/// Whether the line on screen has a translation; the window is only as tall as the lines it draws.
static HAS_TRANSLATION: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(self) -> i32 {
        self.right - self.left
    }
    pub fn height(self) -> i32 {
        self.bottom - self.top
    }
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= f64::from(self.left)
            && x < f64::from(self.right)
            && y >= f64::from(self.top)
            && y < f64::from(self.bottom)
    }
    fn overlap_area(self, other: Rect) -> i64 {
        let width = i64::from(self.right.min(other.right)) - i64::from(self.left.max(other.left));
        let height = i64::from(self.bottom.min(other.bottom)) - i64::from(self.top.max(other.top));
        if width <= 0 || height <= 0 {
            0
        } else {
            width * height
        }
    }
}

/// The lock button (and the transparent gutter right of the lyric area that holds it) in CSS px: as tall as the main
/// line, so it scales with the font and never props the window open.
pub fn lock_css(settings: &FloatingLyricsSettings) -> f64 {
    (f64::from(settings.font_size_px) * LINE_HEIGHT).ceil().max(1.0)
}

/// Lyric area width in CSS px: the chosen width, narrowed so area plus lock gutter fit a monitor `monitor_css` wide.
pub fn lyric_width_css(settings: &FloatingLyricsSettings, monitor_css: Option<f64>) -> f64 {
    let chosen = f64::from(settings.width_px);
    match monitor_css {
        Some(width) if width > 0.0 => chosen.min((width - lock_css(settings)).max(1.0)),
        _ => chosen,
    }
}

/// Window height in CSS px: the main line plus the translation line when one is drawn (text height x 1.1, no padding).
pub fn window_height_css(settings: &FloatingLyricsSettings, has_translation: bool) -> f64 {
    let font = f64::from(settings.font_size_px);
    let translation = if settings.show_translation && has_translation {
        font * f64::from(settings.translation_scale_percent) / 100.0
    } else {
        0.0
    };
    ((font + translation) * LINE_HEIGHT).ceil().max(1.0)
}

fn valid_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// The lock gutter in physical px.
pub fn gutter_px(settings: &FloatingLyricsSettings, scale: f64) -> i32 {
    (lock_css(settings) * valid_scale(scale)).round() as i32
}

/// The lock/unlock button in physical pixels: `lock_css` square at the right edge, vertically centred (the page draws
/// it at the same place).
pub fn unlock_button_rect(window: Rect, scale: f64, lock_css: f64) -> Rect {
    let size = (lock_css * valid_scale(scale)).round() as i32;
    let right = window.right;
    let top = window.top + (window.height() - size) / 2;
    Rect {
        left: right - size,
        top,
        right,
        bottom: top + size,
    }
}

/// Where the window goes: the saved position moved fully inside the work area it overlaps most, or, with no saved
/// position or one on no monitor any more, at the centre of the first (primary) work area.
pub fn place_on_monitors(
    position: Option<FloatingLyricsPosition>,
    size: (i32, i32),
    gutter: i32,
    work_areas: &[Rect],
) -> FloatingLyricsPosition {
    let (width, height) = size;
    let clamp_into = |area: Rect, x: i32, y: i32| FloatingLyricsPosition {
        x: x.clamp(area.left, (area.right - width).max(area.left)),
        y: y.clamp(area.top, (area.bottom - height).max(area.top)),
    };
    let Some(primary) = work_areas.first().copied() else {
        return position.unwrap_or(FloatingLyricsPosition { x: 0, y: 0 });
    };
    if let Some(saved) = position {
        let window = Rect {
            left: saved.x,
            top: saved.y,
            right: saved.x + width,
            bottom: saved.y + height,
        };
        if let Some(best) = work_areas
            .iter()
            .copied()
            .max_by_key(|area| area.overlap_area(window))
            .filter(|area| area.overlap_area(window) > 0)
        {
            return clamp_into(best, saved.x, saved.y);
        }
    }
    let centre = centre_on(primary, size, gutter);
    clamp_into(primary, centre.x, centre.y)
}

/// The position that puts the lyric area of a window of `size` (the window less its `gutter` px lock gutter on the
/// right) at the exact centre of `bounds`.
pub fn centre_on(bounds: Rect, size: (i32, i32), gutter: i32) -> FloatingLyricsPosition {
    let lyric = (size.0 - gutter.max(0)).max(0);
    FloatingLyricsPosition {
        x: bounds.left + (bounds.width() - lyric) / 2,
        y: bounds.top + (bounds.height() - size.1) / 2,
    }
}

/// 「水平置中」 / 「垂直置中」: centre the window on one axis of the monitor that holds its centre point (the first
/// monitor when none does), keeping the other coordinate.
pub fn centre_axis(window: Rect, monitors: &[Rect], horizontal: bool, gutter: i32) -> FloatingLyricsPosition {
    let (cx, cy) = (
        window.left + (window.width() - gutter.max(0)) / 2,
        window.top + window.height() / 2,
    );
    let holds = |area: &&Rect| cx >= area.left && cx < area.right && cy >= area.top && cy < area.bottom;
    let Some(monitor) = monitors.iter().find(holds).or(monitors.first()) else {
        return FloatingLyricsPosition {
            x: window.left,
            y: window.top,
        };
    };
    let centre = centre_on(*monitor, (window.width(), window.height()), gutter);
    if horizontal {
        FloatingLyricsPosition {
            x: centre.x,
            y: window.top,
        }
    } else {
        FloatingLyricsPosition {
            x: window.left,
            y: centre.y,
        }
    }
}

/// Centre the open window on one axis (see `centre_axis`); the move event saves the position.
pub fn centre_window_axis(app: &AppHandle, horizontal: bool) {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    let Some(rect) = window_rect(&window) else {
        return;
    };
    let monitors: Vec<Rect> = window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|monitor| Rect {
            left: monitor.position().x,
            top: monitor.position().y,
            right: monitor.position().x + monitor.size().width as i32,
            bottom: monitor.position().y + monitor.size().height as i32,
        })
        .collect();
    let gutter = current().map_or(0, |settings| gutter_px(&settings, window.scale_factor().unwrap_or(1.0)));
    place(&window, Some(centre_axis(rect, &monitors, horizontal, gutter)));
}

/// Width in CSS px of the monitor holding the window (the primary one when closed): the width slider's maximum.
pub fn screen_width_css(app: &AppHandle) -> Option<f64> {
    let monitor = match app.get_webview_window(WINDOW_LABEL) {
        Some(window) => window.current_monitor().ok().flatten(),
        None => None,
    }
    .or_else(|| app.primary_monitor().ok().flatten())?;
    Some(f64::from(monitor.size().width) / valid_scale(monitor.scale_factor()))
}

pub fn is_window_label(label: &str) -> bool {
    label == WINDOW_LABEL
}

pub fn current() -> Option<FloatingLyricsSettings> {
    CURRENT.lock().ok().and_then(|guard| guard.clone())
}

/// Open, update or close the window to match the settings, and broadcast them.
pub async fn apply(app: &AppHandle, settings: &FloatingLyricsSettings) -> Result<(), String> {
    if let Ok(mut current) = CURRENT.lock() {
        *current = Some(settings.clone());
    }
    let _ = app.emit(SETTINGS_EVENT, settings);
    if !settings.enabled {
        shutdown(app);
        return Ok(());
    }
    let window = match app.get_webview_window(WINDOW_LABEL) {
        Some(window) => window,
        None => open_window(app)?,
    };
    window
        .set_size(window_size(&window, settings))
        .map_err(|error| error.to_string())?;
    place(&window, settings.position);
    window
        .set_ignore_cursor_events(settings.locked)
        .map_err(|error| error.to_string())?;
    if !settings.locked {
        let _ = window.emit(HOVER_EVENT, false);
    }
    window.show().map_err(|error| error.to_string())?;
    start_watcher(app);
    Ok(())
}

/// Lyric area (narrowed to the monitor) plus the lock gutter, by the lines drawn.
fn window_size(window: &WebviewWindow, settings: &FloatingLyricsSettings) -> LogicalSize<f64> {
    let monitor_css = window
        .current_monitor()
        .ok()
        .flatten()
        .map(|monitor| f64::from(monitor.size().width) / valid_scale(monitor.scale_factor()));
    LogicalSize::new(
        lyric_width_css(settings, monitor_css) + lock_css(settings),
        window_height_css(settings, HAS_TRANSLATION.load(Ordering::SeqCst)),
    )
}

/// The page says whether the line it draws has a translation: grow or shrink the window to the lines (top-left kept,
/// then kept on screen).
pub fn fit_lines(app: &AppHandle, has_translation: bool) {
    if HAS_TRANSLATION.swap(has_translation, Ordering::SeqCst) == has_translation {
        return;
    }
    let (Some(settings), Some(window)) = (current(), app.get_webview_window(WINDOW_LABEL)) else {
        return;
    };
    if settings.enabled && window.set_size(window_size(&window, &settings)).is_ok() {
        if let Some(rect) = window_rect(&window) {
            place(
                &window,
                Some(FloatingLyricsPosition {
                    x: rect.left,
                    y: rect.top,
                }),
            );
        }
    }
}

/// The 面板 arrows in the settings: one physical px in any direction, kept on the work area (the move event saves the position).
pub fn nudge(app: &AppHandle, dx: i32, dy: i32) {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    if let Some(rect) = window_rect(&window) {
        place(
            &window,
            Some(FloatingLyricsPosition {
                x: rect.left + dx.clamp(-1, 1),
                y: rect.top + dy.clamp(-1, 1),
            }),
        );
    }
}

/// Stop the watcher and close the window.
pub fn shutdown(app: &AppHandle) {
    if let Some(stop) = STOP.lock().ok().and_then(|mut guard| guard.take()) {
        stop.store(true, Ordering::Relaxed);
    }
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.destroy();
    }
}

/// The window moved (dragged, or placed by us): remember the position and save it once moving stops.
pub fn remember_position(app: &AppHandle, position: PhysicalPosition<i32>) {
    let position = FloatingLyricsPosition {
        x: position.x,
        y: position.y,
    };
    {
        let Ok(mut guard) = CURRENT.lock() else {
            return;
        };
        let Some(current) = guard.as_mut() else {
            return;
        };
        if !current.enabled || current.position == Some(position) {
            return;
        }
        current.position = Some(position);
    }
    let generation = SAVE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SAVE_DELAY);
        if SAVE_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        let state = app.state::<crate::AppState>();
        if let Err(error) = state.settings.update(|stored| {
            stored.floating_lyrics.position = Some(position);
            Ok(())
        }) {
            eprintln!("floating lyrics position: {error}");
        }
    });
}

fn open_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    let window = WebviewWindowBuilder::new(
        app,
        WINDOW_LABEL,
        WebviewUrl::App("index.html?view=floating-lyrics".into()),
    )
    .title("浮動歌詞")
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .skip_taskbar(true)
    .always_on_top(true)
    .resizable(false)
    .focusable(false)
    .visible(false)
    .inner_size(
        f64::from(crate::settings::FloatingLyricsSettings::DEFAULT_WIDTH_PX),
        44.0,
    )
    .build()
    .map_err(|error| format!("無法開啟浮動歌詞視窗：{error}"))?;
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

fn work_areas(window: &WebviewWindow) -> Vec<Rect> {
    let mut areas: Vec<Rect> = Vec::new();
    let primary = window.primary_monitor().ok().flatten();
    let all = window.available_monitors().unwrap_or_default();
    for monitor in primary.iter().chain(all.iter()) {
        let area = monitor.work_area();
        let rect = Rect {
            left: area.position.x,
            top: area.position.y,
            right: area.position.x + area.size.width as i32,
            bottom: area.position.y + area.size.height as i32,
        };
        if rect.width() > 0 && rect.height() > 0 && !areas.contains(&rect) {
            areas.push(rect);
        }
    }
    areas
}

fn window_rect(window: &WebviewWindow) -> Option<Rect> {
    let position = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    Some(Rect {
        left: position.x,
        top: position.y,
        right: position.x + size.width as i32,
        bottom: position.y + size.height as i32,
    })
}

/// Move the window to `wanted` kept inside the visible monitors; None: the default place.
fn place(window: &WebviewWindow, wanted: Option<FloatingLyricsPosition>) {
    let Some(rect) = window_rect(window) else {
        return;
    };
    let size = (rect.width(), rect.height());
    let gutter = current().map_or(0, |settings| gutter_px(&settings, window.scale_factor().unwrap_or(1.0)));
    // No saved position (first open, or 重設位置): the exact centre of the primary monitor.
    let wanted = wanted.or_else(|| {
        let monitor = window.primary_monitor().ok().flatten()?;
        let bounds = Rect {
            left: monitor.position().x,
            top: monitor.position().y,
            right: monitor.position().x + monitor.size().width as i32,
            bottom: monitor.position().y + monitor.size().height as i32,
        };
        (bounds.width() > 0 && bounds.height() > 0).then(|| centre_on(bounds, size, gutter))
    });
    let target = place_on_monitors(wanted, size, gutter, &work_areas(window));
    if target.x != rect.left || target.y != rect.top {
        let _ = window.set_position(PhysicalPosition::new(target.x, target.y));
    }
}

fn start_watcher(app: &AppHandle) {
    let Ok(mut guard) = STOP.lock() else { return };
    if guard.is_some() {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    *guard = Some(Arc::clone(&stop));
    let app = app.clone();
    std::thread::Builder::new()
        .name("floating-lyrics".into())
        .spawn(move || watch(app, stop))
        .ok();
}

fn watch(app: AppHandle, stop: Arc<AtomicBool>) {
    let mut hovering = false;
    let mut clickable: Option<bool> = None;
    let mut last_locked: Option<bool> = None;
    let mut steps: u32 = 0;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(POLL_STEP);
        let Some(settings) = current().filter(|settings| settings.enabled) else {
            break;
        };
        let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
            break;
        };
        if last_locked != Some(settings.locked) {
            last_locked = Some(settings.locked);
            clickable = Some(!settings.locked);
            hovering = false;
        }
        steps = steps.wrapping_add(1);
        if steps.is_multiple_of(PLACEMENT_CHECK_STEPS) {
            if let Some(rect) = window_rect(&window) {
                place(
                    &window,
                    Some(FloatingLyricsPosition {
                        x: rect.left,
                        y: rect.top,
                    }),
                );
            }
        }
        if !settings.locked {
            continue;
        }
        let Some(rect) = window_rect(&window) else {
            continue;
        };
        let scale = window.scale_factor().unwrap_or(1.0);
        let (inside, over_button) = match app.cursor_position() {
            Ok(cursor) => (
                rect.contains(cursor.x, cursor.y),
                unlock_button_rect(rect, scale, lock_css(&settings)).contains(cursor.x, cursor.y),
            ),
            Err(_) => (false, false),
        };
        if inside != hovering {
            hovering = inside;
            let _ = window.emit(HOVER_EVENT, inside);
        }
        if clickable != Some(over_button) {
            clickable = Some(over_button);
            let _ = window.set_ignore_cursor_events(!over_button);
        }
    }
    if let Ok(mut guard) = STOP.lock() {
        if guard
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &stop))
        {
            *guard = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn height_follows_font_and_translation() {
        let mut settings = FloatingLyricsSettings {
            font_size_px: 30,
            translation_scale_percent: 50,
            show_translation: true,
            ..Default::default()
        };
        // (30 + 15) * 1.375 = 61.875 -> 62; 30 * 1.375 = 41.25 -> 42. Only the text: no padding, no lock minimum.
        assert_eq!(window_height_css(&settings, true), 62.0);
        assert_eq!(window_height_css(&settings, false), 42.0, "no translation on this line: shrink");
        settings.show_translation = false;
        assert_eq!(window_height_css(&settings, true), 42.0);
        settings.font_size_px = 8;
        assert_eq!(window_height_css(&settings, true), 11.0, "small text: a small panel");
        assert_eq!(lock_css(&settings), 11.0, "the lock follows the main line");
    }

    #[test]
    fn lyric_width_leaves_room_for_the_lock_gutter() {
        let settings = FloatingLyricsSettings {
            font_size_px: 32,
            width_px: 3000,
            ..Default::default()
        };
        assert_eq!(lock_css(&settings), 44.0);
        assert_eq!(lyric_width_css(&settings, Some(1920.0)), 1920.0 - 44.0);
        assert_eq!(lyric_width_css(&settings, None), 3000.0);
        assert_eq!(gutter_px(&settings, 1.5), 66);
    }

    #[test]
    fn unlock_button_scales_with_the_font_and_dpi() {
        let window = rect(1000, 500, 2600, 620);
        assert_eq!(unlock_button_rect(window, 1.0, 32.0), rect(2568, 544, 2600, 576));
        let big = unlock_button_rect(window, 2.5, 32.0);
        assert_eq!((big.width(), big.height()), (80, 80));
        assert_eq!(big.right, 2600);
        assert_eq!(big.top, 500 + (120 - 80) / 2);
        assert_eq!(
            unlock_button_rect(window, f64::NAN, 32.0),
            unlock_button_rect(window, 1.0, 32.0)
        );
    }

    #[test]
    fn centring_one_axis_uses_the_monitor_under_the_window_centre() {
        let monitors = [rect(0, 0, 1920, 1080), rect(1920, 0, 4480, 1440)];
        // Centre point (2420, 150) is on the second monitor.
        let window = rect(2020, 100, 2820, 200);
        assert_eq!(
            centre_axis(window, &monitors, true, 0),
            FloatingLyricsPosition { x: 1920 + (2560 - 800) / 2, y: 100 }
        );
        // With a 40 px lock gutter only the 760 px lyric area is centred.
        assert_eq!(
            centre_axis(window, &monitors, true, 40),
            FloatingLyricsPosition { x: 1920 + (2560 - 760) / 2, y: 100 }
        );
        assert_eq!(
            centre_axis(window, &monitors, false, 40),
            FloatingLyricsPosition { x: 2020, y: (1440 - 100) / 2 }
        );
        // Off every monitor: the first one.
        assert_eq!(
            centre_axis(rect(-5000, 10, -4800, 60), &monitors, true, 0),
            FloatingLyricsPosition { x: (1920 - 200) / 2, y: 10 }
        );
    }

    #[test]
    fn placement_keeps_the_window_on_a_visible_monitor() {
        let primary = rect(0, 0, 3840, 2100);
        let right = rect(3840, 712, 6400, 2112);
        let areas = [primary, right];
        let size = (1600, 120);
        let default = place_on_monitors(None, size, 0, &areas);
        assert_eq!(
            place_on_monitors(None, size, 100, &areas).x,
            (3840 - 1500) / 2,
            "first use: the lyric area at the centre"
        );
        assert_eq!(
            default,
            FloatingLyricsPosition {
                x: 1120,
                y: (2100 - 120) / 2
            }
        );
        assert_eq!(
            centre_on(rect(0, 0, 2560, 1440), size, 0),
            FloatingLyricsPosition { x: 480, y: 660 }
        );
        assert_eq!(
            centre_on(rect(-1920, 0, 0, 1080), (200, 100), 0),
            FloatingLyricsPosition { x: -1060, y: 490 }
        );
        let inside = FloatingLyricsPosition { x: 4000, y: 1000 };
        assert_eq!(place_on_monitors(Some(inside), size, 0, &areas), inside);
        let hanging = FloatingLyricsPosition { x: 5500, y: 2050 };
        assert_eq!(
            place_on_monitors(Some(hanging), size, 0, &areas),
            FloatingLyricsPosition { x: 4800, y: 1992 }
        );
        let gone = FloatingLyricsPosition { x: -5000, y: 200 };
        assert_eq!(place_on_monitors(Some(gone), size, 0, &areas), default);
        let straddling = FloatingLyricsPosition { x: 3000, y: 800 };
        assert_eq!(
            place_on_monitors(Some(straddling), size, 0, &areas),
            FloatingLyricsPosition { x: 2240, y: 800 }
        );
        assert_eq!(place_on_monitors(Some(inside), size, 0, &[]), inside);
    }
}
