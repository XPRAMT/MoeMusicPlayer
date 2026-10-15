// Shared touch/pen swipe-to-dismiss gesture (Svelte action) plus pure decision helpers.
// Mouse input is ignored entirely, so desktop behaviour is unchanged.

export const SWIPE_AXIS_SLOP_PX = 10;
export const SWIPE_DISMISS_DISTANCE_RATIO = 0.3;
export const SWIPE_DISMISS_VELOCITY_PX_PER_MS = 0.5;
export const SWIPE_SETTLE_MS = 180;
export const SWIPE_IGNORE_SELECTOR = 'input, select, textarea, [role="slider"], [contenteditable=""], [contenteditable="true"], [data-swipe-ignore]';

/** @typedef {'left' | 'right' | 'down' | 'up'} SwipeDirection */
/** @typedef {{ x: number, y: number, t: number }} SwipeSample */
/** @typedef {{ direction: SwipeDirection, onDismiss: () => void, enabled?: boolean }} SwipeDismissOptions */

/** @type {Record<SwipeDirection, 'x' | 'y'>} */
const AXIS = { left: 'x', right: 'x', down: 'y', up: 'y' };
/** @type {Record<SwipeDirection, number>} */
const SIGN = { left: -1, right: 1, down: 1, up: -1 };

/** Lock to an axis once movement passes the slop: 'x' | 'y' | null (undecided). */
/** @param {number} dx @param {number} dy @returns {'x' | 'y' | null} */
export function resolveSwipeAxis(dx, dy, slopPx = SWIPE_AXIS_SLOP_PX) {
  if (Math.hypot(dx, dy) < slopPx) return null;
  return Math.abs(dx) > Math.abs(dy) ? 'x' : 'y';
}

/** Distance travelled in the dismiss direction, clamped to >= 0 (the element never moves the other way). */
/** @param {SwipeDirection} direction @param {number} dx @param {number} dy */
export function getSwipeOffset(direction, dx, dy) {
  const raw = AXIS[direction] === 'x' ? dx : dy;
  return Math.max(0, raw * SIGN[direction]);
}

/** @param {SwipeDirection} direction @param {'x' | 'y'} axis */
export function isSwipeAxisForDirection(direction, axis) {
  return AXIS[direction] === axis;
}

/** Velocity in the dismiss direction (px/ms) from the last two samples. */
/** @param {SwipeDirection} direction @param {SwipeSample} prev @param {SwipeSample} last */
export function getSwipeVelocity(direction, prev, last) {
  const dt = last.t - prev.t;
  if (!(dt > 0)) return 0;
  const d = AXIS[direction] === 'x' ? last.x - prev.x : last.y - prev.y;
  return (d * SIGN[direction]) / dt;
}

/** @param {{ offsetPx: number, sizePx: number, velocityPxPerMs: number }} input */
export function shouldDismissSwipe({ offsetPx, sizePx, velocityPxPerMs }, ratio = SWIPE_DISMISS_DISTANCE_RATIO, velocity = SWIPE_DISMISS_VELOCITY_PX_PER_MS) {
  if (!(offsetPx > 0)) return false;
  if (sizePx > 0 && offsetPx >= sizePx * ratio) return true;
  return velocityPxPerMs >= velocity;
}

/** @param {string} pointerType */
export function isSwipePointer(pointerType) {
  return pointerType === 'touch' || pointerType === 'pen';
}

/** @param {SwipeDirection} direction @param {number} offset */
function translateFor(direction, offset) {
  const v = offset * SIGN[direction];
  return AXIS[direction] === 'x' ? `${v}px 0` : `0 ${v}px`;
}

/**
 * use:swipeDismiss={{ direction: 'left', onDismiss, enabled }}
 * Uses the CSS `translate` property so it composes with existing transform animations.
 */
/** @param {HTMLElement} node @param {SwipeDismissOptions} params */
export function swipeDismiss(node, params) {
  let options = params;
  /** @type {{ x: number, y: number, id: number } | null} */
  let start = null;
  /** @type {'x' | 'y' | null} */
  let axis = null;
  /** @type {SwipeSample[]} */
  let samples = [];
  let suppressClick = false;
  const reduced = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
  const size = () => (AXIS[options.direction] === 'x' ? node.offsetWidth : node.offsetHeight);

  /** @param {boolean} animate */
  function reset(animate) {
    node.style.transition = animate && !reduced() ? `translate ${SWIPE_SETTLE_MS}ms ease` : '';
    node.style.translate = '';
    if (animate && !reduced()) setTimeout(() => { if (!start) node.style.transition = ''; }, SWIPE_SETTLE_MS);
  }

  /** @param {PointerEvent} event */
  function down(event) {
    if (options.enabled === false || !isSwipePointer(event.pointerType) || !event.isPrimary) return;
    if (event.target instanceof Element && event.target.closest(SWIPE_IGNORE_SELECTOR)) return;
    start = { x: event.clientX, y: event.clientY, id: event.pointerId };
    axis = null;
    samples = [{ x: event.clientX, y: event.clientY, t: event.timeStamp }];
  }

  /** @param {PointerEvent} event */
  function move(event) {
    if (!start || event.pointerId !== start.id) return;
    const dx = event.clientX - start.x;
    const dy = event.clientY - start.y;
    if (!axis) {
      axis = resolveSwipeAxis(dx, dy);
      if (!axis) return;
      if (!isSwipeAxisForDirection(options.direction, axis)) { start = null; return; }
      try { node.setPointerCapture(event.pointerId); } catch { /* pointer already gone */ }
      node.style.transition = 'none';
    }
    samples = [...samples.slice(-1), { x: event.clientX, y: event.clientY, t: event.timeStamp }];
    node.style.translate = translateFor(options.direction, getSwipeOffset(options.direction, dx, dy));
    if (event.cancelable) event.preventDefault();
  }

  /** @param {PointerEvent} event */
  function up(event) {
    if (!start || event.pointerId !== start.id) return;
    const locked = axis !== null;
    const dx = event.clientX - start.x;
    const dy = event.clientY - start.y;
    start = null;
    if (!locked) return;
    suppressClick = true;
    setTimeout(() => { suppressClick = false; }, 0);
    const offsetPx = getSwipeOffset(options.direction, dx, dy);
    const last = { x: event.clientX, y: event.clientY, t: event.timeStamp };
    const velocityPxPerMs = getSwipeVelocity(options.direction, samples[0], last);
    const sizePx = size();
    if (event.type === 'pointerup' && shouldDismissSwipe({ offsetPx, sizePx, velocityPxPerMs })) {
      if (reduced()) { node.style.transition = ''; node.style.translate = ''; options.onDismiss(); return; }
      node.style.transition = `translate ${SWIPE_SETTLE_MS}ms ease-in`;
      node.style.translate = translateFor(options.direction, Math.max(sizePx, offsetPx));
      setTimeout(() => { options.onDismiss(); node.style.transition = ''; node.style.translate = ''; }, SWIPE_SETTLE_MS);
    } else {
      reset(true);
    }
  }

  /** @param {MouseEvent} event */
  function click(event) {
    if (!suppressClick) return;
    suppressClick = false;
    event.preventDefault();
    event.stopPropagation();
  }

  node.addEventListener('pointerdown', down);
  node.addEventListener('pointermove', move);
  node.addEventListener('pointerup', up);
  node.addEventListener('pointercancel', up);
  node.addEventListener('click', click, true);
  return {
    /** @param {SwipeDismissOptions} next */
    update(next) { options = next; if (next.enabled === false) { start = null; reset(false); } },
    destroy() {
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('pointerup', up);
      node.removeEventListener('pointercancel', up);
      node.removeEventListener('click', click, true);
    }
  };
}
