//! Where the floating lyrics panel goes (ADR 0032): the saved position as a ratio of the screen and the rule that keeps
//! the lyric area on screen. Windows (`src-tauri/src/floating_lyrics.rs`) and Android (`FloatingLyricsRules.kt`) both
//! use these, checked against the shared vectors (`placementRatio`, `placementClamp`, `placementRestore`).
//!
//! The saved value is the lyric area's top-left divided by the screen: `x = (areaX - screenLeft) / screenWidth`,
//! `y = (areaY - screenTop) / screenHeight` (not divided by the screen less the area). "Screen" is the whole virtual
//! desktop on Windows (every monitor's bounds) and the display in the current orientation on Android. A ratio does not
//! promise the area is still visible (another screen size, another monitor layout), so a restored position is always
//! clamped afterwards with [`clamp_kept_box`].

/// A pixel rectangle: left and top inclusive, right and bottom exclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PxBounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PxBounds {
    pub fn width(self) -> i32 {
        self.right.saturating_sub(self.left)
    }
    pub fn height(self) -> i32 {
        self.bottom.saturating_sub(self.top)
    }
}

/// The ratio saved for a coordinate `px` on a screen axis starting at `origin` and `extent` px long:
/// `(px - origin) / extent`. None when the axis is empty (nothing sensible to save).
pub fn position_ratio(px: i32, origin: i32, extent: i32) -> Option<f64> {
    (extent > 0).then(|| (f64::from(px) - f64::from(origin)) / f64::from(extent))
}

/// The pixel a saved `ratio` stands for on an axis starting at `origin` and `extent` px long: `origin +
/// floor(ratio * extent + 0.5)` (half rounds up on both platforms). A non-finite ratio or an empty axis gives `origin`.
pub fn position_px(ratio: f64, origin: i32, extent: i32) -> i32 {
    if !ratio.is_finite() || extent <= 0 {
        return origin;
    }
    let offset = (ratio * f64::from(extent) + 0.5).floor();
    (f64::from(origin) + offset).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

/// Keeps the part of the panel that must stay visible inside `bounds`: a box at `origin` of `size` (the lyric area plus
/// the lock column right of it; the transparent margins and the rooms above and below may hang past the edges). Each
/// axis is clamped to `[start, end - size]`; a box larger than the bounds sits at the start.
pub fn clamp_kept_box(origin: (i32, i32), size: (i32, i32), bounds: PxBounds) -> (i32, i32) {
    let axis = |value: i32, low: i32, high: i32, size: i32| {
        let max = high.saturating_sub(size.max(0)).max(low);
        value.clamp(low, max)
    };
    (
        axis(origin.0, bounds.left, bounds.right, size.0),
        axis(origin.1, bounds.top, bounds.bottom, size.1),
    )
}

/// A saved position put back on a screen whose saved-ratio frame is `screen`, kept inside `bounds` (Android: both are
/// the display; Windows: `screen` is the virtual desktop and `bounds` the work area the panel lands on). Returns the
/// kept box's top-left (the lyric area's top-left).
pub fn restore_area_origin(
    ratio: (f64, f64),
    screen: PxBounds,
    size: (i32, i32),
    bounds: PxBounds,
) -> (i32, i32) {
    let x = position_px(ratio.0, screen.left, screen.width());
    let y = position_px(ratio.1, screen.top, screen.height());
    clamp_kept_box((x, y), size, bounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(left: i32, top: i32, right: i32, bottom: i32) -> PxBounds {
        PxBounds {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn the_ratio_is_the_area_origin_over_the_screen() {
        // The example the user accepted: a 1080 x 2400 portrait screen, ratio (0.5, 0.1).
        let screen = b(0, 0, 1080, 2400);
        assert_eq!(position_px(0.5, 0, 1080), 540);
        assert_eq!(position_px(0.1, 0, 2400), 240);
        assert_eq!(position_ratio(540, 0, 1080), Some(0.5));
        assert_eq!(position_ratio(240, 0, 2400), Some(0.1));
        assert_eq!(
            restore_area_origin((0.5, 0.1), screen, (300, 60), screen),
            (540, 240)
        );
        // A desktop that starts left of the primary monitor.
        assert_eq!(position_ratio(-1920, -1920, 4480), Some(0.0));
        assert_eq!(position_px(0.0, -1920, 4480), -1920);
        assert_eq!(position_px(1.0, -1920, 4480), 2560);
        assert_eq!(position_ratio(5, 0, 0), None);
        assert_eq!(position_px(f64::NAN, 7, 100), 7);
        assert_eq!(position_px(0.5, 7, 0), 7);
    }

    #[test]
    fn saving_and_restoring_gives_back_every_pixel() {
        for extent in [1, 7, 1080, 2400, 2560, 4480, 7680] {
            for px in 0..extent.min(3000) {
                let ratio = position_ratio(px, 0, extent).unwrap();
                assert_eq!(position_px(ratio, 0, extent), px, "{px} of {extent}");
            }
        }
    }

    #[test]
    fn only_the_kept_box_stays_inside() {
        let screen = b(0, 0, 1080, 2400);
        assert_eq!(clamp_kept_box((-10, -5), (300, 60), screen), (0, 0));
        assert_eq!(clamp_kept_box((900, 2390), (300, 60), screen), (780, 2340));
        assert_eq!(clamp_kept_box((100, 200), (300, 60), screen), (100, 200));
        // Wider than the screen: at the start.
        assert_eq!(clamp_kept_box((500, 10), (2000, 60), screen), (0, 10));
        // A ratio from a larger screen is pulled back on.
        assert_eq!(
            restore_area_origin((0.9, 0.95), screen, (300, 60), screen),
            (780, 2280)
        );
    }
}
