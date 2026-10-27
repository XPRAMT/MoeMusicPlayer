// Shared touch/pen swipe-to-dismiss gesture (Svelte action) plus pure decision helpers.
// Mouse input is ignored unless the element opts in with `mouseActions` (the desktop now-playing page), and then only
// those actions apply to a mouse drag with the primary button.

export const SWIPE_AXIS_SLOP_PX = 10;
export const SWIPE_DISMISS_DISTANCE_RATIO = 0.3;
export const SWIPE_DISMISS_VELOCITY_PX_PER_MS = 0.5;
export const SWIPE_SETTLE_MS = 180;
export const SWIPE_IGNORE_SELECTOR = 'input, select, textarea, [role="slider"], [contenteditable=""], [contenteditable="true"], [data-swipe-ignore]';

/** @typedef {'left' | 'right' | 'down' | 'up'} SwipeDirection */
/** @typedef {{ x: number, y: number, t: number }} SwipeSample */
/**
 * Either one dismiss direction (`direction` + `onDismiss`) or several (`actions`). `follow: false` keeps the
 * element still: nothing moves while dragging and the action fires only on release. `feedbackTarget` makes the
 * `damped` rubber-band effect play on another element (the lyrics area swipe moves the cover). `mouseActions` turns on
 * mouse dragging with its own set of actions (touch and pen keep using `actions`).
 * @typedef {{ direction?: SwipeDirection, onDismiss?: () => void, actions?: Partial<Record<SwipeDirection, () => void>>, mouseActions?: Partial<Record<SwipeDirection, () => void>>, follow?: boolean, damped?: boolean, enabled?: boolean, feedbackTarget?: () => HTMLElement | null }} SwipeDismissOptions
 */

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

/** Direction of a locked swipe from the sign of the locked axis. @param {'x' | 'y'} axis @param {number} dx @param {number} dy @returns {SwipeDirection} */
export function resolveSwipeDirection(axis, dx, dy) {
  if (axis === 'x') return dx < 0 ? 'left' : 'right';
  return dy > 0 ? 'down' : 'up';
}

/** @param {SwipeDismissOptions} options @returns {Partial<Record<SwipeDirection, () => void>>} */
export function getSwipeActions(options) {
  if (options.actions) return options.actions;
  return options.direction && options.onDismiss ? { [options.direction]: options.onDismiss } : {};
}

/** The actions that apply to this kind of pointer: `mouseActions` for a mouse, the regular ones for touch and pen. @param {SwipeDismissOptions} options @param {string} pointerType */
export function getSwipeActionsFor(options, pointerType) {
  return pointerType === 'mouse' ? options.mouseActions ?? {} : getSwipeActions(options);
}

/** Elements that use a mouse drag themselves: the enlarged cover pans (a shrunk cover keeps the swipe). */
export const SWIPE_MOUSE_DRAG_OWNER_SELECTOR = '[data-cover-pannable]';

/** A mouse drag starts a swipe only with the primary button, and not on an element that uses the drag itself (the enlarged cover pans). @param {{ pointerType: string, button: number }} event @param {boolean} onDraggableTarget */
export function isSwipeMouseStart(event, onDraggableTarget) {
  return event.pointerType === 'mouse' && event.button === 0 && !onDraggableTarget;
}

/** @param {string} pointerType */
export function isSwipePointer(pointerType) {
  return pointerType === 'touch' || pointerType === 'pen';
}

/** Max travel (px) of a `damped` swipe: subtle rubber-band feedback that never reveals the edge. */
export const SWIPE_DAMPED_MAX_PX = 24;
/** Rubber-band curve: ~1:1 at first, asymptotically capped at maxPx. @param {number} offsetPx */
export function getDampedSwipeOffset(offsetPx, maxPx = SWIPE_DAMPED_MAX_PX) {
  if (!Number.isFinite(offsetPx) || offsetPx <= 0) return 0;
  return maxPx * (1 - Math.exp(-offsetPx / (maxPx * 3)));
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
  /** @type {SwipeDirection | null} */
  let direction = null;
  let pointerKind = 'touch';
  let suppressClick = false;
  const reduced = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
  const follows = () => options.follow !== false;
  /** The element the rubber-band feedback plays on (the node itself unless `feedbackTarget` names another). */
  const fxNode = () => options.feedbackTarget?.() ?? node;
  const damped = () => options.follow === false && options.damped === true && !reduced();

  /** @param {boolean} animate */
  function reset(animate) {
    const fx = fxNode();
    fx.style.transition = animate && !reduced() ? `translate ${SWIPE_SETTLE_MS}ms ease, scale ${SWIPE_SETTLE_MS}ms ease, opacity ${SWIPE_SETTLE_MS}ms ease` : '';
    fx.style.translate = '';
    fx.style.scale = '';
    fx.style.opacity = '';
    if (animate && !reduced()) setTimeout(() => { if (!start) fx.style.transition = ''; }, SWIPE_SETTLE_MS);
  }

  /** @param {PointerEvent} event */
  function down(event) {
    if (options.enabled === false || !event.isPrimary) return;
    const target = event.target instanceof Element ? event.target : null;
    if (event.pointerType === 'mouse') {
      if (!options.mouseActions || !isSwipeMouseStart(event, target?.closest(SWIPE_MOUSE_DRAG_OWNER_SELECTOR) != null)) return;
    } else if (!isSwipePointer(event.pointerType)) return;
    if (target?.closest(SWIPE_IGNORE_SELECTOR)) return;
    pointerKind = event.pointerType;
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
      const resolved = resolveSwipeDirection(axis, dx, dy);
      if (!getSwipeActionsFor(options, pointerKind)[resolved]) { start = null; return; }
      direction = resolved;
      // A mouse drag would otherwise select the text under it.
      if (pointerKind === 'mouse') { try { getSelection()?.removeAllRanges(); } catch { /* no selection API */ } }
      try { node.setPointerCapture(event.pointerId); } catch { /* pointer already gone */ }
      if (follows()) node.style.transition = 'none';
      else if (damped()) fxNode().style.transition = 'none';
      // Promote to its own compositor layer only while dragging (see .is-swiping in app.css).
      if (follows()) node.classList.add('is-swiping');
    }
    if (!direction) return;
    samples = [...samples.slice(-1), { x: event.clientX, y: event.clientY, t: event.timeStamp }];
    if (follows()) node.style.translate = translateFor(direction, getSwipeOffset(direction, dx, dy));
    else if (damped()) {
      const d = getDampedSwipeOffset(getSwipeOffset(direction, dx, dy));
      const k = d / SWIPE_DAMPED_MAX_PX;
      const fx = fxNode();
      fx.style.translate = translateFor(direction, d);
      fx.style.scale = String(1 - 0.03 * k);
      fx.style.opacity = String(1 - 0.15 * k);
    }
    if (event.cancelable) event.preventDefault();
  }

  /** @param {PointerEvent} event */
  function up(event) {
    if (!start || event.pointerId !== start.id) return;
    node.classList.remove('is-swiping');
    const locked = axis !== null;
    const dx = event.clientX - start.x;
    const dy = event.clientY - start.y;
    start = null;
    const dir = direction;
    direction = null;
    // Release capture right away so a dismiss that hides this node never keeps the next touch.
    try { if (node.hasPointerCapture?.(event.pointerId)) node.releasePointerCapture(event.pointerId); } catch { /* gone */ }
    if (!locked || !dir) return;
    const action = getSwipeActionsFor(options, pointerKind)[dir];
    if (!action) { if (damped()) reset(true); return; }
    if (options.damped) reset(true);
    suppressClick = true;
    setTimeout(() => { suppressClick = false; }, 0);
    const offsetPx = getSwipeOffset(dir, dx, dy);
    const last = { x: event.clientX, y: event.clientY, t: event.timeStamp };
    const velocityPxPerMs = getSwipeVelocity(dir, samples[0], last);
    const sizePx = AXIS[dir] === 'x' ? node.offsetWidth : node.offsetHeight;
    if (event.type === 'pointerup' && shouldDismissSwipe({ offsetPx, sizePx, velocityPxPerMs })) {
      if (!follows() || reduced()) { node.style.transition = ''; node.style.translate = ''; action(); return; }
      node.style.transition = `translate ${SWIPE_SETTLE_MS}ms ease-in`;
      node.style.translate = translateFor(dir, Math.max(sizePx, offsetPx));
      setTimeout(() => { action(); node.style.transition = ''; node.style.translate = ''; }, SWIPE_SETTLE_MS);
    } else if (follows()) {
      reset(true);
    }
  }

  // Pointer preventDefault does not stop the native gesture: without this the WebView starts a
  // scroll/fling for the locked swipe, and a tap while that fling runs only cancels it (no click),
  // which left the dock dead for a moment after collapsing the playback page from the cover.
  /** @param {TouchEvent} event */
  function touchMove(event) {
    if (direction && event.cancelable) event.preventDefault();
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
  node.addEventListener('touchmove', touchMove, { passive: false });
  return {
    /** @param {SwipeDismissOptions} next */
    update(next) { options = next; if (next.enabled === false) { start = null; reset(false); } },
    destroy() {
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('pointerup', up);
      node.removeEventListener('pointercancel', up);
      node.removeEventListener('click', click, true);
      node.removeEventListener('touchmove', touchMove);
    }
  };
}
