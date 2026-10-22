// Lyric cue lead (ADR 0029): on the Now Playing page a line change animates for LYRIC_CUE_LEAD_MS and starts that much
// early, so the animation ends exactly when the line starts. The same constant drives the scroll tween in JS and the
// colour/opacity transitions in CSS (`--lyric-cue-ms`).

export const LYRIC_CUE_LEAD_MS = 200;

/**
 * Playback positions at which each line is shown (`startMs - offsetMs - lead`). The lead is LYRIC_CUE_LEAD_MS, shortened
 * to the gap from the previous line when that is smaller, and for the first line never before position 0. Without
 * `leading` (paused, stopped) every line is shown at its own start. Non-decreasing because the start times are.
 */
export function displayCuePositions(startTimesMs: readonly number[], offsetMs: number, leading: boolean): number[] {
  return startTimesMs.map((start, index) => {
    const position = start - offsetMs;
    if (!leading) return position;
    const room = index === 0 ? Math.max(0, position) : Math.max(0, start - startTimesMs[index - 1]);
    return position - Math.min(LYRIC_CUE_LEAD_MS, room);
  });
}

/** Index of the line shown at `positionMs` (-1 before the first). */
export function findDisplayCueIndex(displayPositionsMs: readonly number[], positionMs: number): number {
  if (!Number.isFinite(positionMs)) return -1;
  let low = 0;
  let high = displayPositionsMs.length;
  while (low < high) {
    const middle = low + Math.floor((high - low) / 2);
    if (displayPositionsMs[middle] <= positionMs) low = middle + 1; else high = middle;
  }
  return low - 1;
}

/** The next position after `positionMs` at which another line is shown, or null. */
export function nextDisplayCuePosition(displayPositionsMs: readonly number[], positionMs: number): number | null {
  const index = findDisplayCueIndex(displayPositionsMs, positionMs) + 1;
  return index < displayPositionsMs.length ? displayPositionsMs[index] : null;
}

/**
 * How long the change to line `index` animates when it is shown at `positionMs`: the time left until the line really
 * starts (so a shortened lead, or resuming inside the lead, animates for what remains), capped at LYRIC_CUE_LEAD_MS.
 */
export function cueAnimationMs(startTimesMs: readonly number[], offsetMs: number, index: number, positionMs: number): number {
  if (index < 0 || index >= startTimesMs.length) return 0;
  return Math.max(0, Math.min(LYRIC_CUE_LEAD_MS, startTimesMs[index] - offsetMs - positionMs));
}

/** ease-out cubic, 0..1 → 0..1. */
export function easeOutCubic(t: number): number {
  const clamped = Math.min(1, Math.max(0, t));
  return 1 - (1 - clamped) ** 3;
}
