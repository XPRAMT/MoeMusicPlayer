// Now-playing cover: pure geometry for reading the stage's rect without the swipe feedback (ADR 0036). The swipe
// (`swipe-dismiss.js` damped mode) moves and scales the stage with the individual `translate` / `scale` properties, which
// `getBoundingClientRect` includes; the zoom's own geometry (`cover-zoom.js`) must not see them.

/** @typedef {{ left: number, top: number, width: number, height: number }} Rect */
/** @typedef {{ x: number, y: number, scale: number }} SwipeFeedback */

/**
 * The swipe feedback (`swipe-dismiss.js` damped mode) from the computed `translate` and `scale` values ('none', '',
 * '12px', '12px 3px', '0.97', '0.97 0.97').
 * @param {string | undefined | null} translate @param {string | undefined | null} scale @returns {SwipeFeedback}
 */
export function parseSwipeFeedback(translate, scale) {
  const t = typeof translate === 'string' && translate !== 'none' ? translate.trim().split(/\s+/).map((part) => parseFloat(part)) : [];
  const s = typeof scale === 'string' && scale !== 'none' ? parseFloat(scale.trim().split(/\s+/)[0]) : NaN;
  return { x: Number.isFinite(t[0]) ? t[0] : 0, y: Number.isFinite(t[1]) ? t[1] : 0, scale: Number.isFinite(s) && s > 0 ? s : 1 };
}

/**
 * The stage's painted client rect without the swipe feedback, from a rect read with it (`getBoundingClientRect`
 * includes the individual `translate` and `scale`). The feedback pivots on the layout centre C: R = C + t + s (V - C);
 * `pan` is the stage's own transform translate (V's centre - C).
 * @param {Rect} rect @param {SwipeFeedback} feedback @param {{ x: number, y: number }} pan @returns {Rect}
 */
export function withoutSwipeFeedback(rect, feedback, pan) {
  if (feedback.x === 0 && feedback.y === 0 && feedback.scale === 1) return rect;
  const s = feedback.scale;
  const width = rect.width / s;
  const height = rect.height / s;
  const centreX = rect.left + rect.width / 2 - feedback.x - s * pan.x + pan.x;
  const centreY = rect.top + rect.height / 2 - feedback.y - s * pan.y + pan.y;
  return { left: centreX - width / 2, top: centreY - height / 2, width, height };
}
