//! Native renderer of the Windows taskbar and floating lyrics windows (ADR 0034).
//!
//! All lyrics windows live on one dedicated UI thread (with its dispatcher queue) with one D3D11 device, one
//! Direct2D device, one DirectWrite factory and one Windows.UI.Composition compositor (one desktop window target per
//! window). Each lyric line is rasterised once (with its shadow or outline) into a cached `CompositionDrawingSurface`.
//! While a window moves, the renderer thread samples the plans (`sampling`) from its playback clock on every
//! composition frame (`DwmFlush`) and commits only values that changed by a device pixel (or 1/255 opacity); nothing
//! is rasterised per frame, and an idle renderer sleeps. The edge fade is one
//! `CompositionMaskBrush` per window over a `CompositionVisualSurface` of the content. A window is
//! re-planned only at a line change, a seek, pause/resume, a playback correction, new lyrics, a setting, or a
//! size/DPI change. The rules themselves are `player-lyrics-rules`. Needs Windows 10 1809 or later (otherwise no
//! lyrics windows are created).

pub mod plan;
pub mod sampling;

pub use player_lyrics_rules::{Align, Animation, TextEffect};

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::NativeLyrics;

#[derive(Clone, Debug, PartialEq)]
pub struct LyricLine {
    pub start_ms: f64,
    pub text: String,
    pub translation: String,
}

/// The lyrics of one track as the main window prepared them: synced lines only, Traditional Chinese already chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct LyricsData {
    pub track_id: String,
    pub lines: Vec<LyricLine>,
    /// The lyrics' own offset (LRC `[offset:]`).
    pub offset_ms: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Playing,
    Paused,
    Buffering,
    Stopped,
}

impl PlayState {
    pub fn parse(value: &str) -> Self {
        match value {
            "playing" => PlayState::Playing,
            "paused" => PlayState::Paused,
            "buffering" => PlayState::Buffering,
            _ => PlayState::Stopped,
        }
    }
    /// The lyrics show while a track is loaded (`getDisplayActiveLyricIndex`).
    pub fn shows_lyrics(self) -> bool {
        self != PlayState::Stopped
    }
}

/// A reading of the audio engine's own clock: the backend's played position `position_ms` as read at `at`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioClock {
    pub position_ms: f64,
    pub at: std::time::Instant,
    pub playing: bool,
}

/// Reads the audio engine's clock (cheap; called on every frame while lyrics move).
pub type ClockSource = std::sync::Arc<dyn Fn() -> Option<AudioClock> + Send + Sync>;

/// One playback sample forwarded by the main window (ADR 0028).
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackSample {
    pub track_id: Option<String>,
    pub state: PlayState,
    pub position_ms: f64,
    pub duration_ms: Option<f64>,
}

/// The look of one window, from `TaskbarLyricsSettings` / `FloatingLyricsSettings`.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub align: Align,
    /// CSS px of the main line (fitted to the height).
    pub font_size_px: f64,
    pub translation_scale_percent: f64,
    /// Text colour (custom, or black/white from the taskbar theme, chosen by the caller).
    pub color: [u8; 3],
    pub effect: TextEffect,
    pub show_translation: bool,
    pub opacity_percent: f64,
    pub animation: Animation,
}

/// The shadow or outline colour: the opposite of the text (`getTaskbarLyricsEffectRgb`).
pub fn effect_rgb(color: [u8; 3]) -> [u8; 3] {
    let brightness = 0.299 * color[0] as f64 + 0.587 * color[1] as f64 + 0.114 * color[2] as f64;
    if brightness >= 128.0 {
        [0, 0, 0]
    } else {
        [255, 255, 255]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Click-through, over a taskbar.
    Taskbar,
    /// The floating window: draggable (move mode) or click-through with a padlock button (fixed mode).
    Floating,
}

/// Floating-window chrome: the lock state and the padlock tooltips (already translated by the main window).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Chrome {
    pub locked: bool,
    /// Tooltip while locked (「解除鎖定（可移動）」).
    pub unlock_tip: String,
    /// Tooltip while movable (「鎖定位置（滑鼠穿透）」).
    pub lock_tip: String,
}

/// What the renderer reports back (called on the renderer thread; keep it short).
pub trait Events: Send + Sync + 'static {
    /// The padlock was clicked.
    fn lock_clicked(&self, id: &str);
    /// The window moved (dragged); physical screen position of its top-left corner.
    fn moved(&self, id: &str, x: i32, y: i32);
    /// Whether the window now draws a translation line (the floating window fits its height to it).
    fn has_translation(&self, id: &str, has: bool);
    /// Something asked the window to close (WM_CLOSE, e.g. logoff): close the app the normal way.
    fn close_requested(&self, id: &str);
}

/// Floating layout constants (`FLOATING_LYRICS_LAYOUT`, floating_lyrics.rs).
pub const FLOATING_LINE_HEIGHT: f64 = 1.375;
pub const FLOATING_PANEL_LINE_HEIGHT: f64 = 1.875;
pub const FLOATING_SIDE_MARGIN_EM: f64 = 0.25;
pub const TASKBAR_LINE_HEIGHT: f64 = 1.4;
/// Taskbar lyrics: `padding: 0 4px`.
pub const TASKBAR_SIDE_PADDING_CSS: f64 = 4.0;

pub fn floating_lock_css(font_px: f64) -> f64 {
    (font_px.max(1.0) * FLOATING_LINE_HEIGHT).ceil().max(1.0)
}
pub fn floating_margin_css(font_px: f64) -> f64 {
    (font_px.max(1.0) * FLOATING_SIDE_MARGIN_EM).ceil().max(1.0)
}

/// `fitTaskbarLyricsFontSize`: the chosen size, shrunk so the line(s) fit the height.
pub fn fit_font_size(
    font_px: f64,
    height_px: f64,
    translation_scale_percent: f64,
    line_height: f64,
) -> f64 {
    if height_px.is_nan() || height_px <= 0.0 {
        return font_px;
    }
    let lines = line_height * (1.0 + translation_scale_percent.max(0.0) / 100.0);
    font_px.min((height_px / lines).floor()).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitting_and_effect_colour_match_the_js() {
        assert_eq!(fit_font_size(20.0, 48.0, 100.0, 1.4), 17.0);
        assert_eq!(fit_font_size(20.0, 0.0, 100.0, 1.4), 20.0);
        assert_eq!(fit_font_size(50.0, 2.0, 0.0, 1.4), 1.0);
        assert_eq!(effect_rgb([0, 225, 255]), [0, 0, 0]);
        assert_eq!(effect_rgb([20, 20, 20]), [255, 255, 255]);
        assert_eq!(floating_lock_css(28.0), 39.0);
        assert_eq!(floating_margin_css(28.0), 7.0);
    }
}
