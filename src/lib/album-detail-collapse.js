export const MIN_ALBUM_DETAIL_COVER_SIZE_PX = 64;

/** Keep the furthest downward scroll distance until the list returns to its top. */
/** @param {number} previousPeak @param {number} scrollTop @returns {number} */
export function updateAlbumDetailScrollPeak(previousPeak, scrollTop) {
  const current = Number.isFinite(scrollTop) ? Math.max(0, scrollTop) : 0;
  if (current === 0) return 0;
  const previous = Number.isFinite(previousPeak) ? Math.max(0, previousPeak) : 0;
  return Math.max(previous, current);
}

/** Shrink one CSS pixel per scroll pixel, without going below the compact cover size. */
/** @param {number} baseSize @param {number} scrollPeak @returns {number} */
export function getAlbumDetailCoverSize(baseSize, scrollPeak) {
  const base = Number.isFinite(baseSize) ? Math.max(MIN_ALBUM_DETAIL_COVER_SIZE_PX, baseSize) : 148;
  const peak = Number.isFinite(scrollPeak) ? Math.max(0, scrollPeak) : 0;
  return Math.max(MIN_ALBUM_DETAIL_COVER_SIZE_PX, base - peak);
}
