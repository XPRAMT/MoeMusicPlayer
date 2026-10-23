// Now Playing lyric scroll glide. A glide is a cubic Hermite curve from the position and velocity the list has when it
// starts to the target at rest, so retargeting it mid-flight (the per-track delay stepped again, a wheel or a held
// button) keeps both position and speed continuous: one animation that bends to the new target instead of a restart
// or a jump per step.

export interface ScrollGlide {
  readonly from: number;
  /** px per ms at the start. */
  readonly velocity: number;
  readonly to: number;
  readonly startedAt: number;
  readonly durationMs: number;
}

/** Ease-out cubic glide from rest (the normal cue advance, ADR 0029): Hermite with start speed 3·distance/duration. */
export function restingGlide(from: number, to: number, startedAt: number, durationMs: number): ScrollGlide {
  const duration = Math.max(1, durationMs);
  return { from, velocity: (3 * (to - from)) / duration, to, startedAt, durationMs: duration };
}

/** Position of `glide` at time `now` (clamped to its ends). */
export function glidePosition(glide: ScrollGlide, now: number): number {
  const d = glide.durationMs;
  const t = Math.min(1, Math.max(0, (now - glide.startedAt) / d));
  if (t >= 1) return glide.to;
  const t2 = t * t;
  const t3 = t2 * t;
  const h00 = 2 * t3 - 3 * t2 + 1;
  const h10 = t3 - 2 * t2 + t;
  const h01 = -2 * t3 + 3 * t2;
  return h00 * glide.from + h10 * d * glide.velocity + h01 * glide.to;
}

/** Velocity (px per ms) of `glide` at time `now`; 0 once it has ended. */
export function glideVelocity(glide: ScrollGlide, now: number): number {
  const d = glide.durationMs;
  const t = (now - glide.startedAt) / d;
  if (t >= 1 || t < 0) return t < 0 ? glide.velocity : 0;
  const t2 = t * t;
  const dh00 = 6 * t2 - 6 * t;
  const dh10 = 3 * t2 - 4 * t + 1;
  const dh01 = -6 * t2 + 6 * t;
  return (dh00 * glide.from + dh01 * glide.to) / d + dh10 * glide.velocity;
}

/**
 * A glide to `to` that continues `current` (if it is still running at `now`) with its position and speed, or starts at
 * rest from `scrollTop`.
 */
export function retargetGlide(current: ScrollGlide | null, scrollTop: number, to: number, now: number, durationMs: number): ScrollGlide {
  const running = current !== null && now < current.startedAt + current.durationMs;
  const from = running ? glidePosition(current, now) : scrollTop;
  const velocity = running ? glideVelocity(current, now) : 0;
  return { from, velocity, to, startedAt: now, durationMs: Math.max(1, durationMs) };
}

/** Round a scroll position to whole device pixels (the 1-device-pixel rule). */
export function snapToDevicePixel(position: number, devicePixelRatio: number): number {
  const ratio = Number.isFinite(devicePixelRatio) && devicePixelRatio > 0 ? devicePixelRatio : 1;
  return Math.round(position * ratio) / ratio;
}

export type TimedNavigationKind = 'cue' | 'glide' | 'instant';

/**
 * How the timed list moves to a newly shown line.
 * - `instant`: a seek, a track change or a layout/viewport change (always lands at once);
 * - `glide`: the lyric delay changed, or the line changed some other way while a glide is still running (for example
 *   a cue advance that a delay step moved by more than one line): bend the running glide;
 * - `cue`: the normal advance to the next line while playing (ADR 0029, ends on the line start);
 * - otherwise `instant` (nothing in flight to continue).
 */
export function timedNavigationKind(input: {
  sameTrackAndLayout: boolean;
  seeked: boolean;
  offsetChanged: boolean;
  playing: boolean;
  previousIndex: number;
  index: number;
  glideRunning?: boolean;
}): TimedNavigationKind {
  if (!input.sameTrackAndLayout || input.seeked) return 'instant';
  if (input.offsetChanged) return input.index >= 0 ? 'glide' : 'instant';
  if (input.playing && input.previousIndex >= 0 && input.index === input.previousIndex + 1) return 'cue';
  return input.glideRunning && input.index >= 0 ? 'glide' : 'instant';
}

export type TimedScrollPlan = 'retarget' | 'rest' | 'instant' | 'none';

/**
 * What the scroll does for a navigation (decided on the frame, with the glide state of that frame). Every animated
 * change goes through the one glide: while a glide runs, a cue advance or a delay change retargets it (keeping position
 * and speed); a standalone cue advance starts the ADR 0029 ease-out from rest. A cue advance with under 16 ms left is
 * instant only when nothing is gliding.
 */
export function planTimedScroll(input: {
  kind: TimedNavigationKind;
  glideRunning: boolean;
  animationMs: number;
  reducedMotion: boolean;
  distancePx: number;
}): TimedScrollPlan {
  if (input.reducedMotion || input.kind === 'instant') return 'instant';
  if (input.glideRunning) return 'retarget';
  if (Math.abs(input.distancePx) <= 1) return 'none';
  if (input.kind === 'glide') return 'retarget';
  return input.animationMs >= 16 ? 'rest' : 'instant';
}
