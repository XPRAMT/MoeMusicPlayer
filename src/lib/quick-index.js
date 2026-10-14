const QUICK_INDEX_CACHE_LIMIT = 32;
const quickIndexCache = new Map();

/**
 * Build the virtual rows from one complete grouping snapshot. The parent only
 * publishes `ready` after the full scan, so no mirrored group-row state or
 * second synchronization pass is required before tracks can render.
 * @template Item
 * @param {{ ready: boolean, groups: Array<{ label: string, beforeIndex: number }>, start: number, end: number, totalCount: number, itemAt: (index: number) => Item | null, getKey: (item: Item, index: number) => string | number }} options
 */
export function buildPagedVirtualRows({ ready, groups, start, end, totalCount, itemAt, getKey }) {
  /** @type {Array<{ kind: 'group', visualIndex: number, label: string, key: string } | { kind: 'item', visualIndex: number, index: number, item: Item | null, key: string | number }>} */
  const rows = [];
  if (!ready) return rows;
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
  return rows;
}

/**
 * Report the visible data range as soon as viewport measurements are ready.
 * Grouping controls row visibility, not the initial data request needed to
 * discover the total count and start the grouping scan.
 * @param {{ metricsReady: boolean, start: number, end: number, groups: Array<{ label: string, beforeIndex: number }>, totalCount: number, onRange: (range: { start: number, end: number }) => void }} options
 */
export function reportPagedRange({ metricsReady, start, end, groups, totalCount, onRange }) {
  if (!metricsReady) return false;
  onRange(visualRangeToDataRange(start, end, groups, totalCount));
  return true;
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
 * Scan a sorted paged list and build groups from the same ICU4X rules used by
 * the backend list comparator. Only groups present in this list are returned.
 * @template Item
 * @param {{
 *   totalCount: number,
 *   fetchPage: (request: { offset: number, limit: number }) => Promise<{ items: Item[] }>,
 *   getName: (item: Item) => unknown,
 *   getGroupings: (names: string[]) => Promise<{ labels: string[], groups: string[] }>,
 *   pageSize?: number,
 *   isCurrent?: () => boolean,
 * }} options
 */
export async function buildQuickIndexGroups({
  totalCount,
  fetchPage,
  getName,
  getGroupings,
  pageSize = 500,
  isCurrent = () => true,
}) {
  const count = Number.isSafeInteger(totalCount) ? Math.max(0, totalCount) : 0;
  const safePageSize = Math.max(1, Math.min(500, Math.floor(pageSize)));
  /** Keep only first characters while scanning; full TrackSummary pages are released per batch. */
  const initials = [];
  const uniqueInitials = new Set();

  for (let offset = 0; offset < count; offset += safePageSize) {
    if (!isCurrent()) return null;
    const page = await fetchPage({ offset, limit: Math.min(safePageSize, count - offset) });
    if (!isCurrent()) return null;
    /** @type {Item[]} */
    const items = Array.isArray(page?.items) ? page.items : [];
    if (items.length === 0) break;
    for (const item of items) {
      // Classify the actual first character. Leading punctuation stays in the symbol group.
      const initial = Array.from(String(getName(item) ?? '').trim())[0] ?? '';
      initials.push(initial);
      uniqueInitials.add(initial);
    }
    if (items.length < safePageSize) break;
  }

  if (!isCurrent()) return null;
  const requestedInitials = [...uniqueInitials];
  const response = await getGroupings(requestedInitials);
  if (!isCurrent()) return null;
  if (!Array.isArray(response.groups) || response.groups.length !== requestedInitials.length) {
    throw new Error('ICU4X 首字索引回傳的群組數量與字元數不一致。');
  }
  const groupByInitial = new Map(requestedInitials.map((initial, index) => [
    initial,
    initial.length === 0
      ? '未知'
      : typeof response.groups[index] === 'string' && response.groups[index].length > 0
        ? response.groups[index]
        : '#',
  ]));
  /** @type {Array<{ label: string, beforeIndex: number }>} */
  const groups = [];
  /** @type {string | null} */
  let currentLabel = null;
  initials.forEach((initial, index) => {
    const label = groupByInitial.get(initial) ?? '#';
    if (label !== currentLabel) {
      groups.push({ label, beforeIndex: index });
      currentLabel = label;
    }
  });

  return { labels: [...new Set(groups.map(({ label }) => label))], groups };
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
