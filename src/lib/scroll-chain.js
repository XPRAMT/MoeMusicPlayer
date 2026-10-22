/**
 * Wheel scroll chaining without the pause (Svelte action `use:scrollChain`).
 *
 * Browsers latch a wheel gesture to the element it started in: once an inner list reaches its end, the rest of that
 * gesture does nothing, and only a new gesture after a short pause scrolls the page around it. This action hands the
 * part of a vertical wheel step the element cannot use to the nearest scrollable ancestor, in the same event, so the
 * outer page moves on at once. Horizontal wheels, ctrl+wheel (zoom) and wheels a control inside already used
 * (`defaultPrevented`, e.g. the delay buttons or the channel-matrix cells) are left alone. Touch is not handled: touch
 * scrolling already chains natively once `overscroll-behavior` is not set.
 */

/** Pixels per line when the wheel reports lines (Firefox, some mice); browsers use about this. */
export const WHEEL_LINE_PX = 16;

/**
 * A wheel delta in CSS pixels.
 * @param {number} delta `deltaY`
 * @param {number} mode `deltaMode`: 0 pixels, 1 lines, 2 pages
 * @param {number} pageHeight the scrolled element's visible height, for page mode
 */
export function wheelDeltaPx(delta, mode, pageHeight) {
  if (!Number.isFinite(delta)) return 0;
  if (mode === 1) return delta * WHEEL_LINE_PX;
  if (mode === 2) return delta * Math.max(0, pageHeight);
  return delta;
}

/**
 * How much of a vertical step the element itself takes and how much is left over for the outer scroller.
 * @param {number} delta pixels, positive = down
 * @param {{ scrollTop: number, scrollHeight: number, clientHeight: number }} box
 * @returns {{ inner: number, outer: number }}
 */
export function splitWheelDelta(delta, box) {
  const max = Math.max(0, box.scrollHeight - box.clientHeight);
  const top = Math.min(max, Math.max(0, box.scrollTop));
  const room = delta > 0 ? max - top : -top;
  if (delta === 0 || Math.abs(room) >= Math.abs(delta)) return { inner: delta, outer: 0 };
  return { inner: room + 0, outer: delta - room }; // + 0 turns -0 into 0
}

/** @param {Element} element */
function canScrollY(element) {
  const overflowY = getComputedStyle(element).overflowY;
  return (overflowY === 'auto' || overflowY === 'scroll' || overflowY === 'overlay')
    && element.scrollHeight > element.clientHeight;
}

/** Nearest ancestor that scrolls vertically (the settings page, the quick-settings drawer, …). @param {Element} element */
export function findScrollParent(element) {
  for (let parent = element.parentElement; parent; parent = parent.parentElement) {
    if (canScrollY(parent)) return parent;
  }
  return document.scrollingElement instanceof HTMLElement ? document.scrollingElement : null;
}

/** @param {HTMLElement} node */
export function scrollChain(node) {
  /** @param {WheelEvent} event */
  function onWheel(event) {
    if (event.defaultPrevented || event.ctrlKey) return;
    if (Math.abs(event.deltaX) > Math.abs(event.deltaY)) return;
    const delta = wheelDeltaPx(event.deltaY, event.deltaMode, node.clientHeight);
    const { inner, outer } = splitWheelDelta(delta, node);
    if (outer === 0) return; // the list can take the whole step: native scrolling, unchanged
    const parent = findScrollParent(node);
    if (!parent) return;
    event.preventDefault();
    if (inner !== 0) node.scrollBy({ top: inner, behavior: 'auto' });
    parent.scrollBy({ top: outer, behavior: 'auto' });
  }
  node.addEventListener('wheel', onWheel, { passive: false });
  return {
    destroy() {
      node.removeEventListener('wheel', onWheel);
    },
  };
}