/** Pages of cards prefetched on each side of the visible cards (a page = the number of cards in view). */
export const ALBUM_PREFETCH_PAGES_PER_SIDE = 1;
/** Hard cap on the whole prefetch window, however large the viewport. */
export const ALBUM_PREFETCH_MAX_CARDS = 120;
/** Quiet time after the visible range last changed before any prefetch work starts. */
export const ALBUM_PREFETCH_QUIET_MS = 200;

/**
 * Which cards (list indexes) to prefetch around the visible cards `[start, end)`, nearest first, the side the user is
 * scrolling toward before the other side. One "page" is `end - start` cards (columns x visible rows), so the window follows
 * the window size and the card-width setting; each side is limited to half of `maxCards`.
 * @param {{ start: number, end: number, total: number, direction?: number, pages?: number, maxCards?: number }} options
 * @returns {number[]}
 */
export function getAlbumPrefetchIndices({ start, end, total, direction = 1, pages = ALBUM_PREFETCH_PAGES_PER_SIDE, maxCards = ALBUM_PREFETCH_MAX_CARDS }) {
  if (![start, end, total].every(Number.isSafeInteger) || start < 0 || end <= start || total <= 0) return [];
  const cap = Math.max(0, Math.floor(maxCards));
  const sideCap = Math.floor(cap / 2);
  const span = Math.min(sideCap, Math.max(0, Math.floor(pages)) * (end - start));
  const after = [];
  for (let index = end; index < Math.min(total, end + span); index += 1) after.push(index);
  const before = [];
  for (let index = Math.min(start, total) - 1; index >= Math.max(0, start - span); index -= 1) before.push(index);
  const ordered = direction < 0 ? [...before, ...after] : [...after, ...before];
  return ordered.slice(0, cap);
}
