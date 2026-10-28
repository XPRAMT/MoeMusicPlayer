export const MIN_ALBUM_DETAIL_COVER_SIZE_PX = 80;
export const DESKTOP_ALBUM_DETAIL_COVER_SIZE_PX = 200;
export const NARROW_ALBUM_DETAIL_COVER_SIZE_PX = 160;

/** The maximum amount of the fixed album heading that can be uncovered. */
/** @param {number} baseSize @returns {number} */
export function getAlbumDetailCollapseRange(baseSize) {
  const base = Number.isFinite(baseSize) ? Math.max(MIN_ALBUM_DETAIL_COVER_SIZE_PX, baseSize) : DESKTOP_ALBUM_DETAIL_COVER_SIZE_PX;
  return Math.max(0, base - MIN_ALBUM_DETAIL_COVER_SIZE_PX);
}

/** Shrink one CSS pixel per scroll pixel, without going below the compact cover size. */
/** @param {number} baseSize @param {number} scrollPeak @returns {number} */
export function getAlbumDetailCoverSize(baseSize, scrollPeak) {
  const base = Number.isFinite(baseSize) ? Math.max(MIN_ALBUM_DETAIL_COVER_SIZE_PX, baseSize) : DESKTOP_ALBUM_DETAIL_COVER_SIZE_PX;
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
/**
 * Collapse progress 0..1 of the album detail heading: the clamped collapse offset over the collapse range. The one value
 * the cover, the summary and the column header all read (as `--album-detail-collapse-progress`).
 *
 * @param {number} collapseOffset @param {number} collapseRange
 * @returns {number}
 */
export function getAlbumDetailCollapseProgress(collapseOffset, collapseRange) {
  const range = Number.isFinite(collapseRange) ? Math.max(0, collapseRange) : 0;
  if (range <= 0) return 0;
  const offset = Number.isFinite(collapseOffset) ? collapseOffset : 0;
  return Math.min(1, Math.max(0, offset / range));
}

/**
 * Writes the progress at most once per animation frame while scrolling (`set`), at once for mount, restore and layout
 * changes (`set(progress, true)`), and again unconditionally after something rewrote the element's style (`reapply`).
 * Unchanged values are not written.
 *
 * @param {{requestFrame: (callback: () => void) => number, cancelFrame: (handle: number) => void, write: (progress: number) => void}} options
 */
export function createCollapseProgressWriter({ requestFrame, cancelFrame, write }) {
  let latest = 0;
  let written = Number.NaN;
  /** @type {number | null} */
  let pending = null;
  const cancelPending = () => {
    if (pending === null) return;
    cancelFrame(pending);
    pending = null;
  };
  const commit = (/** @type {boolean} */ force) => {
    if (!force && latest === written) return;
    written = latest;
    write(latest);
  };
  return {
    /** @param {number} progress @param {boolean} [immediate] */
    set(progress, immediate = false) {
      latest = progress;
      if (immediate) {
        cancelPending();
        commit(false);
      } else if (pending === null) {
        pending = requestFrame(() => {
          pending = null;
          commit(false);
        });
      }
    },
    reapply() {
      cancelPending();
      commit(true);
    },
    dispose: cancelPending,
  };
}
