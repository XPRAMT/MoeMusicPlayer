/** @typedef {import('./ipc').TrackPage} TrackPage */
/** @typedef {import('./ipc').TrackSummary} TrackSummary */
/** @typedef {import('./ipc').TrackPageRequest} TrackPageRequest */
/** @typedef {(request: TrackPageRequest) => Promise<TrackPage>} FetchTrackPage */

export const TRACK_PAGE_SIZE = 40;
export const TRACK_LIST_MAX_CACHED_PAGES = 6;
export const TRACK_LIST_MAX_CONCURRENT_REQUESTS = 2;

/**
 * @typedef {Object} VirtualRange
 * @property {number} start Inclusive first row index.
 * @property {number} end Exclusive row index.
 */

/**
 * Return the bounded row interval needed for a fixed-height virtual list.
 * `scrollTop` starts at the first data row; the sticky header is excluded.
 *
 * @param {number} scrollTop
 * @param {number} viewportHeight
 * @param {number} totalCount
 * @param {number} rowHeight
 * @param {number} [overscan]
 * @param {number} [headerHeight]
 * @returns {VirtualRange}
 */
export function getVirtualRange(
  scrollTop,
  viewportHeight,
  totalCount,
  rowHeight,
  overscan = 8,
  headerHeight = 36,
) {
  const count = Number.isFinite(totalCount) ? Math.max(0, Math.floor(totalCount)) : 0;
  const safeRowHeight = Number.isFinite(rowHeight) && rowHeight > 0 ? rowHeight : 1;
  const safeTop = Number.isFinite(scrollTop) ? Math.max(0, scrollTop) : 0;
  const safeViewport = Number.isFinite(viewportHeight) ? Math.max(0, viewportHeight) : 0;
  const safeHeader = Number.isFinite(headerHeight) ? Math.max(0, headerHeight) : 0;
  const safeOverscan = Number.isFinite(overscan) ? Math.max(0, Math.floor(overscan)) : 0;

  const firstVisible = Math.floor(safeTop / safeRowHeight);
  const lastVisibleExclusive = Math.ceil(
    Math.max(0, safeTop + safeViewport - safeHeader) / safeRowHeight,
  );

  return {
    start: Math.max(0, Math.min(count, firstVisible - safeOverscan)),
    end: Math.max(
      0,
      Math.min(count, lastVisibleExclusive + safeOverscan),
    ),
  };
}

/**
 * Build the rendered window while carrying the source revision into Svelte's
 * dependency graph. The revision changes even when page lengths and totals do
 * not, so a visible range is rebuilt as its cached page is populated.
 *
 * @param {PagedTrackList} data
 * @param {VirtualRange} range
 * @param {number} revision
 * @returns {{ revision: number, rows: Array<{ index: number, track: TrackSummary | null }> }}
 */
export function buildVirtualRows(data, range, revision) {
  const rows = [];
  for (let index = range.start; index < range.end; index += 1) {
    rows.push({ index, track: data.trackAt(index) });
  }
  return { revision, rows };
}

/**
 * @typedef {Object} LoadState
 * @property {number} offset
 * @property {number} generation
 * @property {string} query
 * @property {Promise<void>} promise
 * @property {() => void} resolve
 */

/**
 * Holds only a bounded set of backend pages. In-flight responses from an older
 * query/reset generation are ignored; the caller supplies the backend fetcher.
 */
export class PagedTrackList {
  /**
   * @param {FetchTrackPage} fetchPage
   * @param {{
   *   pageSize?: number,
   *   maxCachedPages?: number,
   *   maxConcurrentRequests?: number,
   *   onChange?: () => void,
   * }} [options]
   */
  constructor(fetchPage, options = {}) {
    this.fetchPage = fetchPage;
    this.pageSize = Math.max(1, Math.floor(options.pageSize ?? TRACK_PAGE_SIZE));
    this.maxCachedPages = Math.max(1, Math.floor(options.maxCachedPages ?? TRACK_LIST_MAX_CACHED_PAGES));
    this.maxConcurrentRequests = Math.max(
      1,
      Math.floor(options.maxConcurrentRequests ?? TRACK_LIST_MAX_CONCURRENT_REQUESTS),
    );
    this.onChange = options.onChange ?? (() => {});

    /** @type {string} */
    this.query = '';
    /** @type {string | number | null} */
    this.resetKey = null;
    /** @type {number | null} */
    this.totalCount = null;
    /** @type {number} */
    this.generation = 0;
    /** @type {number} */
    this.revision = 0;
    /** @type {Map<number, TrackPage>} */
    this.pages = new Map();
    /** @type {Map<number, string>} */
    this.errors = new Map();
    /** @type {Map<number, LoadState>} */
    this.loads = new Map();
    /** @type {LoadState[]} */
    this.queue = [];
    /** @type {Set<LoadState>} */
    this.activeLoads = new Set();
    /** @type {{ start: number, end: number } | null} */
    this.lastRange = null;
  }

  /**
   * @param {string} query
   * @param {string | number | null} resetKey
   * @returns {boolean} True when state was reset.
   */
  reset(query, resetKey = null) {
    const normalizedQuery = query.trim();
    if (normalizedQuery === this.query && Object.is(resetKey, this.resetKey)) return false;

    this.query = normalizedQuery;
    this.resetKey = resetKey;
    this.generation += 1;
    this.totalCount = null;
    this.pages.clear();
    this.errors.clear();
    this.lastRange = null;
    for (const state of this.queue) state.resolve();
    this.queue = [];
    this.loads.clear();
    this.notify();
    return true;
  }

  /** @returns {{ query: string, totalCount: number | null, cachedPageCount: number, cachedItemCount: number, pendingPageCount: number, errors: Array<{ offset: number, message: string }>, generation: number, revision: number }} */
  snapshot() {
    let cachedItemCount = 0;
    for (const page of this.pages.values()) cachedItemCount += page.items.length;

    return {
      query: this.query,
      totalCount: this.totalCount,
      cachedPageCount: this.pages.size,
      cachedItemCount,
      pendingPageCount: [...this.loads.values()].filter((state) => state.generation === this.generation).length,
      errors: [...this.errors.entries()]
        .map(([offset, message]) => ({ offset, message }))
        .sort((left, right) => left.offset - right.offset),
      generation: this.generation,
      revision: this.revision,
    };
  }

  /**
   * @param {number} index
   * @returns {TrackSummary | null}
   */
  trackAt(index) {
    if (!Number.isInteger(index) || index < 0 || (this.totalCount !== null && index >= this.totalCount)) {
      return null;
    }

    const offset = Math.floor(index / this.pageSize) * this.pageSize;
    const page = this.pages.get(offset);
    if (!page) return null;
    this.pages.delete(offset);
    this.pages.set(offset, page);
    return page.items[index - offset] ?? null;
  }

  /**
   * Load the pages intersecting `[start, end)`, with one page of look-ahead.
   * Repeated calls are deduplicated, including while requests are in flight.
   *
   * @param {number} start
   * @param {number} end
   * @returns {Promise<void>}
   */
  async ensureRange(start, end) {
    const safeStart = Math.max(0, Math.floor(Number.isFinite(start) ? start : 0));
    const safeEnd = Math.max(safeStart, Math.floor(Number.isFinite(end) ? end : safeStart));
    this.lastRange = { start: safeStart, end: safeEnd };
    this.scheduleRange();

    const offsets = this.offsetsForRange(safeStart, safeEnd);
    const promises = offsets
      .map((offset) => this.loads.get(offset)?.promise)
      .filter((promise) => promise !== undefined);
    await Promise.all(promises);
  }

  /** @param {number} offset */
  async retry(offset) {
    const safeOffset = Math.max(0, Math.floor(offset / this.pageSize) * this.pageSize);
    this.errors.delete(safeOffset);
    this.notify();
    await this.ensureRange(safeOffset, safeOffset + this.pageSize);
  }

  scheduleRange() {
    if (!this.lastRange) return;

    const { start, end } = this.lastRange;
    /** @type {number[]} */
    const required = [];

    if (this.totalCount === null) {
      required.push(0);
    } else if (this.totalCount > 0 && end > start) {
      const last = Math.min(this.totalCount, end);
      const firstOffset = Math.floor(start / this.pageSize) * this.pageSize;
      const lastOffset = Math.floor((last - 1) / this.pageSize) * this.pageSize;
      for (let offset = firstOffset; offset <= lastOffset; offset += this.pageSize) required.push(offset);
    }

    /** @type {number[]} */
    const wanted = [...required];
    if (this.totalCount !== null && required.length > 0) {
      const prefetchOffset = required[required.length - 1] + this.pageSize;
      if (prefetchOffset < this.totalCount) wanted.push(prefetchOffset);
    }

    const wantedSet = new Set(wanted);
    const retainedQueue = [];
    for (const state of this.queue) {
      if (state.generation === this.generation && wantedSet.has(state.offset)) {
        retainedQueue.push(state);
      } else {
        if (this.loads.get(state.offset) === state) this.loads.delete(state.offset);
        state.resolve();
      }
    }
    this.queue = retainedQueue;

    for (const offset of wanted) {
      if (this.pages.has(offset) || this.errors.has(offset) || this.loads.has(offset)) continue;
      this.queue.push(this.createLoadState(offset));
    }

    const priority = new Map(wanted.map((offset, index) => [offset, index]));
    this.queue.sort((left, right) => (priority.get(left.offset) ?? Number.MAX_SAFE_INTEGER)
      - (priority.get(right.offset) ?? Number.MAX_SAFE_INTEGER));
    this.drainQueue();
  }

  /** @param {number} offset @returns {LoadState} */
  createLoadState(offset) {
    /** @type {() => void} */
    let resolve = () => {};
    const promise = new Promise((done) => { resolve = () => done(undefined); });
    const state = {
      offset,
      generation: this.generation,
      query: this.query,
      promise,
      resolve,
    };
    this.loads.set(offset, state);
    return state;
  }

  drainQueue() {
    while (this.activeLoads.size < this.maxConcurrentRequests && this.queue.length > 0) {
      const state = this.queue.shift();
      if (!state) return;
      if (state.generation !== this.generation || this.loads.get(state.offset) !== state) {
        state.resolve();
        continue;
      }

      this.activeLoads.add(state);
      void this.fetchOne(state);
    }
  }

  /** @param {LoadState} state */
  async fetchOne(state) {
    try {
      const page = await this.fetchPage({
        query: state.query || null,
        offset: state.offset,
        limit: this.pageSize,
      });
      if (!this.isCurrent(state)) return;

      if (!this.isValidPage(page, state.offset)) {
        throw new Error('曲庫分頁資料格式不符，請重新載入。');
      }
      if (this.totalCount !== null && page.totalCount !== this.totalCount) {
        throw new Error('曲庫在瀏覽期間有變更，請重新載入列表。');
      }

      this.totalCount = page.totalCount;
      this.errors.delete(state.offset);
      this.pages.delete(state.offset);
      this.pages.set(state.offset, page);
      this.evictOldPages();
    } catch (error) {
      if (this.isCurrent(state)) this.errors.set(state.offset, errorMessage(error));
    } finally {
      this.activeLoads.delete(state);
      if (this.loads.get(state.offset) === state) this.loads.delete(state.offset);
      state.resolve();
      if (state.generation === this.generation) {
        this.notify();
        this.scheduleRange();
      }
      this.drainQueue();
    }
  }

  /** @param {LoadState} state */
  isCurrent(state) {
    return state.generation === this.generation && this.loads.get(state.offset) === state;
  }

  /** @param {TrackPage} page @param {number} offset */
  isValidPage(page, offset) {
    return page !== null
      && typeof page === 'object'
      && Array.isArray(page.items)
      && page.offset === offset
      && page.limit === this.pageSize
      && Number.isInteger(page.totalCount)
      && page.totalCount >= 0
      && page.items.length <= this.pageSize
      && page.offset + page.items.length <= page.totalCount
      && !(page.items.length === 0 && page.offset < page.totalCount);
  }

  evictOldPages() {
    while (this.pages.size > this.maxCachedPages) {
      const oldestOffset = this.pages.keys().next().value;
      if (oldestOffset === undefined) return;
      this.pages.delete(oldestOffset);
    }
  }

  /** @param {number} start @param {number} end @returns {number[]} */
  offsetsForRange(start, end) {
    if (this.totalCount === null) return [0];
    if (this.totalCount === 0 || end <= start) return [];
    const safeEnd = Math.min(end, this.totalCount);
    const firstOffset = Math.floor(start / this.pageSize) * this.pageSize;
    const lastOffset = Math.floor((safeEnd - 1) / this.pageSize) * this.pageSize;
    const offsets = [];
    for (let offset = firstOffset; offset <= lastOffset; offset += this.pageSize) offsets.push(offset);
    return offsets;
  }

  notify() {
    this.revision += 1;
    this.onChange();
  }
}

/** @param {unknown} error */
function errorMessage(error) {
  if (typeof error === 'string' && error.trim()) return error;
  if (error instanceof Error && error.message.trim()) return error.message;
  return '無法載入曲庫頁面，請重試。';
}
