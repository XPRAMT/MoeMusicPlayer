//! Floating lyrics on Windows (ADR 0032): one transparent, frameless, always-on-top native window (ADR 0034)
//! fed by the same events as the taskbar lyrics. Move mode: the page drags the window and shows a lock button on
//! hover. Fixed mode: the window ignores the mouse (WS_EX_TRANSPARENT through `set_ignore_cursor_events`); a 100 ms
//! watcher polls the cursor and makes only the unlock button's rectangle clickable while the cursor is over it.
//! The position is saved in physical pixels and kept inside the monitors' work areas.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HDC, HMONITOR,
    MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowRect, IsWindow, SetWindowPos, HWND_TOPMOST, MONITORINFOF_PRIMARY,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
};

use crate::settings::{FloatingLyricsPosition, FloatingLyricsSettings};

pub const WINDOW_LABEL: &str = "floating-lyrics";
pub const SETTINGS_EVENT: &str = "floating-lyrics-settings";

/// Keep in step with `FLOATING_LYRICS_LAYOUT` in `src/lib/floating-lyrics-settings.js`.
/// Height of one drawn line in em: the text's own height (1.25 em) with a 10% buffer. The lyric area (the part with the
/// move-mode background, where the text is drawn and aligned) is the lines at this height: text height x 1.1.
pub const LINE_HEIGHT: f64 = 1.375;
/// The whole window is text height x 1.5 (= lyric area x 1.5 / 1.1), the lyric area vertically centred: the transparent
/// room above and below is for the text shadow / glow.
pub const PANEL_LINE_HEIGHT: f64 = 1.875;
/// Transparent room left of the lyric area and right of the lock, in em of the main font (for the shadow / glow).
pub const SIDE_MARGIN_EM: f64 = 0.25;

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
    (f64::from(settings.font_size_px) * LINE_HEIGHT)
        .ceil()
        .max(1.0)
}

/// The transparent side margin (CSS px) left of the lyric area and again right of the lock gutter.
pub fn side_margin_css(settings: &FloatingLyricsSettings) -> f64 {
    (f64::from(settings.font_size_px) * SIDE_MARGIN_EM)
        .ceil()
        .max(1.0)
}

/// Everything in the window beside the lyric area, CSS px: both side margins and the lock gutter.
pub fn chrome_width_css(settings: &FloatingLyricsSettings) -> f64 {
    2.0 * side_margin_css(settings) + lock_css(settings)
}

/// Lyric area width in CSS px: the chosen width, narrowed so the whole window (area, lock gutter and side margins) fits
/// a monitor `monitor_css` wide.
pub fn lyric_width_css(settings: &FloatingLyricsSettings, monitor_css: Option<f64>) -> f64 {
    let chosen = f64::from(settings.width_px);
    match monitor_css {
        Some(width) if width > 0.0 => chosen.min((width - chrome_width_css(settings)).max(1.0)),
        _ => chosen,
    }
}

/// Font size sum of the lines drawn: the main line plus the translation line when one is drawn.
fn lines_em_px(settings: &FloatingLyricsSettings, has_translation: bool) -> f64 {
    let font = f64::from(settings.font_size_px);
    let translation = if settings.show_translation && has_translation {
        font * f64::from(settings.translation_scale_percent) / 100.0
    } else {
        0.0
    };
    font + translation
}

/// Lyric area height in CSS px: the lines drawn at text height x 1.1.
/// (The page draws the area as the middle 1.1 / 1.5 of the window; this states the rule for the tests.)
#[cfg_attr(not(test), allow(dead_code))]
pub fn lyric_height_css(settings: &FloatingLyricsSettings, has_translation: bool) -> f64 {
    (lines_em_px(settings, has_translation) * LINE_HEIGHT)
        .ceil()
        .max(1.0)
}

/// Window height in CSS px: text height x 1.5, so the shadow / glow has transparent room above and below the area.
pub fn window_height_css(settings: &FloatingLyricsSettings, has_translation: bool) -> f64 {
    (lines_em_px(settings, has_translation) * PANEL_LINE_HEIGHT)
        .ceil()
        .max(1.0)
}

fn valid_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// What the window holds beside the lyric area, physical px: `lead` left of it (the side margin), `trail` right of it
/// (the lock gutter plus the side margin).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Gutters {
    pub lead: i32,
    pub trail: i32,
}

impl Gutters {
    pub const NONE: Gutters = Gutters { lead: 0, trail: 0 };

    fn lyric_width(self, window_width: i32) -> i32 {
        (window_width - self.lead.max(0) - self.trail.max(0)).max(0)
    }
}

/// The margins and the lock gutter in physical px.
pub fn gutter_px(settings: &FloatingLyricsSettings, scale: f64) -> Gutters {
    let scale = valid_scale(scale);
    let margin = side_margin_css(settings);
    Gutters {
        lead: (margin * scale).round() as i32,
        trail: ((lock_css(settings) + margin) * scale).round() as i32,
    }
}

/// The lock/unlock button in physical pixels: `lock_css` square right of the lyric area, `margin_css` in from the right
/// edge, vertically centred (the page draws it at the same place).
pub fn unlock_button_rect(window: Rect, scale: f64, lock_css: f64, margin_css: f64) -> Rect {
    let size = (lock_css * valid_scale(scale)).round() as i32;
    let right = window.right - (margin_css.max(0.0) * valid_scale(scale)).round() as i32;
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
    gutter: Gutters,
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

/// The position that puts the lyric area of a window of `size` (the window less its `gutter` margins and lock gutter)
/// at the exact centre of `bounds`.
pub fn centre_on(bounds: Rect, size: (i32, i32), gutter: Gutters) -> FloatingLyricsPosition {
    let lyric = gutter.lyric_width(size.0);
    FloatingLyricsPosition {
        x: bounds.left + (bounds.width() - lyric) / 2 - gutter.lead.max(0),
        y: bounds.top + (bounds.height() - size.1) / 2,
    }
}

/// 「水平置中」 / 「垂直置中」: centre the window on one axis of the monitor that holds its centre point (the first
/// monitor when none does), keeping the other coordinate.
pub fn centre_axis(
    window: Rect,
    monitors: &[Rect],
    horizontal: bool,
    gutter: Gutters,
) -> FloatingLyricsPosition {
    let (cx, cy) = (
        window.left + gutter.lead.max(0) + gutter.lyric_width(window.width()) / 2,
        window.top + window.height() / 2,
    );
    let holds =
        |area: &&Rect| cx >= area.left && cx < area.right && cy >= area.top && cy < area.bottom;
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

/// The native window (ADR 0034): its HWND as created by the renderer.
static HWND_VALUE: AtomicIsize = AtomicIsize::new(0);

#[derive(Clone, Copy)]
struct Native(HWND);

fn window() -> Option<Native> {
    let raw = HWND_VALUE.load(Ordering::SeqCst);
    let hwnd = HWND(raw as *mut _);
    (raw != 0 && unsafe { IsWindow(Some(hwnd)) }.as_bool()).then_some(Native(hwnd))
}

fn to_rect(raw: RECT) -> Rect {
    Rect {
        left: raw.left,
        top: raw.top,
        right: raw.right,
        bottom: raw.bottom,
    }
}

/// Every monitor: (bounds, work area, primary).
fn monitors() -> Vec<(Rect, Rect, bool)> {
    unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = &mut *(data.0 as *mut Vec<(Rect, Rect, bool)>);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            list.push((
                to_rect(info.rcMonitor),
                to_rect(info.rcWork),
                info.dwFlags & MONITORINFOF_PRIMARY != 0,
            ));
        }
        TRUE
    }
    let mut list: Vec<(Rect, Rect, bool)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut list as *mut _ as isize));
    }
    // The primary monitor first (the default place and the first work area).
    list.sort_by_key(|m| !m.2);
    list
}

impl Native {
    fn rect(self) -> Option<Rect> {
        let mut raw = RECT::default();
        unsafe { GetWindowRect(self.0, &mut raw) }.ok()?;
        Some(to_rect(raw))
    }
    fn scale(self) -> f64 {
        f64::from(unsafe { GetDpiForWindow(self.0) }.max(96)) / 96.0
    }
    /// The monitor holding the window: (bounds, scale).
    fn monitor(self) -> Option<(Rect, f64)> {
        let monitor = unsafe { MonitorFromWindow(self.0, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        unsafe { GetMonitorInfoW(monitor, &mut info) }
            .as_bool()
            .then(|| (to_rect(info.rcMonitor), self.scale()))
    }
    fn set_position(self, x: i32, y: i32) {
        let _ = unsafe {
            SetWindowPos(
                self.0,
                Some(HWND_TOPMOST),
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
        };
    }
    fn set_size(self, width: i32, height: i32) {
        let _ = unsafe {
            SetWindowPos(
                self.0,
                None,
                0,
                0,
                width,
                height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
    }
    fn show(self) {
        let _ = unsafe {
            SetWindowPos(
                self.0,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        };
    }
}

/// Centre the open window on one axis (see `centre_axis`); the move event saves the position.
pub fn centre_window_axis(_app: &AppHandle, horizontal: bool) {
    let Some(window) = window() else { return };
    let Some(rect) = window.rect() else { return };
    let bounds: Vec<Rect> = monitors().iter().map(|m| m.0).collect();
    let gutter = current().map_or(Gutters::NONE, |settings| {
        gutter_px(&settings, window.scale())
    });
    place(window, Some(centre_axis(rect, &bounds, horizontal, gutter)));
}

/// Width in CSS px of the monitor holding the window (the primary one when closed): the width slider's maximum.
pub fn screen_width_css(_app: &AppHandle) -> Option<f64> {
    match window().and_then(Native::monitor) {
        Some((bounds, scale)) => Some(f64::from(bounds.width()) / valid_scale(scale)),
        None => {
            let primary = monitors().into_iter().next()?;
            let dpi = unsafe {
                let monitor = MonitorFromPoint(
                    POINT {
                        x: primary.0.left,
                        y: primary.0.top,
                    },
                    MONITOR_DEFAULTTOPRIMARY,
                );
                let (mut x, mut y) = (96u32, 96u32);
                let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y);
                x.max(96)
            };
            Some(f64::from(primary.0.width()) / (f64::from(dpi) / 96.0))
        }
    }
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
    let renderer =
        crate::native_lyrics::renderer(app).ok_or_else(|| "無法開啟浮動歌詞視窗".to_owned())?;
    let window = match window() {
        Some(window) => window,
        None => {
            let hwnd = renderer
                .create(WINDOW_LABEL, player_lyrics_native::Kind::Floating, None)
                .ok_or_else(|| "無法開啟浮動歌詞視窗".to_owned())?;
            HWND_VALUE.store(hwnd, Ordering::SeqCst);
            Native(HWND(hwnd as *mut _))
        }
    };
    renderer.set_look(
        WINDOW_LABEL,
        Some(crate::native_lyrics::floating_look(settings)),
    );
    crate::native_lyrics::apply_floating_chrome(app, settings);
    let (width, height) = window_size(window, settings);
    window.set_size(width, height);
    place(window, settings.position);
    renderer.set_click_through(WINDOW_LABEL, settings.locked);
    if !settings.locked {
        renderer.set_hover(WINDOW_LABEL, false);
    }
    window.show();
    start_watcher(app);
    Ok(())
}

/// Lyric area (narrowed to the monitor) plus the lock gutter and the side margins; text height x 1.5 tall (physical px).
fn window_size(window: Native, settings: &FloatingLyricsSettings) -> (i32, i32) {
    let monitor_css = window
        .monitor()
        .map(|(bounds, scale)| f64::from(bounds.width()) / valid_scale(scale));
    let scale = window.scale();
    let width = lyric_width_css(settings, monitor_css) + chrome_width_css(settings);
    let height = window_height_css(settings, HAS_TRANSLATION.load(Ordering::SeqCst));
    (
        (width * scale).round() as i32,
        (height * scale).round() as i32,
    )
}

/// The renderer says whether the line it draws has a translation: grow or shrink the window to the lines (top-left
/// kept, then kept on screen).
pub fn fit_lines(_app: &AppHandle, has_translation: bool) {
    if HAS_TRANSLATION.swap(has_translation, Ordering::SeqCst) == has_translation {
        return;
    }
    let (Some(settings), Some(window)) = (current(), window()) else {
        return;
    };
    if settings.enabled {
        let (width, height) = window_size(window, &settings);
        window.set_size(width, height);
        if let Some(rect) = window.rect() {
            place(
                window,
                Some(FloatingLyricsPosition {
                    x: rect.left,
                    y: rect.top,
                }),
            );
        }
    }
}

/// The 面板 arrows in the settings: one physical px in any direction, kept on the work area (the move event saves the position).
pub fn nudge(_app: &AppHandle, dx: i32, dy: i32) {
    let Some(window) = window() else { return };
    if let Some(rect) = window.rect() {
        place(
            window,
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
    if HWND_VALUE.swap(0, Ordering::SeqCst) != 0 {
        if let Some(renderer) = crate::native_lyrics::renderer(app) {
            renderer.destroy(WINDOW_LABEL);
        }
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

fn work_areas() -> Vec<Rect> {
    let mut areas: Vec<Rect> = Vec::new();
    for (_, work, _) in monitors() {
        if work.width() > 0 && work.height() > 0 && !areas.contains(&work) {
            areas.push(work);
        }
    }
    areas
}

/// Move the window to `wanted` kept inside the visible monitors; None: the default place.
fn place(window: Native, wanted: Option<FloatingLyricsPosition>) {
    let Some(rect) = window.rect() else { return };
    let size = (rect.width(), rect.height());
    let gutter = current().map_or(Gutters::NONE, |settings| {
        gutter_px(&settings, window.scale())
    });
    // No saved position (first open, or 重設位置): the exact centre of the primary monitor.
    let wanted = wanted.or_else(|| {
        let bounds = monitors().first()?.0;
        (bounds.width() > 0 && bounds.height() > 0).then(|| centre_on(bounds, size, gutter))
    });
    let target = place_on_monitors(wanted, size, gutter, &work_areas());
    if target.x != rect.left || target.y != rect.top {
        window.set_position(target.x, target.y);
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
        let Some(window) = window() else {
            break;
        };
        let Some(renderer) = crate::native_lyrics::renderer(&app) else {
            break;
        };
        if last_locked != Some(settings.locked) {
            last_locked = Some(settings.locked);
            clickable = Some(!settings.locked);
            hovering = false;
        }
        steps = steps.wrapping_add(1);
        if steps.is_multiple_of(PLACEMENT_CHECK_STEPS) {
            if let Some(rect) = window.rect() {
                place(
                    window,
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
        let Some(rect) = window.rect() else {
            continue;
        };
        let mut cursor = POINT::default();
        let (inside, over_button) = if unsafe { GetCursorPos(&mut cursor) }.is_ok() {
            let (x, y) = (f64::from(cursor.x), f64::from(cursor.y));
            (
                rect.contains(x, y),
                unlock_button_rect(
                    rect,
                    window.scale(),
                    lock_css(&settings),
                    side_margin_css(&settings),
                )
                .contains(x, y),
            )
        } else {
            (false, false)
        };
        if inside != hovering {
            hovering = inside;
            renderer.set_hover(WINDOW_LABEL, inside);
        }
        if clickable != Some(over_button) {
            clickable = Some(over_button);
            renderer.set_click_through(WINDOW_LABEL, !over_button);
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
        // Lyric area = text height x 1.1: (30 + 15) * 1.375 = 61.875 -> 62; 30 * 1.375 = 41.25 -> 42.
        assert_eq!(lyric_height_css(&settings, true), 62.0);
        assert_eq!(
            lyric_height_css(&settings, false),
            42.0,
            "no translation on this line: shrink"
        );
        // Window = text height x 1.5: 45 * 1.875 = 84.375 -> 85; 30 * 1.875 = 56.25 -> 57.
        assert_eq!(window_height_css(&settings, true), 85.0);
        assert_eq!(window_height_css(&settings, false), 57.0);
        settings.show_translation = false;
        assert_eq!(window_height_css(&settings, true), 57.0);
        settings.font_size_px = 8;
        assert_eq!(
            window_height_css(&settings, true),
            15.0,
            "small text: a small panel"
        );
        assert_eq!(lock_css(&settings), 11.0, "the lock follows the main line");
        assert_eq!(side_margin_css(&settings), 2.0);
    }

    #[test]
    fn lyric_width_leaves_room_for_the_lock_gutter() {
        let settings = FloatingLyricsSettings {
            font_size_px: 32,
            width_px: 3000,
            ..Default::default()
        };
        assert_eq!(lock_css(&settings), 44.0);
        assert_eq!(side_margin_css(&settings), 8.0);
        // The window (8 + area + 44 + 8) fits the monitor.
        assert_eq!(lyric_width_css(&settings, Some(1920.0)), 1920.0 - 60.0);
        assert_eq!(lyric_width_css(&settings, None), 3000.0);
        assert_eq!(
            gutter_px(&settings, 1.5),
            Gutters {
                lead: 12,
                trail: 78
            }
        );
    }

    #[test]
    fn unlock_button_scales_with_the_font_and_dpi() {
        let window = rect(1000, 500, 2600, 620);
        assert_eq!(
            unlock_button_rect(window, 1.0, 32.0, 0.0),
            rect(2568, 544, 2600, 576)
        );
        assert_eq!(
            unlock_button_rect(window, 1.0, 32.0, 8.0),
            rect(2560, 544, 2592, 576),
            "in from the margin"
        );
        let big = unlock_button_rect(window, 2.5, 32.0, 8.0);
        assert_eq!((big.width(), big.height()), (80, 80));
        assert_eq!(big.right, 2600 - 20);
        assert_eq!(big.top, 500 + (120 - 80) / 2);
        assert_eq!(
            unlock_button_rect(window, f64::NAN, 32.0, 8.0),
            unlock_button_rect(window, 1.0, 32.0, 8.0)
        );
    }

    #[test]
    fn centring_one_axis_uses_the_monitor_under_the_window_centre() {
        let monitors = [rect(0, 0, 1920, 1080), rect(1920, 0, 4480, 1440)];
        // Centre point (2420, 150) is on the second monitor.
        let window = rect(2020, 100, 2820, 200);
        assert_eq!(
            centre_axis(window, &monitors, true, Gutters::NONE),
            FloatingLyricsPosition {
                x: 1920 + (2560 - 800) / 2,
                y: 100
            }
        );
        // With a 40 px lock gutter only the 760 px lyric area is centred.
        let lock = Gutters { lead: 0, trail: 40 };
        assert_eq!(
            centre_axis(window, &monitors, true, lock),
            FloatingLyricsPosition {
                x: 1920 + (2560 - 760) / 2,
                y: 100
            }
        );
        // A 10 px side margin on each side: the 740 px area is centred, the window starts 10 px left of it.
        let margins = Gutters {
            lead: 10,
            trail: 50,
        };
        assert_eq!(
            centre_axis(window, &monitors, true, margins),
            FloatingLyricsPosition {
                x: 1920 + (2560 - 740) / 2 - 10,
                y: 100
            }
        );
        assert_eq!(
            centre_axis(window, &monitors, false, lock),
            FloatingLyricsPosition {
                x: 2020,
                y: (1440 - 100) / 2
            }
        );
        // Off every monitor: the first one.
        assert_eq!(
            centre_axis(rect(-5000, 10, -4800, 60), &monitors, true, Gutters::NONE),
            FloatingLyricsPosition {
                x: (1920 - 200) / 2,
                y: 10
            }
        );
    }

    #[test]
    fn placement_keeps_the_window_on_a_visible_monitor() {
        let primary = rect(0, 0, 3840, 2100);
        let right = rect(3840, 712, 6400, 2112);
        let areas = [primary, right];
        let size = (1600, 120);
        let default = place_on_monitors(None, size, Gutters::NONE, &areas);
        assert_eq!(
            place_on_monitors(
                None,
                size,
                Gutters {
                    lead: 0,
                    trail: 100
                },
                &areas
            )
            .x,
            (3840 - 1500) / 2,
            "first use: the lyric area at the centre"
        );
        assert_eq!(
            place_on_monitors(
                None,
                size,
                Gutters {
                    lead: 20,
                    trail: 80
                },
                &areas
            )
            .x,
            (3840 - 1500) / 2 - 20,
            "first use with side margins: still the lyric area at the centre"
        );
        assert_eq!(
            default,
            FloatingLyricsPosition {
                x: 1120,
                y: (2100 - 120) / 2
            }
        );
        assert_eq!(
            centre_on(rect(0, 0, 2560, 1440), size, Gutters::NONE),
            FloatingLyricsPosition { x: 480, y: 660 }
        );
        assert_eq!(
            centre_on(rect(-1920, 0, 0, 1080), (200, 100), Gutters::NONE),
            FloatingLyricsPosition { x: -1060, y: 490 }
        );
        let inside = FloatingLyricsPosition { x: 4000, y: 1000 };
        assert_eq!(
            place_on_monitors(Some(inside), size, Gutters::NONE, &areas),
            inside
        );
        let hanging = FloatingLyricsPosition { x: 5500, y: 2050 };
        assert_eq!(
            place_on_monitors(Some(hanging), size, Gutters::NONE, &areas),
            FloatingLyricsPosition { x: 4800, y: 1992 }
        );
        let gone = FloatingLyricsPosition { x: -5000, y: 200 };
        assert_eq!(
            place_on_monitors(Some(gone), size, Gutters::NONE, &areas),
            default
        );
        let straddling = FloatingLyricsPosition { x: 3000, y: 800 };
        assert_eq!(
            place_on_monitors(Some(straddling), size, Gutters::NONE, &areas),
            FloatingLyricsPosition { x: 2240, y: 800 }
        );
        assert_eq!(
            place_on_monitors(Some(inside), size, Gutters::NONE, &[]),
            inside
        );
    }
}
