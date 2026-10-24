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

/// Playback samples within this of the running clock are ignored (about 1 device px of glide).
pub const CLOCK_TOLERANCE_MS: f64 = 4.0;
/// Larger differences are a seek and jump; smaller ones are absorbed by a rate change (ADR 0028 / 0034).
pub const CLOCK_JUMP_MS: f64 = 250.0;
/// Wall time over which a small drift is absorbed.
pub const CLOCK_SLEW_MS: f64 = 750.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Correction {
    Keep,
    Slew(Motion),
    Jump,
}

/// The renderer's playing clock from its last base: position `base_ms`, running at `rate` lyrics ms per wall ms for
/// the first `slew_ms` wall ms (a drift correction), then at 1. Continuous in position by construction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub base_ms: f64,
    pub rate: f64,
    pub slew_ms: f64,
}

impl Motion {
    pub fn at(base_ms: f64) -> Self {
        Motion {
            base_ms,
            rate: 1.0,
            slew_ms: 0.0,
        }
    }

    /// Position `dt_ms` wall ms after the base (playing).
    pub fn position(&self, dt_ms: f64) -> f64 {
        let dt = dt_ms.max(0.0);
        self.base_ms
            + if dt <= self.slew_ms {
                self.rate * dt
            } else {
                self.rate * self.slew_ms + (dt - self.slew_ms)
            }
    }

    /// The correction still running `dt_ms` after the base: (rate, lyrics ms until it ends).
    pub fn slew(&self, dt_ms: f64) -> Option<(f64, f64)> {
        let dt = dt_ms.max(0.0);
        (dt < self.slew_ms).then_some((self.rate, self.rate * (self.slew_ms - dt)))
    }

    /// A sample `sample_ms` arrives `dt_ms` after the base while playing on: `Ok(None)` keeps the motion, `Ok(Some)`
    /// continues from the current position with a rate that absorbs the drift over `CLOCK_SLEW_MS`, `Jump` is a
    /// seek (jump to the sample).
    pub fn correct(&self, dt_ms: f64, sample_ms: f64) -> Correction {
        let predicted = self.position(dt_ms);
        let drift = sample_ms - predicted;
        if drift.abs() < CLOCK_TOLERANCE_MS {
            Correction::Keep
        } else if drift.abs() < CLOCK_JUMP_MS {
            Correction::Slew(Motion {
                base_ms: predicted,
                rate: 1.0 + drift / CLOCK_SLEW_MS,
                slew_ms: CLOCK_SLEW_MS,
            })
        } else {
            Correction::Jump
        }
    }
}

/// Audio-clock sync (ADR 0034): the mean error of the audio engine's position readings against the renderer's
/// clock over this much wall time decides a correction. The readings step with the backend's buffer (a few ms to
/// tens of ms); the mean over a window cancels that, so the correction is not driven by the steps.
pub const AUDIO_SYNC_WINDOW_MS: f64 = 1000.0;
/// A mean error under this is left alone; larger ones up to `CLOCK_JUMP_MS` are absorbed over `AUDIO_SLEW_MS`.
pub const AUDIO_SYNC_TOLERANCE_MS: f64 = 3.0;
/// Wall time over which an audio-clock error is absorbed: a 10 ms error changes the speed by 0.25 %.
pub const AUDIO_SLEW_MS: f64 = 4000.0;

/// The running mean of audio-clock errors over `AUDIO_SYNC_WINDOW_MS`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioSync {
    started_ms: f64,
    sum: f64,
    count: usize,
}

impl AudioSync {
    pub fn reset(&mut self) {
        *self = AudioSync::default();
    }

    /// One reading `error_ms` (audio position minus the renderer's clock, both at the reading's instant) at wall
    /// time `wall_ms`; returns the window's mean error when the window is complete.
    pub fn observe(&mut self, wall_ms: f64, error_ms: f64) -> Option<f64> {
        if self.count == 0 {
            self.started_ms = wall_ms;
        }
        self.sum += error_ms;
        self.count += 1;
        if wall_ms - self.started_ms >= AUDIO_SYNC_WINDOW_MS && self.count >= 4 {
            let mean = self.sum / self.count as f64;
            self.reset();
            return Some(mean);
        }
        None
    }
}

impl Motion {
    /// The audio-clock correction for a window's mean error `error_ms` at `dt_ms` after the base: keep, a slow
    /// slew (`AUDIO_SLEW_MS`, so the speed changes by a fraction of a percent), or a jump (a seek or a stall).
    pub fn audio_correct(&self, dt_ms: f64, error_ms: f64) -> Correction {
        if error_ms.abs() < AUDIO_SYNC_TOLERANCE_MS {
            Correction::Keep
        } else if error_ms.abs() < CLOCK_JUMP_MS {
            Correction::Slew(Motion {
                base_ms: self.position(dt_ms),
                rate: 1.0 + error_ms / AUDIO_SLEW_MS,
                slew_ms: AUDIO_SLEW_MS,
            })
        } else {
            Correction::Jump
        }
    }
}

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
    /// Lyrics ms per wall ms while a small drift is being absorbed (1 otherwise, always > 0).
    pub rate: f64,
    /// Lyrics time at which `rate` returns to 1 (`None` = no correction running).
    pub slew_until_ms: Option<f64>,
    /// Word-timed lyrics (ADR 0041 option C): the glide's knots `(lyrics ms, strip px)` are the word starts
    /// (`rules::word_glide_knots`), the target is the area's centre, and every cell stays at factor 1 (the static
    /// centre mask dims instead of the line fades). `None`: the line glide.
    pub word_knots: Option<&'a [(f64, f64)]>,
}

impl StripInput<'_> {
    /// The correction in effect: its rate and the lyrics time it ends (a knot of the plan).
    fn slew(&self) -> Option<(f64, f64)> {
        self.slew_until_ms
            .filter(|&u| u > self.at_ms && self.playing && self.rate > 0.0)
            .map(|u| (self.rate, u))
    }

    /// Wall seconds from now until lyrics time `t` (the clock runs at `rate` until the slew ends, then at 1).
    pub fn wall_secs(&self, t: f64) -> f64 {
        let p = self.at_ms;
        match self.slew() {
            Some((r, u)) if t > u => (u - p) / r / 1000.0 + (t - u) / 1000.0,
            Some((r, _)) => (t - p) / r / 1000.0,
            None => (t - p) / 1000.0,
        }
    }

    /// Lyrics ms per wall ms at lyrics time `t` (just after it).
    fn rate_at(&self, t: f64) -> f64 {
        match self.slew() {
            Some((r, u)) if t < u => r,
            _ => 1.0,
        }
    }
}

fn offset_at(input: &StripInput, t: f64) -> f64 {
    if let Some(knots) = input.word_knots {
        return rules::glide_cubic(knots, t, input.view_width / 2.0)[0];
    }
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
    if let Some(knots) = input.word_knots {
        return rules::glide_cubic(knots, t, input.view_width / 2.0);
    }
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
    if input.word_knots.is_some() {
        return 1.0;
    }
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
            let next = match input.word_knots {
                Some(knots) => rules::next_knot_after(knots, t),
                None => rules::next_strip_break(input.starts, t, arrival),
            };
            match next {
                Some(next) if next > t => {
                    breaks.push(next);
                    t = next;
                }
                _ => break,
            }
        }
    }
    let secs = |t: f64| input.wall_secs(t);
    // The end of a drift correction changes the time mapping, so it is a knot too (no re-plan needed there).
    let mut knots = breaks.clone();
    if let Some((_, u)) = input.slew() {
        if u < breaks.last().copied().unwrap_or(f64::INFINITY) {
            knots.push(u);
            knots.sort_by(f64::total_cmp);
        }
    }
    // Every break is a knot of the glide curve, so each stretch between two of them is one cubic piece; the
    // coefficients come from the rule in ms and are rescaled to seconds. A re-plan samples the same global curve
    // (its tangents only read neighbouring knots), so the hand-over between plans has no kink.
    let mut offset: Cubics = Vec::new();
    let mut from = p;
    for &b in &knots {
        let c = cubic_at(input, from);
        // u (lyrics ms) = r * 1000 * s (wall seconds) within one piece.
        let k = input.rate_at(from) * 1e3;
        offset.push((secs(from), [c[0], c[1] * k, c[2] * k * k, c[3] * k * k * k]));
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
    if let Some((_, u)) = input.slew() {
        if u <= horizon {
            times.push(u);
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
        wake_ms: breaks.first().map(|&b| secs(b) * 1000.0),
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
            rate: 1.0,
            slew_until_ms: None,
            word_knots: None,
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
    fn word_timed_plans_centre_each_word_and_keep_every_cell_whole() {
        let starts = [1000.0, 3000.0];
        let layout = rules::layout_strip(&[100.0, 80.0], &[60.0, 90.0], 20.0);
        let lines = [
            rules::KnotLine {
                start_ms: 1000.0,
                cell_start: layout.starts[0],
                word_starts: vec![1000.0, 1400.0, 1900.0],
                word_centres: vec![10.0, 45.0, 85.0],
            },
            rules::KnotLine {
                start_ms: 3000.0,
                cell_start: layout.starts[1],
                word_starts: vec![],
                word_centres: vec![],
            },
        ];
        let knots = rules::word_glide_knots(&lines);
        for at in [0.0, 1000.0, 1200.0, 2500.0] {
            let mut i = input(&starts, &layout, at, true);
            i.word_knots = Some(&knots);
            let plan = plan_strip(&i);
            // The translation row has no knots: it is a cell of the same strip, so one offset moves both rows and
            // no cell fades (the centre mask dims instead).
            for (_, keys) in &plan.opacity {
                assert!(keys.iter().all(|k| (k.1 - 1.0).abs() < 1e-12));
            }
            let mut t = 0.0;
            let mut last = f64::INFINITY;
            while t <= plan.offset.last().unwrap().0 {
                let v = cubic_value_at(&plan.offset, t);
                assert!((v - offset_at(&i, at + t * 1000.0)).abs() < 1e-6);
                // Per frame (240 Hz) the strip only moves left and never jumps.
                assert!(v <= last + 1e-9 && (last - v).abs() < 5.0 || last.is_infinite());
                last = v;
                t += 1.0 / 240.0;
            }
            assert_eq!(
                plan.wake_ms,
                rules::next_knot_after(&knots, at).map(|b| b - at)
            );
        }
        // Each word start is centred.
        for &(t, v) in &knots {
            let i = StripInput {
                word_knots: Some(&knots),
                ..input(&starts, &layout, t, true)
            };
            assert!((offset_at(&i, t) - (200.0 - v)).abs() < 1e-9);
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

    /// Replays a jittery, drifting sample feed through the renderer's clock and re-plans (every correction and
    /// every line start), evaluating what DWM shows each ms: the strip never moves back and never steps.
    #[test]
    fn a_jittery_drifting_sample_feed_never_steps_or_reverses_the_strip() {
        let starts: Vec<f64> = (0..40)
            .map(|i| 1000.0 + i as f64 * 1700.0 + (i % 3) as f64 * 400.0)
            .collect();
        let widths: Vec<f64> = (0..40).map(|i| 150.0 + (i * 37 % 120) as f64).collect();
        let layout = rules::layout_strip(&widths, &[], 20.0);
        // Pseudo random latency 0..25 ms, a 0.3 % fast player and one 120 ms hiccup.
        let mut seed = 7u64;
        let mut rand = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as f64 / (1u64 << 31) as f64
        };
        let truth = |w: f64| w * 1.003 + if w > 30_000.0 { 120.0 } else { 0.0 };
        let mut motion = Motion::at(0.0);
        let mut base_wall = 0.0;
        let plan_at = |motion: &Motion, base_wall: f64, w: f64| {
            let dt = w - base_wall;
            let at = motion.position(dt);
            let slew = motion.slew(dt);
            let i = StripInput {
                starts: &starts,
                layout: &layout,
                view_width: 400.0,
                final_line: None,
                at_ms: at,
                playing: true,
                inactive_percent: 50,
                rate: slew.map_or(1.0, |s| s.0),
                slew_until_ms: slew.map(|s| at + s.1),
                word_knots: None,
            };
            plan_strip(&i)
        };
        let mut plan = plan_at(&motion, base_wall, 0.0);
        let mut plan_wall = 0.0;
        let mut wake = plan.wake_ms;
        let mut next_sample = 500.0 + rand() * 25.0;
        let mut last: Option<f64> = None;
        let mut w = 0.0;
        while w < 60_000.0 {
            let replan = if w >= next_sample {
                let sample = truth(w - rand() * 25.0);
                match motion.correct(w - base_wall, sample) {
                    Correction::Keep => false,
                    Correction::Slew(m) => {
                        motion = m;
                        base_wall = w;
                        true
                    }
                    Correction::Jump => panic!("a sample at {w} jumped"),
                }
            } else {
                false
            };
            if w >= next_sample {
                next_sample += 500.0;
            }
            if replan || wake.is_some_and(|ms| w - plan_wall >= ms) {
                plan = plan_at(&motion, base_wall, w);
                plan_wall = w;
                wake = plan.wake_ms;
            }
            let x = cubic_value_at(&plan.offset, (w - plan_wall) / 1000.0);
            if let Some(prev) = last {
                assert!(x <= prev + 1e-6, "backwards at {w}: {prev} -> {x}");
                assert!(prev - x < 1.0, "step at {w}: {prev} -> {x}");
            }
            last = Some(x);
            w += 1.0;
        }
    }

    /// Readings that step like a buffered backend (10 ms chunks, read every 40 ms at arbitrary phases) against an
    /// exact clock: the window mean stays within the tolerance, so no correction is made; a real 20 ms offset is
    /// found and absorbed by a speed change under 1 %.
    #[test]
    fn audio_sync_ignores_buffer_steps_and_absorbs_real_offsets() {
        let stepped = |t: f64| (t / 10.0).floor() * 10.0 + 5.0;
        let motion = Motion::at(0.0);
        let mut sync = AudioSync::default();
        let mut decisions = Vec::new();
        let mut t = 0.0;
        while t < 10_000.0 {
            let at = t + (t * 0.37) % 7.0;
            if let Some(e) = sync.observe(at, stepped(at) - motion.position(at)) {
                decisions.push(motion.audio_correct(at, e));
            }
            t += 40.0;
        }
        assert!(decisions.len() >= 8);
        assert!(
            decisions.iter().all(|d| *d == Correction::Keep),
            "{decisions:?}"
        );
        let mut sync = AudioSync::default();
        let mut t = 0.0;
        let found = loop {
            let at = t + (t * 0.37) % 7.0;
            if let Some(e) = sync.observe(at, stepped(at) + 20.0 - motion.position(at)) {
                break e;
            }
            t += 40.0;
        };
        assert!((found - 20.0).abs() < 3.0, "{found}");
        match motion.audio_correct(t, found) {
            Correction::Slew(m) => assert!((m.rate - 1.0).abs() < 0.01),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_slew_plan_follows_the_corrected_clock() {
        let starts = [1000.0, 3000.0, 5000.0, 9000.0];
        let layout = rules::layout_strip(&[100.0, 50.0, 80.0, 120.0], &[], 20.0);
        let motion = Motion {
            base_ms: 2000.0,
            rate: 1.1,
            slew_ms: 750.0,
        };
        let mut i = input(&starts, &layout, 2000.0, true);
        i.rate = 1.1;
        i.slew_until_ms = Some(2000.0 + 1.1 * 750.0);
        let plan = plan_strip(&i);
        let mut t = 0.0;
        while t < 3.0 {
            let want = offset_at(&i, motion.position(t * 1000.0));
            assert!(
                (cubic_value_at(&plan.offset, t) - want).abs() < 1e-6,
                "at {t}"
            );
            t += 0.005;
        }
        assert!((plan.wake_ms.unwrap() - (750.0 + (3000.0 - 2825.0))).abs() < 1e-6);
    }
}
