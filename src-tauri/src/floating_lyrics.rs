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
/// The margins and rooms the open window was last placed with: the saved position is the lyric area's top-left, the
/// window's plus these.
static PLACED_WITH: Mutex<Gutters> = Mutex::new(Gutters::NONE);

fn placed_with() -> Gutters {
    PLACED_WITH.lock().map_or(Gutters::NONE, |guard| *guard)
}

/// The window's gutters now: the current settings at the window's DPI and the line on screen.
fn window_gutter(window: Native, settings: &FloatingLyricsSettings) -> Gutters {
    gutter_px(
        settings,
        window.scale(),
        HAS_TRANSLATION.load(Ordering::SeqCst),
    )
}

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
pub fn lyric_height_css(settings: &FloatingLyricsSettings, has_translation: bool) -> f64 {
    (lines_em_px(settings, has_translation) * LINE_HEIGHT)
        .ceil()
        .max(1.0)
}

/// Lyric area height in physical px: the lines drawn at text height x 1.1, rounded up so they always fit.
pub fn area_height_px(settings: &FloatingLyricsSettings, scale: f64, has_translation: bool) -> i32 {
    (lyric_height_css(settings, has_translation) * valid_scale(scale)).ceil() as i32
}

fn valid_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// What the window holds around the lyric area, physical px: `lead` left of it (the side margin), `trail` right of it
/// (the lock gutter plus the side margin), `top` above it (from the main row alone, so a translation row never moves
/// the first row) and `bottom` below it (follows the whole area). The saved position is the lyric area's top-left, i.e.
/// the window's plus (`lead`, `top`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Gutters {
    pub lead: i32,
    pub trail: i32,
    pub top: i32,
    pub bottom: i32,
}

impl Gutters {
    pub const NONE: Gutters = Gutters {
        lead: 0,
        trail: 0,
        top: 0,
        bottom: 0,
    };

    /// The window position of a lyric area whose top-left is `area`.
    pub fn window_from_area(self, area: FloatingLyricsPosition) -> FloatingLyricsPosition {
        FloatingLyricsPosition {
            x: area.x - self.lead.max(0),
            y: area.y - self.top.max(0),
        }
    }

    /// The lyric area's top-left of a window at `window` (what is saved).
    pub fn area_from_window(self, window: FloatingLyricsPosition) -> FloatingLyricsPosition {
        FloatingLyricsPosition {
            x: window.x + self.lead.max(0),
            y: window.y + self.top.max(0),
        }
    }

    /// The lyric area's height in a window `window_height` tall.
    fn area_height(self, window_height: i32) -> i32 {
        (window_height - self.top.max(0) - self.bottom.max(0)).max(0)
    }

    fn lyric_width(self, window_width: i32) -> i32 {
        (window_width - self.lead.max(0) - self.trail.max(0)).max(0)
    }
}

/// The margins, the lock gutter and the rooms above and below the lyric area in physical px.
pub fn gutter_px(settings: &FloatingLyricsSettings, scale: f64, has_translation: bool) -> Gutters {
    let scale = valid_scale(scale);
    let margin = side_margin_css(settings);
    Gutters {
        lead: (margin * scale).round() as i32,
        trail: ((lock_css(settings) + margin) * scale).round() as i32,
        top: player_lyrics_native::floating_top_room_px(f64::from(settings.font_size_px), scale),
        bottom: player_lyrics_native::floating_bottom_room_px(area_height_px(
            settings,
            scale,
            has_translation,
        )),
    }
}

/// The lock/unlock button in physical pixels: `lock_css` square right of the lyric area, `margin_css` in from the right
/// edge, vertically centred on the lyric area (the renderer draws it at the same place).
pub fn unlock_button_rect(
    window: Rect,
    scale: f64,
    lock_css: f64,
    margin_css: f64,
    gutter: Gutters,
) -> Rect {
    let size = (lock_css * valid_scale(scale)).round() as i32;
    let right = window.right - (margin_css.max(0.0) * valid_scale(scale)).round() as i32;
    let top = window.top
        + player_lyrics_native::floating_lock_top_px(
            gutter.top,
            gutter.area_height(window.height()),
            size,
        );
    Rect {
        left: right - size,
        top,
        right,
        bottom: top + size,
    }
}

/// How far the window's transparent edges may hang past the work area (physical px): the side margin left of the
/// lyric area, the side margin right of the lock column (so the padlock stays reachable) and the transparent room
/// above and below the lyric area. The lyric text area itself always stays inside.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Insets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Insets {
    #[cfg(test)]
    pub const NONE: Insets = Insets {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
}

/// The insets of a floating window with `gutter`: the margins and the rooms above and below the lyric area may hang
/// past the work area, the area itself stays inside (ADR 0032).
pub fn text_insets(gutter: Gutters) -> Insets {
    Insets {
        left: gutter.lead.max(0),
        top: gutter.top.max(0),
        right: gutter.lead.max(0),
        bottom: gutter.bottom.max(0),
    }
}

/// Where the lyric area's horizontal centre stays when the window's width changes: the new left edge for a window
/// that was `old_width` wide at `left` and becomes `new_width` (gutters before and after).
pub fn resize_around_centre(
    left: i32,
    old_width: i32,
    old_gutter: Gutters,
    new_width: i32,
    new_gutter: Gutters,
) -> i32 {
    let centre2 = 2 * (left + old_gutter.lead.max(0)) + old_gutter.lyric_width(old_width);
    (centre2 - new_gutter.lyric_width(new_width)) / 2 - new_gutter.lead.max(0)
}

/// Where `apply` puts the window (before `place` keeps it on screen). `shown`: the rect of a window that was already
/// open, None for one just created (it starts as a 1 x 1 px window at 0,0, so its rect says nothing). The saved
/// position is the lyric area's top-left. A new window goes to it (less the margin and the top room). An open window
/// owns its position (the saved one may lag a drag by the save delay): its lyric area stays where it is, its
/// horizontal centre kept when its width changes (the 寬度 slider) and its top-left kept when the font changes the
/// margin or the top room (`old` is what the window was placed with, `gutter` what it is now); with no saved position
/// (重設位置) it goes to the default place.
pub fn apply_target(
    shown: Option<Rect>,
    new_width: i32,
    old: Gutters,
    gutter: Gutters,
    saved: Option<FloatingLyricsPosition>,
) -> Option<FloatingLyricsPosition> {
    let (Some(rect), Some(_)) = (shown, saved) else {
        return saved.map(|area| gutter.window_from_area(area));
    };
    let area_y = rect.top + old.top.max(0);
    Some(FloatingLyricsPosition {
        x: resize_around_centre(rect.left, rect.width(), old, new_width, gutter),
        y: area_y - gutter.top.max(0),
    })
}

/// Where the window goes: the saved position moved fully inside the work area it overlaps most, or, with no saved
/// position or one on no monitor any more, at the centre of the first (primary) work area (the whole window kept
/// inside; the renderer uses `place_on_monitors_with` and its text insets).
#[cfg(test)]
pub fn place_on_monitors(
    position: Option<FloatingLyricsPosition>,
    size: (i32, i32),
    gutter: Gutters,
    work_areas: &[Rect],
) -> FloatingLyricsPosition {
    place_on_monitors_with(position, size, gutter, Insets::NONE, work_areas)
}

/// As `place_on_monitors`, keeping only the lyric text area inside the work area: the `insets` (transparent margins
/// and shadow room) may go past its edges.
pub fn place_on_monitors_with(
    position: Option<FloatingLyricsPosition>,
    size: (i32, i32),
    gutter: Gutters,
    insets: Insets,
    work_areas: &[Rect],
) -> FloatingLyricsPosition {
    let (width, height) = size;
    let clamp_into = |area: Rect, x: i32, y: i32| {
        let (low_x, low_y) = (area.left - insets.left, area.top - insets.top);
        FloatingLyricsPosition {
            x: x.clamp(low_x, (area.right - width + insets.right).max(low_x)),
            y: y.clamp(low_y, (area.bottom - height + insets.bottom).max(low_y)),
        }
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
        y: bounds.top + (bounds.height() - gutter.area_height(size.1)) / 2 - gutter.top.max(0),
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
        window.top + gutter.top.max(0) + gutter.area_height(window.height()) / 2,
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
    let gutter = current().map_or(Gutters::NONE, |settings| window_gutter(window, &settings));
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
    let (window, created) = match window() {
        Some(window) => (window, false),
        None => {
            let hwnd = renderer
                .create(WINDOW_LABEL, player_lyrics_native::Kind::Floating, None)
                .ok_or_else(|| "無法開啟浮動歌詞視窗".to_owned())?;
            HWND_VALUE.store(hwnd, Ordering::SeqCst);
            (Native(HWND(hwnd as *mut _)), true)
        }
    };
    renderer.set_look(
        WINDOW_LABEL,
        Some(crate::native_lyrics::floating_look(settings)),
    );
    crate::native_lyrics::apply_floating_chrome(app, settings);
    let (width, height) = window_size(window, settings);
    let shown = if created { None } else { window.rect() };
    let gutter = window_gutter(window, settings);
    let wanted = apply_target(shown, width, placed_with(), gutter, settings.position);
    // The move events from here on are saved with this conversion.
    if let Ok(mut guard) = PLACED_WITH.lock() {
        *guard = gutter;
    }
    window.set_size(width, height);
    place(window, wanted);
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
    let gutter = window_gutter(window, settings);
    let area = area_height_px(settings, scale, HAS_TRANSLATION.load(Ordering::SeqCst));
    (
        (width * scale).round() as i32,
        gutter.top + area + gutter.bottom,
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

/// A mouse drag ended: apply the placement rule (only the lyric text area must stay in the work area) to where it
/// was dropped; the move event saves the result.
pub fn settle_after_drag() {
    let Some(window) = window() else { return };
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
    // The saved position is the lyric area's top-left, not the window's.
    let position = placed_with().area_from_window(FloatingLyricsPosition {
        x: position.x,
        y: position.y,
    });
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
    let gutter = current().map_or(Gutters::NONE, |settings| window_gutter(window, &settings));
    // No saved position (first open, or 重設位置): the exact centre of the primary monitor.
    let wanted = wanted.or_else(|| {
        let bounds = monitors().first()?.0;
        (bounds.width() > 0 && bounds.height() > 0).then(|| centre_on(bounds, size, gutter))
    });
    let insets = text_insets(gutter);
    let target = place_on_monitors_with(wanted, size, gutter, insets, &work_areas());
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
                    window_gutter(window, &settings),
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
        // The window is the room above (main row only) + the area + the room below (follows the area).
        let height = |settings: &FloatingLyricsSettings, translated: bool| {
            let g = gutter_px(settings, 1.0, translated);
            g.top + area_height_px(settings, 1.0, translated) + g.bottom
        };
        assert_eq!(height(&settings, true), 6 + 62 + 8);
        assert_eq!(height(&settings, false), 6 + 42 + 6);
        settings.show_translation = false;
        assert_eq!(lyric_height_css(&settings, true), 42.0);
        settings.font_size_px = 8;
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
            gutter_px(&settings, 1.5, false),
            Gutters {
                lead: 12,
                trail: 78,
                top: 9,
                bottom: 9
            }
        );
    }

    #[test]
    fn unlock_button_scales_with_the_font_and_dpi() {
        let window = rect(1000, 500, 2600, 620);
        let g = Gutters::NONE;
        assert_eq!(
            unlock_button_rect(window, 1.0, 32.0, 0.0, g),
            rect(2568, 544, 2600, 576)
        );
        assert_eq!(
            unlock_button_rect(window, 1.0, 32.0, 8.0, g),
            rect(2560, 544, 2592, 576),
            "in from the margin"
        );
        let big = unlock_button_rect(window, 2.5, 32.0, 8.0, g);
        assert_eq!((big.width(), big.height()), (80, 80));
        assert_eq!(big.right, 2600 - 20);
        assert_eq!(big.top, 500 + (120 - 80) / 2);
        assert_eq!(
            unlock_button_rect(window, f64::NAN, 32.0, 8.0, g),
            unlock_button_rect(window, 1.0, 32.0, 8.0, g)
        );
        // With rooms the lock is centred on the lyric area, not on the window.
        let rooms = Gutters {
            top: 10,
            bottom: 30,
            ..Gutters::NONE
        };
        assert_eq!(
            unlock_button_rect(window, 1.0, 32.0, 0.0, rooms).top,
            500 + 10 + (80 - 32) / 2
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
        let lock = Gutters {
            trail: 40,
            ..Gutters::NONE
        };
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
            ..Gutters::NONE
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
    fn the_text_edge_may_touch_the_work_area_while_the_margins_go_past() {
        let area = rect(0, 0, 1920, 1040);
        let size = (1000, 150);
        let insets = Insets {
            left: 7,
            top: 20,
            right: 7,
            bottom: 20,
        };
        // Dragged past the left/top: the window hangs out by exactly its margins, the text edge sits on 0.
        let at = place_on_monitors_with(
            Some(FloatingLyricsPosition { x: -300, y: -60 }),
            size,
            Gutters::NONE,
            insets,
            &[area],
        );
        assert_eq!(at, FloatingLyricsPosition { x: -7, y: -20 });
        // Past the right/bottom: the lock column's right edge (window right less its margin) on 1920.
        let at = place_on_monitors_with(
            Some(FloatingLyricsPosition { x: 1500, y: 1000 }),
            size,
            Gutters::NONE,
            insets,
            &[area],
        );
        assert_eq!(at, FloatingLyricsPosition { x: 927, y: 910 });
        assert_eq!(at.x + size.0 - insets.right, 1920);
        assert_eq!(at.y + size.1 - insets.bottom, 1040);
        // Without insets the old rule: the whole window inside.
        assert_eq!(
            place_on_monitors(
                Some(FloatingLyricsPosition { x: -300, y: -60 }),
                size,
                Gutters::NONE,
                &[area]
            ),
            FloatingLyricsPosition { x: 0, y: 0 }
        );
    }

    #[test]
    fn apply_uses_the_saved_position_for_a_new_window_and_keeps_an_open_one() {
        let gutter = Gutters {
            lead: 7,
            trail: 77,
            top: 20,
            bottom: 20,
        };
        // The saved position is the lyric area's top-left; the window is that less the margin and the top room.
        let saved = Some(FloatingLyricsPosition { x: 1234, y: 567 });
        let window = Some(FloatingLyricsPosition { x: 1227, y: 547 });
        // A new window (still 1 x 1 at 0,0 from CreateWindowEx) goes to the saved position, not a resize of 0,0.
        assert_eq!(apply_target(None, 1000, gutter, gutter, saved), window);
        assert_eq!(apply_target(None, 1000, gutter, gutter, None), None);
        // An open window stays where it is even when the stored position lags a drag.
        assert_eq!(
            apply_target(Some(rect(300, 200, 1300, 350)), 1000, gutter, gutter, saved),
            Some(FloatingLyricsPosition { x: 300, y: 200 })
        );
        // ... and keeps its lyric area centre on a width change.
        assert_eq!(
            apply_target(Some(rect(500, 200, 1500, 350)), 1400, gutter, gutter, saved),
            Some(FloatingLyricsPosition { x: 300, y: 200 })
        );
        // A bigger font (more margin and top room) moves the window, not the lyric area.
        let bigger = Gutters {
            lead: 10,
            trail: 90,
            top: 28,
            bottom: 28,
        };
        let at =
            apply_target(Some(rect(300, 200, 1300, 350)), 1000, gutter, bigger, saved).unwrap();
        assert_eq!(
            bigger.area_from_window(at),
            FloatingLyricsPosition {
                x: 300 + 7 + (1000 - 84) / 2 - (1000 - 100) / 2,
                y: 220
            }
        );
        // 重設位置 (no saved position): the default place.
        assert_eq!(
            apply_target(Some(rect(300, 200, 1300, 350)), 1000, gutter, gutter, None),
            None
        );
    }

    #[test]
    fn the_saved_position_is_the_lyric_areas_top_left() {
        let gutter = Gutters {
            lead: 7,
            trail: 77,
            top: 20,
            bottom: 20,
        };
        let area = FloatingLyricsPosition { x: 400, y: 300 };
        let window = gutter.window_from_area(area);
        assert_eq!(window, FloatingLyricsPosition { x: 393, y: 280 });
        // Save then restore is the identity, whatever the margin and room.
        assert_eq!(gutter.area_from_window(window), area);
        let other = Gutters {
            lead: 12,
            trail: 90,
            top: 31,
            bottom: 40,
        };
        assert_eq!(other.area_from_window(other.window_from_area(area)), area);
    }

    #[test]
    fn a_translation_row_never_moves_the_first_row() {
        let settings = FloatingLyricsSettings {
            font_size_px: 30,
            translation_scale_percent: 50,
            show_translation: true,
            ..Default::default()
        };
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let plain = gutter_px(&settings, scale, false);
            let translated = gutter_px(&settings, scale, true);
            // The room above is the same: with the window's top fixed, the area's top-left does not move.
            assert_eq!(plain.top, translated.top);
            assert_eq!(plain.lead, translated.lead);
            let (plain_h, translated_h) = (
                plain.top + area_height_px(&settings, scale, false) + plain.bottom,
                translated.top + area_height_px(&settings, scale, true) + translated.bottom,
            );
            assert!(
                translated_h > plain_h,
                "the window grows below the main row"
            );
            let top_left = FloatingLyricsPosition { x: 600, y: 400 };
            assert_eq!(
                plain.window_from_area(top_left),
                translated.window_from_area(top_left)
            );
            // The renderer recovers the same area from the window height.
            for (g, translated) in [(plain, false), (translated, true)] {
                let h = g.top + area_height_px(&settings, scale, translated) + g.bottom;
                assert_eq!(
                    player_lyrics_native::floating_area_height_px(f64::from(h), f64::from(g.top))
                        as i32,
                    area_height_px(&settings, scale, translated)
                );
            }
        }
    }

    #[test]
    fn the_lyric_area_is_what_stays_on_screen_and_what_is_centred() {
        let g = Gutters {
            lead: 7,
            trail: 77,
            top: 20,
            bottom: 24,
        };
        let work = rect(0, 0, 1920, 1040);
        let size = (1000, 20 + 110 + 24);
        // Dragged far up-left: the area's top-left lands on the work area's.
        let at = place_on_monitors_with(
            Some(FloatingLyricsPosition { x: -100, y: -100 }),
            size,
            g,
            text_insets(g),
            &[work],
        );
        assert_eq!(
            g.area_from_window(at),
            FloatingLyricsPosition { x: 0, y: 0 }
        );
        // Far down-right: the area's bottom is on the work area's bottom, the lock column's right edge on its right.
        let at = place_on_monitors_with(
            Some(FloatingLyricsPosition { x: 1500, y: 1000 }),
            size,
            g,
            text_insets(g),
            &[work],
        );
        assert_eq!(at.y + size.1 - g.bottom, 1040);
        assert_eq!(at.x + size.0 - g.lead, 1920);
        // Centred: the area (not the window) is at the monitor's centre; the other axis is kept.
        let centred = centre_on(work, size, g);
        assert_eq!(
            g.area_from_window(centred).y,
            (1040 - 110) / 2,
            "area centred vertically"
        );
        assert_eq!(g.area_from_window(centred).x, (1920 - (1000 - 84)) / 2);
        let window = rect(100, 300, 1100, 300 + size.1);
        assert_eq!(centre_axis(window, &[work], false, g).x, 100);
        assert_eq!(centre_axis(window, &[work], true, g).y, 300);
        assert_eq!(centre_axis(window, &[work], false, g).y, centred.y);
        // Reset (no saved position) puts the area at the centre of the primary monitor.
        assert_eq!(
            place_on_monitors_with(None, size, g, text_insets(g), &[work]),
            centred
        );
        // Nudging one pixel moves the area one pixel.
        let nudged = place_on_monitors_with(
            Some(FloatingLyricsPosition {
                x: centred.x,
                y: centred.y + 1,
            }),
            size,
            g,
            text_insets(g),
            &[work],
        );
        assert_eq!(
            g.area_from_window(nudged).y,
            g.area_from_window(centred).y + 1
        );
    }

    #[test]
    fn a_width_change_keeps_the_lyric_area_centre() {
        let gutter = Gutters {
            lead: 7,
            trail: 77,
            ..Gutters::NONE
        };
        let centre = |left: i32, width: i32| left + 7 + (width - 84) / 2;
        let left = resize_around_centre(500, 1000, gutter, 1400, gutter);
        assert_eq!(left, 300);
        assert_eq!(centre(left, 1400), centre(500, 1000));
        let left = resize_around_centre(500, 1000, gutter, 600, gutter);
        assert_eq!(left, 700);
        assert_eq!(centre(left, 600), centre(500, 1000));
        // Odd differences round to within half a pixel.
        let left = resize_around_centre(500, 1000, gutter, 1001, gutter);
        assert!((centre(left, 1001) - centre(500, 1000)).abs() <= 1);
        // Widened past the edge: the clamp shifts it just enough, the width is kept.
        let area = rect(0, 0, 1920, 1040);
        let wide = resize_around_centre(100, 1000, gutter, 1600, gutter);
        let at = place_on_monitors_with(
            Some(FloatingLyricsPosition { x: wide, y: 100 }),
            (1600, 150),
            gutter,
            Insets {
                left: 7,
                top: 0,
                right: 7,
                bottom: 0,
            },
            &[area],
        );
        assert_eq!(at.x, -7);
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
                    trail: 100,
                    ..Gutters::NONE
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
                    trail: 80,
                    ..Gutters::NONE
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
