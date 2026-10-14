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

/** @param {{ position: number, groupingReady: boolean, totalCount: number | null, pending: boolean, virtualTotalCount: number, rowHeight: number, viewportHeight: number }} options */
export function getRestoredListScrollTop({ position, groupingReady, totalCount, pending, virtualTotalCount, rowHeight, viewportHeight }) {
  if (!groupingReady || totalCount === null || pending) return null;
  const maxScrollTop = Math.max(0, virtualTotalCount * rowHeight - viewportHeight);
  return Math.min(Math.max(0, position), maxScrollTop);
}

/** @param {{ groupsChanged: boolean, hasPreviousSnapshot: boolean, scrollTop: number }} options */
export function shouldAdjustGroupAnchor({ groupsChanged, hasPreviousSnapshot, scrollTop }) {
  return groupsChanged && hasPreviousSnapshot && scrollTop > 0;
}

/** Choose a near-square index grid based on the number of available labels.
 * @param {number} labelCount
 */
export function getQuickIndexColumnCount(labelCount) {
  const count = Number.isSafeInteger(labelCount) ? Math.max(0, labelCount) : 0;
  return Math.max(1, Math.round(Math.sqrt(count)));
}

/** The quick-find panel uses a compact marker for the Unknown group. */
/** @param {string} label */
export function getQuickIndexDisplayLabel(label) {
  return label === '未知' ? '？' : label;
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
