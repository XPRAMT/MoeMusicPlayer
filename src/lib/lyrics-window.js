/** @typedef {import('./ipc').LyricLine} LyricLine */
/** @typedef {import('./ipc').LyricsPreferences} LyricsPreferences */

/** @typedef {{ index: number, startMs: number, line: LyricLine }} TimedLyric */
/** @typedef {{ offsets: Float64Array, heights: Float64Array, gaps: Float64Array, visibleIndices: Uint32Array, totalHeight: number }} LyricsLayout */

export const TIMED_LYRIC_RADIUS = 6;
export const LYRICS_ROW_GAP_PX = 2;
export const LYRICS_ROW_VERTICAL_PADDING_PX = 14;
export const LYRICS_ROW_BORDER_PX = 1;
export const LYRICS_PRIMARY_MAX_LINES = 2;
export const LYRICS_PRIMARY_LINE_HEIGHT = 1.35;
export const LYRICS_AUXILIARY_LINE_HEIGHT = 1.25;

/** @param {LyricLine[] | null | undefined} lines @returns {TimedLyric[]} */
export function buildTimedLyricTimeline(lines) {
  if (!Array.isArray(lines)) return [];

  return lines
    .flatMap((line, index) => (
      typeof line.startMs === 'number' && Number.isFinite(line.startMs)
        ? [{ index, startMs: line.startMs, line }]
        : []
    ))
    .sort((left, right) => left.startMs - right.startMs || left.index - right.index);
}

/**
 * Build immutable geometry for one lyric document or timed timeline. Call this only when
 * lyrics, visible auxiliary text, font sizes, or line-gap preference changes.
 * @param {LyricLine[] | TimedLyric[]} items
 * @param {LyricsPreferences} preferences
 * @param {number} lineGapPx
 * @returns {LyricsLayout}
 */
export function buildLyricsLayout(items, preferences, lineGapPx = 24) {
  const count = Array.isArray(items) ? items.length : 0;
  const heights = new Float64Array(count);
  const gaps = new Float64Array(count);
  const offsets = new Float64Array(count + 1);
  let lastVisible = -1;
  let visibleCount = 0;

  for (let index = 0; index < count; index += 1) {
    const item = items[index];
    const line = item && typeof item === 'object' && 'line' in item ? item.line : item;
    heights[index] = getLyricsLineHeight(line, preferences);
    if (heights[index] > 0) {
      lastVisible = index;
      visibleCount += 1;
    }
  }

  const safeGap = normalizeGap(lineGapPx);
  const visibleIndices = new Uint32Array(visibleCount);
  let totalHeight = 0;
  let visibleIndex = 0;
  for (let index = 0; index < count; index += 1) {
    offsets[index] = totalHeight;
    if (heights[index] > 0) {
      visibleIndices[visibleIndex] = index;
      visibleIndex += 1;
      if (index !== lastVisible) gaps[index] = safeGap;
    }
    totalHeight += heights[index] + gaps[index];
  }
  offsets[count] = totalHeight;
  return { offsets, heights, gaps, visibleIndices, totalHeight };
}

/**
 * The primary text keeps its existing two-line clamp with two reserved line slots when present.
 * Auxiliary text contributes a line only when the preference is enabled and this lyric has text.
 * @param {LyricLine} line
 * @param {LyricsPreferences} preferences
 */
export function getLyricsLineHeight(line, preferences) {
  const hasPrimary = hasVisibleText(line?.text);
  const hasTranslation = preferences.showTranslation && hasVisibleText(line?.translation);
  const hasRomanization = preferences.showRomanization && hasVisibleText(line?.romanization);
  const slots = Number(hasPrimary) + Number(hasTranslation) + Number(hasRomanization);
  if (slots === 0) return 0;

  const primaryHeight = hasPrimary
    ? preferences.primaryFontSizePx * LYRICS_PRIMARY_LINE_HEIGHT * LYRICS_PRIMARY_MAX_LINES
    : 0;
  const auxiliaryHeight = (Number(hasTranslation) + Number(hasRomanization))
    * preferences.auxiliaryFontSizePx
    * LYRICS_AUXILIARY_LINE_HEIGHT;
  return primaryHeight + auxiliaryHeight + Math.max(0, slots - 1) * LYRICS_ROW_GAP_PX
    + LYRICS_ROW_VERTICAL_PADDING_PX + LYRICS_ROW_BORDER_PX;
}

/** @param {unknown} value */
function hasVisibleText(value) {
  return typeof value === 'string' && value.trim().length > 0;
}

/** @param {number} value */
function normalizeGap(value) {
  return Number.isFinite(value) ? Math.max(0, Math.min(64, Math.round(value))) : 24;
}

/**
 * Return the last timed line at or before the playback position. -1 means before the first cue.
 * @param {TimedLyric[]} timeline @param {number} positionMs @param {number} [offsetMs]
 */
export function findActiveLyricIndex(timeline, positionMs, offsetMs = 0) {
  if (timeline.length === 0 || !Number.isFinite(positionMs)) return -1;
  const targetMs = positionMs + (Number.isFinite(offsetMs) ? offsetMs : 0);
  let low = 0;
  let high = timeline.length;
  while (low < high) {
    const middle = low + Math.floor((high - low) / 2);
    if (timeline[middle].startMs <= targetMs) low = middle + 1;
    else high = middle;
  }
  return low - 1;
}

/** Return the first nonzero-height row whose accumulated interval contains the offset. */
/** @param {LyricsLayout} layout @param {number} offset */
export function findLyricsRowAtOffset(layout, offset) {
  const visiblePosition = findVisiblePositionAtOffset(layout, offset);
  return visiblePosition < 0 ? -1 : layout.visibleIndices[visiblePosition];
}

/** @param {LyricsLayout} layout @param {number} offset */
function findVisiblePositionAtOffset(layout, offset) {
  if (layout.visibleIndices.length === 0 || layout.totalHeight <= 0) return -1;
  const target = Math.max(0, Math.min(layout.totalHeight, Number.isFinite(offset) ? offset : 0));
  let low = 0;
  let high = layout.visibleIndices.length;
  while (low < high) {
    const middle = low + Math.floor((high - low) / 2);
    const index = layout.visibleIndices[middle];
    if (layout.offsets[index + 1] <= target) low = middle + 1;
    else high = middle;
  }
  return Math.min(low, layout.visibleIndices.length - 1);
}

/**
 * Window around the timed playback index. The range is bounded by the radius, and spacers use
 * the same prefix offsets used by playback auto-scroll.
 * @param {LyricsLayout} layout @param {number} activeIndex
 * @param {number} [radius]
 * @returns {{ start: number, end: number, beforeHeight: number, afterHeight: number, rows: number[] }}
 */
export function getTimedLyricWindow(layout, activeIndex, radius = TIMED_LYRIC_RADIUS) {
  const count = layout.heights.length;
  if (count === 0) return { start: 0, end: 0, beforeHeight: 0, afterHeight: 0, rows: [] };
  const center = activeIndex < 0 ? findLyricsRowAtOffset(layout, 0) : Math.min(activeIndex, count - 1);
  if (center < 0) return { start: 0, end: 0, beforeHeight: 0, afterHeight: 0, rows: [] };
  const start = Math.max(0, center - Math.max(0, radius));
  const end = Math.min(count, center + Math.max(0, radius) + 1);
  return {
    start,
    end,
    beforeHeight: layout.offsets[start],
    afterHeight: layout.totalHeight - layout.offsets[end],
    rows: Array.from({ length: end - start }, (_, index) => index + start),
  };
}

/**
 * Variable-height plain lyric window. It uses binary search for both viewport edges and bounded
 * row-count overscan; no full-document walk occurs during scrolling.
 * @param {LyricsLayout} layout @param {number} scrollTop @param {number} viewportHeight
 * @param {number} [overscan]
 */
export function getPlainLyricWindow(layout, scrollTop, viewportHeight, overscan = TIMED_LYRIC_RADIUS) {
  const count = layout.visibleIndices.length;
  if (count === 0) return { start: 0, end: 0, beforeHeight: 0, afterHeight: 0, rows: [] };
  const safeScrollTop = Math.max(0, scrollTop);
  const first = findVisiblePositionAtOffset(layout, safeScrollTop);
  const lastTarget = safeScrollTop + Math.max(1, viewportHeight) - 0.001;
  const last = findVisiblePositionAtOffset(layout, lastTarget);
  if (first < 0 || last < 0) return { start: 0, end: 0, beforeHeight: 0, afterHeight: layout.totalHeight, rows: [] };
  const startPosition = Math.max(0, first - Math.max(0, overscan));
  const endPosition = Math.min(count, last + Math.max(0, overscan) + 1);
  const rows = Array.from(layout.visibleIndices.slice(startPosition, endPosition));
  const start = rows[0] ?? 0;
  const end = rows.length > 0 ? rows[rows.length - 1] + 1 : start;
  return {
    start,
    end,
    beforeHeight: layout.offsets[start],
    afterHeight: layout.totalHeight - layout.offsets[end],
    rows,
  };
}
