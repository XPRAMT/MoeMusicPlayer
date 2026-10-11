<script lang="ts">
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
    DEFAULT_TRACK_COLUMN_WIDTHS,
    TRACK_COLUMN_DEFINITIONS,
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
  import {
    PagedListController,
    TRACK_PAGE_SIZE,
  } from './track-list-data.js';
  import PagedVirtualList from './PagedVirtualList.svelte';
  import type { VirtualListRow } from './PagedVirtualList.svelte';
  import TrackColumnCells from './TrackColumnCells.svelte';

  interface Props {
    query: string;
    resetKey?: string | number;
    selectedTrackId: string | null;
    playbackReady: boolean;
    isSendingPlaybackCommand: boolean;
    onPlay: (track: TrackSummary) => void | Promise<void>;
    onTotalCount?: (count: number | null) => void;
    fetchPage?: (request: TrackPageRequest) => Promise<TrackPage>;
    columns?: TrackListColumnPreference[];
    columnWidths?: TrackColumnWidths;
    onColumnResize?: (columnId: TrackListColumnPreference['id'], widthPx: number) => void;
    sort?: LibraryListSort;
    onSort?: (columnId: TrackListColumnPreference['id']) => void;
    fieldFilter?: TrackPageRequest['fieldFilter'];
    livePlayCount?: { trackId: string; playedMs: number } | null;
  }

  let {
    query,
    resetKey = '',
    selectedTrackId,
    playbackReady,
    isSendingPlaybackCommand,
    onPlay,
    onTotalCount,
    fetchPage,
    columns = DEFAULT_TRACK_COLUMN_PREFERENCES,
    columnWidths = DEFAULT_TRACK_COLUMN_WIDTHS,
    onColumnResize,
    sort = DEFAULT_LIBRARY_SORT,
    onSort,
    livePlayCount = null,
    fieldFilter = null,
  }: Props = $props();

  let normalizedColumns = $derived(normalizeTrackColumnPreferences(columns));
  let normalizedWidths = $derived(normalizeTrackColumnWidths(columnWidths));
  let normalizedSort = $derived(normalizeLibrarySort(sort));
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
  const listData = new PagedListController<TrackSummary, string>(
    ({ scope, offset, limit }) => {
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
      return fetchPage
        ? fetchPage(request)
        : invokeCommand('library_get_page', request);
    },
    { listName: '曲庫', onChange: () => { snapshot = listData.snapshot(); } },
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
    void listData.ensureRange(0, TRACK_PAGE_SIZE);
  });

  $effect(() => {
    onTotalCount?.(snapshot.totalCount);
  });

  function retryFirstError(): void {
    const firstError = snapshot.errors[0];
    if (firstError) void listData.retry(firstError.offset);
  }

  function playRow(track: TrackSummary): void {
    if (playbackReady && !isSendingPlaybackCommand) void onPlay(track);
  }

  function handleRange(range: { start: number; end: number }): void {
    void listData.ensureRange(range.start, range.end);
  }

  function itemAt(index: number): TrackSummary | null {
    return listData.itemAt(index);
  }

  function handleSort(columnId: string): void {
    onSort?.(columnId as TrackListColumnPreference['id']);
  }

  function handleResize(columnId: string, widthPx: number): void {
    onColumnResize?.(columnId as TrackListColumnPreference['id'], widthPx);
  }
</script>

<div class="track-list-shell">
{#if snapshot.errors.length > 0}
  <div class="track-list-error" role="alert">
    <span>{getErrorText(snapshot.errors[0]?.message)}</span>
    <button type="button" class="text-button" onclick={retryFirstError}>重試載入</button>
  </div>
{/if}

{#if snapshot.totalCount === 0}
  <div class="track-list-empty" role="status">{query.trim() ? '找不到相符曲目。' : '曲庫目前是空的。'}</div>
{:else}
  <PagedVirtualList
    totalCount={snapshot.totalCount}
    pending={snapshot.pendingPageCount > 0}
    revision={snapshot.revision}
    generation={snapshot.generation}
    {itemAt}
    getKey={(track) => track.id}
    isSelected={(track) => selectedTrackId === track.id}
    rowHeight={57}
    compactRowHeight={53}
    shortDesktopRowHeight={49}
    headerHeight={37}
    compactHeaderHeight={33}
    fillAvailable={true}
    listId="library-track"
    ariaLabel="曲庫曲目；使用方向鍵瀏覽，按 Enter 播放目前曲目；點擊欄位標題排序"
    columnCount={listColumnCount}
    gridTemplate={listGridTemplate}
    minContentWidth={listMinWidth}
    className="track-list-viewport"
    rowClassName="track-row virtual-track-row configurable-track-grid"
    listName="曲庫"
    onRange={handleRange}
    onPlay={(track) => playRow(track)}
  >
    {#snippet header()}
      <div class="track-table-head track-list-header configurable-track-grid" role="row" aria-rowindex="1">
        <TrackColumnCells
          variant="header"
          columns={visibleColumns}
          sortable={true}
          sort={normalizedSort}
          onSort={handleSort}
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
          indexLabel={formatListOrder(row.index)}
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
        <TrackColumnCells variant="placeholder" columns={visibleColumns} placeholderTitle="正在載入曲目…" />
      {/if}
    {/snippet}
  </PagedVirtualList>
{/if}
</div>

<style>
  .track-list-shell {
    display: flex;
    min-width: 0;
    min-height: 0;
    flex: 1 1 auto;
    flex-direction: column;
  }

  :global(.track-list-viewport) {
    position: relative;
    width: 100%;
    overflow: auto;
    overscroll-behavior: contain;
    scrollbar-gutter: stable;
    outline: none;
  }

  :global(.track-list-viewport:focus-visible) {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }

  :global(.track-list-header) {
    box-sizing: border-box;
    height: var(--paged-header-height, 37px);
    min-height: var(--paged-header-height, 37px);
  }

  :global(.virtual-track-row) {
    min-height: 0;
  }

  .track-list-error,
  .track-list-empty {
    color: var(--muted);
    font-size: var(--font-body);
  }

  .track-list-error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin: 0 0 10px;
    padding: 10px 12px;
    border: 1px solid color-mix(in srgb, var(--status-danger) 40%, transparent);
    border-radius: 9px;
    color: var(--status-danger);
    background: color-mix(in srgb, var(--status-danger) 12%, transparent);
  }

  .track-list-error .text-button {
    flex: 0 0 auto;
  }

  .track-list-empty {
    padding: 28px 16px;
    text-align: center;
  }
</style>
