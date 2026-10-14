const QUICK_INDEX_CACHE_LIMIT = 32;
const quickIndexCache = new Map();

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
 *   onProgress?: (partial: { labels: string[], groups: Array<{ label: string, beforeIndex: number }> }) => void,
 * }} options
 */
export async function buildQuickIndexGroups({
  totalCount,
  fetchPage,
  getName,
  getGroupings,
  pageSize = 500,
  isCurrent = () => true,
  onProgress,
}) {
  const count = Number.isSafeInteger(totalCount) ? Math.max(0, totalCount) : 0;
  const safePageSize = Math.max(1, Math.min(500, Math.floor(pageSize)));
  /** @type {Array<{ label: string, beforeIndex: number }>} */
  const groups = [];
  /** @type {string | null} */
  let currentLabel = null;

  for (let offset = 0; offset < count; offset += safePageSize) {
    if (!isCurrent()) return null;
    const page = await fetchPage({ offset, limit: Math.min(safePageSize, count - offset) });
    if (!isCurrent()) return null;
    /** @type {Item[]} */
    const items = Array.isArray(page?.items) ? page.items : [];
    if (items.length === 0) break;
    const names = items.map((item) => String(getName(item) ?? '').trim());
    // Classify the actual first character. Leading punctuation stays in the symbol group.
    const groupingNames = names.map((name) => Array.from(name)[0] ?? '');
    const response = await getGroupings(groupingNames);
    if (!isCurrent()) return null;
    if (!Array.isArray(response.groups) || response.groups.length !== items.length) {
      throw new Error('ICU4X 首字索引回傳的群組數量與曲目不一致。');
    }
    response.groups.forEach((label, localIndex) => {
      const group = names[localIndex].length === 0
        ? '未知'
        : typeof label === 'string' && label.length > 0 ? label : '#';
      if (group !== currentLabel) {
        groups.push({ label: group, beforeIndex: offset + localIndex });
        currentLabel = group;
      }
    });
    if (isCurrent()) {
      onProgress?.({
        labels: [...new Set(groups.map(({ label }) => label))],
        groups: [...groups],
      });
    }
    if (items.length < safePageSize) break;
  }

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
