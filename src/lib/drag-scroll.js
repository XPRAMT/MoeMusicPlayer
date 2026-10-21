// Drag-to-scroll for list viewports (Svelte action).
//
// Input that arrives as plain pointer movement (a mouse, or a touch screen driven through remote-control software on
// Windows) cannot pan a scroll container natively, and a drag across the rows used to select their text instead.
// Native touch panning still wins where it works: the browser then ends our pointer with `pointercancel` and we stop.

export const DRAG_SCROLL_SLOP_PX = 6;
export const DRAG_SCROLL_IGNORE_SELECTOR = 'input, select, textarea, [role="slider"], [contenteditable=""], [contenteditable="true"], [data-drag-scroll-ignore]';

/** Whether a press at (x, y) inside the node's box lands on its own scrollbar. @param {{ clientWidth: number, clientHeight: number }} node @param {{ x: number, y: number }} local */
export function isOnScrollbar(node, local) {
  return local.x >= node.clientWidth || local.y >= node.clientHeight;
}

/** Scroll offsets after dragging by (dx, dy) from the offsets at the press, clamped to the scrollable range. @param {{ left: number, top: number }} origin @param {number} dx @param {number} dy @param {{ maxLeft: number, maxTop: number }} limits */
export function dragScrollOffsets(origin, dx, dy, limits) {
  return {
    left: Math.min(Math.max(0, origin.left - dx), Math.max(0, limits.maxLeft)),
    top: Math.min(Math.max(0, origin.top - dy), Math.max(0, limits.maxTop)),
  };
}

/**
 * use:dragScroll={() => dragIsBlocked}
 * @param {HTMLElement} node
 * @param {() => boolean} [blocked] true while something else (a row reorder) owns the drag
 */
export function dragScroll(node, blocked = () => false) {
  let isBlocked = blocked;
  /** @type {{ id: number, x: number, y: number, left: number, top: number } | null} */
  let start = null;
  let dragging = false;
  let suppressClick = false;

  /** @param {PointerEvent} event */
  function down(event) {
    if (!event.isPrimary || event.button !== 0 || isBlocked()) return;
    if (event.target instanceof Element && event.target.closest(DRAG_SCROLL_IGNORE_SELECTOR)) return;
    const box = node.getBoundingClientRect();
    if (isOnScrollbar(node, { x: event.clientX - box.left, y: event.clientY - box.top })) return;
    start = { id: event.pointerId, x: event.clientX, y: event.clientY, left: node.scrollLeft, top: node.scrollTop };
    dragging = false;
  }

  /** @param {PointerEvent} event */
  function move(event) {
    if (!start || event.pointerId !== start.id) return;
    if (isBlocked()) { end(event, false); return; }
    const dx = event.clientX - start.x;
    const dy = event.clientY - start.y;
    if (!dragging) {
      if (Math.hypot(dx, dy) < DRAG_SCROLL_SLOP_PX) return;
      dragging = true;
      try { node.setPointerCapture(event.pointerId); } catch { /* pointer already gone */ }
      try { getSelection()?.removeAllRanges(); } catch { /* no selection API */ }
    }
    const next = dragScrollOffsets(start, dx, dy, {
      maxLeft: node.scrollWidth - node.clientWidth,
      maxTop: node.scrollHeight - node.clientHeight,
    });
    node.scrollLeft = next.left;
    node.scrollTop = next.top;
    if (event.cancelable) event.preventDefault();
  }

  /** @param {PointerEvent} event @param {boolean} [swallowClick] */
  function end(event, swallowClick = true) {
    if (!start || event.pointerId !== start.id) return;
    const wasDragging = dragging;
    start = null;
    dragging = false;
    try { if (node.hasPointerCapture?.(event.pointerId)) node.releasePointerCapture(event.pointerId); } catch { /* gone */ }
    if (wasDragging && swallowClick && event.type === 'pointerup') {
      suppressClick = true;
      setTimeout(() => { suppressClick = false; }, 0);
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
  node.addEventListener('pointerup', end);
  node.addEventListener('pointercancel', end);
  node.addEventListener('click', click, true);
  return {
    /** @param {() => boolean} next */
    update(next) { isBlocked = next ?? (() => false); },
    destroy() {
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('pointerup', end);
      node.removeEventListener('pointercancel', end);
      node.removeEventListener('click', click, true);
    },
  };
}
