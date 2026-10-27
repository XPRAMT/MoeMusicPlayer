// Buffering cue (ADR 0028): the progress bar thumb pulses (app.css, one cycle per BUFFERING_PULSE_PERIOD_MS) while
// the playback state is `buffering`. It starts as soon as the state is seen, and every started cycle plays to its end:
// it only stops at a cycle boundary (back at full brightness) once buffering is over. Buffering again during the last
// cycle just carries on. The cue is the pulse alone; no text changes.
export const BUFFERING_PULSE_PERIOD_MS = 1000;

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
