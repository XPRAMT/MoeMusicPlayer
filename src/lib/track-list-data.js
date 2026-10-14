/** @typedef {import('./ipc').TrackPage} TrackPage */
/** @typedef {import('./ipc').TrackSummary} TrackSummary */
/** @template Item @typedef {{ items: Item[], offset: number, limit: number, totalCount: number }} PagedResponse */
/** @template Scope @typedef {{ scope: Scope, offset: number, limit: number }} PagedRequest */
/** @template Item, Scope @typedef {(request: PagedRequest<Scope>) => Promise<PagedResponse<Item>>} FetchPage */

export const TRACK_PAGE_SIZE = 1000;
export const TRACK_PAGE_SIZE_MAX = TRACK_PAGE_SIZE;
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

/** Return the next active row for shared arrow/page navigation; null means the key is not handled. */
export function getNextActiveIndex(
  /** @type {string} */ key,
  /** @type {number} */ activeIndex,
  /** @type {number} */ totalCount,
  /** @type {number} */ viewportHeight,
  /** @type {number} */ rowHeight,
) {
  if (!Number.isFinite(totalCount) || totalCount <= 0) return null;
  const count = Math.floor(totalCount);
  const safeActive = Math.max(0, Math.min(count - 1, Math.floor(activeIndex)));
  const pageStep = Math.max(1, Math.floor(
    (Number.isFinite(viewportHeight) ? viewportHeight : 0)
      / (Number.isFinite(rowHeight) && rowHeight > 0 ? rowHeight : 1),
  ));
  switch (key) {
    case 'ArrowDown': return Math.min(count - 1, safeActive + 1);
    case 'ArrowUp': return Math.max(0, safeActive - 1);
    case 'Home': return 0;
    case 'End': return count - 1;
    case 'PageDown': return Math.min(count - 1, safeActive + pageStep);
    case 'PageUp': return Math.max(0, safeActive - pageStep);
    default: return null;
  }
}

/**
 * Build the rendered window while carrying the source revision into Svelte's
 * dependency graph. The revision changes even when page lengths and totals do
 * not, so a visible range is rebuilt as its cached page is populated.
 *
 * @template Item
 * @param {{ itemAt(index: number): Item | null }} data
 * @param {VirtualRange} range
 * @param {number} revision
 * @param {(item: Item, index: number) => string | number} [keyOf]
 * @returns {{ revision: number, rows: Array<{ index: number, item: Item | null, track: Item | null, key: string | number }> }}
 */
export function buildVirtualRows(data, range, revision, keyOf = (_item, index) => index) {
  const rows = [];
  for (let index = range.start; index < range.end; index += 1) {
    const item = data.itemAt(index);
    rows.push({ index, item, track: item, key: item === null ? `loading:${index}` : keyOf(item, index) });
  }
  return { revision, rows };
}

/**
 * @typedef {Object} LoadState
 * @property {number} offset
 * @property {number} generation
 * @property {any} scope
 * @property {Promise<void>} promise
 * @property {() => void} resolve
 */

/**
 * @template Item, Scope
 * Holds a bounded set of pages for any renderer-facing paged endpoint. In-flight
 * responses from an older scope/reset generation are ignored.
 */
export class PagedListController {
  /**
   * @param {FetchPage<Item, Scope>} fetchPage
   * @param {{
   *   pageSize?: number,
   *   maxCachedPages?: number,
   *   maxConcurrentRequests?: number,
   *   listName?: string,
   *   onChange?: () => void,
   * }} [options]
   */
  constructor(fetchPage, options = {}) {
    this.fetchPage = fetchPage;
    this.pageSize = Math.max(1, Math.min(
      TRACK_PAGE_SIZE_MAX,
      Math.floor(options.pageSize ?? TRACK_PAGE_SIZE),
    ));
    this.maxCachedPages = Math.max(1, Math.floor(options.maxCachedPages ?? TRACK_LIST_MAX_CACHED_PAGES));
    this.maxConcurrentRequests = Math.max(
      1,
      Math.floor(options.maxConcurrentRequests ?? TRACK_LIST_MAX_CONCURRENT_REQUESTS),
    );
    this.listName = options.listName ?? '列表';
    this.onChange = options.onChange ?? (() => {});

    /** @type {Scope | null} */
    this.scope = null;
    /** @type {unknown} */
    this.resetKey = undefined;
    /** @type {number | null} */
    this.totalCount = null;
    /** @type {number} */
    this.generation = 0;
    /** @type {number} */
    this.revision = 0;
    /** @type {Map<number, PagedResponse<Item>>} */
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
    /** @type {{ generation: number, expected: Set<number>, staged: Map<number, PagedResponse<Item>> } | null} */
    this.refreshState = null;
  }

  /**
   * @param {Scope | null} scope
   * @param {unknown} [resetKey]
   * @returns {boolean} True when state was reset.
   */
  reset(scope, resetKey = scope) {
    if (Object.is(scope, this.scope) && Object.is(resetKey, this.resetKey)) return false;

    this.scope = scope;
    this.resetKey = resetKey;
    this.generation += 1;
    this.totalCount = null;
    this.pages.clear();
    this.errors.clear();
    this.lastRange = null;
    this.refreshState = null;
    for (const state of this.queue) state.resolve();
    this.queue = [];
    this.loads.clear();
    this.notify();
    // Seed totalCount and the first visible rows before viewport measurement.
    // A missed initial ResizeObserver notification must not leave the list
    // waiting for its first user scroll to start loading.
    void this.ensureRange(0, this.pageSize);
    return true;
  }

  /**
   * Invalidate cached rows without changing the list scope, item count, or the
   * last requested viewport range. Useful when an in-place reorder changes
   * item contents but should not move the user's scroll position.
   */
  refresh() {
    this.generation += 1;
    this.errors.clear();
    for (const state of this.queue) state.resolve();
    this.queue = [];
    this.loads.clear();
    if (this.totalCount === null || !this.lastRange) {
      this.pages.clear();
      this.refreshState = null;
    } else {
      this.refreshState = {
        generation: this.generation,
        expected: new Set(this.offsetsForRange(this.lastRange.start, this.lastRange.end)),
        staged: new Map(),
      };
    }
    this.notify();
    this.scheduleRange();
  }

  /** @returns {{ scope: unknown, totalCount: number | null, cachedPageCount: number, cachedItemCount: number, pendingPageCount: number, refreshing: boolean, errors: Array<{ offset: number, message: string }>, generation: number, revision: number }} */
  snapshot() {
    let cachedItemCount = 0;
    for (const page of this.pages.values()) cachedItemCount += page.items.length;

    return {
      scope: this.scope,
      totalCount: this.totalCount,
      cachedPageCount: this.pages.size,
      cachedItemCount,
      pendingPageCount: [...this.loads.values()].filter((state) => state.generation === this.generation).length,
      refreshing: this.refreshState?.generation === this.generation,
      errors: [...this.errors.entries()]
        .map(([offset, message]) => ({ offset, message }))
        .sort((left, right) => left.offset - right.offset),
      generation: this.generation,
      revision: this.revision,
    };
  }

  /**
   * @param {number} index
   * @returns {Item | null}
   */
  itemAt(index) {
    if (!Number.isInteger(index) || index < 0 || (this.totalCount !== null && index >= this.totalCount)) {
      return null;
    }

    const pages = [...this.pages.entries()].sort(([left], [right]) => right - left);
    for (const [offset, page] of pages) {
      if (index < page.offset || index >= page.offset + page.items.length) continue;
      this.pages.delete(offset);
      this.pages.set(offset, page);
      /** @type {Item | undefined} */
      return page.items[index - page.offset] ?? null;
    }
    return null;
  }

  /** Backwards-compatible alias for the library list. */
  /** @param {number} index */
  trackAt(index) {
    return this.itemAt(index);
  }

  /**
   * Load only the pages intersecting `[start, end)`.
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
    while (true) {
      this.scheduleRange();
      const offsets = this.offsetsForRange(safeStart, safeEnd);
      const promises = offsets
        .map((offset) => this.loads.get(offset)?.promise)
        .filter((promise) => promise !== undefined);
      if (promises.length === 0) return;

      // The first response establishes totalCount and schedules the real pages.
      // A scope reset or refresh can invalidate a request generation while this
      // call is waiting. Recompute until the latest range has no pending work.
      await Promise.all(promises);
    }
  }

  /** @param {number} offset */
  async retry(offset) {
    const safeOffset = Math.max(0, Math.floor(offset / this.pageSize) * this.pageSize);
    this.errors.delete(safeOffset);
    this.notify();
    if (this.refreshState?.generation === this.generation && this.lastRange) {
      await this.ensureRange(this.lastRange.start, this.lastRange.end);
      return;
    }
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

    if (this.refreshState?.generation === this.generation) {
      const nextExpected = new Set(required);
      this.refreshState.expected = nextExpected;
      for (const offset of this.refreshState.staged.keys()) {
        if (!nextExpected.has(offset)) this.refreshState.staged.delete(offset);
      }
      if (required.length === 0) this.commitRefresh();
    }

    const wanted = [...required];

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
      const mustRefresh = this.refreshState?.generation === this.generation
        && this.refreshState.expected.has(offset);
      const cachedStart = Math.max(start, offset);
      const cachedEnd = Math.min(
        end,
        offset + this.pageSize,
        this.totalCount ?? Number.POSITIVE_INFINITY,
      );
      if ((!mustRefresh && this.totalCount !== null && this.hasCachedRange(cachedStart, cachedEnd))
        || this.errors.has(offset)
        || this.loads.has(offset)) continue;
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
      scope: this.scope,
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
        scope: state.scope,
        offset: state.offset,
        limit: this.pageSize,
      });
      if (!this.isCurrent(state)) return;

      if (!this.isValidPage(page, state.offset)) {
        throw new Error(`${this.listName}分頁資料格式不符，請重新載入。`);
      }
      if (this.totalCount !== null && page.totalCount !== this.totalCount) {
        throw new Error(`${this.listName}在瀏覽期間有變更，請重新載入列表。`);
      }

      this.totalCount = page.totalCount;
      this.errors.delete(state.offset);
      const refreshState = this.refreshState;
      if (refreshState?.generation === this.generation) {
        if (refreshState.expected.has(state.offset)) {
          refreshState.staged.set(state.offset, page);
          if ([...refreshState.expected].every((offset) => refreshState.staged.has(offset))) {
            this.commitRefresh();
          }
        }
      } else {
        this.pages.delete(state.offset);
        this.pages.set(state.offset, page);
        this.evictOldPages();
      }
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

  /** @param {PagedResponse<Item>} page @param {number} offset */
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

  /** @param {number} start @param {number} end */
  hasCachedRange(start, end) {
    if (end <= start) return true;
    let coveredUntil = start;
    const pages = [...this.pages.values()].sort((left, right) => left.offset - right.offset);
    for (const page of pages) {
      const pageEnd = page.offset + page.items.length;
      if (pageEnd <= coveredUntil) continue;
      if (page.offset > coveredUntil) return false;
      coveredUntil = pageEnd;
      if (coveredUntil >= end) return true;
    }
    return false;
  }

  evictOldPages() {
    while (this.pages.size > this.maxCachedPages) {
      const oldestOffset = this.pages.keys().next().value;
      if (oldestOffset === undefined) return;
      this.pages.delete(oldestOffset);
    }
  }

  commitRefresh() {
    const refreshState = this.refreshState;
    if (!refreshState || refreshState.generation !== this.generation) return;
    /** @type {Array<[number, PagedResponse<Item>]>} */
    const refreshedPages = [];
    for (const offset of [...refreshState.expected].sort((left, right) => left - right)) {
      const page = refreshState.staged.get(offset);
      if (!page) return;
      refreshedPages.push([offset, page]);
    }

    this.pages.clear();
    for (const [offset, page] of refreshedPages) {
      this.pages.set(offset, page);
    }
    this.refreshState = null;
    this.evictOldPages();
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

/** @deprecated Use PagedListController with an explicit scope. @extends {PagedListController<TrackSummary, string>} */
export class PagedTrackList extends PagedListController {
  /**
   * @param {(request: { query: string | null, offset: number, limit: number }) => Promise<PagedResponse<import('./ipc').TrackSummary>>} fetchPage
   * @param {Object} [options]
   */
  constructor(fetchPage, options = {}) {
    super(({ scope, offset, limit }) => fetchPage({
      query: typeof scope === 'string' && scope ? scope : null,
      offset,
      limit,
    }), options);
  }

  /** @param {string | null} query @param {string | number | null} [resetKey] */
  reset(query, resetKey = query) {
    return super.reset(query?.trim() ?? '', resetKey);
  }

  snapshot() {
    return { ...super.snapshot(), query: typeof this.scope === 'string' ? this.scope : '' };
  }
}

/** @param {unknown} error */
function errorMessage(error) {
  if (typeof error === 'string' && error.trim()) return error;
  if (error instanceof Error && error.message.trim()) return error.message;
  return '無法載入曲庫頁面，請重試。';
}
