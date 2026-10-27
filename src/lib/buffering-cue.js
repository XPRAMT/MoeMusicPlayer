// Buffering cue (ADR 0028): the progress bar thumb pulses (app.css, one cycle per BUFFERING_PULSE_PERIOD_MS) once the
// playback state has been `buffering` for BUFFERING_PULSE_DELAY_MS, so the brief buffering after an ordinary seek does
// not show it. While the state is `buffering` the shell samples every BUFFERING_POLL_MS instead of every 500 ms, so a
// short spell is really seen to end in time. Every started cycle plays to its end: it only stops at a cycle boundary
// (back at full brightness) once buffering is over; buffering again during the last cycle just carries on. The cue is
// the pulse alone; no text changes.
export const BUFFERING_PULSE_PERIOD_MS = 1000;
/** Buffering must have been seen continuously this long before the pulse starts. */
export const BUFFERING_PULSE_DELAY_MS = 250;
/** Snapshot interval while the state is `buffering`. */
export const BUFFERING_POLL_MS = 100;

/**
 * Snapshot poll interval for this state: BUFFERING_POLL_MS while buffering, otherwise the normal interval.
 * @param {string | null | undefined} state
 * @param {number} normalMs
 * @returns {number}
 */
export function playbackPollIntervalMs(state, normalMs) {
  return state === 'buffering' ? BUFFERING_POLL_MS : normalMs;
}

/**
 * Milliseconds until the pulse may start (0 when it already may), or null when not buffering.
 * @param {string | null | undefined} state
 * @param {number} bufferingSinceMs performance.now() when `buffering` was first seen (this spell)
 * @param {number} nowMs
 * @returns {number | null}
 */
export function msUntilBufferingPulse(state, bufferingSinceMs, nowMs) {
  if (state !== 'buffering') return null;
  return Math.max(0, BUFFERING_PULSE_DELAY_MS - Math.max(0, nowMs - bufferingSinceMs));
}

/**
 * When a buffering spell from `startMs` to `endMs` starts the pulse: `startMs + BUFFERING_PULSE_DELAY_MS`, or null
 * when it ended before that.
 * @param {number} startMs
 * @param {number} endMs
 * @returns {number | null}
 */
export function bufferingPulseStartMs(startMs, endMs) {
  return endMs - startMs >= BUFFERING_PULSE_DELAY_MS ? startMs + BUFFERING_PULSE_DELAY_MS : null;
}

/**
 * Whether the thumb should pulse for this playback snapshot state.
 * @param {string | null | undefined} state playback snapshot state
 * @returns {boolean}
 */
export function bufferingCueVisible(state) {
  return state === 'buffering';
}

/**
 * Next "pulsing" flag. `state`: the buffering flag changed (start at once; on reduced motion there is no animation and
 * so no cycle boundary, so stop at once). `cycle-end`: a pulse cycle ended (animationiteration); keep going only while
 * still buffering.
 * @param {boolean} pulsing
 * @param {{ type: 'state', buffering: boolean, reducedMotion?: boolean } | { type: 'cycle-end', buffering: boolean }} event
 * @returns {boolean}
 */
export function nextBufferingPulse(pulsing, event) {
  if (event.buffering) return true;
  if (event.type === 'cycle-end') return false;
  return event.reducedMotion ? false : pulsing;
}

/**
 * When a pulse that started at `startMs` stops if buffering ends at `bufferingEndMs`: the end of the cycle running at
 * that moment, at least one full cycle.
 * @param {number} startMs
 * @param {number} bufferingEndMs
 * @param {number} [periodMs]
 * @returns {number}
 */
export function bufferingPulseStopMs(startMs, bufferingEndMs, periodMs = BUFFERING_PULSE_PERIOD_MS) {
  const elapsed = Math.max(0, bufferingEndMs - startMs);
  const cycles = Math.max(1, Math.ceil(elapsed / periodMs));
  return startMs + cycles * periodMs;
}
