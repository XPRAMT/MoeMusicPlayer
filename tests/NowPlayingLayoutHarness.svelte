<script lang="ts">
  import { tick } from 'svelte';
  import LyricsView from '../src/lib/LyricsView.svelte';
  import NowPlayingArrangement from '../src/lib/NowPlayingArrangement.svelte';
  import NowPlayingLayoutSwitch from '../src/lib/NowPlayingLayoutSwitch.svelte';
  import type { LyricsCandidate, LyricsTrackResult, NowPlayingLayout, TrackLyrics } from '../src/lib/ipc';

  interface LayoutSnapshot {
    documentWidth: number;
    documentHeight: number;
    viewportWidth: number;
    viewportHeight: number;
    sidebarDisplay: string;
    outerOverflow: string;
    outerScrollHeight: number;
    outerClientHeight: number;
    outerScrollTop: number;
    shellHeight: number;
    topbar: { top: number; bottom: number; height: number };
    dock: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    nowPlaying: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    card: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    panes: Array<{ name: string; left: number; right: number; top: number; bottom: number; width: number; height: number }>;
    artworkChildren: Array<{ className: string; left: number; right: number; top: number; bottom: number; width: number; height: number }>;
    cover: { left: number; right: number; top: number; bottom: number; width: number; height: number; objectFit: string; naturalWidth: number; naturalHeight: number };
    returnButton: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    layoutSwitch: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    lyricsToolbar: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    dockControls: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    artworkCopyOverflowY: string;
    lyricsViewport: { clientHeight: number; scrollHeight: number; scrollTop: number; overflowY: string };
    layout: NowPlayingLayout;
  }

  interface LayoutHarnessApi {
    setLayout: (layout: NowPlayingLayout) => Promise<void>;
    snapshot: () => LayoutSnapshot;
    scrollLyrics: () => Promise<boolean>;
    attemptOuterScroll: () => Promise<number>;
  }

  declare global {
    interface Window { nowPlayingLayoutHarness: LayoutHarnessApi; }
  }

  let layout = $state<NowPlayingLayout>('a');

  function makeLyrics(trackId: string): TrackLyrics {
    return {
      trackId,
      source: 'local',
      title: '版面驗收曲目',
      artist: '測試演出者',
      album: '版面測試',
      durationMs: 120_000,
      offsetMs: 0,
      synced: true,
      lines: Array.from({ length: 120 }, (_, index) => ({
        startMs: index * 1_000,
        text: `同步歌詞第 ${index + 1} 行`,
        translation: null,
        romanization: null,
      })),
    };
  }

  const lyricsApi = {
    async getTrack({ trackId }: { trackId: string }): Promise<LyricsTrackResult> {
      return { lyrics: makeLyrics(trackId), candidates: [], status: 'ready', error: null };
    },
    async search(): Promise<LyricsTrackResult> {
      return { lyrics: null, candidates: [], status: 'empty', error: null };
    },
    async selectCandidate({ trackId }: { trackId: string; candidateId: string }): Promise<TrackLyrics> {
      return makeLyrics(trackId);
    },
    async cancelSearch(): Promise<void> {},
  };

  async function setLayout(nextLayout: NowPlayingLayout): Promise<void> {
    layout = nextLayout;
    await tick();
  }

  function rect(element: Element | null): { top: number; bottom: number; left: number; right: number; width: number; height: number } {
    if (!element) return { top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0 };
    const value = element.getBoundingClientRect();
    return { top: value.top, bottom: value.bottom, left: value.left, right: value.right, width: value.width, height: value.height };
  }

  function snapshot(): LayoutSnapshot {
    const root = document.documentElement;
    const outer = document.querySelector<HTMLElement>('[data-testid="outer-page-scroll"]');
    const card = document.querySelector<HTMLElement>('.now-playing-card');
    const cover = document.querySelector<HTMLImageElement>('[data-testid="cover-image"]');
    const lyrics = document.querySelector<HTMLElement>('[data-testid="timed-lyrics"]');
    const paneElements = [...document.querySelectorAll<HTMLElement>('.now-playing-card > [data-layout-pane]')];
    const artworkChildren = [...document.querySelectorAll<HTMLElement>('.now-playing-artwork > *')];
    const coverRect = rect(document.querySelector('[data-testid="cover-stage"]'));
    const topbarRect = rect(document.querySelector('[data-testid="layout-topbar"]'));
    const dockRect = rect(document.querySelector('[data-testid="layout-dock"]'));
    const nowPlayingRect = rect(document.querySelector('.now-playing-view'));
    const cardRect = rect(card);
    const returnRect = rect(document.querySelector('.now-playing-return'));
    const layoutSwitchRect = rect(document.querySelector('.now-playing-layout-switch'));
    const lyricsToolbarRect = rect(document.querySelector('.lyrics-display-controls'));
    const dockControlsRect = rect(document.querySelector('.dock-controls'));
    const artworkCopy = document.querySelector<HTMLElement>('.now-playing-copy');

    return {
      documentWidth: root.scrollWidth,
      documentHeight: root.scrollHeight,
      viewportWidth: root.clientWidth,
      viewportHeight: root.clientHeight,
      sidebarDisplay: getComputedStyle(document.querySelector('.sidebar')!).display,
      outerOverflow: outer ? getComputedStyle(outer).overflowY : 'missing',
      outerScrollHeight: outer?.scrollHeight ?? 0,
      outerClientHeight: outer?.clientHeight ?? 0,
      outerScrollTop: outer?.scrollTop ?? -1,
      shellHeight: document.querySelector<HTMLElement>('.app-shell')?.getBoundingClientRect().height ?? 0,
      topbar: { top: topbarRect.top, bottom: topbarRect.bottom, height: topbarRect.height },
      dock: dockRect,
      nowPlaying: nowPlayingRect,
      card: cardRect,
      panes: paneElements.map((pane) => {
        const value = rect(pane);
        return { name: pane.dataset.layoutPane ?? '', left: value.left, right: value.right, top: value.top, bottom: value.bottom, width: value.width, height: value.height };
      }),
      artworkChildren: artworkChildren.map((child) => ({ className: child.className.toString(), ...rect(child) })),
      cover: {
        ...coverRect,
        objectFit: cover ? getComputedStyle(cover).objectFit : 'missing',
        naturalWidth: cover?.naturalWidth ?? 0,
        naturalHeight: cover?.naturalHeight ?? 0,
      },
      returnButton: returnRect,
      layoutSwitch: layoutSwitchRect,
      lyricsToolbar: lyricsToolbarRect,
      dockControls: dockControlsRect,
      artworkCopyOverflowY: artworkCopy ? getComputedStyle(artworkCopy).overflowY : 'missing',
      lyricsViewport: {
        clientHeight: lyrics?.clientHeight ?? 0,
        scrollHeight: lyrics?.scrollHeight ?? 0,
        scrollTop: lyrics?.scrollTop ?? 0,
        overflowY: lyrics ? getComputedStyle(lyrics).overflowY : 'missing',
      },
      layout,
    };
  }

  async function scrollLyrics(): Promise<boolean> {
    await tick();
    const lyrics = document.querySelector<HTMLElement>('[data-testid="timed-lyrics"]');
    if (!lyrics || lyrics.clientHeight <= 0) return false;
    const initial = lyrics.scrollTop;
    const maxScroll = lyrics.scrollHeight - lyrics.clientHeight;
    if (maxScroll <= 1) return false;
    lyrics.scrollTop = Math.min(maxScroll, initial + Math.max(80, lyrics.clientHeight));
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    return lyrics.scrollTop > initial + 1;
  }

  async function attemptOuterScroll(): Promise<number> {
    const outer = document.querySelector<HTMLElement>('[data-testid="outer-page-scroll"]');
    if (!outer) return -1;
    outer.scrollTop = 100;
    await tick();
    return outer.scrollTop;
  }

  $effect(() => {
    window.nowPlayingLayoutHarness = { setLayout, snapshot, scrollLyrics, attemptOuterScroll };
  });
</script>

<div class="app-shell now-playing-shell" data-active-view="now-playing" data-testid="now-playing-shell">
  <aside class="sidebar" aria-label="隱藏的側邊導覽"><span>側欄</span></aside>
  <main class="workspace">
    <header class="topbar" data-testid="layout-topbar"><strong>MOEMUSIC / NOW PLAYING</strong></header>
    <div class="page-scroll" data-testid="outer-page-scroll">
      <div class="page-content">
        <section class="now-playing-view" aria-label="正在播放版面">
          <NowPlayingLayoutSwitch {layout} variant="compact" onChange={setLayout} />
          <NowPlayingArrangement {layout}>
            {#snippet artwork()}
              <div class="cover-stage" data-testid="cover-stage">
                <img
                  class="cover-stage-image"
                  data-testid="cover-image"
                  src="data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%22400%22 height=%22300%22 viewBox=%220 0 400 300%22%3E%3Crect width=%22400%22 height=%22300%22 fill=%22%2355d9ff%22/%3E%3C/svg%3E"
                  alt="測試封面"
                />
              </div>
              <div class="now-playing-copy">
                <p class="section-kicker">NOW PLAYING</p>
        <h2>這是一段刻意加長的曲目標題，用來確認不同尺寸的正在播放頁會限制標題行數，並且不會把返回曲庫控制擠出畫面範圍或造成另一個可以捲動的資訊欄位。</h2>
                <p class="now-playing-artist">測試演出者</p>
                <div class="play-state-chip ready">播放中</div>
                <button class="outline-button now-playing-return" type="button">返回曲庫</button>
              </div>
            {/snippet}
            {#snippet lyrics()}
              <LyricsView
                trackId="layout-probe"
                positionMs={50_000}
                isPlaying={true}
                playbackState="playing"
                api={lyricsApi}
              />
            {/snippet}
          </NowPlayingArrangement>
          <div class="playback-note"><span class="note-icon">i</span><p>播放狀態由原生音訊服務提供。</p></div>
        </section>
      </div>
    </div>
  </main>
  <footer class="player-dock" data-testid="layout-dock">
    <div class="dock-track"><button class="dock-art" type="button" aria-label="目前歌曲封面">M</button><div class="dock-track-copy"><strong>版面驗收曲目</strong><span>測試演出者</span></div></div>
    <div class="dock-center"><div class="dock-controls"><button class="control-button" type="button" aria-label="上一首">‹</button><button class="play-button" type="button" aria-label="播放">▶</button><button class="control-button" type="button" aria-label="下一首">›</button></div><div class="progress-row"><span>0:50</span><input type="range" min="0" max="120" value="50" aria-label="播放進度"/><span>2:00</span></div></div>
    <div class="dock-volume"><span>音量</span><input class="volume-slider" type="range" min="0" max="1" step="0.01" value="0.5" aria-label="音量"/></div>
  </footer>
</div>
