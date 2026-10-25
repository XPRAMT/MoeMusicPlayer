<script lang="ts">
  import { tr } from './i18n.js';
  import { isGroupedSortField } from './grouped-sorts.js';
  import {
    getErrorText,
    invokeCommand,
    type TrackPage,
    type TrackPageRequest,
    type TrackSummary,
    type TrackListColumnPreference,
    type TrackColumnWidths,
    type LibraryListSort,
  } from './ipc';
  import { formatDuration } from './format';
  import { formatListOrder } from './list-order.js';
  import {
    DEFAULT_LIBRARY_SORT,
    DEFAULT_TRACK_COLUMN_PREFERENCES,
    DEFAULT_TRACK_ROW_HEIGHT_PX,
    DEFAULT_TRACK_COLUMN_WIDTHS,
    TRACK_COLUMN_DEFINITIONS,
    displayTrackTitle,
    formatTrackColumnValue,
    isHiResTrack,
    normalizeLibrarySort,
    normalizeTrackColumnPreferences,
    normalizeTrackColumnWidths,
    trackListColumnCount,
    trackListGridTemplate,
    trackListMinWidthPx,
    visibleTrackColumns,
  } from './track-columns.js';
  import { withLivePlayedMs } from './live-play-count.js';
  import { PagedListController } from './track-list-data.js';
  import { appTooltip } from './app-tooltip.js';
  import PagedVirtualList from './PagedVirtualList.svelte';
  import type { VirtualListRow } from './PagedVirtualList.svelte';
  import TrackColumnCells from './TrackColumnCells.svelte';
  import { cacheQuickIndex, getCachedQuickIndex, getQuickIndexBuildKey } from './quick-index.js';

  interface Props {
    query: string;
    resetKey?: string | number;
    listId?: string;
    selectedTrackId: string | null;
    playbackReady: boolean;
    isSendingPlaybackCommand: boolean;
    onPlay: (track: TrackSummary) => void | Promise<void>;
    onViewportScrollTopChange?: (scrollTop: number, collapseOffset: number) => void;
    onTotalCount?: (count: number | null) => void;
    sortable?: boolean;
    groupingEnabled?: boolean;
    quickIndexOverride?: { labels: string[]; groups: { label: string; beforeIndex: number }[] } | null;
    sortableIndex?: boolean;
    sortIndicator?: { field: string; direction: 'asc' | 'desc' } | null;
    indexSortLabel?: string;
    onIndexSort?: () => void;
    getIndexLabel?: (track: TrackSummary, index: number) => string;
    quickIndexArtistNames?: string[];
    quickIndexAlbumNames?: string[];
    fetchPage?: (request: TrackPageRequest) => Promise<TrackPage>;
    columns?: TrackListColumnPreference[];
    columnWidths?: TrackColumnWidths;
    rowHeightPx?: number;
    leadingOverlayHeight?: number;
    leadingCollapseRange?: number;
    onColumnResize?: (columnId: TrackListColumnPreference['id'], widthPx: number) => void;
    sort?: LibraryListSort;
    onSort?: (columnId: TrackListColumnPreference['id']) => void;
    fieldFilter?: TrackPageRequest['fieldFilter'];
    livePlayCount?: { trackId: string; playedMs: number }[];
    /** Shows the floating "locate the playing track" button (library list only): the playing track's id, or null when
     *  nothing plays. Left out, there is no button. */
    locateTrackId?: string | null;
  }

  let {
    query,
    resetKey = '',
    listId = 'library-track',
    selectedTrackId,
    playbackReady,
    isSendingPlaybackCommand,
    onPlay,
    onViewportScrollTopChange,
    onTotalCount,
    sortable = true,
    groupingEnabled = true,
    quickIndexOverride,
    sortableIndex = false,
    sortIndicator,
    indexSortLabel,
    onIndexSort,
    getIndexLabel = (_track, index) => formatListOrder(index),
    quickIndexArtistNames,
    quickIndexAlbumNames,
    fetchPage,
    columns = DEFAULT_TRACK_COLUMN_PREFERENCES,
    columnWidths = DEFAULT_TRACK_COLUMN_WIDTHS,
    rowHeightPx = DEFAULT_TRACK_ROW_HEIGHT_PX,
    leadingOverlayHeight = 0,
    leadingCollapseRange = 0,
    onColumnResize,
    sort = DEFAULT_LIBRARY_SORT,
    onSort,
    livePlayCount = [],
    fieldFilter = null,
    locateTrackId,
  }: Props = $props();

  let normalizedColumns = $derived(normalizeTrackColumnPreferences(columns));
  let normalizedWidths = $derived(normalizeTrackColumnWidths(columnWidths));
  let normalizedSort = $derived(normalizeLibrarySort(sort));
  let visibleSortIndicator = $derived(sortIndicator === undefined ? normalizedSort : sortIndicator);
  const listScrollResetKey = $derived(JSON.stringify({ query: query.trim(), fieldFilter, sort: normalizedSort }));
  let visibleColumns = $derived(visibleTrackColumns(normalizedColumns).map(({ id }) =>
    TRACK_COLUMN_DEFINITIONS.find((definition) => definition.id === id)!,
  ));
  let listGridTemplate = $derived(trackListGridTemplate(normalizedColumns, normalizedWidths));
  let listMinWidth = $derived(trackListMinWidthPx(normalizedColumns, normalizedWidths));
  let listColumnCount = $derived(trackListColumnCount(normalizedColumns));

  type ListSnapshot = ReturnType<PagedListController<TrackSummary, string>['snapshot']>;

  let snapshot = $state<ListSnapshot>({
    scope: null,
    totalCount: null,
    cachedPageCount: 0,
    cachedItemCount: 0,
    pendingPageCount: 0,
    refreshing: false,
    errors: [],
    generation: 0,
    revision: 0,
  });
  let quickIndex = $state<{ labels: string[]; groups: { label: string; beforeIndex: number }[] } | null>(null);
  let quickIndexReady = $state(false);
  let visibleQuickIndex = $derived(!groupingEnabled ? null : quickIndexOverride === undefined ? quickIndex : quickIndexOverride);
  let quickIndexGeneration = 0;
  let currentQuickIndexBuildKey: string | null = null;

  async function requestTrackPage(request: TrackPageRequest): Promise<TrackPage> {
    return fetchPage ? fetchPage(request) : invokeCommand('library_get_page', request);
  }

  const listData = new PagedListController<TrackSummary, string>(
    async ({ scope, offset, limit }) => {
      const requestScope = JSON.parse(scope || '{"query":null,"fieldFilter":null,"sort":null}') as {
        query: string | null;
        fieldFilter: TrackPageRequest['fieldFilter'];
        sort: LibraryListSort;
      };
      const request: TrackPageRequest = {
        query: requestScope.query,
        fieldFilter: requestScope.fieldFilter,
        sort: requestScope.sort,
        offset,
        limit,
      };
      return requestTrackPage(request);
    },
    { listName: tr('曲庫'), onChange: () => { snapshot = listData.snapshot(); } },
  );

  $effect(() => {
    const normalizedQuery = query.trim();
    const currentResetKey = resetKey;
    const currentFieldFilter = fieldFilter;
    const currentSort = normalizeLibrarySort(sort);
    const scope = JSON.stringify({
      query: normalizedQuery || null,
      fieldFilter: currentFieldFilter ?? null,
      sort: currentSort,
    });
    listData.reset(scope, currentResetKey);
  });

  $effect(() => {
    const count = snapshot.totalCount;
    const currentSort = normalizedSort;
    const currentQuery = query.trim() || null;
    const currentFieldFilter = fieldFilter ?? null;
    const activeSort = visibleSortIndicator;
    const shouldGroup = groupingEnabled && sortable && activeSort
      && isGroupedSortField(activeSort.field);
    const cacheKey = getQuickIndexBuildKey({
      listId,
      resetKey,
      query: currentQuery,
      fieldFilter: currentFieldFilter,
      sort: currentSort,
      count,
      activeSort,
      groupingEnabled,
      sortable,
    });
    if (cacheKey === currentQuickIndexBuildKey) return;
    currentQuickIndexBuildKey = cacheKey;
    const generation = ++quickIndexGeneration;
    quickIndex = null;
    quickIndexReady = !shouldGroup || count === 0;
    if (!shouldGroup || count === null || count <= 0) return;
    const cached = getCachedQuickIndex(cacheKey);
    if (cached) {
      quickIndex = cached;
      quickIndexReady = true;
      return;
    }
    quickIndexReady = false;
    const isCurrent = () => generation === quickIndexGeneration;
    void (async () => {
      const response = fetchPage
        ? quickIndexArtistNames
          ? await invokeCommand('library_get_quick_index_for_artists', {
            artistNames: quickIndexArtistNames,
            sort: currentSort,
          })
          : quickIndexAlbumNames
            ? await invokeCommand('library_get_quick_index_for_album_tracks', {
              albumNames: quickIndexAlbumNames,
              sort: currentSort,
            })
            : null
        : await invokeCommand('library_get_quick_index_for_tracks', {
          query: currentQuery,
          fieldFilter: currentFieldFilter,
          sort: currentSort,
        });
      if (!response) return null;
      return { labels: response.labels, groups: response.groups };
    })().then((result) => {
      if (isCurrent() && result) {
        cacheQuickIndex(cacheKey, result);
        quickIndex = result;
      }
      if (isCurrent()) quickIndexReady = true;
    }).catch((reason: unknown) => {
      if (isCurrent()) {
        console.error(tr('無法建立曲目首字索引'), reason);
        quickIndexReady = true;
      }
    });
  });

  $effect(() => {
    onTotalCount?.(snapshot.totalCount);
  });

  function retryFirstError(): void {
    const firstError = snapshot.errors[0];
    if (firstError) void listData.retry(firstError.offset);
  }

  function playRow(track: TrackSummary): void {
    // A command in flight does not block choosing a track: the latest choice wins (playback-command-order.js).
    if (playbackReady) void onPlay(track);
  }

  function handleRange(range: { start: number; end: number }): void {
    void listData.ensureRange(range.start, range.end);
  }

  function itemAt(index: number): TrackSummary | null {
    return listData.itemAt(index);
  }

  let scrollToIndexRequest = $state<{ index: number; token: number } | null>(null);
  let scrollRequestToken = 0;
  let locating = $state(false);
  let locateError = $state<string | null>(null);
  let showLocate = $derived(locateTrackId !== undefined && !fetchPage);

  /** Scroll the list to the playing track: ask the backend where it is in this list's order, load that page, scroll. */
  async function locateCurrentTrack(): Promise<void> {
    if (locating || !locateTrackId || snapshot.totalCount === 0) return;
    locating = true;
    locateError = null;
    try {
      const currentQuery = query.trim() || null;
      const index = await invokeCommand('library_get_track_position', {
        query: currentQuery,
        fieldFilter: fieldFilter ?? null,
        sort: normalizedSort,
        trackId: locateTrackId,
      });
      if (index === null) {
        locateError = tr('目前的列表（含搜尋與篩選）中沒有這首曲目。');
        return;
      }
      const start = Math.max(0, index - 250);
      await listData.ensureRange(start, start + 500);
      scrollRequestToken += 1;
      scrollToIndexRequest = { index, token: scrollRequestToken };
    } catch (error) {
      locateError = getErrorText(error);
    } finally {
      locating = false;
    }
  }

  function handleSort(columnId: string): void {
    onSort?.(columnId as TrackListColumnPreference['id']);
  }

  function handleResize(columnId: string, widthPx: number): void {
    onColumnResize?.(columnId as TrackListColumnPreference['id'], widthPx);
  }
</script>

<div class="track-list-shell" class:album-detail-overlay={leadingOverlayHeight > 0} class:has-locate={showLocate}>
{#if snapshot.errors.length > 0}
  <div class="track-list-error" role="alert">
    <span>{getErrorText(snapshot.errors[0]?.message)}</span>
    <button type="button" class="text-button" onclick={retryFirstError}>{tr("重試載入")}</button>
  </div>
{/if}

{#if locateError}
  <div class="locate-status" role="status">{locateError}</div>
{/if}

{#if snapshot.totalCount === 0}
  <div class="track-list-empty" role="status">{query.trim() ? tr('找不到相符曲目。') : tr('曲庫目前是空的。')}</div>
{:else}
  <PagedVirtualList
    totalCount={snapshot.totalCount}
    pending={snapshot.pendingPageCount > 0}
    revision={snapshot.revision}
    generation={snapshot.generation}
    {itemAt}
    getKey={(track) => track.id}
    isSelected={(track) => selectedTrackId === track.id}
    rowHeight={rowHeightPx}
    compactRowHeight={rowHeightPx}
    shortDesktopRowHeight={rowHeightPx}
    headerHeight={37}
    compactHeaderHeight={37}
    fillAvailable={true}
    {leadingOverlayHeight} {leadingCollapseRange}
    {listId}
    scrollResetKey={listScrollResetKey}
    groupRows={visibleQuickIndex?.groups}
    groupingReady={!groupingEnabled || (quickIndexOverride === undefined ? quickIndexReady : quickIndexOverride !== null)}
    quickIndexLabels={visibleQuickIndex?.labels}
    ariaLabel={sortable
      ? tr('曲庫曲目；使用方向鍵瀏覽，按 Enter 播放目前曲目；點擊欄位標題排序')
      : tr('曲目；使用方向鍵瀏覽，按 Enter 播放目前曲目')}
    columnCount={listColumnCount}
    gridTemplate={listGridTemplate}
    minContentWidth={listMinWidth}
    className="track-list-viewport"
    rowClassName="track-row virtual-track-row configurable-track-grid"
    listName={tr("曲庫")}
    {scrollToIndexRequest}
    onRange={handleRange}
    onPlay={(track) => playRow(track)}
    {onViewportScrollTopChange}
  >
    {#snippet header()}
      <div class="track-table-head track-list-header configurable-track-grid" role="row" aria-rowindex="1">
        <TrackColumnCells
          variant="header"
          columns={visibleColumns}
          {sortable}
          sortableIndex={sortableIndex}
          sort={visibleSortIndicator}
          {indexSortLabel}
          onSort={handleSort}
          {onIndexSort}
          resizable={true}
          onResize={handleResize}
        />
      </div>
    {/snippet}
    {#snippet row(row: VirtualListRow<TrackSummary>)}
      {#if row.item}
        <TrackColumnCells
          variant="item"
          columns={visibleColumns}
          indexLabel={getIndexLabel(row.item!, row.index)}
          cells={visibleColumns.map((column) => {
            const item = withLivePlayedMs(row.item!, livePlayCount);
            return {
              id: column.id,
              text: formatTrackColumnValue(column.id, item, formatDuration),
              hiRes: column.id === 'audioFormat' ? isHiResTrack(item) : false,
            };
          })}
        />
      {:else}
        <TrackColumnCells variant="placeholder" columns={visibleColumns} />
      {/if}
    {/snippet}
  </PagedVirtualList>
  {#if showLocate}
    <button
      type="button"
      class="locate-fab"
      aria-label={tr("定位到目前播放曲目")}
      use:appTooltip={tr('定位到目前播放曲目')}
      disabled={!locateTrackId || locating}
      onclick={() => void locateCurrentTrack()}
    ><span class="locate-fab-dot" aria-hidden="true"></span></button>
  {/if}
{/if}
</div>

<style>
  :global(.track-list-viewport) {
    position: relative;
    width: 100%;
    overflow: auto;
    overscroll-behavior: none;
    scrollbar-gutter: stable;
    outline: none;
  }

  :global(.track-list-viewport:focus-visible) {
    outline: var(--focus-ring);
    outline-offset: 2px;
  }

  .track-list-shell.album-detail-overlay {
    position: absolute;
    inset: 0;
    z-index: 1;
  }

  :global(.track-list-header) {
    box-sizing: border-box;
    height: var(--paged-header-height, 37px);
    min-height: var(--paged-header-height, 37px);
  }

  :global(.virtual-track-row) {
    min-height: var(--paged-row-height, 50px);
  }

  .track-list-shell.has-locate {
    position: relative;
  }

</style>
