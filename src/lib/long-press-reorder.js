/**
 * Shared long-press drag-reorder rules for touch WebViews (Play Queue rows, Settings > track columns).
 *
 * Rows keep `touch-action: pan-x pan-y` so a quick swipe scrolls natively (both axes). Pointer events
 * alone cannot stop the browser from claiming that pan: once the finger moves after the long-press,
 * Chromium/Android WebView starts scrolling and fires `pointercancel`, killing the drag. The standard
 * fix is a non-passive `touchmove` listener that calls preventDefault while a drag is active – the
 * long-press fires before any movement beyond the slop, so the very first move is already blocked
 * and the browser never starts panning. Moving beyond the slop before the timer cancels the
 * long-press and the gesture stays a normal scroll.
 */

/** Hold time before a row can be dragged. */
export const LONG_PRESS_REORDER_MS = 300;
/** Movement (CSS px) that cancels a pending long-press so the gesture becomes a scroll. */
export const LONG_PRESS_REORDER_SLOP_PX = 8;

/** @param {number} dx @param {number} dy @param {number} [slopPx] */
export function exceedsLongPressSlop(dx, dy, slopPx = LONG_PRESS_REORDER_SLOP_PX) {
  return dx * dx + dy * dy > slopPx * slopPx;
}

/**
 * @typedef {'idle' | 'pending' | 'dragging'} LongPressPhase
 * @param {LongPressPhase} phase
 * @param {{ type: 'down' } | { type: 'move', dx: number, dy: number } | { type: 'timer' } | { type: 'up' } | { type: 'cancel' }} event
 * @returns {LongPressPhase}
 */
export function reduceLongPressPhase(phase, event) {
  switch (event.type) {
    case 'down':
      return 'pending';
    case 'move':
      if (phase === 'pending' && exceedsLongPressSlop(event.dx, event.dy)) return 'idle';
      return phase;
    case 'timer':
      return phase === 'pending' ? 'dragging' : phase;
    default:
      return 'idle';
  }
}

/** Only an active drag blocks native touch scrolling; pending and idle let the browser pan. @param {LongPressPhase} phase */
export function shouldBlockTouchScroll(phase) {
  return phase === 'dragging';
}

/**
 * Svelte action: while `isDragging()` is true, prevent native touch panning (non-passive touchmove)
 * and the long-press context menu / text selection callout.
 * @param {HTMLElement} node
 * @param {() => boolean} isDragging
 */
export function longPressDragGuard(node, isDragging) {
  let check = isDragging;
  const onTouchMove = (/** @type {TouchEvent} */ event) => {
    if (check() && event.cancelable) event.preventDefault();
  };
  const onContextMenu = (/** @type {Event} */ event) => {
    if (check()) event.preventDefault();
  };
  node.addEventListener('touchmove', onTouchMove, { passive: false });
  node.addEventListener('contextmenu', onContextMenu);
  return {
    /** @param {() => boolean} next */
    update(next) { check = next; },
    destroy() {
      node.removeEventListener('touchmove', onTouchMove);
      node.removeEventListener('contextmenu', onContextMenu);
    },
  };
}
