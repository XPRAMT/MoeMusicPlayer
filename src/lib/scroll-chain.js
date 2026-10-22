/**
 * Wheel scroll chaining without the pause (Svelte action `use:scrollChain`).
 *
 * Browsers latch a wheel gesture to the element it started in: once an inner list reaches its end, the rest of that
 * gesture does nothing, and only a new gesture after a short pause scrolls the page around it. This action hands the
 * part of a vertical wheel step the element cannot use to the nearest scrollable ancestor, in the same event, so the
 * outer page moves on at once, with the browser's own smooth scrolling (`behavior: 'smooth'`; instant under
 * `prefers-reduced-motion: reduce`). Horizontal wheels, ctrl+wheel (zoom) and wheels a control inside already used
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

/** A pending smooth-scroll target older than this (or already reached) is dropped; the next step starts from scrollTop. */
export const SMOOTH_TARGET_TTL_MS = 450;

/**
 * Where a smooth scroll of `delta` should end. Consecutive wheel steps add up on the pending target instead of
 * restarting from the half-way `scrollTop` (which would lose distance); the target is dropped once it is reached,
 * once it is stale, or when the step goes the other way (the user turned around or scrolled the box themselves).
 * @param {{ target: number, at: number } | null} pending
 * @param {number} current scrollTop now
 * @param {number} delta pixels
 * @param {number} max scrollHeight - clientHeight
 * @param {number} now ms
 * @returns {number}
 */
export function nextSmoothTarget(pending, current, delta, max, now) {
  let base = current;
  if (pending && now - pending.at < SMOOTH_TARGET_TTL_MS && Math.abs(pending.target - current) > 1
    && Math.sign(pending.target - current) === Math.sign(delta)) {
    base = pending.target;
  }
  return Math.min(Math.max(0, max), Math.max(0, base + delta));
}

/** 'auto' when the system asks for reduced motion, otherwise the browser's own smooth scrolling. */
function scrollBehavior() {
  return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth';
}

/** @type {WeakMap<Element, { target: number, at: number }>} */
const pendingTargets = new WeakMap();

/** The scrollTop an element is heading to (its pending smooth target), or where it is. @param {Element} element @param {number} now */
function headingTop(element, now) {
  const pending = pendingTargets.get(element);
  if (pending && now - pending.at < SMOOTH_TARGET_TTL_MS && Math.abs(pending.target - element.scrollTop) > 1) return pending.target;
  return element.scrollTop;
}

/** Smoothly scroll `element` by `delta`, accumulating on its pending target. @param {Element} element @param {number} delta @param {number} now */
function smoothScrollBy(element, delta, now) {
  const max = element.scrollHeight - element.clientHeight;
  const target = nextSmoothTarget(pendingTargets.get(element) ?? null, element.scrollTop, delta, max, now);
  pendingTargets.set(element, { target, at: now });
  element.scrollTo({ top: target, behavior: scrollBehavior() });
}

/** @param {HTMLElement} node */
export function scrollChain(node) {
  /** @param {WheelEvent} event */
  function onWheel(event) {
    if (event.defaultPrevented || event.ctrlKey) return;
    if (Math.abs(event.deltaX) > Math.abs(event.deltaY)) return;
    const now = performance.now();
    const delta = wheelDeltaPx(event.deltaY, event.deltaMode, node.clientHeight);
    const top = headingTop(node, now);
    const { inner, outer } = splitWheelDelta(delta, { scrollTop: top, scrollHeight: node.scrollHeight, clientHeight: node.clientHeight });
    // The list can take the whole step and no smooth scroll of ours is running on it: native scrolling, unchanged.
    if (outer === 0 && top === node.scrollTop) return;
    const parent = outer === 0 ? null : findScrollParent(node);
    if (outer !== 0 && !parent) return;
    event.preventDefault();
    if (inner !== 0) smoothScrollBy(node, inner, now);
    if (parent) smoothScrollBy(parent, outer, now);
  }
  function onScrollEnd() {
    pendingTargets.delete(node);
  }
  node.addEventListener('wheel', onWheel, { passive: false });
  node.addEventListener('scrollend', onScrollEnd);
  return {
    destroy() {
      node.removeEventListener('wheel', onWheel);
      node.removeEventListener('scrollend', onScrollEnd);
    },
  };
}
