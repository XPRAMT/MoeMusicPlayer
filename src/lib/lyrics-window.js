/** @typedef {import('./ipc').LyricLine} LyricLine */
/** @typedef {import('./ipc').LyricsPreferences} LyricsPreferences */

/** @typedef {{ index: number, startMs: number, line: LyricLine }} TimedLyric */
/** @typedef {{ heights: Float64Array, gaps: Float64Array, visibleIndices: Uint32Array, tree: Float64Array, offsets: Float64Array, totalHeight: number, generation: number }} LyricsLayout */

export const TIMED_LYRIC_RADIUS = 6;
export const LYRICS_ROW_GAP_PX = 2;
export const LYRICS_ROW_VERTICAL_PADDING_PX = 14;
export const LYRICS_ROW_BORDER_PX = 1;
export const LYRICS_PRIMARY_MAX_LINES = 2;
export const LYRICS_PRIMARY_LINE_HEIGHT = 1.35;
export const LYRICS_AUXILIARY_LINE_HEIGHT = 1.25;

let nextGeneration = 1;

/** @param {LyricLine[] | null | undefined} lines @returns {TimedLyric[]} */
export function buildTimedLyricTimeline(lines) {
  if (!Array.isArray(lines)) return [];
  return lines
    .flatMap((line, index) => (typeof line.startMs === 'number' && Number.isFinite(line.startMs)
      ? [{ index, startMs: line.startMs, line }]
      : []))
    .sort((left, right) => left.startMs - right.startMs || left.index - right.index);
}

/** Build once per document/preferences/width. Rendered row measurements update the Fenwick tree. */
/** @param {(LyricLine | TimedLyric)[]} items @param {LyricsPreferences} preferences @param {number} [lineGapPx] @param {number} [_widthKey] @returns {LyricsLayout} */
export function buildLyricsLayout(items, preferences, lineGapPx = 24, _widthKey = 0) {
  const count = Array.isArray(items) ? items.length : 0;
  const heights = new Float64Array(count);
  const gaps = new Float64Array(count);
  let visibleCount = 0;
  let lastVisible = -1;
  for (let index = 0; index < count; index += 1) {
    const item = items[index];
    const line = item && typeof item === 'object' && 'line' in item ? item.line : item;
    heights[index] = getLyricsLineEstimate(line, preferences);
    if (heights[index] > 0) { visibleCount += 1; lastVisible = index; }
  }
  const safeGap = normalizeGap(lineGapPx);
  const visibleIndices = new Uint32Array(visibleCount);
  const tree = new Float64Array(count + 1);
  let visibleCursor = 0;
  for (let index = 0; index < count; index += 1) {
    if (heights[index] > 0) {
      visibleIndices[visibleCursor++] = index;
      if (index !== lastVisible) gaps[index] = safeGap;
    }
    tree[index + 1] = heights[index] + gaps[index];
  }
  // Linear Fenwick construction; no per-item O(log N) insertion during layout creation.
  for (let index = 1; index <= count; index += 1) {
    const parent = index + (index & -index);
    if (parent <= count) tree[parent] += tree[index];
  }
  const layout = /** @type {LyricsLayout} */ ({ heights, gaps, visibleIndices, tree, offsets: new Float64Array(0), totalHeight: fenwickPrefix(tree, count), generation: nextGeneration++ });
  // Compatibility snapshot for tests/harnesses; the application hot path uses getLyricsOffset.
  Object.defineProperty(layout, 'offsets', {
    get() {
      const offsets = new Float64Array(heights.length + 1);
      for (let index = 0; index <= heights.length; index += 1) offsets[index] = getLyricsOffset(layout, index);
      return offsets;
    },
  });
  return layout;
}

/** Base row estimate assumes a single primary line and only present auxiliary lines. */
/** @param {LyricLine} line @param {LyricsPreferences} preferences */
export function getLyricsLineEstimate(line, preferences) {
  const hasPrimary = hasVisibleText(line?.text);
  const hasTranslation = preferences.showTranslation && hasVisibleText(line?.translation);
  const hasRomanization = preferences.showRomanization && hasVisibleText(line?.romanization);
  const slots = Number(hasPrimary) + Number(hasTranslation) + Number(hasRomanization);
  if (!slots) return 0;
  return (hasPrimary ? preferences.primaryFontSizePx * LYRICS_PRIMARY_LINE_HEIGHT : 0)
    + (Number(hasTranslation) + Number(hasRomanization)) * preferences.auxiliaryFontSizePx * LYRICS_AUXILIARY_LINE_HEIGHT
    + Math.max(0, slots - 1) * LYRICS_ROW_GAP_PX
    + LYRICS_ROW_VERTICAL_PADDING_PX + LYRICS_ROW_BORDER_PX;
}

/** Correct one rendered row to its measured border-box height in O(log N). */
/** @param {LyricsLayout} layout @param {number} index @param {number} measuredHeight */
export function updateLyricsRowHeight(layout, index, measuredHeight) {
  if (!Number.isInteger(index) || index < 0 || index >= layout.heights.length || layout.heights[index] <= 0) return false;
  const nextHeight = Number.isFinite(measuredHeight) ? Math.max(0, measuredHeight) : layout.heights[index];
  const delta = nextHeight - layout.heights[index];
  if (Math.abs(delta) < 0.25) return false;
  layout.heights[index] = nextHeight;
  for (let cursor = index + 1; cursor < layout.tree.length; cursor += cursor & -cursor) layout.tree[cursor] += delta;
  layout.totalHeight += delta;
  return true;
}

/** Prefix height before row index. */
/** @param {LyricsLayout} layout @param {number} index */
export function getLyricsOffset(layout, index) {
  return fenwickPrefix(layout.tree, Math.max(0, Math.min(layout.heights.length, index)));
}

/** @param {Float64Array} tree @param {number} endExclusive */
function fenwickPrefix(tree, endExclusive) {
  let sum = 0;
  for (let cursor = endExclusive; cursor > 0; cursor -= cursor & -cursor) sum += tree[cursor];
  return sum;
}

/** @param {unknown} value */
function hasVisibleText(value) { return typeof value === 'string' && value.trim().length > 0; }
/** @param {number} value */
function normalizeGap(value) { return Number.isFinite(value) ? Math.max(0, Math.min(64, Math.round(value))) : 24; }

/** @param {TimedLyric[]} timeline @param {number} positionMs @param {number} [offsetMs] */
export function findActiveLyricIndex(timeline, positionMs, offsetMs = 0) {
  if (timeline.length === 0 || !Number.isFinite(positionMs)) return -1;
  const targetMs = positionMs + (Number.isFinite(offsetMs) ? offsetMs : 0);
  let low = 0; let high = timeline.length;
  while (low < high) { const middle = low + Math.floor((high - low) / 2); if (timeline[middle].startMs <= targetMs) low = middle + 1; else high = middle; }
  return low - 1;
}

/** @param {LyricsLayout} layout @param {number} offset */
function findVisiblePositionAtOffset(layout, offset) {
  const visible = layout.visibleIndices;
  if (!visible.length || layout.totalHeight <= 0) return -1;
  const target = Math.max(0, Math.min(layout.totalHeight - Number.EPSILON, Number.isFinite(offset) ? offset : 0));
  let low = 0; let high = visible.length;
  while (low < high) {
    const middle = low + Math.floor((high - low) / 2);
    if (getLyricsOffset(layout, visible[middle] + 1) <= target) low = middle + 1;
    else high = middle;
  }
  return Math.min(low, visible.length - 1);
}

/** @param {LyricsLayout} layout @param {number} offset */
export function findLyricsRowAtOffset(layout, offset) {
  const visiblePosition = findVisiblePositionAtOffset(layout, offset);
  return visiblePosition < 0 ? -1 : layout.visibleIndices[visiblePosition];
}

/** @param {LyricsLayout} layout @param {number} activeIndex @param {number} [radius] */
export function getTimedLyricWindow(layout, activeIndex, radius = TIMED_LYRIC_RADIUS) {
  const count = layout.heights.length;
  if (!count) return { start: 0, end: 0, beforeHeight: 0, afterHeight: 0, rows: [] };
  const center = activeIndex < 0 ? findLyricsRowAtOffset(layout, 0) : Math.min(activeIndex, count - 1);
  if (center < 0) return { start: 0, end: 0, beforeHeight: 0, afterHeight: 0, rows: [] };
  const start = Math.max(0, center - Math.max(0, radius));
  const end = Math.min(count, center + Math.max(0, radius) + 1);
  const rows = [];
  for (let index = start; index < end; index += 1) if (layout.heights[index] > 0) rows.push(index);
  const first = rows[0] ?? end;
  const last = rows.length ? rows[rows.length - 1] + 1 : first;
  return { start: first, end: last, beforeHeight: getLyricsOffset(layout, first), afterHeight: layout.totalHeight - getLyricsOffset(layout, last), rows };
}

/** @param {LyricsLayout} layout @param {number} scrollTop @param {number} viewportHeight @param {number} [overscan] */
export function getPlainLyricWindow(layout, scrollTop, viewportHeight, overscan = TIMED_LYRIC_RADIUS) {
  const count = layout.visibleIndices.length;
  if (!count) return { start: 0, end: 0, beforeHeight: 0, afterHeight: 0, rows: [] };
  const first = findVisiblePositionAtOffset(layout, Math.max(0, scrollTop));
  const last = findVisiblePositionAtOffset(layout, Math.max(0, scrollTop) + Math.max(1, viewportHeight) - 0.001);
  if (first < 0 || last < 0) return { start: 0, end: 0, beforeHeight: 0, afterHeight: layout.totalHeight, rows: [] };
  const startPosition = Math.max(0, first - Math.max(0, overscan));
  const endPosition = Math.min(count, last + Math.max(0, overscan) + 1);
  const rows = Array.from(layout.visibleIndices.slice(startPosition, endPosition));
  const start = rows[0] ?? 0;
  const end = rows.length ? rows[rows.length - 1] + 1 : start;
  return { start, end, beforeHeight: getLyricsOffset(layout, start), afterHeight: layout.totalHeight - getLyricsOffset(layout, end), rows };
}
