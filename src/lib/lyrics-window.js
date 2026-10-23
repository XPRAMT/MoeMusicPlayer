/** @typedef {import('./ipc').LyricLine} LyricLine */
/** @typedef {import('./ipc').LyricsPreferences & { primaryLineHeight?: number, auxiliaryLineHeight?: number }} LyricsPreferences */

/** @typedef {{ index: number, startMs: number, line: LyricLine }} TimedLyric */
/** @typedef {{ heights: Float64Array, gaps: Float64Array, visibleIndices: Uint32Array, tree: Float64Array, offsets: Float64Array, totalHeight: number, generation: number }} LyricsLayout */

export const PLAIN_LYRIC_OVERSCAN = 6;
export const LYRICS_ROW_GAP_PX = 2;
/** 2 x --lyric-shadow-blur (3px) in LyricsView.svelte: row padding equals the first shadow radius. */
export const LYRICS_ROW_VERTICAL_PADDING_PX = 6;
export const LYRICS_ROW_BORDER_PX = 0;
export const LYRICS_PRIMARY_MAX_LINES = 2;
/** Fallback line-height ratios; LyricsView measures the real ink fit per font (see computeLyricsInkLineHeight). */
export const LYRICS_PRIMARY_LINE_HEIGHT = 1.35;
export const LYRICS_AUXILIARY_LINE_HEIGHT = 1.35;

/** Glyph samples: tallest Latin/CJK ascenders and Latin descenders used to measure the ink box. */
export const LYRICS_INK_ASCENT_SAMPLE = 'Hbdfhklt\u570b\u611b\u6200';
export const LYRICS_INK_DESCENT_SAMPLE = 'gjpqy\u570b\u611b';

/**
 * Smallest line-height (in em) whose line box holds the glyph ink (ascenders + descenders) given how the
 * browser places the baseline: the font content area (fontAscent + fontDescent) is centred in the line box,
 * so the line may shrink by twice the smaller of the ink margins. Ratios are per 1em; invalid input -> fallback.
 * @param {{ fontAscent: number, fontDescent: number, inkAscent: number, inkDescent: number }} metrics
 * @param {number} [fallback]
 */
export function computeLyricsInkLineHeight(metrics, fallback = LYRICS_PRIMARY_LINE_HEIGHT) {
  const { fontAscent, fontDescent, inkAscent, inkDescent } = metrics ?? {};
  if (![fontAscent, fontDescent, inkAscent, inkDescent].every((value) => Number.isFinite(value) && value >= 0)) return fallback;
  if (!(fontAscent + fontDescent > 0) || !(inkAscent + inkDescent > 0)) return fallback;
  const ratio = fontAscent + fontDescent + 2 * Math.max(inkAscent - fontAscent, inkDescent - fontDescent);
  if (!(ratio > 0.5 && ratio < 3)) return fallback;
  return Math.ceil(ratio * 1000) / 1000;
}

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
  return (hasPrimary ? preferences.primaryFontSizePx * (preferences.primaryLineHeight ?? LYRICS_PRIMARY_LINE_HEIGHT) : 0)
    + (Number(hasTranslation) + Number(hasRomanization)) * (preferences.primaryFontSizePx * preferences.auxiliaryScalePercent / 100) * (preferences.auxiliaryLineHeight ?? LYRICS_AUXILIARY_LINE_HEIGHT)
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
function normalizeGap(value) { return Number.isFinite(value) ? Math.max(0, Math.min(50, Math.round(value))) : 24; }

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

/**
 * Timed lyrics window by scroll position (not by the active line), so hand scrolling never shows blank spacers.
 * Scroll positions are in the viewport's coordinates; `edgePadding` is the head spacer above the first row.
 * `extraScrollTops` (a running glide's start and target) widen the range to their union with the current view,
 * so rows the glide brings in are rendered before it moves.
 * @param {LyricsLayout} layout @param {number} scrollTop @param {number} viewportHeight @param {number} [edgePadding]
 * @param {number[]} [extraScrollTops] @param {number} [overscan]
 */
export function getTimedLyricWindow(layout, scrollTop, viewportHeight, edgePadding = 0, extraScrollTops = [], overscan = PLAIN_LYRIC_OVERSCAN) {
  const tops = [scrollTop, ...extraScrollTops].filter((value) => Number.isFinite(value));
  const low = (tops.length ? Math.min(...tops) : 0) - (Number.isFinite(edgePadding) ? edgePadding : 0);
  const high = (tops.length ? Math.max(...tops) : 0) - (Number.isFinite(edgePadding) ? edgePadding : 0);
  return getPlainLyricWindow(layout, low, high - low + Math.max(1, viewportHeight), overscan);
}

/** True when two windows render the same rows with the same spacer heights. */
/** @param {{ start: number, end: number, beforeHeight: number, afterHeight: number }} left @param {{ start: number, end: number, beforeHeight: number, afterHeight: number }} right */
export function sameLyricWindow(left, right) {
  return left.start === right.start && left.end === right.end
    && left.beforeHeight === right.beforeHeight && left.afterHeight === right.afterHeight;
}

/** @param {LyricsLayout} layout @param {number} scrollTop @param {number} viewportHeight @param {number} [overscan] */
export function getPlainLyricWindow(layout, scrollTop, viewportHeight, overscan = PLAIN_LYRIC_OVERSCAN) {
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
