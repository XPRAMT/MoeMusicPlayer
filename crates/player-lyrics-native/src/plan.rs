//! Platform-neutral planning (ADR 0034): what one lyrics window shows and the piecewise animations DWM runs
//! from "now" until the next re-plan. Re-planned only at a line change (the next glide break), a seek, pause/resume,
//! a playback correction, new lyrics, a setting or a size/DPI change; never per frame.

use player_lyrics_rules as rules;

/// Piecewise-linear keyframes `(seconds from the commit, value)`: linear between keys, constant after the last one.
pub type Keys = Vec<(f64, f64)>;

/// Piecewise-cubic keys `(seconds from the commit, [c0, c1, c2, c3])`: from its time up to the next key the value is
/// `c0 + c1 s + c2 s^2 + c3 s^3` (`s` = seconds since the key's time), the `IDCompositionAnimation::AddCubic` form;
/// the last key is the end value (only `c0`), constant after it.
pub type Cubics = Vec<(f64, [f64; 4])>;

/// How many glide breaks (line starts) one plan covers, so a late wake-up never stalls the strip.
pub const HORIZON_BREAKS: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct StripPlan {
    /// The glide: one cubic segment per stretch between line starts (`rules::strip_cubic`).
    pub offset: Cubics,
    /// Cells to draw (inclusive), `None` when nothing is in view.
    pub cells: Option<(usize, usize)>,
    /// Opacity factor keys per drawn cell (relative to the window opacity).
    pub opacity: Vec<(usize, Keys)>,
    /// Re-plan this many ms from now (the next line start), `None` when nothing changes any more.
    pub wake_ms: Option<f64>,
}

pub struct StripInput<'a> {
    pub starts: &'a [f64],
    pub layout: &'a rules::StripLayout,
    pub view_width: f64,
    pub final_line: Option<rules::FinalLine>,
    /// Lyrics time now (playback position plus the display offset).
    pub at_ms: f64,
    pub playing: bool,
    pub inactive_percent: u32,
}

fn offset_at(input: &StripInput, t: f64) -> f64 {
    rules::strip_offset(
        input.starts,
        input.layout,
        rules::STRIP_ALIGN,
        t,
        input.view_width,
        rules::STRIP_TARGET,
        input.final_line,
    )
}

fn cubic_at(input: &StripInput, t: f64) -> [f64; 4] {
    rules::strip_cubic(
        input.starts,
        input.layout,
        rules::STRIP_ALIGN,
        t,
        input.view_width,
        rules::STRIP_TARGET,
        input.final_line,
    )
}

fn factor_at(input: &StripInput, index: usize, t: f64) -> f64 {
    let focus = rules::strip_focus(input.starts, t, rules::FADE_MS);
    rules::line_opacity_factor(
        rules::line_weight(&focus, index),
        input.inactive_percent,
        rules::Animation::Continuous,
    )
}

/// The continuous strip: glide keys up to `HORIZON_BREAKS` breaks ahead, the cells that can show meanwhile (plus one
/// view width on each side, `lyricsStripWindow`) and their fade keys.
pub fn plan_strip(input: &StripInput) -> StripPlan {
    let p = input.at_ms;
    let arrival = input.final_line.and_then(|f| f.end_ms);
    let mut breaks = Vec::new();
    if input.playing {
        let mut t = p;
        while breaks.len() < HORIZON_BREAKS {
            match rules::next_strip_break(input.starts, t, arrival) {
                Some(next) if next > t => {
                    breaks.push(next);
                    t = next;
                }
                _ => break,
            }
        }
    }
    let secs = |t: f64| (t - p) / 1000.0;
    // Every break is a knot of the glide curve, so each stretch between two of them is one cubic piece; the
    // coefficients come from the rule in ms and are rescaled to seconds. A re-plan samples the same global curve
    // (its tangents only read neighbouring knots), so the hand-over between plans has no kink.
    let mut offset: Cubics = Vec::new();
    let mut from = p;
    for &b in &breaks {
        let c = cubic_at(input, from);
        offset.push((secs(from), [c[0], c[1] * 1e3, c[2] * 1e6, c[3] * 1e9]));
        from = b;
    }
    offset.push((secs(from), [offset_at(input, from), 0.0, 0.0, 0.0]));
    let horizon = breaks.last().copied().unwrap_or(p);
    // Monotone between knots, so the extremes are at the keys.
    let (lo, hi) = offset
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &(_, c)| {
            (lo.min(c[0]), hi.max(c[0]))
        });
    let cells = rules::strip_window(input.layout, hi, lo, input.view_width, input.view_width);

    // The weights are linear between these times: each line start and the start of the fade into it.
    let mut times = vec![p];
    for (j, &start) in input.starts.iter().enumerate() {
        let length = if j == 0 {
            start.max(1.0)
        } else {
            start - input.starts[j - 1]
        };
        let fade = rules::FADE_MS.min(length).max(1.0);
        for t in [start - fade, start] {
            if t > p && t <= horizon {
                times.push(t);
            }
        }
    }
    times.sort_by(f64::total_cmp);
    times.dedup();
    let opacity = match cells {
        Some((first, last)) => (first..=last)
            .map(|i| {
                let keys: Keys = times
                    .iter()
                    .map(|&t| (secs(t), factor_at(input, i, t)))
                    .collect();
                (i, simplify(keys))
            })
            .collect(),
        None => Vec::new(),
    };
    StripPlan {
        offset,
        cells,
        opacity,
        wake_ms: breaks.first().map(|b| b - p),
    }
}

/// Drop keys that lie on the line through their neighbours (constant runs collapse to their ends).
pub fn simplify(keys: Keys) -> Keys {
    if keys.len() < 3 {
        return keys;
    }
    let mut out: Keys = vec![keys[0]];
    for i in 1..keys.len() - 1 {
        let (a, b, c) = (*out.last().unwrap(), keys[i], keys[i + 1]);
        let interpolated = a.1 + (c.1 - a.1) * (b.0 - a.0) / (c.0 - a.0);
        if (interpolated - b.1).abs() > 1e-9 {
            out.push(b);
        }
    }
    out.push(*keys.last().unwrap());
    out
}

/// "None" mode: the line shown and the scroll of a row `overflow_px` wider than the area (0..1 of it reached at 90 %
/// of the line's time).
#[derive(Clone, Debug, PartialEq)]
pub struct LinePlan {
    pub index: Option<usize>,
    /// Scroll progress keys (0..1), multiply by the row's overflow.
    pub scroll: Keys,
    pub wake_ms: Option<f64>,
}

pub fn plan_line(
    starts: &[f64],
    position_ms: f64,
    offset_ms: f64,
    duration_ms: Option<f64>,
    shown: bool,
    playing: bool,
) -> LinePlan {
    let at = position_ms + offset_ms;
    let next_start = starts.iter().copied().find(|&s| s > at);
    let wake_ms = if playing {
        next_start.map(|s| s - at)
    } else {
        None
    };
    let index = rules::active_index(starts, position_ms, offset_ms);
    if index < 0 || !shown {
        return LinePlan {
            index: None,
            scroll: vec![(0.0, 0.0)],
            wake_ms,
        };
    }
    let i = index as usize;
    let line_ms = rules::line_shown_ms(starts, i, duration_ms, offset_ms);
    let from = rules::scroll_progress(at - starts[i], line_ms);
    let end = starts[i] + line_ms * 0.9;
    let scroll = if playing && from < 1.0 && end > at {
        vec![(0.0, from), ((end - at) / 1000.0, 1.0)]
    } else {
        vec![(0.0, from)]
    };
    LinePlan {
        index: Some(i),
        scroll,
        wake_ms,
    }
}

/// Value of piecewise-cubic keys at `t` seconds (what DWM evaluates).
pub fn cubic_value_at(keys: &Cubics, t: f64) -> f64 {
    let Some(&(t0, c)) = keys.first() else {
        return 0.0;
    };
    if t <= t0 {
        return c[0];
    }
    let index = keys.partition_point(|&(k, _)| k <= t) - 1;
    let (k, c) = keys[index];
    if index + 1 == keys.len() {
        return c[0];
    }
    let s = t - k;
    c[0] + s * (c[1] + s * (c[2] + s * c[3]))
}

/// Value of piecewise-linear keys at `t` seconds (for tests and for paused states).
pub fn value_at(keys: &Keys, t: f64) -> f64 {
    let Some(&(t0, v0)) = keys.first() else {
        return 0.0;
    };
    if t <= t0 {
        return v0;
    }
    for pair in keys.windows(2) {
        let ((a, va), (b, vb)) = (pair[0], pair[1]);
        if t <= b {
            return if b > a {
                va + (vb - va) * (t - a) / (b - a)
            } else {
                vb
            };
        }
    }
    keys.last().unwrap().1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(
        starts: &'a [f64],
        layout: &'a rules::StripLayout,
        at: f64,
        playing: bool,
    ) -> StripInput<'a> {
        StripInput {
            starts,
            layout,
            view_width: 400.0,
            final_line: Some(rules::FinalLine {
                end_ms: Some(11000.0),
                last_glyph_center: Some(60.0),
            }),
            at_ms: at,
            playing,
            inactive_percent: 50,
        }
    }

    #[test]
    fn the_planned_keys_reproduce_the_rules_at_every_moment() {
        let starts = [1000.0, 3000.0, 5000.0, 9000.0];
        let layout =
            rules::layout_strip(&[100.0, 50.0, 80.0, 120.0], &[60.0, 90.0, 0.0, 40.0], 20.0);
        for at in [0.0, 850.0, 1000.0, 2900.0, 4000.0, 8900.0] {
            let i = input(&starts, &layout, at, true);
            let plan = plan_strip(&i);
            let horizon = plan.offset.last().unwrap().0;
            let mut t = 0.0;
            while t <= horizon {
                let expected = offset_at(&i, at + t * 1000.0);
                assert!(
                    (cubic_value_at(&plan.offset, t) - expected).abs() < 1e-6,
                    "offset at {at}+{t}"
                );
                for (cell, keys) in &plan.opacity {
                    let want = factor_at(&i, *cell, at + t * 1000.0);
                    assert!(
                        (value_at(keys, t) - want).abs() < 1e-6,
                        "cell {cell} at {at}+{t}"
                    );
                }
                t += 0.01;
            }
            assert_eq!(
                plan.wake_ms,
                rules::next_strip_break(&starts, at, Some(11000.0)).map(|b| b - at)
            );
        }
    }

    #[test]
    fn the_cubic_keys_match_the_curve_every_10_ms_for_uneven_lines() {
        // A 150 ms line and a long gap: every stretch is one AddCubic segment and re-plans sample the same curve.
        let starts = [1000.0, 1150.0, 4000.0, 12000.0, 12500.0];
        let layout = rules::layout_strip(&[100.0, 30.0, 200.0, 60.0, 90.0], &[], 20.0);
        for at in [0.0, 1075.0, 1150.0, 3000.0, 11990.0, 12600.0] {
            let i = input(&starts, &layout, at, true);
            let plan = plan_strip(&i);
            assert!(plan.offset.len() <= HORIZON_BREAKS + 1);
            let horizon = plan.offset.last().unwrap().0;
            let mut t = 0.0;
            while t <= horizon + 1e-9 {
                let expected = offset_at(&i, at + t * 1000.0);
                let got = cubic_value_at(&plan.offset, t);
                assert!(
                    (got - expected).abs() < 1e-6,
                    "at {at}+{t}: {got} vs {expected}"
                );
                t += 0.01;
            }
        }
    }

    #[test]
    fn paused_plans_are_still() {
        let starts = [1000.0, 3000.0];
        let layout = rules::layout_strip(&[100.0, 50.0], &[], 20.0);
        let plan = plan_strip(&input(&starts, &layout, 2900.0, false));
        assert_eq!(plan.offset.len(), 1);
        assert_eq!(plan.wake_ms, None);
        assert!(plan.opacity.iter().all(|(_, keys)| keys.len() == 1));
    }

    #[test]
    fn a_long_line_scrolls_to_its_end_at_ninety_percent() {
        let plan = plan_line(&[1000.0, 5000.0], 1000.0, 0.0, None, true, true);
        assert_eq!(plan.index, Some(0));
        assert_eq!(plan.scroll, vec![(0.0, 0.0), (3.6, 1.0)]);
        assert_eq!(plan.wake_ms, Some(4000.0));
        let paused = plan_line(&[1000.0, 5000.0], 2800.0, 0.0, None, true, false);
        assert_eq!(paused.scroll, vec![(0.0, 0.5)]);
        assert_eq!(
            plan_line(&[1000.0], 500.0, 0.0, None, true, true).index,
            None
        );
    }
}
