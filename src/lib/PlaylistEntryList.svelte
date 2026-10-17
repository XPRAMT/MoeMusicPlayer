<script module lang="ts">
  import type { PlaylistListSort as PlaylistSortCacheValue } from './ipc';
  import { DEFAULT_PLAYLIST_SORT } from './track-columns.js';

  const PLAYLIST_SORT_CACHE_LIMIT = 64;
  const playlistSortById = new Map<string, PlaylistSortCacheValue>();

  function readPlaylistSort(playlistId: string): PlaylistSortCacheValue {
    const cached = playlistSortById.get(playlistId);
    if (!cached) return { ...DEFAULT_PLAYLIST_SORT };
    playlistSortById.delete(playlistId);
    playlistSortById.set(playlistId, cached);
    return { ...cached };
  }

  function writePlaylistSort(playlistId: string, sort: PlaylistSortCacheValue): void {
    playlistSortById.delete(playlistId);
    playlistSortById.set(playlistId, { ...sort });
    if (playlistSortById.size > PLAYLIST_SORT_CACHE_LIMIT) {
      const oldestPlaylistId = playlistSortById.keys().next().value;
      if (oldestPlaylistId !== undefined) playlistSortById.delete(oldestPlaylistId);
    }
  }
</script>

<script lang="ts">
  import { tr } from './i18n.js';
import { getErrorText, invokeCommand, type PlaylistEntrySummary, type PlaylistListSort, type PlaylistPage, type TrackColumnWidths, type TrackListColumnPreference } from './ipc';
import { formatDuration } from './format';
import { formatListOrder } from './list-order.js';
import {
  DEFAULT_PLAYLIST_SORT as INITIAL_PLAYLIST_SORT,
  DEFAULT_TRACK_COLUMN_PREFERENCES,
  DEFAULT_TRACK_ROW_HEIGHT_PX,
  DEFAULT_TRACK_COLUMN_WIDTHS,
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
import { cacheQuickIndex, getCachedQuickIndex } from './quick-index.js';
import { PagedListController } from './track-list-data.js';
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
    groupingEnabled?: boolean;
    onColumnResize?: (columnId: TrackListColumnPreference['id'], widthPx: number) => void;
    livePlayCount?: { trackId: string; playedMs: number }[];
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
    groupingEnabled = true,
    onColumnResize,
    livePlayCount = [],
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
  let sort = $state<PlaylistListSort>({ ...INITIAL_PLAYLIST_SORT });
  let sortPlaylistId = $state<string | null>(null);

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
  let quickIndex = $state<{ labels: string[]; groups: { label: string; beforeIndex: number }[] } | null>(null);
  let quickIndexReady = $state(false);
  let quickIndexGeneration = 0;

  async function requestPlaylistPage(request: { playlistId: string; sort: PlaylistListSort; offset: number; limit: number }): Promise<PlaylistPage> {
    return fetchPage ? fetchPage(request) : invokeCommand('playlist_get_page', request);
  }

  const entries = new PagedListController<PlaylistEntrySummary, string>(
    ({ scope, offset, limit }) => {
      const requestScope = JSON.parse(scope) as { playlistId: string; sort: PlaylistListSort };
      const request = { ...requestScope, offset, limit };
      return requestPlaylistPage(request);
    },
    { listName: tr('播放清單'), onChange: () => { snapshot = entries.snapshot(); } },
  );

  $effect(() => {
    const currentId = playlistId;
    if (currentId !== sortPlaylistId) {
      sort = readPlaylistSort(currentId);
      sortPlaylistId = currentId;
    }
    const currentRevision = resetKey;
    const currentSort = sort;
    entries.reset(JSON.stringify({ playlistId: currentId, sort: currentSort }), currentRevision);
  });

  $effect(() => {
    const count = snapshot.totalCount;
    const currentPlaylistId = playlistId;
    const currentSort = sort;
    const enabled = groupingEnabled && ['title', 'artist', 'album'].includes(currentSort.field);
    const generation = ++quickIndexGeneration;
    quickIndex = null;
    quickIndexReady = !enabled || count === 0;
    if (!enabled || count === null || count <= 0) return;
    const cacheKey = JSON.stringify({ playlistId: currentPlaylistId, resetKey, sort: currentSort, count });
    const cached = getCachedQuickIndex(cacheKey);
    if (cached) {
      quickIndex = cached;
      quickIndexReady = true;
      return;
    }
    quickIndexReady = false;
    const isCurrent = () => generation === quickIndexGeneration;
    void invokeCommand('playlist_get_quick_index_groupings', {
      playlistId: currentPlaylistId,
      sort: currentSort,
    }).then((response) => ({ labels: response.labels, groups: response.groups })).then((result) => {
      if (isCurrent() && result) {
        cacheQuickIndex(cacheKey, result);
        quickIndex = result;
      }
      if (isCurrent()) quickIndexReady = true;
    }).catch((reason: unknown) => {
      if (isCurrent()) {
        console.error(tr('無法建立播放清單首字索引'), reason);
        quickIndexReady = true;
      }
    });
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
    writePlaylistSort(playlistId, sort);
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
    <button type="button" class="text-button" onclick={retryFirstError}>{tr("重試載入")}</button>
  </div>
{/if}

{#if snapshot.totalCount === 0}
  <div class="empty-panel playlist-empty"><h3>{tr("這份清單沒有項目")}</h3><p>{tr("在「設定 → 音樂來源」更新此播放清單來源後再重新整理。")}</p></div>
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
    ariaLabel={tr("播放清單項目；點擊欄位標題排序；使用方向鍵瀏覽，按 Enter 播放目前項目")}
    columnCount={listColumnCount}
    gridTemplate={listGridTemplate}
    minContentWidth={listMinWidth}
    className="track-list-viewport playlist-entry-table"
    scrollResetKey={playlistId}
    groupRows={groupingEnabled ? quickIndex?.groups : []}
    groupingReady={!groupingEnabled || quickIndexReady}
    quickIndexLabels={groupingEnabled ? quickIndex?.labels : []}
    scrollRestoreReady={snapshot.scope === JSON.stringify({ playlistId, sort }) && snapshot.totalCount !== null && snapshot.pendingPageCount === 0}
    rowClassName="track-row virtual-track-row configurable-track-grid playlist-entry-row"
    listName={tr("播放清單")}
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
              text: formatTrackColumnValue(column.id, item, formatDuration, tr('未命名項目')),
              title: column.id === 'title' && !row.item?.hasEnabledMapping ? tr('目前未對應到可播放的曲庫曲目') : undefined,
              hiRes: column.id === 'audioFormat' ? isHiResTrack(item) : false,
            };
          })}
        />
      {:else}
        <TrackColumnCells variant="placeholder" columns={visibleColumns} />
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
