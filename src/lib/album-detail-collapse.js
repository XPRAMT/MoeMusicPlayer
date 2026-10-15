export const MIN_ALBUM_DETAIL_COVER_SIZE_PX = 64;

/** The maximum amount of the fixed album heading that can be uncovered. */
/** @param {number} baseSize @returns {number} */
export function getAlbumDetailCollapseRange(baseSize) {
  const base = Number.isFinite(baseSize) ? Math.max(MIN_ALBUM_DETAIL_COVER_SIZE_PX, baseSize) : 148;
  return Math.max(0, base - MIN_ALBUM_DETAIL_COVER_SIZE_PX);
}

/** Shrink one CSS pixel per scroll pixel, without going below the compact cover size. */
/** @param {number} baseSize @param {number} scrollPeak @returns {number} */
export function getAlbumDetailCoverSize(baseSize, scrollPeak) {
  const base = Number.isFinite(baseSize) ? Math.max(MIN_ALBUM_DETAIL_COVER_SIZE_PX, baseSize) : 148;
  const peak = Number.isFinite(scrollPeak) ? Math.max(0, scrollPeak) : 0;
  return Math.max(MIN_ALBUM_DETAIL_COVER_SIZE_PX, base - peak);
}

/**
 * Viewport top of the album-detail track scroller: the fully collapsed heading
 * plus the sortable header. The scroller never moves or resizes while scrolling.
 *
 * @param {number} headingHeight @param {number} collapseRange @param {number} tableHeaderHeight
 * @returns {number}
 */
export function getAlbumDetailScrollerTop(headingHeight, collapseRange, tableHeaderHeight) {
  const heading = Number.isFinite(headingHeight) ? Math.max(0, headingHeight) : 0;
  const range = Number.isFinite(collapseRange) ? Math.max(0, collapseRange) : 0;
  const header = Number.isFinite(tableHeaderHeight) ? Math.max(0, tableHeaderHeight) : 0;
  return Math.max(0, heading - range) + header;
}

/**
 * Split a native scroll offset into cover collapse and track-list offset. The
 * scroller content starts with a spacer of `collapseRange` px, so the first
 * pixels collapse the cover and later pixels scroll the tracks. The mapping is
 * a pure function of the native offset in both directions.
 *
 * @param {number} nativeScrollTop @param {number} collapseRange
 * @returns {{collapseOffset:number,listScrollTop:number}}
 */
export function getAlbumDetailScrollState(nativeScrollTop, collapseRange) {
  const native = Number.isFinite(nativeScrollTop) ? Math.max(0, nativeScrollTop) : 0;
  const range = Number.isFinite(collapseRange) ? Math.max(0, collapseRange) : 0;
  return {
    collapseOffset: Math.min(range, native),
    listScrollTop: Math.max(0, native - range),
  };
}

/**
 * Native scroll offset for a logical track-list position. A positive list
 * offset implies a fully collapsed cover; at the list top the requested
 * collapse is kept.
 *
 * @param {number} listScrollTop @param {number} collapseRange @param {number} [collapseOffset]
 * @returns {number}
 */
export function getAlbumDetailNativeScrollTop(listScrollTop, collapseRange, collapseOffset = 0) {
  const list = Number.isFinite(listScrollTop) ? Math.max(0, listScrollTop) : 0;
  const range = Number.isFinite(collapseRange) ? Math.max(0, collapseRange) : 0;
  const collapse = Number.isFinite(collapseOffset) ? Math.min(range, Math.max(0, collapseOffset)) : 0;
  return list > 0 ? list + range : collapse;
}