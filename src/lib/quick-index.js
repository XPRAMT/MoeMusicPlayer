const QUICK_INDEX_CACHE_LIMIT = 32;
const quickIndexCache = new Map();

/**
 * Return a stable identity for one complete quick-index build. Snapshot objects
 * may be replaced while pages load even though the list's grouping inputs did
 * not change; callers use this key to avoid restarting the backend snapshot request.
 * @param {{ listId?: string, resetKey?: string | number, query: string | null, fieldFilter: unknown, sort: unknown, count: number | null, activeSort: unknown, groupingEnabled: boolean, sortable: boolean }} input
 */
export function getQuickIndexBuildKey(input) {
  return JSON.stringify({
    listId: input.listId,
    resetKey: input.resetKey,
    query: input.query,
    fieldFilter: input.fieldFilter,
    sort: input.sort,
    count: input.count,
    activeSort: input.activeSort,
    groupingEnabled: input.groupingEnabled,
    sortable: input.sortable,
  });
}

/**
 * Build the virtual rows from one complete backend grouping snapshot. The parent only
 * publishes `ready` after the full scan, so no mirrored group-row state or
 * second synchronization pass is required before tracks can render.
 * @template Item
 * @param {{ ready: boolean, groups: Array<{ label: string, beforeIndex: number }>, start: number, end: number, totalCount: number, revision: number, itemAt: (index: number) => Item | null, getKey: (item: Item, index: number) => string | number }} options
 */
export function buildPagedVirtualRows({ ready, groups, start, end, totalCount, revision, itemAt, getKey }) {
  /** @type {Array<{ kind: 'group', visualIndex: number, label: string, key: string } | { kind: 'item', visualIndex: number, index: number, item: Item | null, key: string | number }>} */
  const rows = [];
  if (!ready) return { revision, rows };
  for (let visualIndex = start; visualIndex < end; visualIndex += 1) {
    let groupRow = null;
    for (let groupIndex = 0; groupIndex < groups.length; groupIndex += 1) {
      if (groups[groupIndex].beforeIndex + groupIndex === visualIndex) {
        groupRow = groups[groupIndex];
        break;
      }
    }
    if (groupRow) {
      rows.push({ kind: 'group', visualIndex, label: groupRow.label, key: `quick-index:${visualIndex}:${groupRow.label}` });
      continue;
    }
    const index = visualIndexToDataIndex(visualIndex, groups);
    if (index === null || index < 0 || index >= totalCount) continue;
    const item = itemAt(index);
    rows.push({ kind: 'item', visualIndex, index, item, key: item === null ? `loading:${index}` : getKey(item, index) });
  }
  return { revision, rows };
}

/**
 * Report the visible data range as soon as viewport measurements are ready.
 * Grouping controls row visibility, not the initial data request needed to
 * discover the total count and request the backend grouping snapshot.
 * @param {{ metricsReady: boolean, start: number, end: number, groups: Array<{ label: string, beforeIndex: number }>, totalCount: number, onRange: (range: { start: number, end: number }) => void }} options
 */
export function reportPagedRange({ metricsReady, start, end, groups, totalCount, onRange }) {
  if (!metricsReady) return false;
  onRange(visualRangeToDataRange(start, end, groups, totalCount));
  return true;
}

/** @param {{ position: number, groupingReady: boolean, totalCount: number | null, pending: boolean, virtualTotalCount: number, rowHeight: number, groupRowHeight?: number, groups?: Array<{ label: string, beforeIndex: number }>, viewportHeight: number }} options */
export function getRestoredListScrollTop({ position, groupingReady, totalCount, pending, virtualTotalCount, rowHeight, groupRowHeight = rowHeight, groups = [], viewportHeight }) {
  if (!groupingReady || totalCount === null || pending) return null;
  const totalHeight = groups.length > 0
    ? getVirtualRowsHeight(totalCount, groups, rowHeight, groupRowHeight)
    : virtualTotalCount * rowHeight;
  const maxScrollTop = Math.max(0, totalHeight - viewportHeight);
  return Math.min(Math.max(0, position), maxScrollTop);
}

/** Height of a virtual list whose inserted group rows are shorter than data rows. */
/** @param {number} totalCount @param {Array<{ label: string, beforeIndex: number }>} groups @param {number} rowHeight @param {number} [groupRowHeight] */
export function getVirtualRowsHeight(totalCount, groups = [], rowHeight, groupRowHeight = rowHeight) {
  const safeRowHeight = Number.isFinite(rowHeight) && rowHeight > 0 ? rowHeight : 1;
  const safeGroupHeight = Number.isFinite(groupRowHeight) && groupRowHeight > 0 ? groupRowHeight : safeRowHeight;
  const count = Number.isSafeInteger(totalCount) ? Math.max(0, totalCount) : 0;
  return count * safeRowHeight + groups.length * safeGroupHeight;
}

/** Pixel offset for a virtual row index with compact inserted group rows. */
/** @param {number} visualIndex @param {Array<{ label: string, beforeIndex: number }>} groups @param {number} rowHeight @param {number} [groupRowHeight] */
export function getVirtualRowTop(visualIndex, groups = [], rowHeight, groupRowHeight = rowHeight) {
  const safeRowHeight = Number.isFinite(rowHeight) && rowHeight > 0 ? rowHeight : 1;
  const safeGroupHeight = Number.isFinite(groupRowHeight) && groupRowHeight > 0 ? groupRowHeight : safeRowHeight;
  let compactRowsBefore = 0;
  for (let index = 0; index < groups.length; index += 1) {
    if (groups[index].beforeIndex + index < visualIndex) compactRowsBefore += 1;
    else break;
  }
  return visualIndex * safeRowHeight - compactRowsBefore * (safeRowHeight - safeGroupHeight);
}

/** Return the virtual row containing a scroll offset. */
/** @param {number} offset @param {number} totalCount @param {Array<{ label: string, beforeIndex: number }>} groups @param {number} rowHeight @param {number} [groupRowHeight] */
export function getVisualIndexAtOffset(offset, totalCount, groups = [], rowHeight, groupRowHeight = rowHeight) {
  const visualCount = Math.max(0, totalCount) + groups.length;
  if (visualCount === 0) return 0;
  const safeOffset = Number.isFinite(offset) ? Math.max(0, offset) : 0;
  let low = 0;
  let high = visualCount;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    const top = getVirtualRowTop(middle, groups, rowHeight, groupRowHeight);
    const isGroup = visualIndexToDataIndex(middle, groups) === null;
    const height = isGroup ? groupRowHeight : rowHeight;
    if (top + height <= safeOffset) low = middle + 1;
    else high = middle;
  }
  return Math.min(visualCount - 1, low);
}

/** Visible virtual row range for a list with compact inserted group rows. */
/** @param {number} scrollTop @param {number} viewportHeight @param {number} totalCount @param {Array<{ label: string, beforeIndex: number }>} groups @param {number} rowHeight @param {number} groupRowHeight @param {number} [overscan] */
export function getGroupedVirtualRange(scrollTop, viewportHeight, totalCount, groups = [], rowHeight, groupRowHeight, overscan = 8) {
  const visualCount = Math.max(0, totalCount) + groups.length;
  if (visualCount === 0) return { start: 0, end: 0 };
  const safeTop = Number.isFinite(scrollTop) ? Math.max(0, scrollTop) : 0;
  const safeViewport = Number.isFinite(viewportHeight) ? Math.max(0, viewportHeight) : 0;
  const safeOverscan = Number.isFinite(overscan) ? Math.max(0, Math.floor(overscan)) : 0;
  const start = getVisualIndexAtOffset(safeTop, totalCount, groups, rowHeight, groupRowHeight);
  const end = getVisualIndexAtOffset(safeTop + safeViewport, totalCount, groups, rowHeight, groupRowHeight);
  return {
    start: Math.max(0, start - safeOverscan),
    end: Math.min(visualCount, end + safeOverscan + 1),
  };
}

/** @param {{ groupsChanged: boolean, hasPreviousSnapshot: boolean, scrollTop: number }} options */
export function shouldAdjustGroupAnchor({ groupsChanged, hasPreviousSnapshot, scrollTop }) {
  return groupsChanged && hasPreviousSnapshot && scrollTop > 0;
}

export const QUICK_INDEX_PANEL_PADDING_PX = 12;
export const QUICK_INDEX_LABEL_GAP_PX = 4;
export const QUICK_INDEX_LABEL_SIZE_EM = 2;

/**
 * Choose a near-square index grid (round(sqrt(labelCount)) columns), then cap the
 * column count so the panel (columns * cell + gaps + padding) fits the available
 * width. Width wins over squareness: the grid grows taller instead of scrolling sideways.
 * Without an available width the near-square count is returned unchanged.
 * @param {number} labelCount
 * @param {{ availableWidthPx?: number, cellSizePx?: number, gapPx?: number, paddingPx?: number }} [layout]
 */
export function getQuickIndexColumnCount(labelCount, layout = {}) {
  const count = Number.isSafeInteger(labelCount) ? Math.max(0, labelCount) : 0;
  const preferred = Math.max(1, Math.round(Math.sqrt(count)));
  const fitting = getQuickIndexMaxFittingColumns(layout);
  return fitting === null ? preferred : Math.max(1, Math.min(preferred, fitting));
}

/**
 * Largest column count whose panel width fits the available width, or null when unknown.
 * @param {{ availableWidthPx?: number, cellSizePx?: number, gapPx?: number, paddingPx?: number }} layout
 */
export function getQuickIndexMaxFittingColumns({ availableWidthPx, cellSizePx = 32, gapPx = QUICK_INDEX_LABEL_GAP_PX, paddingPx = QUICK_INDEX_PANEL_PADDING_PX } = {}) {
  if (!Number.isFinite(availableWidthPx) || availableWidthPx === undefined || availableWidthPx <= 0) return null;
  const cell = Number.isFinite(cellSizePx) && cellSizePx > 0 ? cellSizePx : 32;
  const gap = Number.isFinite(gapPx) && gapPx >= 0 ? gapPx : QUICK_INDEX_LABEL_GAP_PX;
  const padding = Number.isFinite(paddingPx) && paddingPx >= 0 ? paddingPx : QUICK_INDEX_PANEL_PADDING_PX;
  const inner = availableWidthPx - padding * 2;
  return Math.max(1, Math.floor((inner + gap) / (cell + gap)));
}

/**
 * Available panel width: the list area minus margins, never wider than the window.
 * @param {number} areaWidthPx @param {number} windowWidthPx @param {number} [marginPx]
 */
export function getQuickIndexAvailableWidth(areaWidthPx, windowWidthPx, marginPx = 24) {
  const area = Number.isFinite(areaWidthPx) ? areaWidthPx - marginPx : Infinity;
  const win = Number.isFinite(windowWidthPx) ? windowWidthPx - marginPx : Infinity;
  const width = Math.min(area, win);
  return Number.isFinite(width) ? Math.max(0, width) : 0;
}

/** Cell size in px for a label grid whose font size is `fontSizePx`. @param {number} fontSizePx */
export function getQuickIndexCellSizePx(fontSizePx) {
  const font = Number.isFinite(fontSizePx) && fontSizePx > 0 ? fontSizePx : 16;
  return font * QUICK_INDEX_LABEL_SIZE_EM;
}

/** Horizontal padding of a label cell, in em, on top of its text (a one-character label stays 2 em wide). */
export const QUICK_INDEX_LABEL_PADDING_EM = 1;
/** A character this wide or wider is counted as a full em (CJK, kana, fullwidth forms); the rest as narrower text. */
const QUICK_INDEX_NARROW_CHARACTER_EM = 0.62;

/**
 * Estimated width of a label's text in em: a wide character counts 1 em, anything else about 0.6 em ("2024" is
 * roughly 2.5 em, "FLAC" 2.5 em, a single kana or kanji 1 em).
 * @param {string} text
 */
export function estimateQuickIndexTextWidthEm(text) {
  let width = 0;
  for (const character of String(text ?? '')) {
    const code = character.codePointAt(0) ?? 0;
    const wide = code >= 0x1100 && (code <= 0x115f || (code >= 0x2e80 && code <= 0xa4cf) || (code >= 0xac00 && code <= 0xd7a3)
      || (code >= 0xf900 && code <= 0xfaff) || (code >= 0xfe30 && code <= 0xfe6f) || (code >= 0xff00 && code <= 0xff60) || (code >= 0xffe0 && code <= 0xffe6));
    width += wide ? 1 : QUICK_INDEX_NARROW_CHARACTER_EM;
  }
  return width;
}

/**
 * Width in em of the widest cell of a label grid. A cell is never narrower than the 2 em square a one-character
 * label gets, and grows with its text, so labels such as "2024" or "FLAC" are not squeezed into one character's room.
 * @param {string[]} labels the labels as shown (after `getQuickIndexDisplayLabel`)
 */
export function getQuickIndexCellWidthEm(labels) {
  let widest = 0;
  for (const label of labels) widest = Math.max(widest, estimateQuickIndexTextWidthEm(label));
  return Math.max(QUICK_INDEX_LABEL_SIZE_EM, widest + QUICK_INDEX_LABEL_PADDING_EM);
}

/** The quick-find panel uses a compact marker for the Unknown group. */
/** @param {string} label */
export function getQuickIndexDisplayLabel(label) {
  if (label === '未知') return '？';
  const discNumber = /^DISC\s+(\d+)$/i.exec(label)?.[1];
  return discNumber ?? label;
}

/** @param {string} key */
export function getCachedQuickIndex(key) {
  const value = quickIndexCache.get(key);
  if (!value) return null;
  quickIndexCache.delete(key);
  quickIndexCache.set(key, value);
  return value;
}
/** @param {string} key @param {{ labels: string[], groups: Array<{ label: string, beforeIndex: number }> }} value */
export function cacheQuickIndex(key, value) {
  quickIndexCache.delete(key);
  quickIndexCache.set(key, value);
  if (quickIndexCache.size > QUICK_INDEX_CACHE_LIMIT) {
    const oldestKey = quickIndexCache.keys().next().value;
    if (oldestKey !== undefined) quickIndexCache.delete(oldestKey);
  }
}

/**
 * Data row index to its position in the virtual list, including group headings.
 * @param {number} index
 * @param {Array<{ label: string, beforeIndex: number }>} [groups]
 */
export function dataIndexToVisualIndex(index, groups = []) {
  let headingsBeforeOrAt = 0;
  for (let groupIndex = 0; groupIndex < groups.length; groupIndex += 1) {
    if (groups[groupIndex].beforeIndex <= index) headingsBeforeOrAt += 1;
    else break;
  }
  return index + headingsBeforeOrAt;
}

/**
 * Virtual row index to its data row index. A group heading maps to null.
 * @param {number} index
 * @param {Array<{ label: string, beforeIndex: number }>} [groups]
 */
export function visualIndexToDataIndex(index, groups = []) {
  let headingsBefore = 0;
  for (let groupIndex = 0; groupIndex < groups.length; groupIndex += 1) {
    const headingVisualIndex = groups[groupIndex].beforeIndex + groupIndex;
    if (headingVisualIndex === index) return null;
    if (headingVisualIndex < index) headingsBefore += 1;
    else break;
  }
  return index - headingsBefore;
}

/** Resolve a visible virtual row to the data item that should anchor scroll restoration. */
/**
 * @param {number} index
 * @param {Array<{ label: string, beforeIndex: number }>} groups
 * @param {number} totalCount
 */
export function visualIndexToAnchorDataIndex(index, groups, totalCount) {
  const dataIndex = visualIndexToDataIndex(index, groups);
  if (dataIndex !== null) return Math.max(0, Math.min(totalCount - 1, dataIndex));
  const groupIndex = groups.findIndex((group, position) => group.beforeIndex + position === index);
  if (groupIndex < 0) return null;
  return Math.max(0, Math.min(totalCount - 1, groups[groupIndex].beforeIndex));
}

/**
 * Map a virtual range to the contiguous data range needed to render it.
 * @param {number} start
 * @param {number} end
 * @param {Array<{ label: string, beforeIndex: number }>} [groups]
 * @param {number} [totalCount]
 */
export function visualRangeToDataRange(start, end, groups = [], totalCount = 0) {
  let first = null;
  let last = null;
  for (let visualIndex = start; visualIndex < end; visualIndex += 1) {
    const dataIndex = visualIndexToDataIndex(visualIndex, groups);
    if (dataIndex === null) continue;
    if (first === null) first = dataIndex;
    last = dataIndex;
  }
  if (first === null || last === null) return { start: 0, end: 0 };
  return {
    start: Math.max(0, Math.min(totalCount, first)),
    end: Math.max(0, Math.min(totalCount, last + 1)),
  };
}
