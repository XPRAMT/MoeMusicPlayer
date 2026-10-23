// Front-end playback clock (ADR 0028): the shell polls the playback snapshot only every 500 ms, so the progress bar
// and the lyrics advance between samples from a local estimate: the last sampled position plus the time since then
// (performance.now()) at the playback rate (1 while playing, 0 otherwise).
// A new sample corrects the estimate: a deviation under 30 ms is ignored, 30–250 ms is absorbed by running slightly
// faster or slower for CONVERGE_MS (never backwards), anything larger, a new track or a play/pause change jumps.

import type { PlaybackSnapshot } from './ipc';

export const IGNORE_DEVIATION_MS = 30;
export const JUMP_DEVIATION_MS = 250;
export const CONVERGE_MS = 750;

export interface PlaybackClock {
  trackId: string | null;
  /** Estimated position at `baseAt`. */
  baseMs: number;
  /** performance.now() of `baseMs`. */
  baseAt: number;
  /** Rate until `convergeUntil` (1 when not converging). */
  rate: number;
  /** After this time the clock runs at 1 again. */
  convergeUntil: number;
  playing: boolean;
  durationMs: number | null;
  /** Bumped on every jump; an animation keyed on it restarts only for a jump, never for a smooth correction. */
  jumpId: number;
}

export interface PlaybackClockSample {
  trackId: string | null;
  positionMs: number;
  playing: boolean;
  durationMs: number | null;
}

/** What the main window forwards to the taskbar lyrics window after each sample (`taskbar-lyrics-playback`). */
export interface TaskbarLyricsPlayback {
  trackId: string | null;
  positionMs: number;
  durationMs: number | null;
  state: PlaybackSnapshot['state'];
  playing: boolean;
}

export function clampToDuration(positionMs: number, durationMs: number | null): number {
  const lower = Math.max(0, Number.isFinite(positionMs) ? positionMs : 0);
  return durationMs != null && durationMs > 0 ? Math.min(lower, durationMs) : lower;
}

/** Unclamped estimate (used to notice that the track should have ended). */
export function rawPlaybackClockPosition(clock: PlaybackClock, now: number): number {
  if (!clock.playing) return clock.baseMs;
  const elapsed = Math.max(0, now - clock.baseAt);
  const converging = Math.max(0, Math.min(elapsed, clock.convergeUntil - clock.baseAt));
  return clock.baseMs + converging * clock.rate + (elapsed - converging);
}

export function estimatePlaybackPosition(clock: PlaybackClock | null, now: number): number {
  if (!clock) return 0;
  return clampToDuration(rawPlaybackClockPosition(clock, now), clock.durationMs);
}

/** True when a playing estimate has run past the end of the track (the shell has probably moved on). */
export function playbackClockPastEnd(clock: PlaybackClock | null, now: number): boolean {
  return !!clock && clock.playing && clock.durationMs != null && clock.durationMs > 0
    && rawPlaybackClockPosition(clock, now) >= clock.durationMs;
}

function jump(sample: PlaybackClockSample, now: number, previous: PlaybackClock | null): PlaybackClock {
  return {
    jumpId: (previous?.jumpId ?? 0) + 1,
    trackId: sample.trackId,
    baseMs: clampToDuration(sample.positionMs, sample.durationMs),
    baseAt: now,
    rate: 1,
    convergeUntil: now,
    playing: sample.playing,
    durationMs: sample.durationMs,
  };
}

/** Apply a sampled position (taken at `now`) to the clock. */
export function correctPlaybackClock(
  clock: PlaybackClock | null,
  sample: PlaybackClockSample,
  now: number,
  options: { force?: boolean } = {},
): PlaybackClock {
  if (
    !clock
    || options.force
    || clock.trackId !== sample.trackId
    || clock.playing !== sample.playing
    || clock.durationMs !== sample.durationMs
  ) {
    return jump(sample, now, clock);
  }
  const estimate = rawPlaybackClockPosition(clock, now);
  const deviation = sample.positionMs - estimate;
  if (Math.abs(deviation) < IGNORE_DEVIATION_MS) return clock;
  if (Math.abs(deviation) > JUMP_DEVIATION_MS || !sample.playing) return jump(sample, now, clock);
  // Catch up (or wait) over CONVERGE_MS from where the bar is now; the rate stays above 0, so it never moves back.
  return {
    ...clock,
    baseMs: estimate,
    baseAt: now,
    rate: 1 + deviation / CONVERGE_MS,
    convergeUntil: now + CONVERGE_MS,
  };
}

/**
 * Milliseconds until the clock reaches `targetMs` (0 if it already has), or null when it is not running.
 * Used to wake a timer exactly when the next lyric line or the next whole second starts.
 */
export function msUntilPlaybackPosition(clock: PlaybackClock | null, now: number, targetMs: number): number | null {
  if (!clock || !clock.playing || !Number.isFinite(targetMs)) return null;
  const raw = rawPlaybackClockPosition(clock, now);
  if (targetMs <= raw) return 0;
  const convergeLeft = Math.max(0, clock.convergeUntil - Math.max(now, clock.baseAt));
  if (convergeLeft > 0 && raw + convergeLeft * clock.rate >= targetMs) return (targetMs - raw) / clock.rate;
  return convergeLeft + (targetMs - raw - convergeLeft * clock.rate);
}

/** The first playback position after `positionMs` at which a line starts (`startMs - offsetMs`), or null. */
export function nextCuePosition(startTimesMs: readonly number[], offsetMs: number, positionMs: number): number | null {
  const target = positionMs + offsetMs;
  let low = 0;
  let high = startTimesMs.length;
  while (low < high) {
    const middle = low + Math.floor((high - low) / 2);
    if (startTimesMs[middle] <= target) low = middle + 1; else high = middle;
  }
  return low < startTimesMs.length ? startTimesMs[low] - offsetMs : null;
}

/**
 * Where the progress bar starts and how long its linear CSS run to the end lasts (null when it should stand still:
 * paused, unknown length or already at the end).
 */
export function playbackProgressRun(clock: PlaybackClock | null, now: number): { ratio: number; remainingMs: number | null } {
  if (!clock || clock.durationMs == null || clock.durationMs <= 0) return { ratio: 0, remainingMs: null };
  const position = estimatePlaybackPosition(clock, now);
  const ratio = Math.min(1, Math.max(0, position / clock.durationMs));
  const remainingMs = clock.playing ? clock.durationMs - position : 0;
  return { ratio, remainingMs: remainingMs > 0 ? remainingMs : null };
}
