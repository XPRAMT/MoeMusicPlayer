// Placing a `position: fixed` element at viewport coordinates when an ancestor may be its containing block.
//
// An ancestor with transform, filter, backdrop-filter, perspective, paint/layout containment, a matching will-change
// or a container-type becomes the containing block of fixed-position descendants (the quick-settings drawer has
// `backdrop-filter`). `left`/`top` are then measured from that ancestor's padding box and scroll with its content,
// so viewport coordinates have to be converted first.

/**
 * @typedef {{ left: number, top: number, width: number, height: number, clientLeft?: number, clientTop?: number,
 *   clientWidth?: number, clientHeight?: number, scrollLeft?: number, scrollTop?: number }} ContainingBlockBox
 */

const NONE_PROPERTIES = ['transform', 'filter', 'backdropFilter', 'webkitBackdropFilter', 'perspective', 'translate', 'rotate', 'scale'];

/** Whether a computed style (or a plain object with the same camelCase properties) makes its element the containing
 *  block of fixed-position descendants.
 *  @param {Record<string, any> | null | undefined} style */
export function createsFixedContainingBlock(style) {
  if (!style) return false;
  for (const property of NONE_PROPERTIES) {
    const value = style[property];
    if (typeof value === 'string' && value !== '' && value !== 'none') return true;
  }
  const contain = typeof style.contain === 'string' ? style.contain : '';
  if (/\b(paint|layout|strict|content)\b/.test(contain)) return true;
  const willChange = typeof style.willChange === 'string' ? style.willChange : '';
  if (/\b(transform|filter|backdrop-filter|perspective|translate|rotate|scale|contain)\b/.test(willChange)) return true;
  const containerType = typeof style.containerType === 'string' ? style.containerType : '';
  if (containerType !== '' && containerType !== 'normal') return true;
  return false;
}

/** The nearest ancestor of `element` that is the containing block of a fixed-position `element`, or null when the
 *  viewport is. `getStyle` defaults to `getComputedStyle`.
 *  @template {{ parentElement: any }} T
 *  @param {T | null | undefined} element
 *  @param {(node: any) => Record<string, any> | null | undefined} [getStyle]
 *  @returns {any} */
export function findFixedContainingBlock(element, getStyle = (node) => getComputedStyle(node)) {
  for (let node = element?.parentElement ?? null; node; node = node.parentElement) {
    if (createsFixedContainingBlock(getStyle(node))) return node;
  }
  return null;
}

/** The geometry `fixedPlacement` needs from a containing-block element (null stays null).
 *  @param {Element | null | undefined} element
 *  @returns {ContainingBlockBox | null} */
export function measureContainingBlock(element) {
  if (!element) return null;
  const rect = element.getBoundingClientRect();
  return {
    left: rect.left,
    top: rect.top,
    width: rect.width,
    height: rect.height,
    clientLeft: element.clientLeft,
    clientTop: element.clientTop,
    clientWidth: element.clientWidth,
    clientHeight: element.clientHeight,
    scrollLeft: element.scrollLeft,
    scrollTop: element.scrollTop,
  };
}

/** Viewport point → the `left`/`top` a fixed element needs inside `block` (as returned by `measureContainingBlock`;
 *  null = the viewport, returned unchanged).
 *  @param {number} x
 *  @param {number} y
 *  @param {ContainingBlockBox | null | undefined} block */
export function viewportToContainingBlock(x, y, block) {
  if (!block) return { left: x, top: y };
  return {
    left: x - block.left - (block.clientLeft ?? 0) + (block.scrollLeft ?? 0),
    top: y - block.top - (block.clientTop ?? 0) + (block.scrollTop ?? 0),
  };
}

/** @param {number} centre @param {number} size @param {number} min @param {number} max */
function clampCentre(centre, size, min, max) {
  const low = min + size / 2;
  const high = max - size / 2;
  if (low > high) return (min + max) / 2;
  return Math.min(high, Math.max(low, centre));
}

/**
 * Where to put a fixed element centred (via `translate(-50%, -50%)`) on the viewport point `centreX`/`centreY`:
 * the centre is clamped so the `width`×`height` box stays `margin` px inside the viewport and, when there is a
 * containing block, inside that block's visible padding box too; the result is in the block's coordinate space.
 * @param {{ centreX: number, centreY: number, width: number, height: number, viewportWidth: number,
 *   viewportHeight: number, block?: ContainingBlockBox | null, margin?: number }} options
 */
export function fixedPlacement({ centreX, centreY, width, height, viewportWidth, viewportHeight, block = null, margin = 12 }) {
  let minX = 0;
  let minY = 0;
  let maxX = viewportWidth;
  let maxY = viewportHeight;
  if (block) {
    const innerLeft = block.left + (block.clientLeft ?? 0);
    const innerTop = block.top + (block.clientTop ?? 0);
    minX = Math.max(minX, innerLeft);
    minY = Math.max(minY, innerTop);
    maxX = Math.min(maxX, innerLeft + (block.clientWidth ?? block.width));
    maxY = Math.min(maxY, innerTop + (block.clientHeight ?? block.height));
  }
  const x = clampCentre(centreX, width, minX + margin, maxX - margin);
  const y = clampCentre(centreY, height, minY + margin, maxY - margin);
  const placed = viewportToContainingBlock(x, y, block);
  return { left: placed.left, top: placed.top, viewportX: x, viewportY: y };
}
