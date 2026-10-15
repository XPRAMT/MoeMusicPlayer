export const MIN_ALBUM_DETAIL_COVER_SIZE_PX = 64;

/** The maximum amount of the fixed album heading that can be uncovered. */
/** @param {number} baseSize @returns {number} */
export function getAlbumDetailCollapseRange(baseSize) {
  const base = Number.isFinite(baseSize) ? Math.max(MIN_ALBUM_DETAIL_COVER_SIZE_PX, baseSize) : 148;
  return Math.max(0, base - MIN_ALBUM_DETAIL_COVER_SIZE_PX);
}

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

/**
 * Convert a native scroll offset in the stable, full-page scrollport into the
 * logical track-list offset and the cover collapse offset. Downward movement
 * first advances both by one pixel; after the cover is fully collapsed it
 * advances only the track list. Upward movement keeps the cover collapsed
 * until the logical list reaches its top.
 *
 * @param {{rawScrollTop:number,previousRawScrollTop:number,previousListScrollTop:number,previousCollapseOffset:number,baseCoverSize:number}} input
 */
export function mapAlbumDetailNativeScroll(input) {
  const raw = Number.isFinite(input.rawScrollTop) ? Math.max(0, input.rawScrollTop) : 0;
  const previousRaw = Number.isFinite(input.previousRawScrollTop) ? Math.max(0, input.previousRawScrollTop) : 0;
  const previousList = Number.isFinite(input.previousListScrollTop) ? Math.max(0, input.previousListScrollTop) : 0;
  const range = getAlbumDetailCollapseRange(input.baseCoverSize);
  const previousCollapse = Number.isFinite(input.previousCollapseOffset)
    ? Math.min(range, Math.max(0, input.previousCollapseOffset))
    : 0;

  if (raw >= previousRaw) {
    let remaining = raw - previousRaw;
    const collapseStep = Math.min(range - previousCollapse, remaining / 2);
    const collapseOffset = previousCollapse + collapseStep;
    remaining -= collapseStep * 2;
    return {
      listScrollTop: previousList + collapseStep + remaining,
      collapseOffset,
      rawScrollTop: raw,
    };
  }

  const listScrollTop = Math.max(0, previousList - (previousRaw - raw));
  if (listScrollTop === 0) {
    return { listScrollTop: 0, collapseOffset: 0, rawScrollTop: 0 };
  }
  return {
    listScrollTop,
    collapseOffset: previousCollapse,
    rawScrollTop: listScrollTop + previousCollapse,
  };
}

/** Physical scroll position for restoring a saved logical list position. */
/** @param {number} listScrollTop @param {number} collapseOffset @returns {number} */
export function getAlbumDetailNativeScrollTop(listScrollTop, collapseOffset) {
  const list = Number.isFinite(listScrollTop) ? Math.max(0, listScrollTop) : 0;
  const collapse = Number.isFinite(collapseOffset) ? Math.max(0, collapseOffset) : 0;
  return list + collapse;
}
