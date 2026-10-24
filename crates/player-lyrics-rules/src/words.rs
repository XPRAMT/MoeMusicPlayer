//! Word-timed lyrics in the native windows (ADR 0041 option C and the 「無」 recolour).
//!
//! Continuous mode: the word glide (`glide::word_strip_glide`) keeps the word being sung near the lyric area's
//! centre; the line opacity fades are replaced by a static centre mask (`centre_mask_stops`). 「無」 mode: the karaoke
//! edge (`glide::karaoke_glide`) lights the line in the accent colour.

/// Width of the centre mask's fully opaque band, in em of the row.
pub const MASK_BAND_EM: f64 = 1.0;
/// The soft step between the opaque band and its neighbours (em).
pub const MASK_SOFT_EM: f64 = 0.1;

/// The static centre mask of a row as gradient stops `(offset 0..1 of the area width, opacity)`, linear between them:
/// opaque over `MASK_BAND_EM` em centred on the area, then a straight fall to 0 at the area's edges, with a
/// `MASK_SOFT_EM` soft step at the plateau's edges. The inactive-lyrics opacity is not used. Stops are clamped into the
/// area. Empty when the area or the em is empty.
pub fn centre_mask_stops(area_width: f64, em_px: f64) -> Vec<(f64, f64)> {
    if area_width.is_nan() || em_px.is_nan() || area_width <= 0.0 || em_px <= 0.0 {
        return Vec::new();
    }
    let c = area_width / 2.0;
    let a = MASK_BAND_EM * em_px / 2.0;
    let s = MASK_SOFT_EM * em_px / 2.0;
    // Where the soft step starts, on the straight ramp from the edge (0) to the plateau's edge (1).
    let q = if c - a > 0.0 {
        ((c - a - s) / (c - a)).max(0.0)
    } else {
        1.0
    };
    let raw = [
        (0.0, 0.0),
        (c - a - s, q),
        (c - a + s, 1.0),
        (c + a - s, 1.0),
        (c + a + s, q),
        (area_width, 0.0),
    ];
    raw.iter()
        .map(|&(x, o)| (x.clamp(0.0, area_width) / area_width, o))
        .collect()
}

/// The mask's opacity at `x` px from the area's left edge (what the gradient of `centre_mask_stops` draws; where two
/// stops share an offset the later one wins).
pub fn centre_mask_opacity(x: f64, area_width: f64, em_px: f64) -> f64 {
    let stops = centre_mask_stops(area_width, em_px);
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
    fn the_mask_is_an_opaque_em_with_straight_ramps_to_the_edges() {
        let (w, em) = (1000.0, 40.0);
        let at = |x: f64| centre_mask_opacity(x, w, em);
        assert_eq!(at(500.0), 1.0);
        assert_eq!(at(500.0 - 18.0), 1.0);
        assert_eq!(at(500.0 + 18.0), 1.0);
        assert_eq!(at(0.0), 0.0);
        assert_eq!(at(1000.0), 0.0);
        // The ramp is straight: half way from the edge to the plateau is half.
        assert!((at(240.0) - 0.5).abs() < 1e-9);
        assert!((at(760.0) - 0.5).abs() < 1e-9);
        let mut x = 0.0;
        while x <= w {
            assert!((at(x) - at(w - x)).abs() < 1e-9);
            x += 7.0;
        }
        assert_eq!(centre_mask_stops(w, em).len(), 6);
        assert_eq!(centre_mask_opacity(40.0, 80.0, 40.0), 1.0);
        assert!(centre_mask_stops(0.0, 40.0).is_empty());
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
