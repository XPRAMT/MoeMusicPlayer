import { LONG_PRESS_REORDER_MS, LONG_PRESS_REORDER_SLOP_PX } from './long-press-reorder.js';

/** Long-press delay before a queue row can be dragged (shared with track-column settings). */
export const QUEUE_REORDER_LONG_PRESS_MS = LONG_PRESS_REORDER_MS;

/** Pointer movement that cancels a pending long-press before drag starts. */
export const QUEUE_REORDER_MOVE_CANCEL_PX = LONG_PRESS_REORDER_SLOP_PX;

/**
 * Map a pointer Y inside a scrolled virtual viewport to a drop index.
 * @param {{ scrollTop: number, pointerClientY: number, viewportTop: number, rowHeight: number, totalCount: number }} args
 */
export function queueDropIndexFromPointer({
  scrollTop,
  pointerClientY,
  viewportTop,
  rowHeight,
  totalCount,
}) {
  if (!Number.isFinite(rowHeight) || rowHeight <= 0 || !Number.isSafeInteger(totalCount) || totalCount <= 0) {
    return 0;
  }
  const y = scrollTop + (pointerClientY - viewportTop);
  const index = Math.floor(y / rowHeight);
  return Math.max(0, Math.min(totalCount - 1, index));
}

/**
 * Pure reorder helper used by tests and optimistic UI previews.
 * @template T
 * @param {T[]} items
 * @param {number} from
 * @param {number} to
 */
export function moveListItem(items, from, to) {
  if (!Array.isArray(items)) return [];
  const length = items.length;
  if (!Number.isSafeInteger(from) || !Number.isSafeInteger(to) || from < 0 || to < 0 || from >= length || to >= length) {
    return items.slice();
  }
  if (from === to) return items.slice();
  const next = items.slice();
  const [moved] = next.splice(from, 1);
  next.splice(to, 0, moved);
  return next;
}
