<script lang="ts">
  import { getErrorText, invokeCommand, type PlaybackQueuePage, type PlaybackQueuePageItem, type TrackListColumnPreference } from './ipc';
  import { formatDuration } from './format';
  import {
    DEFAULT_TRACK_COLUMN_PREFERENCES,
    TRACK_COLUMN_DEFINITIONS,
    formatTrackColumnValue,
    isHiResTrack,
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
    reorderPage?: (from: number, to: number) => Promise<void>;
    livePlayCount?: { trackId: string; playedMs: number } | null;
    onPlay?: (entry: PlaybackQueuePageItem) => void;
    onTotalCount?: (count: number | null) => void;
  }

  let {
    resetKey,
    cursorChangeKey,
    columns = DEFAULT_TRACK_COLUMN_PREFERENCES,
    fetchPage,
    reorderPage,
    livePlayCount = null,
    onPlay = () => {},
    onTotalCount,
  }: Props = $props();

  let normalizedColumns = $derived(normalizeTrackColumnPreferences(columns));
  let visibleColumns = $derived(visibleTrackColumns(normalizedColumns).map(({ id }) =>
    TRACK_COLUMN_DEFINITIONS.find((definition) => definition.id === id)!,
  ));
  let listGridTemplate = $derived(trackListGridTemplate(normalizedColumns));
  let listMinWidth = $derived(trackListMinWidthPx(normalizedColumns));
  let listColumnCount = $derived(trackListColumnCount(normalizedColumns));
  let currentEntryPosition = $state<number | null>(null);
  let currentTraversalIndex = $state<number | null>(null);
  let cursorError = $state<string | null>(null);
  let locateError = $state<string | null>(null);
  let reorderError = $state<string | null>(null);
  let reordering = $state(false);
  let locating = $state(false);
  let cursorProbeGeneration = 0;
  let reorderGeneration = 0;
  let seenCursorChangeKey: number | undefined;
  let scrollToIndexRequest = $state<{ index: number; token: number } | null>(null);
  let scrollRequestToken = 0;

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

  $effect(() => {
    onTotalCount?.(snapshot.totalCount);
  });

  $effect(() => {
    const currentResetKey = resetKey;
    cursorProbeGeneration += 1;
    currentEntryPosition = null;
    currentTraversalIndex = null;
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
    const cursor = page.cursor;
    currentTraversalIndex = cursor === null || cursor === undefined
      ? null
      : Number.isSafeInteger(cursor) && cursor >= 0
        ? cursor
        : null;
  }

  async function refreshCurrentEntry(): Promise<PlaybackQueuePage | null> {
    const requestedResetKey = resetKey;
    const requestedCursorGeneration = ++cursorProbeGeneration;
    cursorError = null;
    try {
      const page = await requestPage({ offset: 0, limit: 1 });
      updateCurrentEntry(page, requestedResetKey, requestedCursorGeneration);
      return page;
    } catch (error) {
      if (requestedResetKey === resetKey && requestedCursorGeneration === cursorProbeGeneration) {
        cursorError = getErrorText(error);
      }
      return null;
    }
  }

  function itemAt(index: number): PlaybackQueuePageItem | null {
    return queue.itemAt(index);
  }

  function handleRange(range: { start: number; end: number }): void {
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

  async function handleReorder(from: number, to: number): Promise<void> {
    if (reordering || from === to) return;
    reordering = true;
    reorderError = null;
    try {
      if (reorderPage) {
        await reorderPage(from, to);
      } else {
        await invokeCommand('playback_reorder_queue', { from, to });
      }
      cursorProbeGeneration += 1;
      currentEntryPosition = null;
      currentTraversalIndex = null;
      reorderGeneration += 1;
      // reset() is a no-op when scope and resetKey are unchanged; bump the key.
      queue.reset(resetKey, `${resetKey}:reorder:${reorderGeneration}`);
      await queue.ensureRange(0, TRACK_PAGE_SIZE);
    } catch (error) {
      reorderError = getErrorText(error);
    } finally {
      reordering = false;
    }
  }

  async function locateCurrentTrack(): Promise<void> {
    if (locating || snapshot.totalCount === 0) return;
    locating = true;
    locateError = null;
    try {
      let index = currentTraversalIndex;
      if (index === null) {
        const page = await refreshCurrentEntry();
        index = page && page.cursor !== null && Number.isSafeInteger(page.cursor) && page.cursor >= 0
          ? page.cursor
          : null;
      }
      if (index === null) {
        locateError = '目前沒有可定位的播放曲目。';
        return;
      }
      const start = Math.max(0, index - Math.floor(TRACK_PAGE_SIZE / 2));
      await queue.ensureRange(start, start + TRACK_PAGE_SIZE);
      scrollRequestToken += 1;
      scrollToIndexRequest = { index, token: scrollRequestToken };
    } catch (error) {
      locateError = getErrorText(error);
    } finally {
      locating = false;
    }
  }

  const canLocateCurrent = $derived(snapshot.totalCount !== null && snapshot.totalCount > 0);
</script>

<div class="track-list-shell queue-list-shell">
{#if snapshot.errors.length > 0}
  <div class="track-list-error" role="alert">
    <span>{getErrorText(snapshot.errors[0]?.message)}</span>
    <button type="button" class="text-button" onclick={retryFirstError}>重試載入</button>
  </div>
{/if}
{#if cursorError}
  <div class="queue-cursor-error" role="status">佇列位置更新失敗：{cursorError}</div>
{/if}
{#if reorderError}
  <div class="queue-cursor-error" role="alert">調整順序失敗：{reorderError}</div>
{/if}
{#if locateError}
  <div class="queue-cursor-error" role="status">{locateError}</div>
{/if}

{#if snapshot.totalCount === 0}
  <div class="empty-panel queue-empty" role="status">
    <h3>播放佇列是空的</h3>
    <p>從曲庫或播放清單開始播放歌曲後，佇列會顯示在這裡。</p>
  </div>
{:else}
  <PagedVirtualList
    totalCount={snapshot.totalCount}
    pending={snapshot.pendingPageCount > 0 || reordering || locating}
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
    fillAvailable={true}
    listId="playback-queue"
    ariaLabel="目前播放佇列；長按拖曳可調整順序，使用方向鍵瀏覽，按 Enter 播放目前項目"
    columnCount={listColumnCount}
    gridTemplate={listGridTemplate}
    minContentWidth={listMinWidth}
    className="track-list-viewport playback-queue-viewport"
    rowClassName="track-row virtual-track-row configurable-track-grid playback-queue-row"
    listName="播放佇列"
    longPressReorder={true}
    {scrollToIndexRequest}
    onRange={handleRange}
    onPlay={(entry) => {
      if (entry) onPlay(entry);
    }}
    onReorder={handleReorder}
  >
    {#snippet header()}
      <div class="track-table-head track-list-header configurable-track-grid" role="row" aria-rowindex="1">
        <TrackColumnCells variant="header" columns={visibleColumns} />
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
            hiRes: column.id === 'audioFormat' ? isHiResTrack(row.item?.track) : false,
          }))}
        />
      {:else}
        <TrackColumnCells variant="placeholder" columns={visibleColumns} placeholderTitle="正在載入項目…" />
      {/if}
    {/snippet}
  </PagedVirtualList>
  <button
    type="button"
    class="queue-locate-fab"
    aria-label="定位到目前播放曲目"
    title="定位到目前播放曲目"
    disabled={!canLocateCurrent || locating}
    onclick={() => void locateCurrentTrack()}
  ><span class="queue-locate-fab-dot" aria-hidden="true"></span></button>
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

  .queue-list-shell {
    position: relative;
  }

  .queue-empty {
    margin: 12px 0;
  }

  .queue-cursor-error {
    margin: 0 0 8px;
    color: var(--text-soft);
    font-size: var(--font-caption);
  }

  .queue-locate-fab {
    position: absolute;
    right: 28px;
    bottom: 28px;
    z-index: 4;
    display: grid;
    width: 38px;
    height: 38px;
    place-items: center;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: 50%;
    color: var(--text);
    background: color-mix(in srgb, var(--text-inverse) var(--quick-settings-opacity, 70%), transparent);
    -webkit-backdrop-filter: blur(var(--quick-settings-blur, 12px));
    backdrop-filter: blur(var(--quick-settings-blur, 12px));
    cursor: pointer;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.18);
  }

  .queue-locate-fab-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: currentColor;
  }

  .queue-locate-fab:hover:not(:disabled) {
    border-color: color-mix(in srgb, var(--accent-text) 55%, var(--line));
    color: var(--accent-text);
  }

  .queue-locate-fab:disabled {
    cursor: default;
    opacity: 0.45;
  }

  .queue-locate-fab:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
</style>
