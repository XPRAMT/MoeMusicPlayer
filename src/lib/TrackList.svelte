<script lang="ts">
  import {
    getErrorText,
    invokeCommand,
    type TrackPage,
    type TrackPageRequest,
    type TrackSummary,
    type TrackListColumnPreference,
  } from './ipc';
  import { formatDuration, formatTrackIndex } from './format';
  import {
    DEFAULT_TRACK_COLUMN_PREFERENCES,
    TRACK_COLUMN_DEFINITIONS,
    formatTrackColumnValue,
    normalizeTrackColumnPreferences,
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
    livePlayCount = null,
    fieldFilter = null,
  }: Props = $props();

  let normalizedColumns = $derived(normalizeTrackColumnPreferences(columns));
  let visibleColumns = $derived(visibleTrackColumns(normalizedColumns).map(({ id }) =>
    TRACK_COLUMN_DEFINITIONS.find((definition) => definition.id === id)!,
  ));
  let listGridTemplate = $derived(trackListGridTemplate(normalizedColumns));
  let listMinWidth = $derived(trackListMinWidthPx(normalizedColumns));
  let listColumnCount = $derived(trackListColumnCount(normalizedColumns));

  type ListSnapshot = ReturnType<PagedListController<TrackSummary, string>['snapshot']>;

  let snapshot = $state<ListSnapshot>({
    scope: null,
    totalCount: null,
    cachedPageCount: 0,
    cachedItemCount: 0,
    pendingPageCount: 0,
    errors: [],
    generation: 0,
    revision: 0,
  });
  const listData = new PagedListController<TrackSummary, string>(
    ({ scope, offset, limit }) => {
      const requestScope = JSON.parse(scope || '{"query":null,"fieldFilter":null}') as {
        query: string | null;
        fieldFilter: TrackPageRequest['fieldFilter'];
      };
      const request = { ...requestScope, offset, limit };
      return fetchPage
        ? fetchPage(request)
        : invokeCommand('library_get_page', request);
    },
    { listName: '曲庫', onChange: () => { snapshot = listData.snapshot(); } },
  );

  let virtualRange = $state({ start: 0, end: TRACK_PAGE_SIZE });
  let rangeLabel = $derived.by(() => {
    if (snapshot.totalCount === null) return '正在載入曲庫…';
    if (snapshot.totalCount === 0) return '0 首';
    const first = virtualRange.start + 1;
    const last = Math.min(virtualRange.end, snapshot.totalCount);
    return `${first.toLocaleString()}–${last.toLocaleString()} 首，共 ${snapshot.totalCount.toLocaleString()} 首`;
  });

  $effect(() => {
    const normalizedQuery = query.trim();
    const currentResetKey = resetKey;
    const currentFieldFilter = fieldFilter;
    const scope = JSON.stringify({
      query: normalizedQuery || null,
      fieldFilter: currentFieldFilter ?? null,
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
    virtualRange = range;
    void listData.ensureRange(range.start, range.end);
  }

  function itemAt(index: number): TrackSummary | null {
    return listData.itemAt(index);
  }
</script>

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
    maxViewportHeight={680}
    listId="library-track"
    ariaLabel="曲庫曲目；使用方向鍵瀏覽，按 Enter 播放目前曲目"
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
        <TrackColumnCells variant="header" columns={visibleColumns} />
      </div>
    {/snippet}
    {#snippet row(row: VirtualListRow<TrackSummary>)}
      {#if row.item}
        <TrackColumnCells
          variant="item"
          columns={visibleColumns}
          indexLabel={formatTrackIndex(row.item.trackNumber, row.item.discNumber)}
          cells={visibleColumns.map((column) => ({
            id: column.id,
            text: formatTrackColumnValue(column.id, withLivePlayedMs(row.item!, livePlayCount), formatDuration),
          }))}
        />
      {:else}
        <TrackColumnCells variant="placeholder" columns={visibleColumns} placeholderTitle="正在載入曲目…" />
      {/if}
    {/snippet}
  </PagedVirtualList>
{/if}

<div class="track-list-count" role="status" aria-live="polite">{rangeLabel}</div>

<style>
  :global(.track-list-viewport) {
    position: relative;
    width: 100%;
    max-height: min(65vh, 680px);
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
    background: rgba(var(--text-rgb), 0.2);
  }

  :global(.virtual-track-row) {
    min-height: 0;
  }

  .track-list-error,
  .track-list-empty,
  .track-list-count {
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

  .track-list-count {
    min-height: 22px;
    padding-top: 8px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
</style>
