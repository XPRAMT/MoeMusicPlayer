// Buffering cue (ADR 0028): the progress bar thumb pulses only once a buffering spell has lasted this long, so the
// brief buffering after an ordinary seek does not flash it. The cue is the pulse alone; no text changes.
export const BUFFERING_CUE_DELAY_MS = 500;

/**
 * Whether the thumb should pulse, given the playback state and how long it has been in that state.
 * @param {string | null | undefined} state playback snapshot state
 * @param {number} elapsedMs time since the state became `buffering`
 * @returns {boolean}
 */
export function bufferingCueVisible(state, elapsedMs) {
  return state === 'buffering' && Number.isFinite(elapsedMs) && elapsedMs >= BUFFERING_CUE_DELAY_MS;
}

/**
 * How long to wait (one timer, no polling) before the cue shows; `null` when it never will in this state.
 * @param {string | null | undefined} state
 * @param {number} elapsedMs time already spent in this state
 * @returns {number | null}
 */
export function msUntilBufferingCue(state, elapsedMs) {
  if (state !== 'buffering') return null;
  const elapsed = Number.isFinite(elapsedMs) ? Math.max(0, elapsedMs) : 0;
  return Math.max(0, BUFFERING_CUE_DELAY_MS - elapsed);
}
