//! Pure rules of the Windows taskbar / floating lyrics (ADR 0018, 0031, 0032, 0034).
//!
//! Rust port of `src/lib/lyrics-strip.js`, `findActiveLyricIndex` (`lyrics-window.js`), `lyricsDisplayOffsetMs`
//! (`lyrics-timing-offset.js`) and the long-line scroll timing of `TaskbarLyricsView.svelte`
//! (`getLyricScrollProgress`). The JS, this crate and the Android `FloatingLyricsRules.kt` are all checked against
//! `tests/fixtures/lyrics-rules-vectors.json`, so the three cannot drift apart.

use unicode_segmentation::UnicodeSegmentation;

/// The separator between lines of the continuous strip (one em wide).
pub const STRIP_SEPARATOR: char = '\u{3000}';
/// The terminal line advances at an estimated 500 ms per grapheme.
pub const MS_PER_GRAPHEME: f64 = 500.0;
/// A line fades between "current" and "not current" over this long, ending when the next line starts.
pub const FADE_MS: f64 = 200.0;
/// The "none" mode: how long the last line is shown when the track length is unknown.
pub const LAST_LINE_FALLBACK_MS: f64 = 5000.0;
/// Default of the shared "inactive lyrics opacity" setting.
pub const DEFAULT_INACTIVE_OPACITY_PERCENT: u32 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    pub fn parse(value: &str) -> Self {
        match value {
            "center" => Align::Center,
            "right" => Align::Right,
            _ => Align::Left,
        }
    }
}

/// The continuous strip anchors a line by its left edge on the lyric area's centre.
pub const STRIP_ALIGN: Align = Align::Left;
pub const STRIP_TARGET: Align = Align::Center;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Animation {
    None,
    Continuous,
}

impl Animation {
    pub fn parse(value: &str) -> Self {
        if value == "continuous" {
            Animation::Continuous
        } else {
            Animation::None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEffect {
    None,
    Shadow,
    Stroke,
}

/// The effect the native Windows renderer draws: always the one selected. The WebView version turned shadow into an
/// outline in continuous mode because of its per-frame blur cost; natively the shadow is rasterised once per line into
/// the cached surface and the glide is a DWM animation, so continuous mode keeps the shadow (user decision 2026-10-23).
/// Android's `FloatingLyricsRules.effectiveTextEffect` still swaps shadow for an outline.
pub fn native_text_effect(_animation: Animation, effect: TextEffect) -> TextEffect {
    effect
}
pub fn grapheme_count(text: &str) -> usize {
    text.graphemes(true).count()
}

/// The lyrics time at which the terminal line's glide ends: 500 ms per grapheme, capped at the track end.
pub fn final_arrival_ms(line_start_ms: f64, text: &str, track_end_ms: Option<f64>) -> f64 {
    let estimated = line_start_ms + grapheme_count(text).max(1) as f64 * MS_PER_GRAPHEME;
    match track_end_ms {
        Some(end) if end.is_finite() => estimated.min(end),
        _ => estimated,
    }
}

/// Clamp of the per-track delay (seconds, -10..10 in tenths).
pub fn clamp_timing_delay_sec(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    let tenths = js_round(value * 10.0).clamp(-100.0, 100.0);
    tenths / 10.0 + 0.0
}

/// `lyricsDisplayOffsetMs`: the lyrics' own offset plus the per-track delay (positive delay = lyrics later).
pub fn display_offset_ms(lyrics_offset_ms: Option<f64>, delay_sec: f64) -> f64 {
    let own = lyrics_offset_ms.filter(|v| v.is_finite()).unwrap_or(0.0);
    let delay = js_round(-clamp_timing_delay_sec(delay_sec) * 1000.0);
    own + delay + 0.0
}

/// `Math.round` (halves toward +infinity).
fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// The line shown at `position_ms` (index into `starts`, sorted), or -1 before the first line.
pub fn active_index(starts: &[f64], position_ms: f64, offset_ms: f64) -> isize {
    if starts.is_empty() || !position_ms.is_finite() {
        return -1;
    }
    let target = position_ms
        + if offset_ms.is_finite() {
            offset_ms
        } else {
            0.0
        };
    starts.partition_point(|&s| s <= target) as isize - 1
}

/// The "none" mode long-line scroll: 0..1, reached at 90 % of the line's time.
pub fn scroll_progress(elapsed_ms: f64, duration_ms: f64) -> f64 {
    if duration_ms.is_nan() || duration_ms <= 0.0 || !elapsed_ms.is_finite() {
        return 0.0;
    }
    (elapsed_ms / (duration_ms * 0.9)).clamp(0.0, 1.0)
}

/// How long line `index` is shown in the "none" mode (at least 1 s): to the next line, else to the track end, else
/// the fallback.
pub fn line_shown_ms(
    starts: &[f64],
    index: usize,
    duration_ms: Option<f64>,
    offset_ms: f64,
) -> f64 {
    let start = starts[index];
    let next = match starts.get(index + 1) {
        Some(&next) => next,
        None => match duration_ms {
            Some(duration) => duration - offset_ms,
            None => start + LAST_LINE_FALLBACK_MS,
        },
    };
    (next - start).max(1000.0)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StripLayout {
    pub starts: Vec<f64>,
    pub widths: Vec<f64>,
}

/// Cells of the strip: as wide as the wider of main text and translation, one separator apart.
pub fn layout_strip(main_widths: &[f64], translation_widths: &[f64], gap_px: f64) -> StripLayout {
    let mut layout = StripLayout::default();
    let mut x = 0.0;
    for (index, main) in main_widths.iter().enumerate() {
        let width = 0f64
            .max(*main)
            .max(translation_widths.get(index).copied().unwrap_or(0.0));
        layout.starts.push(x);
        layout.widths.push(width);
        x += width + gap_px.max(0.0);
    }
    layout
}

pub fn strip_anchor(layout: &StripLayout, index: usize, align: Align) -> f64 {
    let start = layout.starts.get(index).copied().unwrap_or(0.0);
    let width = layout.widths.get(index).copied().unwrap_or(0.0);
    match align {
        Align::Right => start + width,
        Align::Center => start + width / 2.0,
        Align::Left => start,
    }
}

/// Terminal glide of the last line: `end_ms` and the main text's last glyph centre.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FinalLine {
    pub end_ms: Option<f64>,
    pub last_glyph_center: Option<f64>,
}

/// Horizontal shift of the strip (px) at lyrics time `position_ms` (`lyricsStripOffset`).
pub fn strip_offset(
    starts_ms: &[f64],
    layout: &StripLayout,
    align: Align,
    position_ms: f64,
    view_width_px: f64,
    target_align: Align,
    final_line: Option<FinalLine>,
) -> f64 {
    let count = starts_ms.len().min(layout.starts.len());
    if count == 0 {
        return 0.0;
    }
    let target = match target_align {
        Align::Right => view_width_px,
        Align::Center => view_width_px / 2.0,
        Align::Left => 0.0,
    };
    let index = starts_ms[..count].partition_point(|&s| s <= position_ms) as isize - 1;
    let anchor = if index < 0 {
        strip_anchor(layout, 0, align)
    } else if index as usize >= count - 1 {
        let last = count - 1;
        let start = starts_ms[last];
        let fin = final_line.unwrap_or_default();
        let glyph = fin
            .last_glyph_center
            .unwrap_or(layout.widths.get(last).copied().unwrap_or(0.0))
            .max(0.0);
        match fin.end_ms {
            Some(end) if end.is_finite() && end > start && position_ms > start => {
                let progress = ((position_ms - start) / (end - start)).clamp(0.0, 1.0);
                let from = layout.starts[last];
                from + glyph * progress
            }
            _ => strip_anchor(layout, last, align),
        }
    } else {
        let i = index as usize;
        let span = starts_ms[i + 1] - starts_ms[i];
        let fraction = if span > 0.0 {
            ((position_ms - starts_ms[i]) / span).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let from = strip_anchor(layout, i, align);
        from + (strip_anchor(layout, i + 1, align) - from) * fraction
    };
    target - anchor
}

/// Cells to draw while the strip glides between two offsets, plus `margin_px` on both sides. `None` when empty.
pub fn strip_window(
    layout: &StripLayout,
    from_offset_px: f64,
    to_offset_px: f64,
    view_width_px: f64,
    margin_px: f64,
) -> Option<(usize, usize)> {
    let count = layout.starts.len();
    let left = -from_offset_px.max(to_offset_px) - margin_px.max(0.0);
    let right = -from_offset_px.min(to_offset_px) + view_width_px.max(0.0) + margin_px.max(0.0);
    let mut first = 0;
    while first < count && layout.starts[first] + layout.widths[first] < left {
        first += 1;
    }
    let mut last = first;
    while last < count && layout.starts[last] <= right {
        last += 1;
    }
    (last > first).then(|| (first, last - 1))
}

/// The next lyrics time at which the glide changes (`nextLyricsStripBreak`).
pub fn next_strip_break(
    starts_ms: &[f64],
    position_ms: f64,
    final_arrival_ms: Option<f64>,
) -> Option<f64> {
    let first = *starts_ms.first()?;
    if position_ms < first {
        return Some(first);
    }
    if let Some(&start) = starts_ms.iter().find(|&&s| s > position_ms) {
        return Some(start);
    }
    let last = *starts_ms.last()?;
    final_arrival_ms.filter(|&a| a.is_finite() && a > last && a > position_ms)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Focus {
    /// -1 before the first line.
    pub current: isize,
    /// -1 after the last line starts.
    pub incoming: isize,
    pub progress: f64,
    pub fade_start_ms: Option<f64>,
    pub fade_ms: f64,
}

/// `lyricsStripFocus`: which lines are mid-fade at a lyrics position.
pub fn strip_focus(starts_ms: &[f64], position_ms: f64, fade_ms: f64) -> Focus {
    let current = starts_ms.partition_point(|&s| s <= position_ms) as isize - 1;
    let incoming = current + 1;
    if incoming as usize >= starts_ms.len() {
        return Focus {
            current,
            incoming: -1,
            progress: 0.0,
            fade_start_ms: None,
            fade_ms: 0.0,
        };
    }
    let next = starts_ms[incoming as usize];
    let length = if current >= 0 {
        next - starts_ms[current as usize]
    } else {
        next.max(1.0)
    };
    let duration = fade_ms.max(0.0).min(length).max(1.0);
    let fade_start = next - duration;
    let progress = ((position_ms - fade_start) / duration).clamp(0.0, 1.0);
    Focus {
        current,
        incoming,
        progress,
        fade_start_ms: Some(fade_start),
        fade_ms: duration,
    }
}

/// `lyricsLineWeight`: 0 (not current) .. 1 (current).
pub fn line_weight(focus: &Focus, index: usize) -> f64 {
    let index = index as isize;
    if focus.current < 0 {
        return if focus.incoming < 0 || index == focus.incoming {
            1.0
        } else {
            1.0 - focus.progress
        };
    }
    if index == focus.current {
        1.0 - focus.progress
    } else if index == focus.incoming {
        focus.progress
    } else {
        0.0
    }
}

pub fn normalize_inactive_opacity_percent(value: f64) -> u32 {
    if !value.is_finite() {
        return DEFAULT_INACTIVE_OPACITY_PERCENT;
    }
    js_round(value).clamp(10.0, 100.0) as u32
}

/// `lyricsLineOpacityFactor`: the line's opacity relative to the window opacity (continuous mode only).
pub fn line_opacity_factor(
    weight: f64,
    inactive_opacity_percent: u32,
    animation: Animation,
) -> f64 {
    if animation != Animation::Continuous {
        return 1.0;
    }
    let inactive = inactive_opacity_percent.clamp(10, 100) as f64 / 100.0;
    inactive + (1.0 - inactive) * weight.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_continuous_mode_keeps_the_shadow() {
        assert_eq!(
            native_text_effect(Animation::Continuous, TextEffect::Shadow),
            TextEffect::Shadow
        );
        assert_eq!(
            native_text_effect(Animation::None, TextEffect::Stroke),
            TextEffect::Stroke
        );
    }
    #[test]
    fn graphemes_count_emoji_clusters_once() {
        assert_eq!(grapheme_count("あ👨‍👩‍👧"), 2);
        assert_eq!(final_arrival_ms(1000.0, "", None), 1500.0);
    }

    #[test]
    fn delay_rounds_like_js() {
        assert_eq!(display_offset_ms(Some(100.0), 0.15), -100.0);
        assert_eq!(display_offset_ms(None, -0.0), 0.0);
        assert_eq!(display_offset_ms(None, 99.0), -10000.0);
    }
}
