//! The smoothed glide of word-timed lyrics (ADR 0041 D10), the reference for `WordLyricsRules.kt` and the Now Playing
//! karaoke glide `src/lib/lyrics-glide.js` (checked by the vectors `examples/generate_vectors.rs` writes).
//! `smooth_glide` drives the karaoke colour edge; the continuous strip's glide ignores individual words and is a plain
//! monotone cubic through per-line knots: every line's start, a word-timed line's end, another line's duration cap
//! and, after a last line without word times, the terminal estimate (`word_strip_knots`, `knot_cubic`).
//!
//! Hard knots (the line start and end of a karaoke line) are hit exactly. Between two hard knots the curve
//! follows soft guides (the words) with its speed kept within `GLIDE_MIN_SPEED..GLIDE_MAX_SPEED` x the stretch's
//! average speed: a polyline through the hard knots and the guides (each guide clamped into what the band can reach
//! from the previous point and still reach the end), a tangent run of `GLIDE_KNOT_WINDOW` of the shorter neighbouring
//! stretch on each side of every hard knot, and every other corner rounded by a symmetric quadratic blend over half of
//! each neighbouring run. The speed is piecewise linear between polyline slopes: inside the band, never negative
//! (monotone) and continuous (C1).

/// The slowest the glide moves inside a stretch, as a fraction of the stretch's average speed.
pub const GLIDE_MIN_SPEED: f64 = 0.5;
/// The fastest the glide moves inside a stretch, as a multiple of the stretch's average speed.
pub const GLIDE_MAX_SPEED: f64 = 1.5;
/// Each hard knot's straight run on each side, as a fraction of the shorter neighbouring stretch.
pub const GLIDE_KNOT_WINDOW: f64 = 0.125;

/// One piece of a glide: from lyrics time `t` up to the next piece the value is `v + s u + c u^2` (`u` = time - `t`).
/// The last piece is the rest value (`s = c = 0`); before the first piece the value is the first piece's `v`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlidePiece {
    pub t: f64,
    pub v: f64,
    pub s: f64,
    pub c: f64,
}

/// The smoothed glide through hard knots `(t, v)` (later knots with an earlier or equal time are dropped, values are
/// made non-decreasing) with soft guides `guides[j]` (time order) between knot `j` and knot `j + 1`. `ease_ends`: the
/// curve starts and stops at speed 0 (the strip rests outside it); otherwise at the first / last stretch's average.
pub fn smooth_glide(
    hard: &[(f64, f64)],
    guides: &[Vec<(f64, f64)>],
    ease_ends: bool,
) -> Vec<GlidePiece> {
    let mut knots: Vec<(f64, f64)> = Vec::new();
    let mut between: Vec<&[(f64, f64)]> = Vec::new();
    let mut dropped = false;
    for (j, &(t, v)) in hard.iter().enumerate() {
        if !t.is_finite() || !v.is_finite() {
            dropped = true;
            continue;
        }
        if let Some(&(pt, pv)) = knots.last() {
            if t <= pt {
                dropped = true;
                continue;
            }
            between.push(if dropped {
                &[]
            } else {
                guides.get(j - 1).map(|g| g.as_slice()).unwrap_or(&[])
            });
            knots.push((t, v.max(pv)));
        } else {
            knots.push((t, v));
        }
        dropped = false;
    }
    let n = knots.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![GlidePiece {
            t: knots[0].0,
            v: knots[0].1,
            s: 0.0,
            c: 0.0,
        }];
    }
    let span = |j: usize| knots[j + 1].0 - knots[j].0;
    let avg: Vec<f64> = (0..n - 1)
        .map(|j| (knots[j + 1].1 - knots[j].1) / span(j))
        .collect();
    let mut tangent = Vec::with_capacity(n);
    let mut window = Vec::with_capacity(n);
    for j in 0..n {
        if j == 0 || j == n - 1 {
            let edge = if j == 0 { 0 } else { n - 2 };
            tangent.push(if ease_ends { 0.0 } else { avg[edge] });
            window.push(GLIDE_KNOT_WINDOW * span(edge));
            continue;
        }
        let (a_l, a_r) = (avg[j - 1], avg[j]);
        let lo = GLIDE_MIN_SPEED * a_l.max(a_r);
        let hi = GLIDE_MAX_SPEED * a_l.min(a_r);
        let base = if a_l + a_r > 0.0 {
            2.0 * a_l * a_r / (a_l + a_r)
        } else {
            0.0
        };
        tangent.push(if lo <= hi { base.clamp(lo, hi) } else { hi });
        window.push(GLIDE_KNOT_WINDOW * span(j - 1).min(span(j)));
    }
    // The polyline: (t, v, hard).
    let mut points: Vec<(f64, f64, bool)> = vec![(knots[0].0, knots[0].1, true)];
    for j in 0..n - 1 {
        let lo = GLIDE_MIN_SPEED * avg[j];
        let hi = GLIDE_MAX_SPEED * avg[j];
        let a = (knots[j].0 + window[j], knots[j].1 + tangent[j] * window[j]);
        let b = (
            knots[j + 1].0 - window[j + 1],
            knots[j + 1].1 - tangent[j + 1] * window[j + 1],
        );
        points.push((a.0, a.1, false));
        let mut prev = a;
        for &(t, v) in between[j] {
            if !(t > prev.0 && t < b.0 && v.is_finite()) {
                continue;
            }
            let low = (prev.1 + lo * (t - prev.0)).max(b.1 - hi * (b.0 - t));
            let high = (prev.1 + hi * (t - prev.0)).min(b.1 - lo * (b.0 - t));
            let value = if low <= high {
                v.clamp(low, high)
            } else {
                (low + high) / 2.0
            };
            prev = (t, value);
            points.push((t, value, false));
        }
        points.push((b.0, b.1, false));
        points.push((knots[j + 1].0, knots[j + 1].1, true));
    }
    let slope = |k: usize| (points[k + 1].1 - points[k].1) / (points[k + 1].0 - points[k].0);
    let mut pieces = Vec::new();
    let mut cursor = (points[0].0, points[0].1);
    for k in 1..points.len() {
        let (t, v, hard) = points[k];
        let s_in = slope(k - 1);
        let s_out = if k + 1 < points.len() { slope(k) } else { 0.0 };
        let mut r = 0.0;
        if !hard && k + 1 < points.len() && (s_out - s_in).abs() > 1e-12 {
            r = (t - points[k - 1].0).min(points[k + 1].0 - t) / 2.0;
        }
        if t - r > cursor.0 {
            pieces.push(GlidePiece {
                t: cursor.0,
                v: cursor.1,
                s: s_in,
                c: 0.0,
            });
        }
        if r > 0.0 {
            pieces.push(GlidePiece {
                t: t - r,
                v: v - s_in * r,
                s: s_in,
                c: (s_out - s_in) / (4.0 * r),
            });
            cursor = (t + r, v + s_out * r);
        } else {
            cursor = (t, v);
        }
    }
    pieces.push(GlidePiece {
        t: cursor.0,
        v: cursor.1,
        s: 0.0,
        c: 0.0,
    });
    pieces
}

/// Index of the piece in effect at `time` (`None` before the first).
fn piece_index(pieces: &[GlidePiece], time: f64) -> Option<usize> {
    pieces.partition_point(|p| p.t <= time).checked_sub(1)
}

/// The glide at lyrics time `time` as `[value, speed, half acceleration]`: the value at `time + u` is
/// `value + speed u + half_acceleration u^2` up to the next piece. Zeros for an empty glide.
pub fn glide_at(pieces: &[GlidePiece], time: f64) -> [f64; 3] {
    let Some(first) = pieces.first() else {
        return [0.0; 3];
    };
    match piece_index(pieces, time) {
        None => [first.v, 0.0, 0.0],
        Some(i) => {
            let p = pieces[i];
            let u = time - p.t;
            [p.v + u * (p.s + p.c * u), p.s + 2.0 * p.c * u, p.c]
        }
    }
}

/// The glide's value at `time`.
pub fn glide_value(pieces: &[GlidePiece], time: f64) -> f64 {
    glide_at(pieces, time)[0]
}

/// The next lyrics time after `time` at which the glide's piece changes.
pub fn next_glide_break(pieces: &[GlidePiece], time: f64) -> Option<f64> {
    let i = piece_index(pieces, time).map_or(0, |i| i + 1);
    pieces.get(i).map(|p| p.t)
}

/// The karaoke edge of one line (「無」 mode): the x in the row's own px up to which the line is lit. Hard at the
/// first word's start (its left) and the last word's end (its right: the whole line lit); each word's start (left)
/// and end (right) guide it in between (`karaokeGlide` in JS). Empty without words (the edge is then 0).
pub fn karaoke_glide(spans: &[(f64, f64)], words: &[(f64, f64)]) -> Vec<GlidePiece> {
    let n = spans.len().min(words.len());
    if n == 0 {
        return Vec::new();
    }
    let start = words[0].0;
    let mut end = words[n - 1].1;
    if end.partial_cmp(&start) != Some(std::cmp::Ordering::Greater) {
        end = start + 1.0;
    }
    let mut guides = Vec::new();
    for i in 0..n {
        if i > 0 {
            guides.push((words[i].0, spans[i].0));
        }
        if i < n - 1 && words[i].1 < words[i + 1].0 {
            guides.push((words[i].1, spans[i].1));
        }
    }
    smooth_glide(
        &[(start, spans[0].0), (end, spans[n - 1].1)],
        &[guides],
        false,
    )
}

/// One line of the continuous strip for the word glide: its start, its cell's left edge (px along the strip), its first
/// glyph's centre (px from the cell's left edge), its last glyph's centre (px from the cell's left edge) and either,
/// for a word-timed line, the end of its last word, or, for a line without word times, its duration cap
/// (`crate::line_cap_ms` of its text; `None`: no cap).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KnotLine {
    pub start_ms: f64,
    pub cell_start: f64,
    pub anchor: f64,
    pub end_ms: Option<f64>,
    pub last_centre: f64,
    pub cap_ms: Option<f64>,
}

/// The continuous strip's glide knots `(lyrics ms, strip px on the lyric area's centre)`, for line- and word-timed
/// lyrics alike. Each line is judged on its own; individual word times are ignored. Every line has its start (first
/// glyph's centre on the area's centre). A word-timed line (`end_ms`) also has its end (the last word's end, last
/// glyph's centre); a line without word times that is not the last line has one at `start_ms + cap_ms` (its duration
/// cap, `crate::line_cap_ms`: 1000 ms per wide grapheme, 500 ms per other one), so a long interlude after it does not
/// drag it across the whole gap. An end knot is skipped for a one-glyph line, an end not after the start, when the
/// next line starts at or before it (for a capped line: the next start is within the cap, so nothing changes), or when
/// the gap from the end to the next line's start would move faster than the line itself (the strip then glides from
/// this line's start straight to the next start; the last line keeps its end). When the last line has no word times,
/// `final_arrival_ms` (`final_arrival_ms`: 500 ms per wide grapheme, 250 ms per other one, capped at the track end)
/// adds the terminal knot with its last glyph's centre, as the line glide (`strip_cubic`) does; a word-timed last line
/// uses its own end instead. Times strictly increase (a later knot at or before an earlier one's time replaces it, so
/// the later line's start wins an overlap) and values never decrease.
pub fn word_strip_knots(lines: &[KnotLine], final_arrival_ms: Option<f64>) -> Vec<(f64, f64)> {
    let start_of = |line: &KnotLine| line.cell_start + line.anchor.max(0.0);
    let has_start = |line: &KnotLine| line.start_ms.is_finite() && start_of(line).is_finite();
    let mut knots: Vec<(f64, f64)> = Vec::with_capacity(lines.len() * 2);
    let mut push = |t: f64, v: f64| {
        if !t.is_finite() || !v.is_finite() {
            return;
        }
        while knots.last().is_some_and(|k| k.0 >= t) {
            knots.pop();
        }
        let v = knots.last().map_or(v, |k| v.max(k.1));
        knots.push((t, v));
    };
    for (i, line) in lines.iter().enumerate() {
        let start = start_of(line);
        push(line.start_ms, start);
        // A line without word times ends at its cap, unless it is the last line (the terminal knot below).
        let end = line.end_ms.or_else(|| {
            line.cap_ms
                .filter(|cap| *cap > 0.0 && i + 1 < lines.len())
                .map(|cap| line.start_ms + cap)
        });
        if let Some(end) = end {
            let value = line.cell_start + line.last_centre.max(0.0);
            if end > line.start_ms && value > start {
                // The gap to the next start is faster than the line when (next - value) / (next_ms - end) >
                // (value - start) / (end - start); cross-multiplied (both spans positive) so every language agrees.
                // A capped line only ends before a next line.
                let skip = lines[i + 1..].iter().find(|l| has_start(l)).map_or(
                    line.end_ms.is_none(),
                    |next| {
                        let gap_ms = next.start_ms - end;
                        gap_ms <= 0.0
                            || (start_of(next) - value) * (end - line.start_ms)
                                > (value - start) * gap_ms
                    },
                );
                if !skip {
                    push(end, value);
                }
            }
        }
    }
    if let (Some(arrival), Some(last)) = (final_arrival_ms, lines.last()) {
        if last.end_ms.is_none() && arrival > last.start_ms {
            let start = start_of(last);
            push(
                arrival,
                start.max(last.cell_start + last.last_centre.max(0.0)),
            );
        }
    }
    knots
}

/// The monotone C1 cubic Hermite through `count` knots (equal times get tangent 0). Tangents are 0 at the first and
/// last knot and, inside, the weighted harmonic mean of the neighbouring slopes (Fritsch-Butland; 0 when a slope is not
/// positive). Returns `[value, speed, c2, c3]`: the value at `position + u` is `value + speed u + c2 u^2 + c3 u^3` up
/// to the next knot; before the first and after the last knot the value rests.
pub(crate) fn hermite_cubic(
    count: usize,
    time: impl Fn(usize) -> f64,
    value: impl Fn(usize) -> f64,
    position: f64,
) -> [f64; 4] {
    if count == 0 {
        return [0.0; 4];
    }
    let tangent = |k: usize| {
        if k == 0 || k >= count - 1 {
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
    let mut index: isize = -1;
    while ((index + 1) as usize) < count && time((index + 1) as usize) <= position {
        index += 1;
    }
    if index < 0 {
        return [value(0), 0.0, 0.0, 0.0];
    }
    let i = index as usize;
    if i >= count - 1 {
        return [value(count - 1), 0.0, 0.0, 0.0];
    }
    let t0 = time(i);
    let h = time(i + 1) - t0;
    let v0 = value(i);
    let m0 = tangent(i);
    let m1 = tangent(i + 1);
    let d = (value(i + 1) - v0) / h;
    let c2 = (3.0 * d - 2.0 * m0 - m1) / h;
    let c3 = (m0 + m1 - 2.0 * d) / (h * h);
    let u = position - t0;
    [
        v0 + u * (m0 + u * (c2 + u * c3)),
        m0 + u * (2.0 * c2 + 3.0 * c3 * u),
        c2 + 3.0 * c3 * u,
        c3,
    ]
}

/// The word glide at lyrics time `time` as `[value, speed, c2, c3]` (see `hermite_cubic`) through `knots`.
pub fn knot_cubic(knots: &[(f64, f64)], time: f64) -> [f64; 4] {
    hermite_cubic(knots.len(), |k| knots[k].0, |k| knots[k].1, time)
}

/// The word glide's value at `time`.
pub fn knot_value(knots: &[(f64, f64)], time: f64) -> f64 {
    knot_cubic(knots, time)[0]
}

/// The next lyrics time after `time` at which the word glide has a knot.
pub fn next_knot_break(knots: &[(f64, f64)], time: f64) -> Option<f64> {
    let i = knots.partition_point(|k| k.0 <= time);
    knots.get(i).map(|k| k.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Samples every 120 Hz frame and checks: monotone, C1 (no jump in speed between frames beyond the curve's own
    /// acceleration), hard knots exact, and the speed band of every stretch outside its eased ends.
    fn check(pieces: &[GlidePiece], hard: &[(f64, f64)], ease_ends: bool) {
        for &(t, v) in hard {
            assert!(
                (glide_value(pieces, t) - v).abs() < 1e-9,
                "hard knot {t} {v}"
            );
        }
        for (i, w) in pieces.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            let h = b.t - a.t;
            assert!(h > 0.0);
            assert!(
                (a.v + h * (a.s + a.c * h) - b.v).abs() < 1e-7,
                "C0 at {}",
                b.t
            );
            // C1 everywhere, except the karaoke edge stopping at the line end.
            if ease_ends || i + 2 < pieces.len() {
                let end_s = a.s + 2.0 * a.c * h;
                assert!(
                    (end_s - b.s).abs() < 1e-9,
                    "C1 at {}: {end_s} vs {}",
                    b.t,
                    b.s
                );
            }
        }
        let n = hard.len();
        let avg = |j: usize| (hard[j + 1].1 - hard[j].1) / (hard[j + 1].0 - hard[j].0);
        // A knot's window is exempt from the band where the band cannot hold: an eased end, or neighbours whose
        // bands do not overlap (more than 3x apart).
        let exempt = |j: usize| {
            if j == 0 || j == n - 1 {
                return ease_ends;
            }
            GLIDE_MIN_SPEED * avg(j - 1).max(avg(j)) > GLIDE_MAX_SPEED * avg(j - 1).min(avg(j))
        };
        let window = |j: usize| {
            let l = if j > 0 {
                hard[j].0 - hard[j - 1].0
            } else {
                f64::INFINITY
            };
            let r = if j + 1 < n {
                hard[j + 1].0 - hard[j].0
            } else {
                f64::INFINITY
            };
            GLIDE_KNOT_WINDOW * l.min(r)
        };
        let frame = 1000.0 / 120.0;
        let (t0, t1) = (hard[0].0, hard[hard.len() - 1].0);
        let mut prev = glide_value(pieces, t0 - 100.0);
        let mut t = t0 - 100.0;
        while t < t1 + 100.0 {
            t += frame;
            let [v, s, _] = glide_at(pieces, t);
            assert!(v >= prev - 1e-9, "backwards at {t}");
            assert!(s >= -1e-12);
            prev = v;
            if let Some(j) = (0..n - 1).find(|&j| t >= hard[j].0 && t < hard[j + 1].0) {
                let near = (t - hard[j].0 < 1.5 * window(j) && exempt(j))
                    || (hard[j + 1].0 - t < 1.5 * window(j + 1) && exempt(j + 1));
                if !near {
                    let a = avg(j);
                    assert!(
                        s >= GLIDE_MIN_SPEED * a - 1e-9 && s <= GLIDE_MAX_SPEED * a + 1e-9,
                        "speed {s} outside band of {a} at {t}"
                    );
                }
            }
        }
    }

    #[test]
    fn karaoke_edge_is_hard_at_both_ends_and_inside_the_band() {
        let words = [
            (1000.0, 1100.0),
            (1100.0, 1150.0),
            (1150.0, 1600.0),
            (1700.0, 1720.0),
            (1720.0, 2400.0),
        ];
        let spans = [
            (0.0, 20.0),
            (20.0, 40.0),
            (40.0, 60.0),
            (64.0, 80.0),
            (80.0, 120.0),
        ];
        let p = karaoke_glide(&spans, &words);
        check(&p, &[(1000.0, 0.0), (2400.0, 120.0)], false);
        assert_eq!(glide_value(&p, 500.0), 0.0);
        assert_eq!(glide_value(&p, 9000.0), 120.0);
        assert!(karaoke_glide(&[], &[]).is_empty());
        assert_eq!(glide_value(&[], 5.0), 0.0);
    }

    fn line(start: f64, cell: f64, anchor: f64, end: Option<f64>, last: f64) -> KnotLine {
        KnotLine {
            start_ms: start,
            cell_start: cell,
            anchor,
            end_ms: end,
            last_centre: last,
            cap_ms: None,
        }
    }

    /// A line without word times, capped at `crate::line_cap_ms(text)`.
    fn capped(start: f64, cell: f64, anchor: f64, last: f64, text: &str) -> KnotLine {
        KnotLine {
            cap_ms: Some(crate::line_cap_ms(text)),
            ..line(start, cell, anchor, None, last)
        }
    }

    /// 十年有多长: 重新学会做梦和期望 (9 wide graphemes, 20 px each) at 03:35.19, then a 16 s interlude until
    /// 啦啦啦啦啦啦啦啦 at 03:51.30. The cap (9 × 1000 ms) ends the line at 03:44.19 with its last glyph on the centre;
    /// the remaining 7.11 s glide on to 啦.
    #[test]
    fn a_line_without_word_times_ends_at_its_cap_before_a_long_interlude() {
        let lines = [
            capped(215190.0, 0.0, 10.0, 170.0, "重新学会做梦和期望"),
            capped(231300.0, 220.0, 10.0, 150.0, "啦啦啦啦啦啦啦啦"),
        ];
        let arrival = crate::final_arrival_ms(231300.0, "啦啦啦啦啦啦啦啦", None);
        let knots = word_strip_knots(&lines, Some(arrival));
        assert_eq!(
            knots,
            vec![
                (215190.0, 10.0),
                (224190.0, 170.0),
                (231300.0, 230.0),
                (235300.0, 370.0)
            ]
        );
        // The line moves through its glyphs during the cap, then the interlude takes the rest at a gentler pace.
        let mid = knot_value(&knots, 219690.0);
        assert!(mid > 10.0 && mid < 170.0, "{mid}");
        let gap = knot_value(&knots, 227745.0);
        assert!(gap > 170.0 && gap < 230.0, "{gap}");
        // The next line within the cap: no end knot (the former motion).
        let soon = [
            capped(1000.0, 0.0, 10.0, 170.0, "重新学会做梦和期望"),
            capped(8000.0, 220.0, 10.0, 150.0, "啦啦啦啦啦啦啦啦"),
        ];
        assert_eq!(
            word_strip_knots(&soon, None),
            vec![(1000.0, 10.0), (8000.0, 230.0)]
        );
        // Exactly at the cap: no end knot either.
        let at_cap = [
            capped(1000.0, 0.0, 10.0, 170.0, "重新学会做梦和期望"),
            capped(10000.0, 220.0, 10.0, 150.0, "啦啦啦啦啦啦啦啦"),
        ];
        assert_eq!(
            word_strip_knots(&at_cap, None),
            vec![(1000.0, 10.0), (10000.0, 230.0)]
        );
        // Half-width English: 11 graphemes (the space included) × 500 ms.
        let english = [
            capped(1000.0, 0.0, 5.0, 105.0, "Hello world"),
            capped(20000.0, 130.0, 5.0, 60.0, "Next"),
        ];
        assert_eq!(
            word_strip_knots(&english, None),
            vec![(1000.0, 5.0), (6500.0, 105.0), (20000.0, 135.0)]
        );
        // The last line keeps the terminal estimate, never its cap; one glyph gets no end knot.
        let single = [
            capped(1000.0, 0.0, 10.0, 10.0, "啊"),
            capped(9000.0, 60.0, 10.0, 10.0, "啊"),
        ];
        assert_eq!(
            word_strip_knots(&single, None),
            vec![(1000.0, 10.0), (9000.0, 70.0)]
        );
    }

    #[test]
    fn knots_are_line_starts_and_last_word_ends() {
        let knots = word_strip_knots(
            &[
                line(1000.0, 0.0, 10.0, Some(2000.0), 90.0),
                // No word times: the start knot only.
                line(3000.0, 120.0, 8.0, None, 0.0),
                // One glyph (first centre == last centre): no end knot.
                line(4000.0, 200.0, 6.0, Some(4500.0), 6.0),
                // End not after the start: no end knot.
                line(5000.0, 260.0, 5.0, Some(5000.0), 80.0),
            ],
            None,
        );
        assert_eq!(
            knots,
            vec![
                (1000.0, 10.0),
                (2000.0, 90.0),
                (3000.0, 128.0),
                (4000.0, 206.0),
                (5000.0, 265.0)
            ]
        );
    }

    #[test]
    fn an_overlapping_next_start_wins_and_values_never_decrease() {
        let knots = word_strip_knots(
            &[
                line(1000.0, 0.0, 10.0, Some(3000.0), 90.0),
                line(2500.0, 100.0, 10.0, Some(4000.0), 50.0),
                line(2500.0, 200.0, 4.0, None, 0.0),
            ],
            None,
        );
        assert_eq!(knots, vec![(1000.0, 10.0), (2500.0, 204.0)]);
        assert!(word_strip_knots(&[], None).is_empty());
    }

    /// 拿出下一颗软糖 ends 10 ms before the next line starts (20 px glyphs, 40 px gap): the end knot would force a 60 px
    /// move in 10 ms, so it is skipped and every 120 Hz frame around the boundary moves only a little.
    #[test]
    fn an_end_knot_is_skipped_when_the_gap_is_faster_than_the_line() {
        let lines = [
            line(207810.0, 0.0, 10.0, None, 0.0),
            line(212700.0, 280.0, 10.0, Some(215180.0), 130.0),
            line(215190.0, 460.0, 10.0, None, 0.0),
            line(231300.0, 680.0, 10.0, None, 0.0),
        ];
        let knots = word_strip_knots(&lines, None);
        assert_eq!(
            knots,
            vec![
                (207810.0, 10.0),
                (212700.0, 290.0),
                (215190.0, 470.0),
                (231300.0, 690.0)
            ]
        );
        let frame = 1000.0 / 120.0;
        let mut t = 214000.0;
        let mut prev = knot_value(&knots, t);
        let mut largest: f64 = 0.0;
        while t < 216500.0 {
            t += frame;
            let now = knot_value(&knots, t);
            largest = largest.max(now - prev);
            prev = now;
        }
        assert!(largest < 1.5, "largest frame step {largest} px");
    }

    /// Each line is judged on its own: in 十年有多长 the last line 啦啦啦啦啦啦啦啦 has no word times, so it keeps the
    /// terminal knot (500 ms per wide grapheme from its start, its last glyph's centre) even though an earlier line is
    /// word-timed; it no longer rests on its first glyph.
    #[test]
    fn a_last_line_without_word_times_glides_to_its_last_glyph() {
        let mut lines = vec![
            line(207810.0, 0.0, 10.0, None, 0.0),
            line(212700.0, 280.0, 10.0, Some(215180.0), 130.0),
            line(215190.0, 460.0, 10.0, None, 0.0),
            line(231300.0, 680.0, 10.0, None, 150.0),
        ];
        let arrival = crate::final_arrival_ms(231300.0, "啦啦啦啦啦啦啦啦", None);
        assert_eq!(arrival, 235300.0);
        let knots = word_strip_knots(&lines, Some(arrival));
        assert_eq!(
            knots,
            vec![
                (207810.0, 10.0),
                (212700.0, 290.0),
                (215190.0, 470.0),
                (231300.0, 690.0),
                (235300.0, 830.0)
            ]
        );
        // Capped at the track end.
        let capped = crate::final_arrival_ms(231300.0, "啦啦啦啦啦啦啦啦", Some(233000.0));
        assert_eq!(
            word_strip_knots(&lines, Some(capped)).last(),
            Some(&(233000.0, 830.0))
        );
        // The strip moves through the last line instead of resting at its start.
        assert!(knot_value(&knots, 233300.0) > 700.0 && knot_value(&knots, 233300.0) < 830.0);
        // A word-timed last line uses its own end, never the estimate.
        lines[3].end_ms = Some(240000.0);
        assert_eq!(
            word_strip_knots(&lines, Some(arrival)).last(),
            Some(&(240000.0, 830.0))
        );
        // An arrival not after the last start adds nothing.
        lines[3].end_ms = None;
        assert_eq!(
            word_strip_knots(&lines, Some(231300.0)).last(),
            Some(&(231300.0, 690.0))
        );
    }

    /// A song without word times: when no line's cap ends before the next line starts, the knot glide with the
    /// terminal knot is the line glide (`strip_cubic`) at every time, for several layouts (the windows now take the knot
    /// path for such a song too, so its motion only changes where a cap applies).
    #[test]
    fn without_word_times_the_knot_glide_is_the_line_glide() {
        let mut seed = 5u64;
        let mut rand = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as f64 / (1u64 << 31) as f64
        };
        for case in 0..12 {
            let n = 1 + case * 3;
            let mains: Vec<f64> = (0..n).map(|_| 20.0 + rand() * 300.0).collect();
            let trans: Vec<f64> = (0..n).map(|_| rand() * 350.0).collect();
            let mut layout = crate::layout_strip(&mains, &trans, 24.0);
            layout.anchors = mains.iter().map(|w| rand() * w.min(20.0)).collect();
            let lasts: Vec<f64> = mains.iter().map(|w| w - rand() * 10.0).collect();
            let mut starts = Vec::new();
            let mut t = 300.0 + rand() * 1000.0;
            for i in 0..n {
                starts.push(t);
                t += if i % 5 == 4 {
                    15000.0
                } else {
                    20.0 + rand() * 4000.0
                };
            }
            let arrival = starts[n - 1] + 200.0 + rand() * 4000.0;
            // Every cap reaches the next line's start (half the cases exactly), so no cap applies: the motion must be
            // the line glide's, as before the cap existed.
            let lines: Vec<KnotLine> = (0..n)
                .map(|i| {
                    let room = starts.get(i + 1).map_or(1000.0, |next| next - starts[i]);
                    KnotLine {
                        cap_ms: (case % 3 != 0).then(|| room + (case % 2) as f64 * rand() * 1000.0),
                        ..line(
                            starts[i],
                            layout.starts[i],
                            layout.anchors[i],
                            None,
                            lasts[i],
                        )
                    }
                })
                .collect();
            let knots = word_strip_knots(&lines, Some(arrival));
            let view = 500.0;
            let fin = crate::FinalLine {
                end_ms: Some(arrival),
                last_glyph_center: Some(lasts[n - 1]),
            };
            let mut at = starts[0] - 500.0;
            while at < arrival + 500.0 {
                let line_mode = crate::strip_cubic(
                    &starts,
                    &layout,
                    crate::Align::Left,
                    at,
                    view,
                    crate::Align::Center,
                    Some(fin),
                );
                let [v, s, c2, c3] = knot_cubic(&knots, at);
                let unified = [view / 2.0 - v, -s, -c2, -c3];
                for k in 0..4 {
                    let tol = 1e-9 * (1.0 + line_mode[k].abs());
                    assert!(
                        (unified[k] - line_mode[k]).abs() <= tol,
                        "case {case} at {at} coefficient {k}: {} vs {}",
                        unified[k],
                        line_mode[k]
                    );
                }
                at += 1000.0 / 120.0;
            }
        }
    }

    /// A long pause after a line keeps its end knot; a gap exactly as fast as the line keeps it too; the last line
    /// keeps its end.
    #[test]
    fn an_end_knot_stays_for_a_slow_or_equally_fast_gap() {
        let knots = word_strip_knots(
            &[
                line(1000.0, 0.0, 10.0, Some(2000.0), 110.0),
                line(30000.0, 140.0, 10.0, Some(31000.0), 110.0),
                line(32000.0, 340.0, 10.0, Some(33000.0), 60.0),
            ],
            None,
        );
        assert_eq!(
            knots,
            vec![
                (1000.0, 10.0),
                (2000.0, 110.0),
                (30000.0, 150.0),
                (31000.0, 250.0),
                (32000.0, 350.0),
                (33000.0, 400.0)
            ]
        );
    }

    /// The curve hits every knot, never goes backwards, and its speed is continuous (also across line boundaries and
    /// the gaps between a line's end and the next start), sampled at 120 Hz.
    #[test]
    fn the_word_glide_hits_every_knot_and_is_monotone_and_c1() {
        let mut seed = 11u64;
        let mut rand = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as f64 / (1u64 << 31) as f64
        };
        let mut lines = Vec::new();
        let (mut t, mut cell) = (500.0, 0.0);
        for i in 0..30 {
            let width = 40.0 + rand() * 300.0;
            let dur = 30.0 + rand() * 4000.0;
            let words = i % 6 != 4;
            lines.push(line(
                t,
                cell,
                5.0 + rand() * 5.0,
                words.then_some(t + dur),
                width - 5.0,
            ));
            cell += width + 24.0;
            t += if i % 7 == 6 {
                20000.0
            } else {
                dur * rand() + 50.0 + rand() * 900.0
            };
        }
        let knots = word_strip_knots(&lines, None);
        assert!(knots.len() > lines.len());
        for w in knots.windows(2) {
            assert!(w[1].0 > w[0].0 && w[1].1 >= w[0].1);
        }
        for &(t, v) in &knots {
            assert!((knot_value(&knots, t) - v).abs() < 1e-9, "knot {t}");
        }
        let frame = 1000.0 / 120.0;
        let (t0, t1) = (knots[0].0 - 100.0, knots[knots.len() - 1].0 + 100.0);
        let (mut at, mut prev) = (t0, knot_cubic(&knots, t0));
        while at < t1 {
            at += frame;
            let now = knot_cubic(&knots, at);
            assert!(now[0] >= prev[0] - 1e-9 && now[1] >= -1e-12, "back at {at}");
            prev = now;
        }
        // Exactly at each knot the speed from both sides agrees (C1), also from the end of a line into the next gap.
        for &(t, _) in &knots {
            let before = knot_cubic(&knots, t - 1e-6)[1];
            let after = knot_cubic(&knots, t + 1e-6)[1];
            assert!(
                (before - after).abs() < 1e-4,
                "C1 at {t}: {before} vs {after}"
            );
        }
        assert_eq!(next_knot_break(&knots, 0.0), Some(knots[0].0));
        assert_eq!(next_knot_break(&knots, knots[knots.len() - 1].0), None);
    }

    #[test]
    fn shared_times_and_backward_knots_are_dropped_or_raised() {
        let p = smooth_glide(
            &[(0.0, 0.0), (0.0, 50.0), (100.0, 40.0), (200.0, 10.0)],
            &[vec![], vec![(50.0, 90.0)], vec![]],
            true,
        );
        assert_eq!(glide_value(&p, 0.0), 0.0);
        assert_eq!(glide_value(&p, 100.0), 40.0);
        assert_eq!(glide_value(&p, 300.0), 40.0);
    }
}
