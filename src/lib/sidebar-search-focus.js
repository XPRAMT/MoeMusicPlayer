/**
 * The sidebar search field scrolls with the nav items inside `.sidebar-scroll`. When it is
 * focused programmatically while scrolled out of view, the whole field (border included,
 * plus its `scroll-margin`) must come back into view inside that scroller only — never by
 * scrolling the app shell or other ancestors.
 */

/**
 * Scroll delta (CSS px, added to scrollTop) that brings the target fully into the container.
 * 0 when it is already fully visible. Prefers showing the target's top when it is taller.
 * @param {{ top: number, bottom: number }} container
 * @param {{ top: number, bottom: number }} target
 * @param {number} [marginPx]
 */
export function scrollDeltaToReveal(container, target, marginPx = 0) {
  const margin = Number.isFinite(marginPx) && marginPx > 0 ? marginPx : 0;
  const top = target.top - margin;
  const bottom = target.bottom + margin;
  if (top < container.top) return top - container.top;
  if (bottom > container.bottom) return Math.min(bottom - container.bottom, top - container.top);
  return 0;
}

/**
 * Focus the search input and reveal its field inside the sidebar scroller.
 * @param {HTMLInputElement | undefined | null} input
 */
export function focusSidebarSearch(input) {
  if (!input) return;
  input.focus({ preventScroll: true });
  const field = input.closest('.sidebar-search-field');
  const scroller = input.closest('.sidebar-scroll');
  if (!(field instanceof HTMLElement) || !(scroller instanceof HTMLElement)) return;
  const margin = Number.parseFloat(getComputedStyle(field).scrollMarginTop) || 0;
  const delta = scrollDeltaToReveal(scroller.getBoundingClientRect(), field.getBoundingClientRect(), margin);
  if (delta !== 0) scroller.scrollTop += delta;
}
