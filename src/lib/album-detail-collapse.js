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
 * Convert a native scroll offset in the stable, full-page scrollport into the
 * logical track-list offset and the cover collapse offset. Downward movement
 * first collapses the cover one pixel per pixel, then advances the track list.
 * Upward movement returns the list to its top before restoring the cover one
 * pixel per excess pixel.
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
    const collapseStep = Math.min(range - previousCollapse, remaining);
    const collapseOffset = previousCollapse + collapseStep;
    remaining -= collapseStep;
    return {
      listScrollTop: previousList + remaining,
      collapseOffset,
      rawScrollTop: raw,
    };
  }

  let remaining = previousRaw - raw;
  const listReduction = Math.min(previousList, remaining);
  const listScrollTop = previousList - listReduction;
  remaining -= listReduction;
  const collapseOffset = Math.max(0, previousCollapse - remaining);
  return {
    listScrollTop,
    collapseOffset,
    rawScrollTop: listScrollTop + collapseOffset,
  };
}

/** Apply a wheel delta by collapsing the cover first, then moving the list; reverse that order upward. */
/** @param {{delta:number,listScrollTop:number,collapseOffset:number,baseCoverSize:number}} input */
export function mapAlbumDetailWheelDelta(input) {
  const delta = Number.isFinite(input.delta) ? input.delta : 0;
  const list = Number.isFinite(input.listScrollTop) ? Math.max(0, input.listScrollTop) : 0;
  const range = getAlbumDetailCollapseRange(input.baseCoverSize);
  const collapse = Number.isFinite(input.collapseOffset)
    ? Math.min(range, Math.max(0, input.collapseOffset))
    : 0;

  if (delta >= 0) {
    const collapseStep = Math.min(range - collapse, delta);
    const collapseOffset = collapse + collapseStep;
    const listScrollTop = list + delta - collapseStep;
    return {
      listScrollTop,
      collapseOffset,
    };
  }

  let remaining = -delta;
  const listReduction = Math.min(list, remaining);
  const listScrollTop = list - listReduction;
  remaining -= listReduction;
  return {
    listScrollTop,
    collapseOffset: Math.max(0, collapse - remaining),
  };
}

/** Physical scroll position for restoring a saved logical list position. */
/** @param {number} listScrollTop @param {number} collapseOffset @returns {number} */
export function getAlbumDetailNativeScrollTop(listScrollTop, collapseOffset) {
  const list = Number.isFinite(listScrollTop) ? Math.max(0, listScrollTop) : 0;
  const collapse = Number.isFinite(collapseOffset) ? Math.max(0, collapseOffset) : 0;
  return list + collapse;
}
