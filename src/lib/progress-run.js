// The playback progress bar costs no main-thread work per frame (ADR 0028): the glowing track and the painted thumb are
// moved only by `transform` Web Animations, which run on the compositor. Percentages in translateX refer to the moved
// element's own width, so nothing depends on the track's pixel width (no ResizeObserver, a width change just works).
//
//   .progress-thumb-mover  as wide as the thumb's travel (track width - thumb): translateX(ratio * 100%)
//   .progress-band         3 x the track width, centred on the track's left edge: translateX(ratio * 100 / 3 %)
//
// The band carries the fixed glow pattern (the playhead's glow centre sits in the middle of the band), so moving the
// band by ratio x track width puts the glow at the playhead.

/** Width of `.progress-band` as a multiple of the track width. */
export const PROGRESS_BAND_SPAN = 3;

/** @param {number} value */
function percent(value) {
  return `${Number((value * 100).toFixed(5))}%`;
}

/**
 * The two transforms for a playhead at `ratio` (0..1).
 * @param {number} ratio
 */
export function progressTransforms(ratio) {
  const clamped = Number.isFinite(ratio) ? Math.min(1, Math.max(0, ratio)) : 0;
  return {
    thumb: `translateX(${percent(clamped)})`,
    band: `translateX(${percent(clamped / PROGRESS_BAND_SPAN)})`,
  };
}

/**
 * Keyframes that run linearly from `ratio` to the end of the track.
 * @param {number} ratio
 */
export function progressRunKeyframes(ratio) {
  const from = progressTransforms(ratio);
  const to = progressTransforms(1);
  return {
    thumb: [{ transform: from.thumb }, { transform: to.thumb }],
    band: [{ transform: from.band }, { transform: to.band }],
  };
}

/** @typedef {{ ratio: number, remainingMs: number | null, key: string }} ProgressMotion */

/**
 * use:progressRun={{ ratio, remainingMs, key }} on `.progress-hit`. `remainingMs === null` parks the bar at `ratio`;
 * otherwise it runs linearly from `ratio` to 1 in `remainingMs`. The run restarts only when the params change by value
 * (the owner changes `key` for a restart at the same numbers).
 * @param {HTMLElement} node @param {ProgressMotion} params
 */
export function progressRun(node, params) {
  const thumb = /** @type {HTMLElement | null} */ (node.querySelector('.progress-thumb-mover'));
  const band = /** @type {HTMLElement | null} */ (node.querySelector('.progress-band'));
  /** @type {Animation[]} */
  let animations = [];
  /** @type {ProgressMotion | null} */
  let applied = null;

  function stop() {
    for (const animation of animations) animation.cancel();
    animations = [];
  }

  /** @param {ProgressMotion} next */
  function apply(next) {
    if (applied && applied.ratio === next.ratio && applied.remainingMs === next.remainingMs && applied.key === next.key) return;
    applied = { ...next };
    stop();
    const parked = progressTransforms(next.ratio);
    if (thumb) thumb.style.transform = parked.thumb;
    if (band) band.style.transform = parked.band;
    if (next.remainingMs === null || !(next.remainingMs > 0) || typeof node.animate !== 'function') return;
    const keyframes = progressRunKeyframes(next.ratio);
    const timing = { duration: next.remainingMs, easing: 'linear', fill: /** @type {FillMode} */ ('forwards') };
    if (thumb) animations.push(thumb.animate(keyframes.thumb, timing));
    if (band) animations.push(band.animate(keyframes.band, timing));
  }

  apply(params);
  return {
    /** @param {ProgressMotion} next */
    update(next) { apply(next); },
    destroy() { stop(); },
  };
}
