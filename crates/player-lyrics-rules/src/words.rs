//! Word-timed lyrics in the native windows (ADR 0041 option C and the 「無」 recolour).
//!
//! Continuous mode: the glide's knots are the word starts, so the word being sung sits at the lyric area's centre
//! (`word_knots`), with a speed cap that lets a too-fast word arrive a little early (`limit_knot_speed`); the same
//! monotone C1 curve as the line glide (`glide_cubic`). The line opacity fades are replaced by a static centre mask
//! (`word_mask_stops`). 「無」 mode: the word being sung is drawn in the accent colour (`sung_word`).

/// The cap on the glide speed into a knot: this many times the average speed of the knot's line.
pub const WORD_SPEED_FACTOR: f64 = 2.0;
/// A knot moved earlier by the speed cap never arrives more than this early.
pub const WORD_MAX_LEAD_MS: f64 = 600.0;
/// Width of the centre mask's fully opaque band, and of each neighbouring band, in em of the row.
pub const MASK_BAND_EM: f64 = 1.0;
/// The soft step between the opaque band and its neighbours (em).
pub const MASK_SOFT_EM: f64 = 0.1;

/// One line of the strip for the word glide: its start, its cell's left edge (px along the strip) and, for a word-timed
/// line, each word's start time and its centre (px from the cell's left edge).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KnotLine {
    pub start_ms: f64,
    pub cell_start: f64,
    pub word_starts: Vec<f64>,
    pub word_centres: Vec<f64>,
}

/// A knot of the glide: at lyrics time `t` the strip point `v` sits on the target; `line` is the line it belongs to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knot {
    pub t: f64,
    pub v: f64,
    pub line: usize,
}

/// The raw knots: every word start (its centre) of a word-timed line, the line start (its left edge, as in the line
/// glide) of a line without words.
pub fn word_knots(lines: &[KnotLine]) -> Vec<Knot> {
    let mut out = Vec::new();
    for (line, l) in lines.iter().enumerate() {
        let n = l.word_starts.len().min(l.word_centres.len());
        if n == 0 {
            out.push(Knot {
                t: l.start_ms,
                v: l.cell_start,
                line,
            });
            continue;
        }
        for i in 0..n {
            out.push(Knot {
                t: l.word_starts[i],
                v: l.cell_start + l.word_centres[i].max(0.0),
                line,
            });
        }
    }
    out
}

/// The cap (px per lyrics ms) of each line: `WORD_SPEED_FACTOR` x the line's average speed over its own knots;
/// `None` for a line with fewer than two knots or no span.
pub fn line_speed_caps(knots: &[Knot], line_count: usize) -> Vec<Option<f64>> {
    (0..line_count)
        .map(|line| {
            let mine: Vec<&Knot> = knots.iter().filter(|k| k.line == line).collect();
            let (first, last) = (mine.first()?, mine.last()?);
            let (dt, dv) = (last.t - first.t, last.v - first.v);
            (mine.len() >= 2 && dt > 0.0 && dv > 0.0).then(|| WORD_SPEED_FACTOR * dv / dt)
        })
        .collect()
}

/// Applies the speed cap: a segment into a knot faster than its line's cap moves the knot before it earlier (it
/// arrives early: the strip anticipates instead of jumping), never more than `WORD_MAX_LEAD_MS` and never before the
/// knot before that; then keeps times strictly increasing (of knots sharing a time the later one stays) and values
/// non-decreasing. The cap is soft: where both bounds bind the segment stays faster than the cap.
pub fn limit_knot_speed(knots: &[Knot], line_count: usize) -> Vec<Knot> {
    let caps = line_speed_caps(knots, line_count);
    let mut k: Vec<Knot> = knots.to_vec();
    for i in 1..k.len() {
        if k[i].v < k[i - 1].v {
            k[i].v = k[i - 1].v;
        }
    }
    let original: Vec<f64> = k.iter().map(|x| x.t).collect();
    for i in (1..k.len()).rev() {
        let Some(cap) = caps.get(k[i].line).copied().flatten() else {
            continue;
        };
        let need = (k[i].v - k[i - 1].v) / cap;
        if k[i].t - k[i - 1].t >= need {
            continue;
        }
        let mut t = (k[i].t - need).max(original[i - 1] - WORD_MAX_LEAD_MS);
        if i >= 2 {
            t = t.max(k[i - 2].t);
        }
        k[i - 1].t = k[i - 1].t.min(t);
    }
    let mut out: Vec<Knot> = Vec::with_capacity(k.len());
    for knot in k {
        while out.last().is_some_and(|p| p.t >= knot.t) {
            out.pop();
        }
        out.push(knot);
    }
    out
}

/// The monotone C1 cubic Hermite glide through `(t, v)` knots (the same curve as `strip_cubic`): 0 tangents at the
/// ends, Fritsch-Butland inside. Returns `[c0, c1, c2, c3]` of the strip offset (`target - v`) in `u = t - position`.
pub fn glide_cubic(knots: &[(f64, f64)], position_ms: f64, target: f64) -> [f64; 4] {
    let n = knots.len();
    if n == 0 {
        return [0.0; 4];
    }
    let time = |k: usize| knots[k].0;
    let value = |k: usize| knots[k].1;
    let tangent = |k: usize| {
        if k == 0 || k >= n - 1 {
            return 0.0;
        }
        let h0 = time(k) - time(k - 1);
        let h1 = time(k + 1) - time(k);
        if h0 <= 0.0 || h1 <= 0.0 {
            return 0.0;
        }
        let d0 = (value(k) - value(k - 1)) / h0;
        let d1 = (value(k + 1) - value(k)) / h1;
        if d0 <= 0.0 || d1 <= 0.0 {
            return 0.0;
        }
        let w0 = 2.0 * h1 + h0;
        let w1 = h1 + 2.0 * h0;
        (w0 + w1) / (w0 / d0 + w1 / d1)
    };
    let index = knots.partition_point(|k| k.0 <= position_ms) as isize - 1;
    if index < 0 {
        return [target - value(0), 0.0, 0.0, 0.0];
    }
    let i = index as usize;
    if i >= n - 1 {
        return [target - value(n - 1), 0.0, 0.0, 0.0];
    }
    let t0 = time(i);
    let h = time(i + 1) - t0;
    let v0 = value(i);
    let m0 = tangent(i);
    let m1 = tangent(i + 1);
    let d = (value(i + 1) - v0) / h;
    let c2 = (3.0 * d - 2.0 * m0 - m1) / h;
    let c3 = (m0 + m1 - 2.0 * d) / (h * h);
    let u = position_ms - t0;
    let anchor = v0 + u * (m0 + u * (c2 + u * c3));
    let velocity = m0 + u * (2.0 * c2 + 3.0 * c3 * u);
    [target - anchor, -velocity, -(c2 + 3.0 * c3 * u), -c3]
}

/// The knots of the word glide, ready for `glide_cubic`.
pub fn word_glide_knots(lines: &[KnotLine]) -> Vec<(f64, f64)> {
    limit_knot_speed(&word_knots(lines), lines.len())
        .into_iter()
        .map(|k| (k.t, k.v))
        .collect()
}

/// The lyrics times at which the word glide's pieces change (its knots after `position_ms`).
pub fn next_knot_after(knots: &[(f64, f64)], position_ms: f64) -> Option<f64> {
    knots.iter().map(|k| k.0).find(|&t| t > position_ms)
}

/// The static centre mask of a row as gradient stops `(offset 0..1 of the area width, opacity)`, linear between them:
/// opaque over `MASK_BAND_EM` em centred on the area, `inactive` over the next `MASK_BAND_EM` em on each side (a
/// `MASK_SOFT_EM` step between), then a linear fade to 0 at the area's edges. Stops are clamped into the area, so a
/// narrow area loses its fade first and then its neighbour bands. Empty when the area or the em is empty.
pub fn word_mask_stops(area_width: f64, em_px: f64, inactive_percent: u32) -> Vec<(f64, f64)> {
    if area_width.is_nan() || em_px.is_nan() || area_width <= 0.0 || em_px <= 0.0 {
        return Vec::new();
    }
    let inactive = inactive_percent.clamp(10, 100) as f64 / 100.0;
    let c = area_width / 2.0;
    let a = MASK_BAND_EM * em_px / 2.0;
    let b = a + MASK_BAND_EM * em_px;
    let s = MASK_SOFT_EM * em_px / 2.0;
    let raw = [
        (0.0, 0.0),
        (c - b, inactive),
        (c - a - s, inactive),
        (c - a + s, 1.0),
        (c + a - s, 1.0),
        (c + a + s, inactive),
        (c + b, inactive),
        (area_width, 0.0),
    ];
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (x, o) in raw {
        let x = x.clamp(0.0, area_width);
        out.push((x / area_width, o));
    }
    out
}

/// The mask's opacity at `x` px from the area's left edge (what the gradient of `word_mask_stops` draws; where two
/// stops share an offset the later one wins).
pub fn word_mask_opacity(x: f64, area_width: f64, em_px: f64, inactive_percent: u32) -> f64 {
    let stops = word_mask_stops(area_width, em_px, inactive_percent);
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
    (0..i).rev().find(|&j| t < words[j].1 && words[j].1 > words[j].0)
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

    fn line(start: f64, cell: f64, words: &[(f64, f64)]) -> KnotLine {
        KnotLine {
            start_ms: start,
            cell_start: cell,
            word_starts: words.iter().map(|w| w.0).collect(),
            word_centres: words.iter().map(|w| w.1).collect(),
        }
    }

    #[test]
    fn word_knots_put_each_word_start_on_its_centre() {
        let lines = [
            line(1000.0, 0.0, &[(1000.0, 10.0), (1300.0, 30.0), (1600.0, 50.0)]),
            line(3000.0, 100.0, &[]),
        ];
        let k = word_knots(&lines);
        assert_eq!(
            k.iter().map(|k| (k.t, k.v)).collect::<Vec<_>>(),
            vec![(1000.0, 10.0), (1300.0, 30.0), (1600.0, 50.0), (3000.0, 100.0)]
        );
        // Even timing, even spacing: nothing to limit, and the curve hits every knot.
        let knots = word_glide_knots(&lines);
        assert_eq!(knots.len(), 4);
        for &(t, v) in &knots {
            assert!((glide_cubic(&knots, t, 200.0)[0] - (200.0 - v)).abs() < 1e-9);
        }
    }

    #[test]
    fn the_speed_cap_moves_a_rushed_knot_earlier_within_its_bounds() {
        // Average speed 0.1 px/ms (cap 0.2); the 3rd word comes 20 ms after the 2nd but is 40 px further.
        let lines = [line(
            0.0,
            0.0,
            &[(0.0, 0.0), (1000.0, 50.0), (1020.0, 90.0), (2000.0, 200.0)],
        )];
        let caps = line_speed_caps(&word_knots(&lines), 1);
        assert!((caps[0].unwrap() - 0.2).abs() < 1e-12);
        let k = limit_knot_speed(&word_knots(&lines), 1);
        // 40 px at 0.2 px/ms needs 200 ms: the 2nd word arrives 180 ms early; the others stay.
        assert_eq!(k[1].t, 820.0);
        assert_eq!((k[2].t, k[3].t, k[0].t), (1020.0, 2000.0, 0.0));
        for w in k.windows(2) {
            let speed = (w[1].v - w[0].v) / (w[1].t - w[0].t);
            assert!(speed <= 0.2 + 1e-9, "{speed}");
        }
        // Never more than WORD_MAX_LEAD_MS early.
        let lines = [line(
            0.0,
            0.0,
            &[(0.0, 0.0), (5000.0, 10.0), (5001.0, 900.0), (10000.0, 1000.0)],
        )];
        let k = limit_knot_speed(&word_knots(&lines), 1);
        assert_eq!(k[1].t, 5000.0 - WORD_MAX_LEAD_MS);
    }

    #[test]
    fn shared_times_and_backward_values_never_jump() {
        let lines = [line(
            0.0,
            0.0,
            &[(0.0, 0.0), (500.0, 20.0), (500.0, 40.0), (900.0, 30.0)],
        )];
        let k = limit_knot_speed(&word_knots(&lines), 1);
        for w in k.windows(2) {
            assert!(w[1].t > w[0].t && w[1].v >= w[0].v, "{k:?}");
        }
    }

    /// Dense, uneven word timing sampled every 120 Hz frame: the offset never moves back, never steps further than
    /// the curve's own speed allows, and the speed stays under the cap's bound (3x of PCHIP over the capped slope).
    #[test]
    fn per_frame_the_word_glide_never_goes_back_or_jumps() {
        let mut lines = Vec::new();
        let mut seed = 11u64;
        let mut rand = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as f64 / (1u64 << 31) as f64
        };
        let mut t = 500.0;
        let mut cell = 0.0;
        for i in 0..30 {
            let mut words = Vec::new();
            let mut x = 0.0;
            let n = 3 + i % 9;
            for _ in 0..n {
                let w = 10.0 + rand() * 60.0;
                words.push((t, x + w / 2.0));
                x += w;
                // Dense and uneven: 15..400 ms between words, sometimes the same time.
                t += if rand() < 0.1 { 0.0 } else { 15.0 + rand() * 385.0 };
            }
            lines.push(line(words[0].0, cell, &words));
            cell += x + 24.0;
            t += 200.0 + rand() * 1500.0;
        }
        // Every other line without words.
        for l in lines.iter_mut().skip(1).step_by(4) {
            l.word_starts.clear();
            l.word_centres.clear();
        }
        let knots = word_glide_knots(&lines);
        let caps = line_speed_caps(&word_knots(&lines), lines.len());
        let max_cap = caps.iter().flatten().fold(0.0f64, |a, &b| a.max(b));
        let frame = 1000.0 / 120.0;
        let c0 = glide_cubic(&knots, 0.0, 300.0);
        let (mut prev, mut prev_speed) = (c0[0], -c0[1]);
        let mut p = 0.0;
        let mut fastest = 0.0f64;
        while p < t + 1000.0 {
            p += frame;
            let c = glide_cubic(&knots, p, 300.0);
            let step = prev - c[0];
            assert!(step >= -1e-9, "backwards at {p}");
            // No jump: a frame moves no further than the curve's own speed at its ends allows (C1, cubic pieces).
            let bound = 1.5 * prev_speed.max(-c[1]) * frame + 1e-6;
            assert!(step <= bound.max(0.5), "jump at {p}: {step} > {bound}");
            fastest = fastest.max(-c[1]);
            prev = c[0];
            prev_speed = -c[1];
        }
        assert!(fastest > 0.0);
        // The speed stays within the Fritsch-Butland bound (3x) of the capped slopes, plus the soft-cap leftovers.
        let over = knots
            .windows(2)
            .filter(|w| (w[1].1 - w[0].1) / (w[1].0 - w[0].0) > max_cap + 1e-9)
            .count();
        assert!(fastest <= 3.0 * max_cap + 1e-9 || over > 0, "{fastest} vs cap {max_cap}");
    }

    #[test]
    fn the_mask_has_an_opaque_em_neighbour_bands_and_an_edge_fade() {
        let (w, em) = (1000.0, 40.0);
        let at = |x: f64| word_mask_opacity(x, w, em, 50);
        assert_eq!(at(500.0), 1.0);
        assert_eq!(at(500.0 - 17.0), 1.0);
        assert_eq!(at(500.0 + 17.0), 1.0);
        assert!((at(480.0) - 0.75).abs() < 1e-9); // the middle of the 0.1 em soft step
        assert_eq!(at(500.0 - 25.0), 0.5);
        assert_eq!(at(500.0 + 59.0), 0.5);
        assert_eq!(at(440.0), 0.5); // 1.5 em from the centre: the fade starts
        assert!((at(220.0) - 0.25).abs() < 1e-9); // half way to the edge
        assert_eq!(at(0.0), 0.0);
        assert_eq!(at(1000.0), 0.0);
        // Symmetric.
        let mut x = 0.0;
        while x <= w {
            assert!((at(x) - at(w - x)).abs() < 1e-9);
            x += 7.0;
        }
        let stops = word_mask_stops(w, em, 50);
        assert_eq!(stops.len(), 8);
        // A narrow area: everything clamped inside, still symmetric and opaque at the centre.
        let narrow = |x: f64| word_mask_opacity(x, 80.0, 40.0, 30);
        assert_eq!(narrow(40.0), 1.0);
        assert_eq!(narrow(0.0), 0.3);
        assert!(word_mask_stops(0.0, 40.0, 50).is_empty());
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
