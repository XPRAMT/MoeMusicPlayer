<script lang="ts">
  import { onMount } from 'svelte';
  import {
    getErrorText,
    invokeCommand,
    type TrackPage,
    type TrackPageRequest,
    type TrackSummary,
  } from './ipc';
  import { formatDuration, formatTrackIndex } from './format';
  import {
    getVirtualRange,
    PagedTrackList,
    TRACK_PAGE_SIZE,
  } from './track-list-data.js';

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

  type ListSnapshot = ReturnType<PagedTrackList['snapshot']>;
  type VirtualRow = { index: number; track: TrackSummary | null };

  let snapshot = $state<ListSnapshot>({
    query: '',
    totalCount: null,
    cachedPageCount: 0,
    cachedItemCount: 0,
    pendingPageCount: 0,
    errors: [],
    generation: 0,
  });
  let scrollElement: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let viewportHeight = $state(480);
  let rowHeight = $state(57);
  let headerHeight = $state(37);
  let maxViewportHeight = $state(680);
  let activeIndex = $state(0);

  const listData = new PagedTrackList(
    (request) => fetchPage
      ? fetchPage(request)
      : invokeCommand('library_get_page', request),
    { onChange: () => { snapshot = listData.snapshot(); } },
  );

  let virtualTotalCount = $derived(snapshot.totalCount ?? TRACK_PAGE_SIZE);
  let viewportPixelHeight = $derived(
    Math.max(
      headerHeight + rowHeight,
      Math.min(maxViewportHeight, headerHeight + virtualTotalCount * rowHeight),
    ),
  );
  let virtualRange = $derived(
    getVirtualRange(
      scrollTop,
      viewportHeight,
      virtualTotalCount,
      rowHeight,
      8,
      headerHeight,
    ),
  );
  let virtualRows = $derived.by(() => {
    const rows: VirtualRow[] = [];
    for (let index = virtualRange.start; index < virtualRange.end; index += 1) {
      rows.push({ index, track: listData.trackAt(index) });
    }
    return rows;
  });
  let activeDescendantId = $derived.by(() => {
    const activeRow = virtualRows.find((row) => row.index === activeIndex && row.track !== null);
    return activeRow ? `library-track-row-${activeRow.index}` : undefined;
  });
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
    activeIndex = 0;
    scrollTop = 0;
    scrollElement?.scrollTo({ top: 0 });
    void listData.ensureRange(0, TRACK_PAGE_SIZE);
  });

  $effect(() => {
    onTotalCount?.(snapshot.totalCount);
  });

  onMount(() => {
    const updateMetrics = () => {
      const narrow = window.matchMedia('(max-width: 620px)').matches;
      const shortDesktop = window.matchMedia('(max-height: 680px) and (min-width: 621px)').matches;
      rowHeight = narrow ? 53 : shortDesktop ? 49 : 57;
      headerHeight = narrow ? 33 : 37;
      maxViewportHeight = Math.max(160, Math.min(680, Math.floor(window.innerHeight * 0.65)));
      viewportHeight = scrollElement?.clientHeight ?? viewportHeight;
    };

    updateMetrics();
    window.addEventListener('resize', updateMetrics);
    return () => window.removeEventListener('resize', updateMetrics);
  });

  function updateViewport(): void {
    if (!scrollElement) return;
    scrollTop = scrollElement.scrollTop;
    viewportHeight = scrollElement.clientHeight;
    const range = getVirtualRange(
      scrollTop,
      viewportHeight,
      snapshot.totalCount ?? TRACK_PAGE_SIZE,
      rowHeight,
      8,
      headerHeight,
    );
    void listData.ensureRange(range.start, range.end);
  }

  function setActiveIndex(index: number): void {
    const maxIndex = Math.max(0, (snapshot.totalCount ?? TRACK_PAGE_SIZE) - 1);
    activeIndex = Math.max(0, Math.min(maxIndex, index));
    void listData.ensureRange(activeIndex, activeIndex + 1);

    if (!scrollElement) return;
    const rowTop = headerHeight + activeIndex * rowHeight;
    const rowBottom = rowTop + rowHeight;
    let nextScrollTop = scrollElement.scrollTop;
    if (rowTop < nextScrollTop + headerHeight) nextScrollTop = Math.max(0, rowTop - headerHeight);
    else if (rowBottom > nextScrollTop + scrollElement.clientHeight) {
      nextScrollTop = rowBottom - scrollElement.clientHeight;
    }
    if (nextScrollTop !== scrollElement.scrollTop) scrollElement.scrollTo({ top: nextScrollTop });
  }

  function handleKeydown(event: KeyboardEvent): void {
    let nextIndex: number | null = null;
    switch (event.key) {
      case 'ArrowDown': nextIndex = activeIndex + 1; break;
      case 'ArrowUp': nextIndex = activeIndex - 1; break;
      case 'Home': nextIndex = 0; break;
      case 'End': nextIndex = (snapshot.totalCount ?? TRACK_PAGE_SIZE) - 1; break;
      case 'PageDown': nextIndex = activeIndex + Math.max(1, Math.floor(viewportHeight / rowHeight)); break;
      case 'PageUp': nextIndex = activeIndex - Math.max(1, Math.floor(viewportHeight / rowHeight)); break;
      case 'Enter':
      case ' ':
        event.preventDefault();
        playActiveTrack();
        return;
      default: return;
    }

    event.preventDefault();
    if (nextIndex !== null) setActiveIndex(nextIndex);
  }

  function playActiveTrack(): void {
    const track = listData.trackAt(activeIndex);
    if (!track || !playbackReady || isSendingPlaybackCommand) return;
    void onPlay(track);
  }

  function retryFirstError(): void {
    const firstError = snapshot.errors[0];
    if (firstError) void listData.retry(firstError.offset);
  }

  function playRow(track: TrackSummary, index: number): void {
    activeIndex = index;
    if (playbackReady && !isSendingPlaybackCommand) void onPlay(track);
  }
</script>

{#if snapshot.errors.length > 0}
  <div class="track-list-error" role="alert">
    <span>{getErrorText(snapshot.errors[0]?.message)}</span>
    <button type="button" class="text-button" onclick={retryFirstError}>重試載入</button>
  </div>
{/if}

{#if snapshot.totalCount === 0}
  <div class="track-list-empty" role="status">{snapshot.query ? '找不到相符曲目。' : '曲庫目前是空的。'}</div>
{:else}
  <div
    class="track-list-viewport"
    bind:this={scrollElement}
    role="grid"
    aria-label="曲庫曲目；使用方向鍵瀏覽，按 Enter 播放目前曲目"
    aria-rowcount={snapshot.totalCount === null ? -1 : snapshot.totalCount + 1}
    aria-colcount="5"
    aria-activedescendant={activeDescendantId}
    aria-busy={snapshot.pendingPageCount > 0}
    tabindex="0"
    style={`height:${viewportPixelHeight}px`}
    onscroll={updateViewport}
    onkeydown={handleKeydown}
  >
    <div class="track-table-head track-list-header" role="row" aria-rowindex="1">
      <span class="column-index" role="columnheader">#</span>
      <span role="columnheader">曲目</span>
      <span class="column-album" role="columnheader">專輯</span>
      <span class="column-duration" role="columnheader">長度</span>
      <span class="column-action" role="columnheader" aria-label="播放操作"></span>
    </div>
    <div
      class="track-list-spacer"
      role="presentation"
      aria-hidden="true"
      style={`height:${virtualTotalCount * rowHeight}px`}
    ></div>
    {#each virtualRows as row (row.index)}
      <div
        id={`library-track-row-${row.index}`}
        class="track-row virtual-track-row"
        class:selected={row.track !== null && selectedTrackId === row.track.id}
        role="row"
        aria-rowindex={row.index + 2}
        aria-selected={row.track !== null && selectedTrackId === row.track.id}
        aria-label={row.track ? `${row.track.title?.trim() || '未命名曲目'}，${row.track.artist?.trim() || '未知演出者'}` : `第 ${row.index + 1} 首，正在載入`}
        style={`height:${rowHeight}px;transform:translateY(${headerHeight + row.index * rowHeight}px)`}
      >
        {#if row.track}
          <span class="track-index column-index" role="gridcell">{formatTrackIndex(row.track.trackNumber, row.track.discNumber)}</span>
          <div class="track-main" role="gridcell">
            <span class="track-title">{row.track.title?.trim() || '未命名曲目'}</span>
            <span class="track-artist">{row.track.artist?.trim() || '未知演出者'}</span>
          </div>
          <span class="track-album column-album" role="gridcell">{row.track.album?.trim() || '未知專輯'}</span>
          <span class="track-duration column-duration" role="gridcell">{formatDuration(row.track.durationMs)}</span>
          <button
            class="row-play column-action"
            type="button"
            tabindex="-1"
            aria-label={`播放 ${row.track.title?.trim() || '未命名曲目'}`}
            title={playbackReady ? '播放曲目' : '播放功能尚未就緒'}
            disabled={!playbackReady || isSendingPlaybackCommand}
            onclick={() => playRow(row.track!, row.index)}
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
      </div>
    {/each}
  </div>
{/if}

<div class="track-list-count" role="status" aria-live="polite">{rangeLabel}</div>

<style>
  .track-list-viewport {
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

  .track-list-viewport:focus-visible {
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

  .track-list-spacer {
    width: 1px;
    pointer-events: none;
  }

  .virtual-track-row {
    position: absolute;
    z-index: 1;
    inset-inline: 0;
    top: 0;
    box-sizing: border-box;
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
