<script lang="ts">
  import { onDestroy } from 'svelte';
  import TrackList from '../src/lib/TrackList.svelte';
  import PlaylistEntryList from '../src/lib/PlaylistEntryList.svelte';
  import NowPlayingArrangement from '../src/lib/NowPlayingArrangement.svelte';
  import NowPlayingLayoutSwitch from '../src/lib/NowPlayingLayoutSwitch.svelte';
  import {
    DEFAULT_TRACK_COLUMN_PREFERENCES,
    normalizeTrackColumnPreferences,
  } from '../src/lib/track-columns.js';
  import type {
    NowPlayingLayout,
    PlaylistEntrySummary,
    PlaylistPage,
    TrackListColumnPreference,
    TrackPage,
    TrackPageRequest,
    TrackSummary,
  } from '../src/lib/ipc';
  import { tick } from 'svelte';

  interface DeferredPage<T> {
    resolve: () => void;
    page: T;
  }

  interface ColumnSnapshot {
    ariaColumnCount: string | null;
    header: string[];
    row: string[] | null;
    skeleton: string[] | null;
    headerCellCount: number;
    rowCellCount: number | null;
    gridTemplate: string;
    scrollWidth: number;
    clientWidth: number;
  }

  interface HarnessApi {
    releasePages: () => Promise<void>;
    setColumns: (columns: TrackListColumnPreference[]) => Promise<void>;
    setLayout: (layout: NowPlayingLayout) => Promise<{
      paneOrder: string[];
      nodesPreserved: boolean;
      lyricsText: string | null;
    }>;
    snapshot: () => {
      library: ColumnSnapshot;
      playlist: ColumnSnapshot;
      page: { documentWidth: number; viewportWidth: number; contentWidth: number; contentClientWidth: number };
      layout: { paneOrder: string[]; paneTops: number[]; lyricsText: string | null };
      switches: Array<{ role: string | null; pressed: Array<string | null> }>;
    };
  }

  declare global {
    interface Window {
      configurableColumnsHarness: HarnessApi;
    }
  }

  let columns = $state(DEFAULT_TRACK_COLUMN_PREFERENCES.map((column) => ({ ...column })));
  let layout = $state<NowPlayingLayout>('a');
  let heldTrackPages: DeferredPage<TrackPage>[] = [];
  let heldPlaylistPages: DeferredPage<PlaylistPage>[] = [];

  function makeTrack(index: number): TrackSummary {
    return {
      id: `column-track-${index}`,
      title: `曲目 ${index + 1}`,
      artist: 'ARTIST 演出者',
      album: index === 0
        ? '崩壞星穹鐵道-行於命途6 Experience the Paths Vol.6'
        : '專輯',
      albumArtist: null,
      trackNumber: index + 1,
      discNumber: 1,
      durationMs: 185_000,
      codec: 'FLAC',
      bitrateBps: null,
      sampleRateHz: 48_000,
      year: 2024,
      bitDepth: 24,
      playedMs: 0,
    };
  }

  function makePlaylistEntry(position: number): PlaylistEntrySummary {
    if (position === 0) {
      return {
        position,
        trackId: null,
        title: 'M3U 項目',
        artist: null,
        album: null,
        durationMs: null,
        year: null,
        codec: null,
        sampleRateHz: null,
        bitDepth: null,
        bitrateBps: null,
        hasEnabledMapping: false,
      };
    }
    return {
      position,
      trackId: 'column-track-1',
      title: '曲目 2',
      artist: 'ARTIST 演出者',
      album: '專輯',
      durationMs: 185_000,
      year: 2024,
      codec: 'FLAC',
      sampleRateHz: 48_000,
      bitDepth: 24,
      bitrateBps: null,
      hasEnabledMapping: true,
    };
  }

  function holdPage<T>(queue: DeferredPage<T>[], page: T): Promise<T> {
    return new Promise((resolve) => {
      queue.push({ page, resolve: () => resolve(page) });
    });
  }

  function fetchTrackPage(request: TrackPageRequest): Promise<TrackPage> {
    const totalCount = 3;
    const count = Math.min(request.limit, Math.max(0, totalCount - request.offset));
    const page: TrackPage = {
      items: Array.from({ length: count }, (_, index) => makeTrack(request.offset + index)),
      offset: request.offset,
      limit: request.limit,
      totalCount,
    };
    return holdPage(heldTrackPages, page);
  }

  function fetchPlaylistPage(request: { playlistId: string; offset: number; limit: number }): Promise<PlaylistPage> {
    const totalCount = 2;
    const count = Math.min(request.limit, Math.max(0, totalCount - request.offset));
    const page: PlaylistPage = {
      items: Array.from({ length: count }, (_, index) => makePlaylistEntry(request.offset + index)),
      offset: request.offset,
      limit: request.limit,
      totalCount,
    };
    return holdPage(heldPlaylistPages, page);
  }

  function childOrder(root: Element | null): string[] {
    return root
      ? [...root.querySelectorAll<HTMLElement>(':scope > [data-layout-pane]')].map((pane) => pane.dataset.layoutPane ?? '')
      : [];
  }

  function listSnapshot(selector: string): ColumnSnapshot {
    const root = document.querySelector<HTMLElement>(selector);
    if (!root) throw new Error(`Missing virtual list: ${selector}`);
    const header = root.querySelector<HTMLElement>('[role="row"][aria-rowindex="1"]');
    const row = root.querySelector<HTMLElement>('.paged-virtual-row[role="row"]');
    const rowIsSkeleton = row?.getAttribute('aria-label')?.includes('正在載入') ?? false;
    const columnsIn = (element: HTMLElement | null) => element
      ? [...element.querySelectorAll<HTMLElement>(':scope > .list-column')].map((cell) => cell.className.match(/list-column-([\w-]+)/)?.[1] ?? '')
      : null;
    return {
      ariaColumnCount: root.getAttribute('aria-colcount'),
      header: columnsIn(header) ?? [],
      row: row && !rowIsSkeleton ? columnsIn(row) : null,
      skeleton: row && rowIsSkeleton ? columnsIn(row) : null,
      headerCellCount: header?.children.length ?? 0,
      rowCellCount: row?.children.length ?? null,
      gridTemplate: root.style.getPropertyValue('--paged-grid-template').trim(),
      scrollWidth: root.scrollWidth,
      clientWidth: root.clientWidth,
    };
  }

  async function releasePages(): Promise<void> {
    const trackPages = heldTrackPages;
    const playlistPages = heldPlaylistPages;
    heldTrackPages = [];
    heldPlaylistPages = [];
    for (const pending of trackPages) pending.resolve();
    for (const pending of playlistPages) pending.resolve();
    await tick();
    await new Promise((resolve) => setTimeout(resolve, 0));
    await tick();
  }

  async function setColumns(nextColumns: TrackListColumnPreference[]): Promise<void> {
    columns = normalizeTrackColumnPreferences(nextColumns);
    await tick();
  }

  async function setLayout(nextLayout: NowPlayingLayout) {
    const previousArtwork = document.querySelector('[data-layout-pane="artwork"]');
    const previousLyrics = document.querySelector('[data-layout-pane="lyrics"]');
    layout = nextLayout;
    await tick();
    return {
      paneOrder: childOrder(document.querySelector('.now-playing-card')),
      nodesPreserved:
        previousArtwork === document.querySelector('[data-layout-pane="artwork"]') &&
        previousLyrics === document.querySelector('[data-layout-pane="lyrics"]'),
      lyricsText: document.querySelector('[data-layout-pane="lyrics"]')?.textContent?.trim() ?? null,
    };
  }

  function snapshot() {
    const content = document.querySelector<HTMLElement>('.page-content');
    const arrangement = document.querySelector<HTMLElement>('.now-playing-card');
    const panes = arrangement ? [...arrangement.querySelectorAll<HTMLElement>(':scope > [data-layout-pane]')] : [];
    return {
      library: listSnapshot('.track-list-viewport'),
      playlist: listSnapshot('.playlist-entry-table'),
      page: {
        documentWidth: document.documentElement.scrollWidth,
        viewportWidth: window.innerWidth,
        contentWidth: content?.scrollWidth ?? 0,
        contentClientWidth: content?.clientWidth ?? 0,
      },
      layout: {
        paneOrder: panes.map((pane) => pane.dataset.layoutPane ?? ''),
        paneTops: panes.map((pane) => Math.round(pane.getBoundingClientRect().top)),
        lyricsText: panes.find((pane) => pane.dataset.layoutPane === 'lyrics')?.textContent?.trim() ?? null,
      },
      switches: [...document.querySelectorAll<HTMLElement>('[role="group"][aria-label="正在播放頁排列"]')]
        .map((group) => ({
          role: group.getAttribute('role'),
          pressed: [...group.querySelectorAll<HTMLButtonElement>('button')].map((button) => button.getAttribute('aria-pressed')),
        })),
    };
  }

  onDestroy(() => {
    delete window.configurableColumnsHarness;
  });

  $effect(() => {
    window.configurableColumnsHarness = { releasePages, setColumns, setLayout, snapshot };
  });
</script>

<div class="app-shell configurable-columns-test-app">
  <aside class="sidebar" aria-label="測試側欄">
    <div class="brand-lockup"><strong>欄位測試</strong></div>
  </aside>
  <main class="workspace">
    <header class="topbar"><strong>測試畫面</strong></header>
    <div class="page-scroll">
      <main class="page-content">
        <section class="library-section" aria-label="曲庫列表">
          <div id="track-list-test">
            <TrackList
              query=""
              resetKey="configurable-columns-test"
              selectedTrackId={null}
              playbackReady={true}
              isSendingPlaybackCommand={false}
              fetchPage={fetchTrackPage}
              {columns}
              onPlay={() => {}}
            />
          </div>
        </section>
        <section class="playlist-section" aria-label="播放清單列表">
          <div id="playlist-list-test">
            <PlaylistEntryList
              playlistId="configurable-columns-test"
              resetKey={0}
              playbackReady={true}
              isSendingPlaybackCommand={false}
              fetchPage={fetchPlaylistPage}
              {columns}
              onPlay={() => {}}
            />
          </div>
        </section>
        <section class="now-playing-view" aria-label="正在播放排列">
          <NowPlayingLayoutSwitch layout={layout} variant="compact" onChange={(value) => (layout = value)} />
          <NowPlayingArrangement {layout}>
            {#snippet artwork()}
              <div class="cover-stage" data-testid="artwork-child">封面測試</div>
            {/snippet}
            {#snippet lyrics()}
              <div class="lyrics-panel-heading"><h3 id="lyrics-heading">歌詞</h3></div>
              <p class="lyrics-placeholder" data-testid="lyrics-child">尚無可顯示歌詞；歌詞載入功能尚未接通。</p>
            {/snippet}
          </NowPlayingArrangement>
          <NowPlayingLayoutSwitch layout={layout} onChange={(value) => (layout = value)} />
        </section>
      </main>
    </div>
  </main>
</div>
