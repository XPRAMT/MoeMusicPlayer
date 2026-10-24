//! Word-timed lyrics in the native windows (ADR 0041 option C and the 「無」 recolour).
//!
//! Continuous mode: the word glide (`glide::word_strip_knots`, `glide::knot_cubic`: line start and last-word end knots) keeps the line being sung near the lyric area's
//! centre; opacity comes only from a static mask fixed to the lyric area (`centre_mask_stops`: the main and secondary opacity). 「無」 mode: the karaoke
//! edge (`glide::karaoke_glide`) lights the line in the accent colour.

/// Width of the mask's fully opaque band at the centre, in em of the main row.
pub const MASK_BAND_EM: f64 = 1.0;
/// Width of the fade from transparent to the secondary opacity at each edge, in em of the main row.
pub const MASK_EDGE_EM: f64 = 1.0;

/// The static continuous-mode mask (experimental, ADR 0041 D10) as gradient stops `(offset 0..1 of the area width,
/// opacity)`, linear between them, fixed to the lyric area: 0 → `secondary` over the first `MASK_EDGE_EM` em, then
/// `secondary` → `main` up to the `MASK_BAND_EM` em plateau at `main` centred on the area, mirrored on the right.
/// `main` and `secondary` are absolute opacities (0..1, each clamped on its own; `secondary` may exceed `main`, a
/// NaN counts as 1). When the area is narrower than four em, the em used shrinks to a quarter of the area so every
/// segment keeps a positive length. Empty when the area or the em is empty.
pub fn centre_mask_stops(
    area_width: f64,
    em_px: f64,
    main: f64,
    secondary: f64,
) -> Vec<(f64, f64)> {
    if area_width.is_nan() || em_px.is_nan() || area_width <= 0.0 || em_px <= 0.0 {
        return Vec::new();
    }
    let unit = |v: f64| if v.is_nan() { 1.0 } else { v.clamp(0.0, 1.0) };
    let (m, a) = (unit(main), unit(secondary));
    let e = em_px.min(area_width / 4.0);
    let edge = MASK_EDGE_EM * e;
    let half = MASK_BAND_EM * e / 2.0;
    let c = area_width / 2.0;
    [
        (0.0, 0.0),
        (edge, a),
        (c - half, m),
        (c + half, m),
        (area_width - edge, a),
        (area_width, 0.0),
    ]
    .iter()
    .map(|&(x, o)| (x / area_width, o))
    .collect()
}

/// The mask's opacity at `x` px from the area's left edge (what the gradient of `centre_mask_stops` draws; where two
/// stops share an offset the later one wins).
pub fn centre_mask_opacity(x: f64, area_width: f64, em_px: f64, main: f64, secondary: f64) -> f64 {
    let stops = centre_mask_stops(area_width, em_px, main, secondary);
    if stops.is_empty() {
        return 0.0;
    }
    let p = (x / area_width).clamp(0.0, 1.0);
    let mut value = stops[0].1;
    for pair in stops.windows(2) {
        let ((a, va), (b, vb)) = (pair[0], pair[1]);
        if p >= a && p <= b {
            value = if b > a {
                va + (vb - va) * (p - a) / (b - a)
            } else {
                vb
            };
            if p < b {
                break;
            }
        }
    }
    value
}

/// The word being sung at lyrics time `t` (`start <= t < end`; a zero-length word is never sung), as in Now Playing.
pub fn sung_word(words: &[(f64, f64)], t: f64) -> Option<usize> {
    let i = words.partition_point(|w| w.0 <= t);
    (0..i)
        .rev()
        .find(|&j| t < words[j].1 && words[j].1 > words[j].0)
}

/// The next lyrics time after `t` at which the sung word changes (a word start or end), `None` after the last.
pub fn next_word_change(words: &[(f64, f64)], t: f64) -> Option<f64> {
    words
        .iter()
        .flat_map(|w| [w.0, w.1])
        .filter(|&x| x > t)
        .min_by(f64::total_cmp)
}

/// Each word's UTF-16 range `(start, len)` in the line text after its leading whitespace is trimmed (the renderers
/// draw the trimmed text); `None` when the words do not spell the line (then the line is drawn whole).
pub fn word_utf16_ranges(line_text: &str, words: &[&str]) -> Option<Vec<(usize, usize)>> {
    if words.is_empty() || words.concat() != line_text {
        return None;
    }
    let lead = line_text.len() - line_text.trim_start().len();
    let lead16 = line_text[..lead].encode_utf16().count() as isize;
    let mut at = 0isize;
    let mut out = Vec::with_capacity(words.len());
    for w in words {
        let len = w.encode_utf16().count() as isize;
        let start = (at - lead16).max(0);
        let end = (at + len - lead16).max(0);
        out.push((start as usize, (end - start) as usize));
        at += len;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mask_ramps_from_the_edge_to_secondary_and_on_to_main() {
        let (w, em) = (1000.0, 40.0);
        let at = |x: f64, m: f64, a: f64| centre_mask_opacity(x, w, em, m, a);
        // main 1, secondary 0.5: the former look.
        assert_eq!(at(0.0, 1.0, 0.5), 0.0);
        assert!((at(20.0, 1.0, 0.5) - 0.25).abs() < 1e-9);
        assert_eq!(at(40.0, 1.0, 0.5), 0.5);
        assert_eq!(at(480.0, 1.0, 0.5), 1.0);
        assert_eq!(at(500.0, 1.0, 0.5), 1.0);
        assert_eq!(at(520.0, 1.0, 0.5), 1.0);
        assert!((at(260.0, 1.0, 0.5) - 0.75).abs() < 1e-9);
        assert_eq!(at(960.0, 1.0, 0.5), 0.5);
        assert_eq!(at(1000.0, 1.0, 0.5), 0.0);
        let mut x = 0.0;
        while x <= w {
            assert!((at(x, 1.0, 0.5) - at(w - x, 1.0, 0.5)).abs() < 1e-9);
            assert!(at(x, 1.0, 0.5) <= 1.0);
            x += 7.0;
        }
        // Absolute values: the plateau is `main`, the ramp starts at `secondary`.
        assert_eq!(at(500.0, 0.6, 0.3), 0.6);
        assert_eq!(at(40.0, 0.6, 0.3), 0.3);
        assert!((at(260.0, 0.6, 0.3) - 0.45).abs() < 1e-9);
        // Secondary above main is not clamped: the mask falls from the ramp start towards the centre.
        assert_eq!(at(40.0, 0.3, 0.8), 0.8);
        assert_eq!(at(500.0, 0.3, 0.8), 0.3);
        assert!((at(260.0, 0.3, 0.8) - 0.55).abs() < 1e-9);
        // A small main.
        assert_eq!(at(500.0, 0.01, 0.1), 0.01);
        // Equal values: flat in the middle, only the em edge fades.
        assert_eq!(centre_mask_opacity(300.0, w, em, 1.0, 1.0), 1.0);
        // Narrow: the em shrinks to a quarter of the area, monotone towards the edges.
        let n = |x: f64| centre_mask_opacity(x, 80.0, 40.0, 1.0, 0.2);
        assert_eq!(n(20.0), 0.2);
        assert_eq!(n(40.0), 1.0);
        assert!(n(10.0) < n(20.0) && n(20.0) < n(25.0) && n(25.0) < n(30.0));
        let narrow = |x: f64| centre_mask_opacity(x, 80.0, 40.0, 0.4, 0.9);
        assert_eq!(narrow(20.0), 0.9);
        assert_eq!(narrow(40.0), 0.4);
        assert_eq!(centre_mask_stops(w, em, 1.0, 0.5).len(), 6);
        assert!(centre_mask_stops(0.0, 40.0, 1.0, 0.5).is_empty());
        // Out of range values are clamped on their own, NaN counts as 1.
        assert_eq!(centre_mask_opacity(500.0, w, em, 3.0, -1.0), 1.0);
        assert_eq!(centre_mask_opacity(40.0, w, em, 3.0, -1.0), 0.0);
        assert_eq!(centre_mask_opacity(500.0, w, em, f64::NAN, 0.5), 1.0);
    }

    #[test]
    fn the_sung_word_and_its_changes() {
        let words = [(1000.0, 1200.0), (1200.0, 1200.0), (1300.0, 1500.0)];
        assert_eq!(sung_word(&words, 999.0), None);
        assert_eq!(sung_word(&words, 1000.0), Some(0));
        assert_eq!(sung_word(&words, 1199.0), Some(0));
        assert_eq!(sung_word(&words, 1200.0), None);
        assert_eq!(sung_word(&words, 1250.0), None);
        assert_eq!(sung_word(&words, 1400.0), Some(2));
        assert_eq!(sung_word(&words, 1500.0), None);
        assert_eq!(next_word_change(&words, 1100.0), Some(1200.0));
        assert_eq!(next_word_change(&words, 1200.0), Some(1300.0));
        assert_eq!(next_word_change(&words, 1500.0), None);
    }

    #[test]
    fn word_ranges_follow_the_trimmed_text_in_utf16() {
        let r = word_utf16_ranges(" 夜空 🌙ab", &[" ", "夜空 ", "🌙", "ab"]).unwrap();
        assert_eq!(r, vec![(0, 0), (0, 3), (3, 2), (5, 2)]);
        assert!(word_utf16_ranges("abc", &["ab"]).is_none());
        assert!(word_utf16_ranges("abc", &[]).is_none());
    }
}
