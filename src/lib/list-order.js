/** Format a zero-based row position as the one-based # shown in a list. */
/** @param {number} position */
export function formatListOrder(position) {
  if (!Number.isSafeInteger(position) || position < 0 || position >= Number.MAX_SAFE_INTEGER) {
    return '—';
  }
  return String(position + 1);
}
