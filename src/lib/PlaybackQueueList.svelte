<script lang="ts">
  import { getErrorText, invokeCommand, type PlaybackQueuePage, type PlaybackQueuePageItem, type TrackListColumnPreference } from './ipc';
  import { formatDuration } from './format';
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
  import { PagedListController, TRACK_PAGE_SIZE } from './track-list-data.js';
  import {
    adaptPlaybackQueuePage,
    isCurrentPlaybackQueueEntry,
    playbackQueueCurrentEntryPosition,
  } from './playback-queue-data.js';
  import PagedVirtualList from './PagedVirtualList.svelte';
  import type { VirtualListRow } from './PagedVirtualList.svelte';
  import TrackColumnCells from './TrackColumnCells.svelte';

  interface Props {
    resetKey: number;
    cursorChangeKey: number;
    columns?: TrackListColumnPreference[];
    fetchPage?: (request: { offset: number; limit: number }) => Promise<PlaybackQueuePage>;
    livePlayCount?: { trackId: string; playedMs: number } | null;
  }

  let {
    resetKey,
    cursorChangeKey,
    columns = DEFAULT_TRACK_COLUMN_PREFERENCES,
    fetchPage,
    livePlayCount = null,
  }: Props = $props();

  let normalizedColumns = $derived(normalizeTrackColumnPreferences(columns));
  let visibleColumns = $derived(visibleTrackColumns(normalizedColumns).map(({ id }) =>
    TRACK_COLUMN_DEFINITIONS.find((definition) => definition.id === id)!,
  ));
  let listGridTemplate = $derived(trackListGridTemplate(normalizedColumns));
  let listMinWidth = $derived(trackListMinWidthPx(normalizedColumns));
  let listColumnCount = $derived(trackListColumnCount(normalizedColumns));
  let visibleRange = $state({ start: 0, end: TRACK_PAGE_SIZE });
  let currentEntryPosition = $state<number | null>(null);
  let cursorError = $state<string | null>(null);
  let cursorProbeGeneration = 0;
  let seenCursorChangeKey: number | undefined;

  type ListSnapshot = ReturnType<PagedListController<PlaybackQueuePageItem, number>['snapshot']>;
  let queueSnapshot = $state<ListSnapshot>({
    scope: null,
    totalCount: null,
    cachedPageCount: 0,
    cachedItemCount: 0,
    pendingPageCount: 0,
    errors: [],
    generation: 0,
    revision: 0,
  });

  const queue = new PagedListController<PlaybackQueuePageItem, number>(
    async ({ scope, offset, limit }) => {
      const requestCursorGeneration = cursorProbeGeneration;
      const page = await requestPage({ offset, limit });
      updateCurrentEntry(page, scope, requestCursorGeneration);
      return adaptPlaybackQueuePage(page, { offset, limit });
    },
    { listName: '播放佇列', onChange: () => { queueSnapshot = queue.snapshot(); } },
  );
  let snapshot = $derived(queueSnapshot);
  let rangeLabel = $derived.by(() => {
    if (snapshot.totalCount === null) return '正在載入播放佇列…';
    if (snapshot.totalCount === 0) return '0 個項目';
    const first = visibleRange.start + 1;
    const last = Math.min(visibleRange.end, snapshot.totalCount);
    return `${first.toLocaleString()}–${last.toLocaleString()} 項，共 ${snapshot.totalCount.toLocaleString()} 項`;
  });

  $effect(() => {
    const currentResetKey = resetKey;
    cursorProbeGeneration += 1;
    currentEntryPosition = null;
    cursorError = null;
    queue.reset(currentResetKey, currentResetKey);
    void queue.ensureRange(0, TRACK_PAGE_SIZE);
  });

  $effect(() => {
    const currentCursorChangeKey = cursorChangeKey;
    if (seenCursorChangeKey === undefined) {
      seenCursorChangeKey = currentCursorChangeKey;
      return;
    }
    if (currentCursorChangeKey === seenCursorChangeKey) return;
    seenCursorChangeKey = currentCursorChangeKey;
    void refreshCurrentEntry();
  });

  async function requestPage(request: { offset: number; limit: number }): Promise<PlaybackQueuePage> {
    return fetchPage
      ? fetchPage(request)
      : invokeCommand('playback_get_queue_page', request);
  }

  function updateCurrentEntry(page: PlaybackQueuePage, requestedResetKey: number, requestedCursorGeneration: number): void {
    if (requestedResetKey !== resetKey || requestedCursorGeneration !== cursorProbeGeneration) return;
    currentEntryPosition = playbackQueueCurrentEntryPosition(page);
  }

  async function refreshCurrentEntry(): Promise<void> {
    const requestedResetKey = resetKey;
    const requestedCursorGeneration = ++cursorProbeGeneration;
    cursorError = null;
    try {
      const page = await requestPage({ offset: 0, limit: 1 });
      updateCurrentEntry(page, requestedResetKey, requestedCursorGeneration);
    } catch (error) {
      if (requestedResetKey === resetKey && requestedCursorGeneration === cursorProbeGeneration) {
        cursorError = getErrorText(error);
      }
    }
  }

  function itemAt(index: number): PlaybackQueuePageItem | null {
    return queue.itemAt(index);
  }

  function handleRange(range: { start: number; end: number }): void {
    visibleRange = range;
    void queue.ensureRange(range.start, range.end);
  }

  function retryFirstError(): void {
    const error = snapshot.errors[0];
    if (error) void queue.retry(error.offset);
  }

  function isCurrent(item: PlaybackQueuePageItem): boolean {
    return isCurrentPlaybackQueueEntry(item, currentEntryPosition);
  }

  function displayValue(id: (typeof TRACK_COLUMN_DEFINITIONS)[number]['id'], item: PlaybackQueuePageItem): string {
    if (!item.track) return id === 'title' ? '曲目資訊暫不可用' : '—';
    return formatTrackColumnValue(id, withLivePlayedMs(item.track, livePlayCount), formatDuration);
  }
</script>

{#if snapshot.errors.length > 0}
  <div class="track-list-error" role="alert">
    <span>{getErrorText(snapshot.errors[0]?.message)}</span>
    <button type="button" class="text-button" onclick={retryFirstError}>重試載入</button>
  </div>
{/if}
{#if cursorError}
  <div class="queue-cursor-error" role="status">佇列位置更新失敗：{cursorError}</div>
{/if}

{#if snapshot.totalCount === 0}
  <div class="empty-panel queue-empty" role="status">
    <h3>播放佇列是空的</h3>
    <p>從曲庫或播放清單開始播放歌曲後，佇列會顯示在這裡。</p>
  </div>
{:else}
  <PagedVirtualList
    totalCount={snapshot.totalCount}
    pending={snapshot.pendingPageCount > 0}
    revision={snapshot.revision}
    generation={snapshot.generation}
    {itemAt}
    getKey={(entry) => entry.entryPosition}
    isSelected={(entry) => isCurrent(entry)}
    rowHeight={57}
    compactRowHeight={53}
    shortDesktopRowHeight={49}
    headerHeight={37}
    compactHeaderHeight={33}
    maxViewportHeight={680}
    listId="playback-queue"
    ariaLabel="目前播放佇列；使用方向鍵瀏覽"
    columnCount={listColumnCount}
    gridTemplate={listGridTemplate}
    minContentWidth={listMinWidth}
    className="track-list-viewport playback-queue-viewport"
    rowClassName="track-row virtual-track-row configurable-track-grid playback-queue-row"
    listName="播放佇列"
    onRange={handleRange}
    onPlay={() => {}}
  >
    {#snippet header()}
      <div class="track-table-head track-list-header configurable-track-grid" role="row" aria-rowindex="1">
        <TrackColumnCells variant="header" columns={visibleColumns} trailingLabel="佇列狀態" />
      </div>
    {/snippet}
    {#snippet row(row: VirtualListRow<PlaybackQueuePageItem>)}
      {#if row.item}
        <TrackColumnCells
          variant="item"
          columns={visibleColumns}
          indexLabel={String(row.item.traversalPosition + 1)}
          cells={visibleColumns.map((column) => ({
            id: column.id,
            text: displayValue(column.id, row.item!),
            title: column.id === 'title' && row.item?.track === null ? '曲目目前無法取得中繼資料' : undefined,
          }))}
        >
          {#snippet trailing()}
            <span class="queue-entry-state column-action" role="gridcell">{isCurrent(row.item!) ? '正在播放' : ''}</span>
          {/snippet}
        </TrackColumnCells>
      {:else}
        <TrackColumnCells variant="placeholder" columns={visibleColumns} placeholderTitle="正在載入項目…" />
      {/if}
    {/snippet}
  </PagedVirtualList>
{/if}

<div class="track-list-count" role="status" aria-live="polite">{rangeLabel}</div>

<style>
  .queue-entry-state {
    overflow: hidden;
    color: var(--accent-text);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .queue-empty {
    margin: 12px 0;
  }

  .queue-cursor-error {
    margin: 0 0 8px;
    color: var(--text-soft);
    font-size: 10px;
  }
</style>
