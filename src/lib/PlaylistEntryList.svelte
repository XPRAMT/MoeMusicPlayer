<script lang="ts">
import { getErrorText, invokeCommand, type PlaylistEntrySummary, type PlaylistListSort, type PlaylistPage, type TrackColumnWidths, type TrackListColumnPreference } from './ipc';
import { formatDuration } from './format';
import { formatListOrder } from './list-order.js';
import {
  DEFAULT_TRACK_COLUMN_PREFERENCES,
  DEFAULT_TRACK_ROW_HEIGHT_PX,
  DEFAULT_TRACK_COLUMN_WIDTHS,
  DEFAULT_PLAYLIST_SORT,
  TRACK_COLUMN_DEFINITIONS,
  formatTrackColumnValue,
  isHiResTrack,
  normalizeTrackColumnPreferences,
  normalizeTrackColumnWidths,
  trackListColumnCount,
  trackListGridTemplate,
  trackListMinWidthPx,
  togglePlaylistSort,
  visibleTrackColumns,
} from './track-columns.js';
import { withLivePlayedMs } from './live-play-count.js';
  import { PagedListController, TRACK_PAGE_SIZE } from './track-list-data.js';
  import PagedVirtualList from './PagedVirtualList.svelte';
  import type { VirtualListRow } from './PagedVirtualList.svelte';
  import TrackColumnCells from './TrackColumnCells.svelte';

  interface Props {
    playlistId: string;
    resetKey: number;
    playbackReady: boolean;
    isSendingPlaybackCommand: boolean;
    onPlay: (entry: PlaylistEntrySummary, playlistId: string) => void;
    onTotalCount?: (count: number | null) => void;
    fetchPage?: (request: { playlistId: string; sort: PlaylistListSort; offset: number; limit: number }) => Promise<PlaylistPage>;
    columns?: TrackListColumnPreference[];
    columnWidths?: TrackColumnWidths;
    rowHeightPx?: number;
    onColumnResize?: (columnId: TrackListColumnPreference['id'], widthPx: number) => void;
    livePlayCount?: { trackId: string; playedMs: number } | null;
    currentTrackId?: string | null;
  }

  let {
    playlistId,
    resetKey,
    playbackReady,
    isSendingPlaybackCommand,
    onPlay,
    onTotalCount,
    fetchPage,
    columns = DEFAULT_TRACK_COLUMN_PREFERENCES,
    columnWidths = DEFAULT_TRACK_COLUMN_WIDTHS,
    rowHeightPx = DEFAULT_TRACK_ROW_HEIGHT_PX,
    onColumnResize,
    livePlayCount = null,
    currentTrackId = null,
  }: Props = $props();

  let normalizedColumns = $derived(normalizeTrackColumnPreferences(columns));
  let normalizedWidths = $derived(normalizeTrackColumnWidths(columnWidths));
  let visibleColumns = $derived(visibleTrackColumns(normalizedColumns).map(({ id }) =>
    TRACK_COLUMN_DEFINITIONS.find((definition) => definition.id === id)!,
  ));
  let listGridTemplate = $derived(trackListGridTemplate(normalizedColumns, normalizedWidths));
  let listMinWidth = $derived(trackListMinWidthPx(normalizedColumns, normalizedWidths));
  let listColumnCount = $derived(trackListColumnCount(normalizedColumns));
  let sort = $state<PlaylistListSort>({ ...DEFAULT_PLAYLIST_SORT });

  type ListSnapshot = ReturnType<PagedListController<PlaylistEntrySummary, string>['snapshot']>;
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
  const entries = new PagedListController<PlaylistEntrySummary, string>(
    ({ scope, offset, limit }) => {
      const requestScope = JSON.parse(scope) as { playlistId: string; sort: PlaylistListSort };
      const request = { ...requestScope, offset, limit };
      return fetchPage
        ? fetchPage(request)
        : invokeCommand('playlist_get_page', request);
    },
    { listName: '播放清單', onChange: () => { snapshot = entries.snapshot(); } },
  );

  $effect(() => {
    const currentId = playlistId;
    const currentRevision = resetKey;
    const currentSort = sort;
    entries.reset(JSON.stringify({ playlistId: currentId, sort: currentSort }), currentRevision);
    void entries.ensureRange(0, TRACK_PAGE_SIZE);
  });

  $effect(() => {
    onTotalCount?.(snapshot.totalCount);
  });

  function itemAt(index: number): PlaylistEntrySummary | null {
    return entries.itemAt(index);
  }

  function handleRange(range: { start: number; end: number }): void {
    void entries.ensureRange(range.start, range.end);
  }

  function retryFirstError(): void {
    const error = snapshot.errors[0];
    if (error) void entries.retry(error.offset);
  }

  function handleSort(columnId: string): void {
    const field = columnId === 'position' || TRACK_COLUMN_DEFINITIONS.some((column) => column.id === columnId)
      ? columnId as PlaylistListSort['field']
      : null;
    if (!field) return;
    sort = togglePlaylistSort(sort, field);
  }

  function playEntry(entry: PlaylistEntrySummary): void {
    if (!entry.trackId || !entry.hasEnabledMapping || !playbackReady || isSendingPlaybackCommand) return;
    onPlay(entry, playlistId);
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
  <div class="empty-panel playlist-empty"><h3>這份清單沒有項目</h3><p>在「設定 → 音樂來源」更新此播放清單來源後再重新整理。</p></div>
{:else}
  <PagedVirtualList
    totalCount={snapshot.totalCount}
    pending={snapshot.pendingPageCount > 0}
    revision={snapshot.revision}
    generation={snapshot.generation}
    {itemAt}
    getKey={(entry) => entry.position}
    isSelected={(entry) => entry.trackId !== null && entry.trackId === currentTrackId}
    rowHeight={rowHeightPx}
    compactRowHeight={rowHeightPx}
    shortDesktopRowHeight={rowHeightPx}
    headerHeight={37}
    compactHeaderHeight={37}
    fillAvailable={true}
    listId={`playlist-${playlistId}`}
    ariaLabel="播放清單項目；點擊欄位標題排序；使用方向鍵瀏覽，按 Enter 播放目前項目"
    columnCount={listColumnCount}
    gridTemplate={listGridTemplate}
    minContentWidth={listMinWidth}
    className="track-list-viewport playlist-entry-table"
    rowClassName="track-row virtual-track-row configurable-track-grid playlist-entry-row"
    listName="播放清單"
    onRange={handleRange}
    onPlay={(entry) => playEntry(entry)}
  >
    {#snippet header()}
      <div class="track-table-head track-list-header configurable-track-grid" role="row" aria-rowindex="1">
        <TrackColumnCells
          variant="header"
          columns={visibleColumns}
          sortable={true}
          sortableIndex={true}
          {sort}
          onSort={handleSort}
          resizable={true}
          onResize={(columnId, widthPx) => onColumnResize?.(columnId as TrackListColumnPreference['id'], widthPx)}
        />
      </div>
    {/snippet}
    {#snippet row(row: VirtualListRow<PlaylistEntrySummary>)}
      {#if row.item}
        <TrackColumnCells
          variant="item"
          columns={visibleColumns}
          indexLabel={formatListOrder(row.item.position)}
          cells={visibleColumns.map((column) => {
            const item = withLivePlayedMs(row.item!, livePlayCount);
            return {
              id: column.id,
              text: formatTrackColumnValue(column.id, item, formatDuration, '未命名項目'),
              title: column.id === 'title' && !row.item?.hasEnabledMapping ? '目前未對應到可播放的曲庫曲目' : undefined,
              hiRes: column.id === 'audioFormat' ? isHiResTrack(item) : false,
            };
          })}
        />
      {:else}
        <TrackColumnCells variant="placeholder" columns={visibleColumns} placeholderTitle="正在載入項目…" />
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
</style>
