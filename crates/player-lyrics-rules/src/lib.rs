//! Pure rules of the Windows taskbar / floating lyrics (ADR 0018, 0031, 0032, 0034).
//!
//! The reference implementation: `examples/generate_vectors.rs` writes `tests/fixtures/lyrics-rules-vectors.json`
//! from these rules (`cargo run -p player-lyrics-rules --example generate_vectors`), and the Android
//! `FloatingLyricsRules.kt` / `WordLyricsRules.kt` and the JS rules the main window still runs (`findActiveLyricIndex`
//! in `lyrics-window.js`, `lyricsDisplayOffsetMs` in `lyrics-timing-offset.js`, `getLyricScrollProgress` and the Now
//! Playing karaoke glide in `lyrics-glide.js`) are checked against it, so they cannot drift apart.

use unicode_segmentation::UnicodeSegmentation;

pub mod east_asian_width;
pub mod glide;
pub mod words;
pub use glide::*;
pub use words::*;

/// The separator between lines of the continuous strip (one em wide).
pub const STRIP_SEPARATOR: char = '\u{3000}';
/// The terminal line's estimated duration per grapheme cluster whose base (first) code point is East Asian Wide or
/// Fullwidth (CJK ideographs, kana, Hangul, full-width forms, wide emoji: `east_asian_width::is_east_asian_wide`) ...
pub const WIDE_GRAPHEME_MS: f64 = 500.0;
/// ... and per any other grapheme cluster (Latin, half-width katakana, ambiguous / neutral symbols, spaces).
pub const NARROW_GRAPHEME_MS: f64 = 250.0;
/// The continuous strip's duration cap of a line without word times (not the last line, ADR 0041 D10): per wide
/// grapheme cluster (same table and segmentation as the terminal estimate) ...
pub const LINE_CAP_WIDE_GRAPHEME_MS: f64 = 1000.0;
/// ... and per any other grapheme cluster (spaces included); an empty line counts as one.
pub const LINE_CAP_NARROW_GRAPHEME_MS: f64 = 500.0;
/// A line fades between "current" and "not current" over this long, ending when the next line starts.
pub const FADE_MS: f64 = 200.0;
/// The "none" mode: how long the last line is shown when the track length is unknown.
pub const LAST_LINE_FALLBACK_MS: f64 = 5000.0;

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

/// The continuous strip anchors a line by its first glyph's centre (`StripLayout::anchors`; the cell's left edge
/// without them) on the lyric area's centre.
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

/// How long the terminal line is estimated to take: [`WIDE_GRAPHEME_MS`] per wide grapheme cluster plus
/// [`NARROW_GRAPHEME_MS`] per other one; an empty line counts as one narrow grapheme.
pub fn terminal_line_estimate_ms(text: &str) -> f64 {
    grapheme_duration_ms(text, WIDE_GRAPHEME_MS, NARROW_GRAPHEME_MS)
}

/// How long the continuous strip may take to glide through a line without word times that is not the last line:
/// [`LINE_CAP_WIDE_GRAPHEME_MS`] per wide grapheme cluster plus [`LINE_CAP_NARROW_GRAPHEME_MS`] per other one; an
/// empty line counts as one narrow grapheme (`KnotLine::cap_ms`).
pub fn line_cap_ms(text: &str) -> f64 {
    grapheme_duration_ms(text, LINE_CAP_WIDE_GRAPHEME_MS, LINE_CAP_NARROW_GRAPHEME_MS)
}

/// `wide` per grapheme cluster whose base code point is East Asian W/F, `narrow` per other one, at least `narrow`.
fn grapheme_duration_ms(text: &str, wide: f64, narrow: f64) -> f64 {
    let total: f64 = text
        .graphemes(true)
        .map(|g| match g.chars().next() {
            Some(c) if east_asian_width::is_east_asian_wide(c) => wide,
            _ => narrow,
        })
        .sum();
    total.max(narrow)
}

/// The lyrics time at which the terminal line's glide ends: the line start plus [`terminal_line_estimate_ms`],
/// capped at the track end.
pub fn final_arrival_ms(line_start_ms: f64, text: &str, track_end_ms: Option<f64>) -> f64 {
    let estimated = line_start_ms + terminal_line_estimate_ms(text);
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
    /// The first glyph's centre of each line (px from its cell's left edge): the `Align::Left` anchor (ADR 0041
    /// D10). Missing entries anchor on the cell's left edge.
    pub anchors: Vec<f64>,
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
        Align::Left => {
            start
                + layout
                    .anchors
                    .get(index)
                    .copied()
                    .unwrap_or(0.0)
                    .clamp(0.0, width.max(0.0))
        }
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
    strip_cubic(
        starts_ms,
        layout,
        align,
        position_ms,
        view_width_px,
        target_align,
        final_line,
    )[0]
}

/// `lyricsStripCubic`: the glide is a monotone C1 cubic Hermite through the knots (line start, that line's anchor)
/// plus the terminal knot (final arrival, last glyph centre). Tangents are 0 at the first and last knot (the strip
/// rests outside them) and, inside, the weighted harmonic mean of the neighbouring slopes (Fritsch-Butland / PCHIP;
/// 0 when a slope is 0 or a neighbour shares the time), which is monotone (Fritsch-Carlson) and bounded by 3x the
/// smaller slope, so a very short line does not speed up its neighbours. Each segment reads only its four nearest
/// knots. Returns `[c0, c1, c2, c3]` of the offset in `u = t - position_ms` (ms), valid up to the next knot.
pub fn strip_cubic(
    starts_ms: &[f64],
    layout: &StripLayout,
    align: Align,
    position_ms: f64,
    view_width_px: f64,
    target_align: Align,
    final_line: Option<FinalLine>,
) -> [f64; 4] {
    let count = starts_ms.len().min(layout.starts.len());
    if count == 0 {
        return [0.0; 4];
    }
    let target = match target_align {
        Align::Right => view_width_px,
        Align::Center => view_width_px / 2.0,
        Align::Left => 0.0,
    };
    let last = count - 1;
    let fin = final_line.unwrap_or_default();
    let end_ms = fin.end_ms.filter(|e| e.is_finite() && *e > starts_ms[last]);
    let knots = count + usize::from(end_ms.is_some());
    let time = |k: usize| {
        if k < count {
            starts_ms[k]
        } else {
            end_ms.unwrap_or(0.0)
        }
    };
    let value = |k: usize| {
        if k < count {
            return strip_anchor(layout, k, align);
        }
        let glyph = fin
            .last_glyph_center
            .unwrap_or(layout.widths.get(last).copied().unwrap_or(0.0))
            .max(0.0);
        strip_anchor(layout, last, align).max(layout.starts[last] + glyph)
    };
    // `anchor` (the knot curve's value) is subtracted from the target; speed and curvature change sign with it.
    let [anchor, velocity, c2, c3] = glide::hermite_cubic(knots, time, value, position_ms);
    [target - anchor, -velocity, -c2, -c3]
}

// Cells to draw while the strip glides between two offsets, plus `margin_px` on both sides. `None` when empty.
pub fn strip_window(
    layout: &StripLayout,
    from_offset_px: f64,
    to_offset_px: f64,
    view_width_px: f64,
    margin_px: f64,
) -> Option<(usize, usize)> {
    let (first, end) = strip_window_span(
        layout,
        from_offset_px,
        to_offset_px,
        view_width_px,
        margin_px,
    );
    (end > first).then(|| (first, end - 1))
}

/// `strip_window` as `(first, end)` with `end` exclusive (`end == first` when there is nothing to draw).
pub fn strip_window_span(
    layout: &StripLayout,
    from_offset_px: f64,
    to_offset_px: f64,
    view_width_px: f64,
    margin_px: f64,
) -> (usize, usize) {
    let count = layout.starts.len();
    let left = -from_offset_px.max(to_offset_px) - margin_px.max(0.0);
    let right = -from_offset_px.min(to_offset_px) + view_width_px.max(0.0) + margin_px.max(0.0);
    let mut first = 0;
    while first < count && layout.starts[first] + layout.widths[first] < left {
        first += 1;
    }
    let mut end = first;
    while end < count && layout.starts[end] <= right {
        end += 1;
    }
    (first, end)
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

/// `lyricsEdgeFadeWidth`: the square edge-fade zone on each side of a lyric area (as wide as the area is high, at most
/// half the area width).
pub fn edge_fade_width(area_width: f64, area_height: f64) -> f64 {
    if area_width.is_nan() || area_height.is_nan() || area_width <= 0.0 || area_height <= 0.0 {
        return 0.0;
    }
    area_height.min(area_width / 2.0)
}

/// `lyricsEdgeFadeOpacity`: the edge fade's opacity factor at `x` px from the lyric area's left edge (1 inside, falling
/// linearly to 0 at either outer edge).
pub fn edge_fade_opacity(x: f64, area_width: f64, area_height: f64) -> f64 {
    let zone = edge_fade_width(area_width, area_height);
    if zone <= 0.0 {
        return 0.0;
    }
    (x.min(area_width - x) / zone).clamp(0.0, 1.0)
}

/// The edge fade as vertical bands for a renderer that cannot draw a gradient mask: `(left, right, opacity)` relative to
/// the area's left edge, `band_px` wide (the last band of a zone may be narrower), each with the opacity at its centre;
/// the middle band (opacity 1) spans between the zones. Empty when the area is empty.
pub fn edge_fade_bands(area_width: f64, area_height: f64, band_px: f64) -> Vec<(f64, f64, f64)> {
    let zone = edge_fade_width(area_width, area_height);
    if zone <= 0.0 {
        return Vec::new();
    }
    let band = band_px.max(0.25);
    let mut left = Vec::new();
    let mut x = 0.0;
    while x < zone - 1e-9 {
        let r = (x + band).min(zone);
        left.push((
            x,
            r,
            edge_fade_opacity((x + r) / 2.0, area_width, area_height),
        ));
        x = r;
    }
    let mut out = left.clone();
    if area_width - 2.0 * zone > 1e-9 {
        out.push((zone, area_width - zone, 1.0));
    }
    out.extend(
        left.iter()
            .rev()
            .map(|&(l, r, o)| (area_width - r, area_width - l, o)),
    );
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn edge_fade_zones_are_square_and_clamped() {
        use super::*;
        assert_eq!(edge_fade_width(400.0, 40.0), 40.0);
        assert_eq!(edge_fade_width(60.0, 40.0), 30.0);
        assert_eq!(edge_fade_width(0.0, 40.0), 0.0);
        assert_eq!(edge_fade_opacity(0.0, 400.0, 40.0), 0.0);
        assert_eq!(edge_fade_opacity(20.0, 400.0, 40.0), 0.5);
        assert_eq!(edge_fade_opacity(200.0, 400.0, 40.0), 1.0);
        assert_eq!(edge_fade_opacity(390.0, 400.0, 40.0), 0.25);
        assert_eq!(edge_fade_opacity(-3.0, 400.0, 40.0), 0.0);
        let bands = edge_fade_bands(400.0, 4.0, 1.0);
        assert_eq!(bands.len(), 9);
        assert_eq!(bands[0], (0.0, 1.0, 0.125));
        assert_eq!(bands[4], (4.0, 396.0, 1.0));
        assert_eq!(bands[8], (399.0, 400.0, 0.125));
        // Narrow area: two zones meet in the middle, no middle band.
        let bands = edge_fade_bands(6.0, 40.0, 1.0);
        assert_eq!(bands.len(), 6);
        assert!(bands.iter().all(|b| b.2 < 1.0));
        assert!(edge_fade_bands(0.0, 4.0, 1.0).is_empty());
    }

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
        assert_eq!(final_arrival_ms(1000.0, "", None), 1250.0);
    }

    #[test]
    fn the_terminal_estimate_weighs_wide_graphemes_double() {
        assert_eq!(terminal_line_estimate_ms("啦啦啦啦啦啦啦啦"), 4000.0);
        assert_eq!(terminal_line_estimate_ms("Hello world"), 2750.0);
        assert_eq!(terminal_line_estimate_ms("愛してる love you"), 4250.0);
        // e + U+0301 is one narrow grapheme, か + U+3099 one wide; half-width ｱ is narrow, ＡＢＣ wide.
        assert_eq!(terminal_line_estimate_ms("e\u{301}"), 250.0);
        assert_eq!(terminal_line_estimate_ms("か\u{3099}"), 500.0);
        assert_eq!(terminal_line_estimate_ms("ｱｲｳ"), 750.0);
        assert_eq!(terminal_line_estimate_ms("ＡＢＣ"), 1500.0);
        assert_eq!(terminal_line_estimate_ms("🙂"), 500.0);
        assert_eq!(
            final_arrival_ms(0.0, "啦啦啦啦啦啦啦啦", Some(3000.0)),
            3000.0
        );
    }

    #[test]
    fn the_line_cap_is_a_second_per_wide_grapheme_and_half_a_second_per_other_one() {
        assert_eq!(line_cap_ms("重新学会做梦和期望"), 9000.0);
        assert_eq!(line_cap_ms("Hello world"), 5500.0);
        assert_eq!(line_cap_ms("愛してる love you"), 8500.0);
        assert_eq!(line_cap_ms(""), 500.0);
        assert_eq!(line_cap_ms("か\u{3099}"), 1000.0);
        assert_eq!(line_cap_ms("ｱｲｳ"), 1500.0);
    }

    #[test]
    fn delay_rounds_like_js() {
        assert_eq!(display_offset_ms(Some(100.0), 0.15), -100.0);
        assert_eq!(display_offset_ms(None, -0.0), 0.0);
        assert_eq!(display_offset_ms(None, 99.0), -10000.0);
    }

    #[test]
    fn the_glide_is_exact_at_the_knots_monotone_and_c1() {
        let layout = layout_strip(&[100.0, 30.0, 200.0, 60.0, 90.0], &[], 20.0);
        let starts = [1000.0, 1150.0, 4000.0, 12000.0, 12500.0];
        let fin = Some(FinalLine {
            end_ms: Some(14000.0),
            last_glyph_center: Some(40.0),
        });
        let at =
            |ms: f64| strip_cubic(&starts, &layout, Align::Left, ms, 300.0, Align::Center, fin);
        for (i, &s) in starts.iter().enumerate() {
            assert!((at(s)[0] - (150.0 - layout.starts[i])).abs() < 1e-9);
        }
        assert!((at(14000.0)[0] - (150.0 - layout.starts[4] - 40.0)).abs() < 1e-9);
        let mut previous = at(0.0)[0];
        let mut t = 0.0;
        while t <= 15000.0 {
            let v = at(t)[0];
            assert!(v <= previous + 1e-9, "backwards at {t}");
            previous = v;
            t += 10.0;
        }
        for knot in [1000.0, 1150.0, 4000.0, 12000.0, 12500.0, 14000.0] {
            let e = 1e-3;
            let b = at(knot - e);
            let before = b[1] + 2.0 * b[2] * e + 3.0 * b[3] * e * e;
            assert!((before - at(knot)[1]).abs() < 1e-4, "speed jump at {knot}");
        }
        assert_eq!(at(1000.0)[1], 0.0);
        assert_eq!(at(14000.0)[1], 0.0);
        // A neighbour of the 150 ms line stays within 3x its own slope.
        let slope = (layout.starts[3] - layout.starts[2]) / 8000.0;
        assert!(at(4000.0)[1].abs() <= 3.0 * slope + 1e-12);
    }
}
