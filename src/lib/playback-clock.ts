// Front-end playback clock (ADR 0028): the shell polls the playback snapshot only every 500 ms, so the progress bar
// and the lyrics advance between samples from a local estimate: the last sampled position plus the time since then
// (performance.now()) at the playback rate (1 while playing, 0 otherwise).
// A new sample corrects the estimate: a deviation under 30 ms is ignored, 30–250 ms is absorbed by running slightly
// faster or slower for CONVERGE_MS (never backwards), anything larger, a new track or a play/pause change jumps.
// A stall (the backend still says "playing" but its position stopped, e.g. a remote stream waiting for data) never
// moves the clock back: it freezes where it is shown and resumes once the real position is moving again and close
// enough (see `correctPlaybackClock`).

import type { PlaybackSnapshot } from './ipc';

export const IGNORE_DEVIATION_MS = 30;
export const JUMP_DEVIATION_MS = 250;
export const CONVERGE_MS = 750;
/** Stall detection needs at least this much wall time since the previous sample (about half a poll interval). */
export const STALL_MIN_WALL_MS = 250;
/** Between two samples the position advanced less than this fraction of the wall time: the playback is stalling. */
export const STALL_MAX_RATE = 0.5;
/** A stalled sample at most this far behind the estimate freezes the clock; further behind it jumps (two polls). */
export const STALL_HOLD_MAX_MS = 1000;

/** The previous sample (a class, so Svelte's deep `$state` never proxies it; updating it reruns nothing). */
export class PlaybackClockSampleRecord {
  positionMs: number;
  at: number;
  constructor(positionMs: number, at: number) {
    this.positionMs = positionMs;
    this.at = at;
  }
}

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
  /** Playing but held at `baseMs` while the real position is stalled (rate 0). `playing` stays true. */
  frozen: boolean;
  /** Bumped when a stall freezes or resumes the clock (not a jump): a run that is stopped or restarted by it. */
  runKey: number;
  /** The last sample applied (position and time), for stall detection. Updated in place, even by ignored samples. */
  lastSample: PlaybackClockSampleRecord;
}

/** True while the clock really advances: playing and not frozen by a stall. */
export function playbackClockRunning(clock: PlaybackClock | null): boolean {
  return !!clock && clock.playing && !clock.frozen;
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
  if (!playbackClockRunning(clock)) return clock.baseMs;
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
  return !!clock && playbackClockRunning(clock) && clock.durationMs != null && clock.durationMs > 0
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
    frozen: false,
    runKey: previous?.runKey ?? 0,
    lastSample: new PlaybackClockSampleRecord(sample.positionMs, now),
  };
}

/** Continue at rate 1 from `fromMs`, absorbing `deviation` over CONVERGE_MS when it is not negligible. */
function run(clock: PlaybackClock, sample: PlaybackClockSample, now: number, fromMs: number, deviation: number, extra: Partial<PlaybackClock> = {}): PlaybackClock {
  const converge = Math.abs(deviation) >= IGNORE_DEVIATION_MS;
  return {
    ...clock,
    ...extra,
    baseMs: fromMs,
    baseAt: now,
    rate: converge ? 1 + deviation / CONVERGE_MS : 1,
    convergeUntil: converge ? now + CONVERGE_MS : now,
    lastSample: new PlaybackClockSampleRecord(sample.positionMs, now),
  };
}

/**
 * Apply a sampled position (taken at `now`) to the clock.
 *
 * Stalls: a playing sample is "stalled" when at least STALL_MIN_WALL_MS passed since the previous sample and the
 * position advanced by less than STALL_MAX_RATE of that time (but did not go back by more than IGNORE_DEVIATION_MS).
 * A stalled sample that is behind the estimate by IGNORE_DEVIATION_MS..STALL_HOLD_MAX_MS freezes the clock where it is
 * shown (rate 0, `runKey` +1, `jumpId` unchanged). While frozen, the clock stays put until the real position is at most
 * IGNORE_DEVIATION_MS behind it, or is moving again and at most JUMP_DEVIATION_MS behind it; then it resumes from the
 * frozen position (`runKey` +1), converging the rest with a rate above 0, so the bar never moves back. A sample ahead
 * by more than JUMP_DEVIATION_MS, behind by more than STALL_HOLD_MAX_MS, a new track, state, duration or `force` jumps.
 */
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
  const previous = clock.lastSample;
  const wallMs = now - previous.at;
  const advancedMs = sample.positionMs - previous.positionMs;
  const judged = sample.playing && wallMs >= STALL_MIN_WALL_MS;
  const stalled = judged && advancedMs >= -IGNORE_DEVIATION_MS && advancedMs < wallMs * STALL_MAX_RATE;
  const moving = judged && advancedMs >= wallMs * STALL_MAX_RATE;
  const record = () => { previous.positionMs = sample.positionMs; previous.at = now; };

  if (clock.frozen) {
    const deviation = sample.positionMs - clock.baseMs;
    if (deviation > JUMP_DEVIATION_MS || deviation < -STALL_HOLD_MAX_MS) return jump(sample, now, clock);
    if (deviation >= -IGNORE_DEVIATION_MS || (moving && deviation >= -JUMP_DEVIATION_MS)) {
      return run(clock, sample, now, clock.baseMs, deviation, { frozen: false, runKey: clock.runKey + 1 });
    }
    record();
    return clock;
  }

  const estimate = rawPlaybackClockPosition(clock, now);
  const deviation = sample.positionMs - estimate;
  if (stalled && deviation <= -IGNORE_DEVIATION_MS && deviation >= -STALL_HOLD_MAX_MS) {
    // Hold where the bar is now; never jump back to the stalled position.
    const shown = clampToDuration(estimate, clock.durationMs);
    return {
      ...clock,
      baseMs: shown,
      baseAt: now,
      rate: 1,
      convergeUntil: now,
      frozen: true,
      runKey: clock.runKey + 1,
      lastSample: new PlaybackClockSampleRecord(sample.positionMs, now),
    };
  }
  if (Math.abs(deviation) < IGNORE_DEVIATION_MS) {
    record();
    return clock;
  }
  if (Math.abs(deviation) > JUMP_DEVIATION_MS || !sample.playing) return jump(sample, now, clock);
  // Catch up (or wait) over CONVERGE_MS from where the bar is now; the rate stays above 0, so it never moves back.
  return run(clock, sample, now, estimate, deviation);
}

/**
 * Milliseconds until the clock reaches `targetMs` (0 if it already has), or null when it is not running (paused or
 * frozen by a stall). Used to wake a timer exactly when the next lyric line or the next whole second starts.
 */
export function msUntilPlaybackPosition(clock: PlaybackClock | null, now: number, targetMs: number): number | null {
  if (!clock || !playbackClockRunning(clock) || !Number.isFinite(targetMs)) return null;
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
 * paused, frozen by a stall, unknown length or already at the end).
 */
export function playbackProgressRun(clock: PlaybackClock | null, now: number): { ratio: number; remainingMs: number | null } {
  if (!clock || clock.durationMs == null || clock.durationMs <= 0) return { ratio: 0, remainingMs: null };
  const position = estimatePlaybackPosition(clock, now);
  const ratio = Math.min(1, Math.max(0, position / clock.durationMs));
  const remainingMs = playbackClockRunning(clock) ? clock.durationMs - position : 0;
  return { ratio, remainingMs: remainingMs > 0 ? remainingMs : null };
}
