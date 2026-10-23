//! Platform-neutral conversion of the plans (`plan::Keys`, `plan::Cubics`) into Windows.UI.Composition key-frame
//! animations (ADR 0034): a duration and key frames at normalised progress, each eased from the previous one.
//!
//! A cubic piece `c0 + c1 s + c2 s^2 + c3 s^3` is exactly a cubic Bézier easing whose time control points are 1/3 and
//! 2/3 (then the Bézier's time coordinate is linear in its parameter, so progress is a cubic polynomial of time); the
//! value control points come from the piece's end slopes. So the compositor evaluates the same curve as before, with
//! no per-frame work in this process.

use crate::plan::{Cubics, Keys};
use player_lyrics_rules as rules;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    Linear,
    /// `CubicBezierEasingFunction((1/3, y1), (2/3, y2))`.
    Cubic(f64, f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// 0..1 of the duration.
    pub progress: f64,
    pub value: f64,
    /// Easing from the previous frame to this one.
    pub ease: Ease,
}

/// A key-frame animation, or a constant when `frames` has a single frame (or the duration is too short to run).
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub duration_secs: f64,
    pub frames: Vec<Frame>,
}

/// Shortest animation worth starting (the compositor's minimum duration is 1 ms).
pub const MIN_DURATION_SECS: f64 = 0.001;

impl Track {
    fn constant(value: f64) -> Self {
        Track {
            duration_secs: 0.0,
            frames: vec![Frame {
                progress: 0.0,
                value,
                ease: Ease::Linear,
            }],
        }
    }

    /// The value the animation holds before it starts and when it is not animated.
    pub fn first_value(&self) -> f64 {
        self.frames.first().map_or(0.0, |f| f.value)
    }

    pub fn is_animated(&self) -> bool {
        self.frames.len() >= 2 && self.duration_secs >= MIN_DURATION_SECS
    }

    /// The value at `t` seconds after the start, as the compositor evaluates it.
    pub fn value_at(&self, t: f64) -> f64 {
        if !self.is_animated() {
            return self.first_value();
        }
        let p = (t / self.duration_secs).clamp(0.0, 1.0);
        let i = self
            .frames
            .partition_point(|f| f.progress <= p)
            .clamp(1, self.frames.len() - 1);
        let (a, b) = (self.frames[i - 1], self.frames[i]);
        if p >= b.progress {
            return b.value;
        }
        let s = if b.progress > a.progress {
            (p - a.progress) / (b.progress - a.progress)
        } else {
            1.0
        };
        let y = match b.ease {
            Ease::Linear => s,
            Ease::Cubic(y1, y2) => bezier(y1, y2, s),
        };
        a.value + (b.value - a.value) * y
    }
}

/// One coordinate of a cubic Bézier from 0 to 1 with control values `y1`, `y2`, at parameter `s`.
pub fn bezier(y1: f64, y2: f64, s: f64) -> f64 {
    let r = 1.0 - s;
    3.0 * r * r * s * y1 + 3.0 * r * s * s * y2 + s * s * s
}

/// Piecewise-linear keys (opacity, scroll progress): linear easing between frames. `add + value * factor`.
pub fn linear(keys: &Keys, add: f64, factor: f64) -> Track {
    let map = |v: f64| add + v * factor;
    let (Some(&(t0, v0)), Some(&(end, last))) = (keys.first(), keys.last()) else {
        return Track::constant(add);
    };
    if keys.len() < 2 {
        return Track::constant(map(v0));
    }
    if end < MIN_DURATION_SECS {
        return Track::constant(map(last));
    }
    let mut frames = vec![Frame {
        progress: 0.0,
        value: map(v0),
        ease: Ease::Linear,
    }];
    for &(t, v) in &keys[if t0 <= 0.0 { 1 } else { 0 }..] {
        frames.push(Frame {
            progress: (t / end).clamp(0.0, 1.0),
            value: map(v),
            ease: Ease::Linear,
        });
    }
    Track {
        duration_secs: end,
        frames,
    }
}

/// The glide (`plan::Cubics`, `add` px added): one Bézier-eased frame per cubic piece.
pub fn cubic(keys: &Cubics, add: f64) -> Track {
    let Some(&(_, c_first)) = keys.first() else {
        return Track::constant(add);
    };
    let end = keys.last().unwrap().0;
    if keys.len() < 2 || end < MIN_DURATION_SECS {
        return Track::constant(add + keys.last().unwrap().1[0]);
    }
    let mut frames = vec![Frame {
        progress: 0.0,
        value: add + c_first[0],
        ease: Ease::Linear,
    }];
    let t_first = keys[0].0;
    if t_first > 0.0 {
        // Held before the first key (as DirectComposition did).
        frames.push(Frame {
            progress: t_first / end,
            value: add + c_first[0],
            ease: Ease::Linear,
        });
    }
    for pair in keys.windows(2) {
        let ((ta, c), (tb, next)) = (pair[0], pair[1]);
        let d = tb - ta;
        let (va, vb) = (c[0], next[0]);
        let delta = vb - va;
        let ease = if d <= 0.0 || delta.abs() < 1e-9 {
            Ease::Linear
        } else {
            // Slopes at both ends in value per unit of normalised time s (0..1 over the piece).
            let start = c[1] * d;
            let finish = (c[1] + 2.0 * c[2] * d + 3.0 * c[3] * d * d) * d;
            Ease::Cubic(start / (3.0 * delta), 1.0 - finish / (3.0 * delta))
        };
        frames.push(Frame {
            progress: (tb / end).clamp(0.0, 1.0),
            value: add + vb,
            ease,
        });
    }
    Track {
        duration_secs: end,
        frames,
    }
}

/// The edge fade as gradient stops `(offset 0..1 of the lyric area width, opacity)`, linear between them; the same
/// function as `rules::edge_fade_opacity`. Empty when the area is empty.
pub fn edge_fade_stops(area_width: f64, area_height: f64) -> Vec<(f64, f64)> {
    let zone = rules::edge_fade_width(area_width, area_height);
    if zone <= 0.0 {
        return Vec::new();
    }
    let z = zone / area_width;
    vec![(0.0, 0.0), (z, 1.0), (1.0 - z, 1.0), (1.0, 0.0)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{self, cubic_value_at, value_at};

    fn sample_every_10ms(end: f64, mut check: impl FnMut(f64)) {
        let mut t = 0.0;
        while t <= end + 0.05 {
            check(t);
            t += 0.01;
        }
    }

    fn strip_input<'a>(
        starts: &'a [f64],
        layout: &'a rules::StripLayout,
        at: f64,
        rate: f64,
        slew_until: Option<f64>,
    ) -> plan::StripInput<'a> {
        plan::StripInput {
            starts,
            layout,
            view_width: 600.0,
            final_line: None,
            at_ms: at,
            playing: true,
            inactive_percent: 50,
            rate,
            slew_until_ms: slew_until,
        }
    }

    fn layout_for(widths: &[f64]) -> rules::StripLayout {
        rules::layout_strip(widths, &[], 24.0)
    }

    #[test]
    fn the_glide_matches_the_plan_every_10ms() {
        let starts = [1000.0, 1150.0, 4000.0, 6500.0, 7000.0, 12000.0];
        let layout = layout_for(&[300.0, 120.0, 800.0, 450.0, 200.0, 500.0]);
        for at in [0.0, 900.0, 1100.0, 2500.0, 6600.0] {
            let planned = plan::plan_strip(&strip_input(&starts, &layout, at, 1.0, None));
            let track = cubic(&planned.offset, 37.0);
            assert!(track.is_animated() && track.frames.len() >= 3, "at {at}");
            let end = planned.offset.last().unwrap().0;
            sample_every_10ms(end, |t| {
                let want = 37.0 + cubic_value_at(&planned.offset, t);
                let got = track.value_at(t);
                assert!((want - got).abs() < 1e-6, "at {at} t {t}: {want} vs {got}");
            });
        }
    }

    #[test]
    fn the_glide_follows_a_drift_slew() {
        let starts = [1000.0, 3000.0, 5000.0, 9000.0];
        let layout = layout_for(&[300.0, 400.0, 500.0, 600.0]);
        for (rate, until) in [(1.1, 2200.0), (0.92, 3400.0), (1.2, 900.0)] {
            let input = strip_input(&starts, &layout, 800.0, rate, Some(until));
            let planned = plan::plan_strip(&input);
            let track = cubic(&planned.offset, 0.0);
            assert!(track.is_animated());
            let end = planned.offset.last().unwrap().0;
            // The slew end is a knot of the plan, and the track follows the plan through it.
            sample_every_10ms(end, |t| {
                let want = cubic_value_at(&planned.offset, t);
                assert!((want - track.value_at(t)).abs() < 1e-6, "rate {rate} t {t}");
            });
            // The plan itself still lands each line start where the rule puts it, at the slewed wall time.
            for &b in &starts[1..] {
                let secs = input.wall_secs(b);
                if secs > 0.0 && secs <= end {
                    let rule = rules::strip_offset(
                        &starts,
                        &layout,
                        rules::STRIP_ALIGN,
                        b,
                        600.0,
                        rules::STRIP_TARGET,
                        None,
                    );
                    assert!((track.value_at(secs) - rule).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn the_fades_match_the_plan_every_10ms() {
        let starts = [500.0, 700.0, 3000.0, 3100.0, 6000.0];
        let layout = layout_for(&[300.0, 400.0, 500.0, 600.0, 200.0]);
        for at in [0.0, 400.0, 650.0, 2950.0] {
            let planned = plan::plan_strip(&strip_input(&starts, &layout, at, 1.0, None));
            for (_, keys) in &planned.opacity {
                let track = linear(keys, 0.0, 1.0);
                let end = keys.last().unwrap().0;
                sample_every_10ms(end, |t| {
                    assert!((value_at(keys, t) - track.value_at(t)).abs() < 1e-9);
                });
            }
        }
    }

    #[test]
    fn the_long_line_scroll_matches_its_keys() {
        let keys: Keys = vec![(0.0, 0.25), (2.4, 1.0)];
        let track = linear(&keys, 12.0, -300.0);
        sample_every_10ms(2.4, |t| {
            let want = 12.0 - 300.0 * value_at(&keys, t);
            assert!((want - track.value_at(t)).abs() < 1e-9);
        });
        assert!(!linear(&vec![(0.0, 0.4)], 1.0, 2.0).is_animated());
        assert_eq!(linear(&vec![(0.0, 0.4)], 1.0, 2.0).first_value(), 1.8);
    }

    #[test]
    fn the_edge_fade_stops_match_the_rule() {
        for (w, h) in [(1000.0, 60.0), (100.0, 80.0), (37.0, 5.0)] {
            let stops = edge_fade_stops(w, h);
            let mut x = 0.0;
            while x <= w {
                let p = x / w;
                let i = stops
                    .partition_point(|s| s.0 <= p)
                    .clamp(1, stops.len() - 1);
                let (a, b) = (stops[i - 1], stops[i]);
                let v = if b.0 > a.0 {
                    a.1 + (b.1 - a.1) * ((p - a.0) / (b.0 - a.0)).clamp(0.0, 1.0)
                } else {
                    b.1
                };
                assert!(
                    (v - rules::edge_fade_opacity(x, w, h)).abs() < 1e-9,
                    "{w}x{h} at {x}"
                );
                x += 0.5;
            }
        }
        assert!(edge_fade_stops(0.0, 10.0).is_empty());
    }
}
