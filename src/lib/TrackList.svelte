<script lang="ts">
  import {
    getErrorText,
    invokeCommand,
    type TrackPage,
    type TrackPageRequest,
    type TrackSummary,
  } from './ipc';
  import { formatDuration, formatTrackIndex } from './format';
  import {
    PagedListController,
    TRACK_PAGE_SIZE,
  } from './track-list-data.js';
  import PagedVirtualList from './PagedVirtualList.svelte';
  import type { VirtualListRow } from './PagedVirtualList.svelte';

  interface Props {
    query: string;
    resetKey?: string | number;
    selectedTrackId: string | null;
    playbackReady: boolean;
    isSendingPlaybackCommand: boolean;
    onPlay: (track: TrackSummary) => void | Promise<void>;
    onTotalCount?: (count: number | null) => void;
    fetchPage?: (request: TrackPageRequest) => Promise<TrackPage>;
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
  }: Props = $props();

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
    ({ scope, offset, limit }) => fetchPage
      ? fetchPage({ query: scope || null, offset, limit })
      : invokeCommand('library_get_page', { query: scope || null, offset, limit }),
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
    listData.reset(normalizedQuery, currentResetKey);
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
    columnCount={5}
    className="track-list-viewport"
    rowClassName="track-row virtual-track-row"
    listName="曲庫"
    onRange={handleRange}
    onPlay={(track) => playRow(track)}
  >
    {#snippet header()}
      <div class="track-table-head track-list-header" role="row" aria-rowindex="1">
        <span class="column-index" role="columnheader">#</span>
        <span role="columnheader">曲目</span>
        <span class="column-album" role="columnheader">專輯</span>
        <span class="column-duration" role="columnheader">長度</span>
        <span class="column-action" role="columnheader" aria-label="播放操作"></span>
      </div>
    {/snippet}
    {#snippet row(row: VirtualListRow<TrackSummary>)}
      {#if row.item}
        <span class="track-index column-index" role="gridcell">{formatTrackIndex(row.item.trackNumber, row.item.discNumber)}</span>
        <div class="track-main" role="gridcell">
          <span class="track-title">{row.item.title?.trim() || '未命名曲目'}</span>
          <span class="track-artist">{row.item.artist?.trim() || '未知演出者'}</span>
        </div>
        <span class="track-album column-album" role="gridcell">{row.item.album?.trim() || '未知專輯'}</span>
        <span class="track-duration column-duration" role="gridcell">{formatDuration(row.item.durationMs)}</span>
          <button
            class="row-play column-action"
            type="button"
            tabindex="-1"
            aria-label={`播放 ${row.item.title?.trim() || '未命名曲目'}`}
            title={playbackReady ? '播放曲目' : '播放功能尚未就緒'}
            disabled={!playbackReady || isSendingPlaybackCommand}
            onclick={row.play}
          >
            <svg viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="m7.3 5.8 7 4.2-7 4.2V5.8Z" fill="currentColor" /></svg>
          </button>
      {:else}
          <span class="track-index column-index" role="gridcell" aria-hidden="true">—</span>
          <div class="track-main" role="gridcell" aria-hidden="true"><span class="track-title track-loading-label">正在載入曲目…</span></div>
          <span class="track-album column-album" role="gridcell" aria-hidden="true">—</span>
          <span class="track-duration column-duration" role="gridcell" aria-hidden="true">—:—</span>
          <span class="column-action" role="gridcell" aria-hidden="true"></span>
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
    border: 1px solid var(--line);
    border-radius: 11px;
    background: rgba(var(--text-rgb), 0.012);
    outline: none;
  }

  :global(.track-list-viewport:focus-visible) {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }

  .track-list-header {
    position: sticky;
    z-index: 2;
    top: 0;
    box-sizing: border-box;
    height: 37px;
    min-height: 37px;
    background: var(--panel);
  }

  :global(.virtual-track-row) {
    min-height: 0;
  }

  .track-list-error,
  .track-list-empty,
  .track-list-count {
    color: var(--muted);
    font-size: 12px;
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

  .track-loading-label {
    color: var(--muted);
    animation: track-loading-pulse 1.4s ease-in-out infinite alternate;
  }

  @keyframes track-loading-pulse {
    from { opacity: 0.4; }
    to { opacity: 0.85; }
  }

  @media (max-width: 620px) {
    .track-list-header {
      height: 33px;
      min-height: 33px;
    }

    .track-list-count,
    .track-list-error,
    .track-list-empty {
      font-size: 10px;
    }
  }

  @media (max-height: 680px) and (min-width: 621px) {
    .track-list-header {
      height: 37px;
      min-height: 37px;
    }
  }
</style>
