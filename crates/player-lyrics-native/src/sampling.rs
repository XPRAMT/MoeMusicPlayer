//! Per-frame sampling of the plans (ADR 0034): the renderer thread waits for each composition frame, evaluates every
//! window's glide, fades and long-line scroll from its own playback clock (the plan's curves, which already carry the
//! drift slewing), and writes a property only when its device value changed: an offset by at least one whole device
//! pixel (offsets are always whole pixels), an opacity by at least 1/255. Nothing is rasterised per frame.

use crate::plan::{cubic_value_at, value_at, Cubics, Keys};
use player_lyrics_rules as rules;

/// One animated property's curve, in seconds since its plan's instant.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    /// The glide: `add + cubics`.
    Cubic { keys: Cubics, add: f64 },
    /// Fades and scroll: `add + keys * factor`.
    Linear { keys: Keys, add: f64, factor: f64 },
}

impl Curve {
    pub fn value_at(&self, t: f64) -> f64 {
        match self {
            Curve::Cubic { keys, add } => add + cubic_value_at(keys, t),
            Curve::Linear { keys, add, factor } => add + value_at(keys, t) * factor,
        }
    }

    /// The time of its last key: constant from then on.
    pub fn end(&self) -> f64 {
        match self {
            Curve::Cubic { keys, .. } => keys.last().map_or(0.0, |k| k.0),
            Curve::Linear { keys, .. } => keys.last().map_or(0.0, |k| k.0),
        }
    }
}

/// What a sampled value becomes on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Offset.X in whole device pixels.
    OffsetX,
    /// Opacity in 1/255 steps.
    Opacity,
}

/// The device value of `v`: whole pixels for an offset, the nearest 1/255 for an opacity.
pub fn quantize(kind: Kind, v: f64) -> f64 {
    match kind {
        Kind::OffsetX => v.round(),
        Kind::Opacity => (v.clamp(0.0, 1.0) * 255.0).round() / 255.0,
    }
}

/// A property to write this frame: `Some(value)` only when its device value differs from `written`.
pub fn changed(kind: Kind, curve: &Curve, t: f64, written: f64) -> Option<f64> {
    let v = quantize(kind, curve.value_at(t));
    (v != written).then_some(v)
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
    use crate::plan;

    fn layout_for(widths: &[f64]) -> rules::StripLayout {
        rules::layout_strip(widths, &[], 24.0)
    }

    fn input<'a>(
        starts: &'a [f64],
        layout: &'a rules::StripLayout,
        motion: &plan::Motion,
        dt_ms: f64,
    ) -> plan::StripInput<'a> {
        let slew = motion.slew(dt_ms);
        let at = motion.position(dt_ms);
        plan::StripInput {
            starts,
            layout,
            view_width: 600.0,
            final_line: None,
            at_ms: at,
            playing: true,
            inactive_percent: 50,
            rate: slew.map_or(1.0, |s| s.0),
            slew_until_ms: slew.map(|s| at + s.1),
        }
    }

    /// Per-frame sampling at 120 Hz over two minutes with re-plans at line changes and at every playback sample
    /// (drift corrections of either sign, up to the jump threshold): the written offset never moves backwards, and
    /// at every re-plan the old and new plan give the same device pixel (no step), so each frame moves by at most
    /// the glide's own per-frame motion.
    #[test]
    fn per_frame_samples_never_step_or_go_back() {
        let starts: Vec<f64> = (0..60)
            .map(|i| 500.0 + i as f64 * 1700.0 + (i % 3) as f64 * 400.0)
            .collect();
        let widths: Vec<f64> = (0..60).map(|i| 150.0 + (i * 97 % 600) as f64).collect();
        let layout = layout_for(&widths);
        let frame = 1000.0 / 120.0;
        let drifts = [
            0.0, 12.0, -9.0, 30.0, -25.0, 5.0, 100.0, -100.0, 240.0, -240.0,
        ];
        let mut motion = plan::Motion::at(0.0);
        let mut base = 0.0;
        let mut planned = plan::plan_strip(&input(&starts, &layout, &motion, 0.0));
        let mut plan_at = 0.0;
        let mut written = f64::NAN;
        let mut next_sample = 500.0;
        let mut samples = 0;
        let mut w = 0.0;
        let mut max_step: f64 = 0.0;
        let mut prev_raw = 0.0;
        while w < 120_000.0 {
            let mut replan = planned.wake_ms.is_some_and(|ms| w - plan_at >= ms);
            if w >= next_sample {
                next_sample += 500.0;
                samples += 1;
                let drift = drifts[samples % drifts.len()];
                let dt = w - base;
                if let plan::Correction::Slew(m) = motion.correct(dt, motion.position(dt) + drift) {
                    motion = m;
                    base = w;
                    replan = true;
                }
            }
            if replan {
                let next = plan::plan_strip(&input(&starts, &layout, &motion, w - base));
                let old = quantize(
                    Kind::OffsetX,
                    cubic_value_at(&planned.offset, (w - plan_at) / 1000.0),
                );
                let new = quantize(Kind::OffsetX, cubic_value_at(&next.offset, 0.0));
                assert!((old - new).abs() <= 1.0, "step at {w}: {old} -> {new}");
                planned = next;
                plan_at = w;
            }
            let curve = Curve::Cubic {
                keys: planned.offset.clone(),
                add: 0.0,
            };
            if let Some(v) = changed(Kind::OffsetX, &curve, (w - plan_at) / 1000.0, written) {
                if !written.is_nan() {
                    assert!(v <= written, "backwards at {w}: {written} -> {v}");
                    // No more than the curve's own motion since the last frame (plus the rounding).
                    let own = prev_raw - curve.value_at((w - plan_at) / 1000.0);
                    assert!(
                        written - v <= own.abs().ceil() + 1.0,
                        "jump at {w}: {written} -> {v}"
                    );
                    max_step = max_step.max(written - v);
                }
                written = v;
            }
            prev_raw = curve.value_at((w - plan_at) / 1000.0);
            w += frame;
        }
        assert!(max_step > 0.0);
    }

    #[test]
    fn writes_only_on_device_changes() {
        let fade = Curve::Linear {
            keys: vec![(0.0, 0.5), (0.2, 1.0)],
            add: 0.0,
            factor: 1.0,
        };
        let mut written = f64::NAN;
        let mut writes = 0;
        let mut t = 0.0;
        while t <= 0.5 {
            if let Some(v) = changed(Kind::Opacity, &fade, t, written) {
                written = v;
                writes += 1;
            }
            t += 1.0 / 120.0;
        }
        // 0.5 -> 1 over 24 frames: one write per frame while it moves, none once it is steady.
        assert!((24..=26).contains(&writes), "{writes} writes");
        assert_eq!(written, 1.0);
        let still = Curve::Linear {
            keys: vec![(0.0, 0.3)],
            add: 10.0,
            factor: -100.0,
        };
        assert_eq!(changed(Kind::OffsetX, &still, 5.0, -20.0), None);
        assert_eq!(quantize(Kind::OffsetX, 12.49), 12.0);
        assert_eq!(quantize(Kind::Opacity, 0.5), 128.0 / 255.0);
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
