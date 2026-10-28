// Now-playing cover re-raster nudge (ADR 0036): pure scheduling rules. The cover stage is a permanent will-change layer,
// which Chrome rasterises at the scale it was promoted at, so a zoomed-in cover is soft. A short translate transition on
// the stage (a sub-pixel move) makes Chrome re-raster the layer at the current scale. `cover-zoom.js` owns the element
// and the timers; everything here is testable decisions.

/** A scale change at most this large counts as "no change" (the zoom is stable). */
export const COVER_RERASTER_SCALE_EPSILON = 0.0005;
/** Scales at or below this need no nudge (the layer is only too sharp, never too soft). */
export const COVER_RERASTER_MIN_SCALE = 1.0001;
/** The nudge: first this inline translate (transition none), then, one frame later, back to none with the transition. */
export const COVER_RERASTER_TRANSLATE = '0.01px 0px';
export const COVER_RERASTER_TRANSITION = 'translate 180ms';
/** The previous transition string is restored this long after the nudge's transition began. */
export const COVER_RERASTER_RESTORE_MS = 260;

/**
 * @typedef {{ lastScale: number, stableSince: number, nudgedScale: number }} RerasterState
 * `nudgedScale`: the scale the layer was last nudged at (1 = never / reset).
 */

/** @param {number} [scale] @param {number} [now] @returns {RerasterState} */
export function createRerasterState(scale = 1, now = 0) {
  return { lastScale: scale, stableSince: now, nudgedScale: 1 };
}

/**
 * Feed the current scale. A change larger than the epsilon restarts the stability clock; a scale back at rest forgets
 * the last nudge (the next zoom-in nudges again).
 * @param {RerasterState} state @param {number} scale @param {number} now @returns {RerasterState}
 */
export function observeRerasterScale(state, scale, now) {
  const changed = Math.abs(scale - state.lastScale) > COVER_RERASTER_SCALE_EPSILON;
  const next = changed ? { ...state, lastScale: scale, stableSince: now } : state;
  return scale <= COVER_RERASTER_MIN_SCALE && next.nudgedScale !== 1 ? { ...next, nudgedScale: 1 } : next;
}

/** The cover content changed (another track's image): the layer needs a nudge again. @param {RerasterState} state @param {number} now @returns {RerasterState} */
export function invalidateReraster(state, now) {
  return { ...state, nudgedScale: 1, stableSince: now };
}

/**
 * What to do now. `skip`: nothing to do (scale at rest or below, or this scale was already nudged). `wait`: a nudge is
 * due but the scale has not been stable for `idleMs`, a gesture / swipe feedback / pan is active, or a nudge is in
 * flight: check again later. `run`: nudge now.
 * @param {RerasterState} state @param {{ now: number, idleMs: number, busy: boolean, inFlight: boolean }} input @returns {'skip' | 'wait' | 'run'}
 */
export function planReraster(state, { now, idleMs, busy, inFlight }) {
  if (state.lastScale <= COVER_RERASTER_MIN_SCALE) return 'skip';
  if (Math.abs(state.lastScale - state.nudgedScale) <= COVER_RERASTER_SCALE_EPSILON) return 'skip';
  if (inFlight || busy || now - state.stableSince < idleMs) return 'wait';
  return 'run';
}

/** @param {RerasterState} state @returns {RerasterState} */
export function markRerastered(state) {
  return { ...state, nudgedScale: state.lastScale };
}

/**
 * Whether a nudge's own inline writes are still what is on the stage: only then may the nudge clear them. A gesture's
 * feedback (translate / scale / opacity) or another transition string means the gesture owns the properties.
 * @param {{ translate?: string, transition?: string }} style @param {{ translate?: string, transition?: string }} mine
 */
export function nudgeOwnsStyle(style, mine) {
  return (mine.translate === undefined || (style.translate || '') === mine.translate)
    && (mine.transition === undefined || (style.transition || '') === mine.transition);
}
