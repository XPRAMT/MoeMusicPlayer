<script lang="ts">
  import { getErrorText, invokeCommand, type PlaylistEntrySummary, type PlaylistPage } from './ipc';
  import { formatDuration } from './format';
  import { PagedListController, TRACK_PAGE_SIZE } from './track-list-data.js';
  import PagedVirtualList from './PagedVirtualList.svelte';
  import type { VirtualListRow } from './PagedVirtualList.svelte';

  interface Props {
    playlistId: string;
    resetKey: number;
    playbackReady: boolean;
    isSendingPlaybackCommand: boolean;
    onPlay: (entry: PlaylistEntrySummary, playlistId: string) => void;
    fetchPage?: (request: { playlistId: string; offset: number; limit: number }) => Promise<PlaylistPage>;
  }

  let {
    playlistId,
    resetKey,
    playbackReady,
    isSendingPlaybackCommand,
    onPlay,
    fetchPage,
  }: Props = $props();

  type ListSnapshot = ReturnType<PagedListController<PlaylistEntrySummary, string>['snapshot']>;
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
  let visibleRange = $state({ start: 0, end: TRACK_PAGE_SIZE });

  const entries = new PagedListController<PlaylistEntrySummary, string>(
    ({ scope, offset, limit }) => fetchPage
      ? fetchPage({ playlistId: scope, offset, limit })
      : invokeCommand('playlist_get_page', { playlistId: scope, offset, limit }),
    { listName: '播放清單', onChange: () => { snapshot = entries.snapshot(); } },
  );

  let rangeLabel = $derived.by(() => {
    if (snapshot.totalCount === null) return '正在載入項目…';
    if (snapshot.totalCount === 0) return '0 個項目';
    const first = visibleRange.start + 1;
    const last = Math.min(visibleRange.end, snapshot.totalCount);
    return `${first.toLocaleString()}–${last.toLocaleString()} 項，共 ${snapshot.totalCount.toLocaleString()} 項`;
  });

  $effect(() => {
    const currentId = playlistId;
    const currentRevision = resetKey;
    entries.reset(currentId, currentRevision);
    void entries.ensureRange(0, TRACK_PAGE_SIZE);
  });

  function itemAt(index: number): PlaylistEntrySummary | null {
    return entries.itemAt(index);
  }

  function handleRange(range: { start: number; end: number }): void {
    visibleRange = range;
    void entries.ensureRange(range.start, range.end);
  }

  function retryFirstError(): void {
    const error = snapshot.errors[0];
    if (error) void entries.retry(error.offset);
  }

  function playEntry(entry: PlaylistEntrySummary): void {
    if (!entry.trackId || !entry.hasEnabledMapping || !playbackReady || isSendingPlaybackCommand) return;
    onPlay(entry, playlistId);
  }
</script>

{#if snapshot.errors.length > 0}
  <div class="track-list-error" role="alert">
    <span>{getErrorText(snapshot.errors[0]?.message)}</span>
    <button type="button" class="text-button" onclick={retryFirstError}>重試載入</button>
  </div>
{/if}

{#if snapshot.totalCount === 0}
  <div class="empty-panel playlist-empty"><h3>這份清單沒有項目</h3><p>可以重新匯入其他播放清單。</p></div>
{:else}
  <PagedVirtualList
    totalCount={snapshot.totalCount}
    pending={snapshot.pendingPageCount > 0}
    revision={snapshot.revision}
    generation={snapshot.generation}
    {itemAt}
    getKey={(entry) => entry.position}
    rowHeight={56}
    compactRowHeight={47}
    shortDesktopRowHeight={49}
    headerHeight={35}
    compactHeaderHeight={30}
    maxViewportHeight={680}
    listId={`playlist-${playlistId}`}
    ariaLabel="播放清單項目；使用方向鍵瀏覽，按 Enter 播放目前項目"
    columnCount={5}
    className="playlist-entry-table"
    rowClassName="playlist-entry-row"
    listName="播放清單"
    onRange={handleRange}
    onPlay={(entry) => playEntry(entry)}
  >
    {#snippet header()}
      <div class="playlist-entry-head" role="row" aria-rowindex="1">
        <span role="columnheader">#</span>
        <span role="columnheader">曲目</span>
        <span class="playlist-entry-album" role="columnheader">專輯／演出者</span>
        <span class="playlist-entry-duration" role="columnheader">長度</span>
        <span role="columnheader" aria-label="播放操作"></span>
      </div>
    {/snippet}
    {#snippet row(row: VirtualListRow<PlaylistEntrySummary>)}
      {#if row.item}
        <span class="playlist-entry-index" role="gridcell">{row.item.position + 1}</span>
        <div class="playlist-entry-title" role="gridcell">
          <strong>{row.item.title?.trim() || '未命名項目'}</strong>
          <small>{row.item.artist?.trim() || (row.item.hasEnabledMapping ? '未知演出者' : '目前未對應到曲庫')}</small>
        </div>
        <span class="playlist-entry-album" role="gridcell">{row.item.album?.trim() || '—'}</span>
        <span class="playlist-entry-duration" role="gridcell">{formatDuration(row.item.durationMs)}</span>
        <button
          class="row-play"
          type="button"
          tabindex="-1"
          aria-label={`播放 ${row.item.title?.trim() || '播放清單項目'}`}
          title={row.item.trackId && row.item.hasEnabledMapping ? '播放曲目' : '這個項目尚未對應到可播放的曲庫曲目'}
          disabled={!row.item.trackId || !row.item.hasEnabledMapping || !playbackReady || isSendingPlaybackCommand}
          onclick={row.play}
        >
          <svg viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="m7.3 5.8 7 4.2-7 4.2V5.8Z" fill="currentColor" /></svg>
        </button>
      {:else}
        <span class="playlist-entry-index" role="gridcell" aria-hidden="true">—</span>
        <div class="playlist-entry-title" role="gridcell" aria-hidden="true"><strong class="track-loading-label">正在載入項目…</strong></div>
        <span class="playlist-entry-album" role="gridcell" aria-hidden="true">—</span>
        <span class="playlist-entry-duration" role="gridcell" aria-hidden="true">—:—</span>
        <span role="gridcell" aria-hidden="true"></span>
      {/if}
    {/snippet}
  </PagedVirtualList>
{/if}

<div class="track-list-count" role="status" aria-live="polite">{rangeLabel}</div>
