//! The smoothed glide of word-timed lyrics (ADR 0041 D10), port of `src/lib/lyrics-glide.js` (the reference) and
//! `WordLyricsRules.kt`, checked by the shared vectors. One pure curve drives the continuous strip's word glide and the
//! karaoke colour edge.
//!
//! Hard knots (line starts, and the line end of a karaoke line) are hit exactly. Between two hard knots the curve
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

/// One line of the strip for the word glide: its start, its cell's left edge (px along the strip), its first glyph's
/// centre (px from the cell's left edge) and, for a word-timed line, each word's start and centre (px from the cell).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KnotLine {
    pub start_ms: f64,
    pub cell_start: f64,
    pub anchor: f64,
    pub word_starts: Vec<f64>,
    pub word_centres: Vec<f64>,
}

/// The continuous strip's word glide (`wordStripGlide` in JS): the strip px that sits on the lyric area's centre. One
/// hard knot per line (its start, its first glyph centre); a word-timed line's later word starts (their centres) guide
/// it; the last line with words ends on its last word. Rests before the first and after the last knot.
pub fn word_strip_glide(lines: &[KnotLine]) -> Vec<GlidePiece> {
    let mut hard = Vec::with_capacity(lines.len() + 1);
    let mut guides = Vec::with_capacity(lines.len() + 1);
    for (index, line) in lines.iter().enumerate() {
        let n = line.word_starts.len().min(line.word_centres.len());
        let mut mine: Vec<(f64, f64)> = (0..n)
            .filter(|&i| line.word_starts[i] > line.start_ms)
            .map(|i| {
                (
                    line.word_starts[i],
                    line.cell_start + line.word_centres[i].max(0.0),
                )
            })
            .collect();
        hard.push((line.start_ms, line.cell_start + line.anchor.max(0.0)));
        if index == lines.len() - 1 {
            if let Some(last) = mine.pop() {
                guides.push(mine);
                hard.push(last);
                continue;
            }
        }
        guides.push(mine);
    }
    smooth_glide(&hard, &guides, true)
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

    #[test]
    fn dense_words_and_long_gaps_keep_the_band_and_the_knots() {
        let mut seed = 7u64;
        let mut rand = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as f64 / (1u64 << 31) as f64
        };
        let mut lines = Vec::new();
        let (mut t, mut cell) = (500.0, 0.0);
        for i in 0..24 {
            let n = if i % 5 == 3 { 0 } else { 2 + i % 9 };
            let start = t;
            let (mut starts, mut centres, mut x) = (Vec::new(), Vec::new(), 0.0);
            for _ in 0..n {
                let w = 8.0 + rand() * 60.0;
                starts.push(t);
                centres.push(x + w / 2.0);
                x += w;
                t += if rand() < 0.15 {
                    0.0
                } else {
                    10.0 + rand() * 300.0
                };
            }
            let width = x.max(40.0);
            lines.push(KnotLine {
                start_ms: start,
                cell_start: cell,
                anchor: 9.0,
                word_starts: starts,
                word_centres: centres,
            });
            cell += width + 24.0;
            // Sometimes a long instrumental gap.
            t += if i % 7 == 6 {
                20000.0
            } else {
                100.0 + rand() * 900.0
            };
        }
        let p = word_strip_glide(&lines);
        let mut hard: Vec<(f64, f64)> = lines
            .iter()
            .map(|l| (l.start_ms, l.cell_start + l.anchor))
            .collect();
        let last = lines.last().unwrap();
        if let Some(i) = (0..last.word_starts.len())
            .rev()
            .find(|&i| last.word_starts[i] > last.start_ms)
        {
            hard.push((last.word_starts[i], last.cell_start + last.word_centres[i]));
        }
        check(&p, &hard, true);
    }

    #[test]
    fn a_line_level_strip_glide_hits_every_line_start() {
        let lines: Vec<KnotLine> = [
            (1000.0, 0.0),
            (3000.0, 120.0),
            (3300.0, 200.0),
            (9000.0, 260.0),
        ]
        .iter()
        .map(|&(t, c)| KnotLine {
            start_ms: t,
            cell_start: c,
            anchor: 10.0,
            ..Default::default()
        })
        .collect();
        let p = word_strip_glide(&lines);
        for l in &lines {
            assert!((glide_value(&p, l.start_ms) - (l.cell_start + 10.0)).abs() < 1e-9);
        }
        assert_eq!(next_glide_break(&p, 0.0), Some(1000.0));
        assert_eq!(next_glide_break(&p, 9000.0), None);
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
