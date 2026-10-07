<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import PlaylistTree from '../src/lib/PlaylistTree.svelte';
  import PlaylistEntryList from '../src/lib/PlaylistEntryList.svelte';
  import PlaybackQueueList from '../src/lib/PlaybackQueueList.svelte';
  import {
    DEFAULT_TRACK_COLUMN_PREFERENCES,
    normalizeTrackColumnPreferences,
  } from '../src/lib/track-columns.js';
  import type {
    PlaybackQueuePage,
    PlaybackQueuePageItem,
    PlaylistEntrySummary,
    PlaylistPage,
    PlaylistSummary,
    TrackListColumnPreference,
    TrackSummary,
  } from '../src/lib/ipc';

  interface HarnessApi {
    setView: (view: 'playlists' | 'queue') => Promise<void>;
    setColumns: (columns: TrackListColumnPreference[]) => Promise<void>;
    setCurrentEntry: (entryPosition: number) => Promise<void>;
    replaceQueue: () => Promise<void>;
    scrollSweep: (stepRows?: number) => Promise<{
      steps: number;
      maxDomRows: number;
      finalDomRows: number;
      calls: number;
      generatedItems: number;
      total: number;
    }>;
    snapshot: () => {
      activeView: string;
      selectedPlaylistId: string;
      queueCurrentRows: string[];
      queueRequestLog: Array<{ offset: number; limit: number; revision: number }>;
      treeHostParent: string | null;
      treeVisible: boolean;
      pageContentWidth: number;
      pageContentClientWidth: number;
      playlistDetailWidth: number;
      queueListWidth: number;
      documentWidth: number;
      viewportWidth: number;
    };
  }

  declare global {
    interface Window { playbackQueueHarness: HarnessApi }
  }

  const totalQueueItems = 100_000;
  let activeView = $state<'playlists' | 'queue'>('playlists');
  let selectedPlaylistId = $state('playlist-hanser');
  let playlistRevision = $state(0);
  let queueResetKey = $state(0);
  let cursorChangeKey = $state(0);
  let currentEntryPosition = 0;
  let queueRevision = 1;
  let requestCount = 0;
  let generatedItems = 0;
  const queueRequestLog: Array<{ offset: number; limit: number; revision: number }> = [];
  let columns = $state(DEFAULT_TRACK_COLUMN_PREFERENCES.map((column) => ({ ...column })));
  const playlists: PlaylistSummary[] = [
    { id: 'playlist-hanser', name: 'Hanser 精選', entryCount: 2 },
    { id: 'playlist-live', name: '現場演出', entryCount: 1_284 },
    { id: 'playlist-relax', name: '放鬆聆聽', entryCount: 89 },
  ];

  function makeTrack(index: number): TrackSummary {
    return {
      id: index < 2 ? 'duplicate-track' : `queue-track-${index}`,
      title: index < 2 ? `重複歌曲項目 ${index + 1}` : `佇列曲目 ${index + 1}`,
      artist: '佇列測試演出者',
      album: '測試專輯',
      albumArtist: null,
      trackNumber: index + 1,
      discNumber: 1,
      durationMs: 180_000,
      codec: 'FLAC',
      bitrateBps: null,
      sampleRateHz: 48_000,
      year: 2024,
      bitDepth: 24,
    };
  }

  function makeQueueItem(traversalPosition: number): PlaybackQueuePageItem {
    const entryPosition = traversalPosition;
    return {
      traversalPosition,
      entryPosition,
      sourcePosition: traversalPosition < 2 ? 0 : traversalPosition,
      trackId: entryPosition < 2 ? 'duplicate-track' : `queue-track-${entryPosition}`,
      track: entryPosition === 11 ? null : makeTrack(entryPosition),
      isCurrent: entryPosition === currentEntryPosition,
    };
  }

  async function fetchQueuePage(request: { offset: number; limit: number }): Promise<PlaybackQueuePage> {
    requestCount += 1;
    queueRequestLog.push({ ...request, revision: queueRevision });
    const count = Math.min(request.limit, Math.max(0, totalQueueItems - request.offset));
    const items = Array.from({ length: count }, (_, index) => makeQueueItem(request.offset + index));
    generatedItems += items.length;
    return {
      revision: queueRevision,
      total: totalQueueItems,
      offset: request.offset,
      cursor: currentEntryPosition,
      currentEntryPosition,
      items,
    };
  }

  function makePlaylistEntry(position: number): PlaylistEntrySummary {
    return {
      position,
      trackId: `playlist-track-${position}`,
      title: `Hanser 清單項目 ${position + 1}`,
      artist: 'Hanser',
      album: '測試專輯',
      durationMs: 180_000,
      hasEnabledMapping: true,
    };
  }

  async function fetchPlaylistPage(request: { playlistId: string; offset: number; limit: number }): Promise<PlaylistPage> {
    const totalCount = playlists.find((playlist) => playlist.id === request.playlistId)?.entryCount ?? 0;
    return {
      items: Array.from({ length: Math.min(request.limit, Math.max(0, totalCount - request.offset)) }, (_, index) => makePlaylistEntry(request.offset + index)),
      offset: request.offset,
      limit: request.limit,
      totalCount,
    };
  }

  async function settle(): Promise<void> {
    await tick();
    await new Promise((resolve) => setTimeout(resolve, 0));
    await tick();
  }

  async function setView(view: 'playlists' | 'queue'): Promise<void> {
    activeView = view;
    await settle();
  }

  async function setColumns(nextColumns: TrackListColumnPreference[]): Promise<void> {
    columns = normalizeTrackColumnPreferences(nextColumns);
    await settle();
  }

  async function setCurrentEntry(entryPosition: number): Promise<void> {
    currentEntryPosition = entryPosition;
    queueRevision += 1;
    cursorChangeKey += 1;
    await settle();
  }

  async function replaceQueue(): Promise<void> {
    queueRevision += 1;
    currentEntryPosition = 3;
    queueResetKey += 1;
    await settle();
  }

  async function scrollSweep(stepRows = 1_000) {
    activeView = 'queue';
    await settle();
    const viewport = document.querySelector<HTMLDivElement>('.playback-queue-viewport');
    if (!viewport) throw new Error('Playback queue viewport did not mount.');
    const rowHeight = viewport.querySelector<HTMLElement>('.playback-queue-row')?.clientHeight ?? 56;
    const maxScrollTop = viewport.scrollHeight - viewport.clientHeight;
    const increment = Math.max(1, stepRows) * rowHeight;
    let maxDomRows = 0;
    let steps = 0;
    const beforeCalls = requestCount;
    const beforeItems = generatedItems;
    for (let scrollTop = 0; scrollTop <= maxScrollTop; scrollTop += increment) {
      viewport.scrollTop = scrollTop;
      viewport.dispatchEvent(new Event('scroll'));
      await settle();
      maxDomRows = Math.max(maxDomRows, document.querySelectorAll('.playback-queue-row').length);
      steps += 1;
    }
    viewport.scrollTop = maxScrollTop;
    viewport.dispatchEvent(new Event('scroll'));
    await settle();
    return {
      steps,
      maxDomRows,
      finalDomRows: document.querySelectorAll('.playback-queue-row').length,
      calls: requestCount - beforeCalls,
      generatedItems: generatedItems - beforeItems,
      total: totalQueueItems,
    };
  }

  function snapshot() {
    const content = document.querySelector<HTMLElement>('.page-content');
    const contentStyle = content ? getComputedStyle(content) : null;
    const contentAvailableWidth = content && contentStyle
      ? content.clientWidth - Number.parseFloat(contentStyle.paddingLeft) - Number.parseFloat(contentStyle.paddingRight)
      : 0;
    const detail = document.querySelector<HTMLElement>('.playlist-detail');
    const queueList = document.querySelector<HTMLElement>('.playback-queue-viewport');
    const tree = document.querySelector<HTMLElement>('.sidebar-playlist-tree-host .playlist-tree');
    return {
      activeView,
      selectedPlaylistId,
      queueCurrentRows: [...document.querySelectorAll<HTMLElement>('.playback-queue-row.selected .column-index')]
        .map((element) => element.textContent?.trim() ?? ''),
      queueRequestLog: queueRequestLog.slice(),
      treeHostParent: document.querySelector('.sidebar-playlist-tree-host')?.parentElement?.className ?? null,
      treeVisible: tree !== null && getComputedStyle(tree).display !== 'none' && tree.getBoundingClientRect().width > 0,
      pageContentWidth: content?.getBoundingClientRect().width ?? 0,
      pageContentClientWidth: contentAvailableWidth,
      playlistDetailWidth: detail?.getBoundingClientRect().width ?? 0,
      queueListWidth: queueList?.getBoundingClientRect().width ?? 0,
      documentWidth: document.documentElement.scrollWidth,
      viewportWidth: window.innerWidth,
    };
  }

  onDestroy(() => {
    delete window.playbackQueueHarness;
  });

  $effect(() => {
    window.playbackQueueHarness = { setView, setColumns, setCurrentEntry, replaceQueue, scrollSweep, snapshot };
  });
</script>

<div class="app-shell playback-queue-test-app" data-active-view={activeView}>
  <aside class="sidebar" aria-label="測試側欄">
    <div class="brand-lockup"><strong>MOE</strong></div>
    <div class="sidebar-caption">你的音樂空間</div>
    <nav class="primary-nav">
      <button class="nav-link" class:active={activeView === 'queue'} type="button" onclick={() => (activeView = 'queue')}>播放佇列</button>
    </nav>
    <div class="sidebar-playlist-tree-host">
      <PlaylistTree
        {playlists}
        {selectedPlaylistId}
        active={activeView === 'playlists'}
        onOpen={() => { activeView = 'playlists'; }}
        onSelect={(id) => { selectedPlaylistId = id; activeView = 'playlists'; }}
      />
    </div>
  </aside>
  <main class="workspace">
    <header class="topbar"><strong>播放清單與播放佇列測試</strong></header>
    <div class="page-scroll">
      <main class="page-content wide-list-page">
        {#if activeView === 'playlists'}
          <section class="playlist-section" aria-label="目前播放清單">
            <div class="playlist-browser">
              <section class="playlist-detail" aria-labelledby="selected-playlist-heading">
                <div class="playlist-detail-heading">
                  <div class="playlist-detail-title">
                    <span class="playlist-detail-artwork" aria-hidden="true">♫</span>
                    <div><p class="section-kicker">PLAYLIST</p><h3 id="selected-playlist-heading">{playlists.find((item) => item.id === selectedPlaylistId)?.name}</h3><p class="playlist-detail-count">播放清單項目</p></div>
                  </div>
                </div>
                {#key selectedPlaylistId}
                  <PlaylistEntryList
                    playlistId={selectedPlaylistId}
                    resetKey={playlistRevision}
                    playbackReady={true}
                    isSendingPlaybackCommand={false}
                    columns={columns}
                    fetchPage={fetchPlaylistPage}
                    onPlay={() => {}}
                  />
                {/key}
              </section>
            </div>
          </section>
        {:else}
          <section class="queue-page" aria-label="目前播放佇列">
            <div class="section-heading"><div><p class="section-kicker">PLAYBACK QUEUE</p><h2>播放佇列</h2></div></div>
            <PlaybackQueueList
              resetKey={queueResetKey}
              cursorChangeKey={cursorChangeKey}
              columns={columns}
              fetchPage={fetchQueuePage}
            />
          </section>
        {/if}
      </main>
    </div>
  </main>
</div>
