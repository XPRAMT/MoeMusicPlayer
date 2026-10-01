<script lang="ts">
  import { tick } from 'svelte';
  import {
    IconArrowsShuffle,
    IconPlayerPlay,
    IconPlayerTrackNext,
    IconPlayerTrackPrev,
    IconRepeat,
  } from '@tabler/icons-svelte-runes';
  import LyricsView from '../src/lib/LyricsView.svelte';
  import NowPlayingArrangement from '../src/lib/NowPlayingArrangement.svelte';
  import NowPlayingLayoutSwitch from '../src/lib/NowPlayingLayoutSwitch.svelte';
  import { calculateArtworkFrame } from '../src/lib/artwork-frame.js';
  import hiResBadgeUrl from '../src/assets/hi-res-badge.png';
  import type { LyricsCandidate, LyricsTrackResult, NowPlayingLayout, TrackLyrics } from '../src/lib/ipc';

  interface LayoutSnapshot {
    documentWidth: number;
    documentHeight: number;
    viewportWidth: number;
    viewportHeight: number;
    sidebarDisplay: string;
    sidebar: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    overlay: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    outerOverflow: string;
    outerScrollHeight: number;
    outerClientHeight: number;
    outerScrollTop: number;
    shellHeight: number;
    topbar: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    dock: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    nowPlaying: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    card: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    panes: Array<{ name: string; left: number; right: number; top: number; bottom: number; width: number; height: number }>;
    artworkChildren: Array<{ className: string; left: number; right: number; top: number; bottom: number; width: number; height: number }>;
    cover: { left: number; right: number; top: number; bottom: number; width: number; height: number; objectFit: string; naturalWidth: number; naturalHeight: number };
    renderedImage: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    typography: { format: number; title: number; artist: number; album: number };
    returnButton: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    layoutSwitch: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    lyricsToolbar: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    dockControls: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    artworkCopyOverflowY: string;
    artworkCopyChildren: string[];
    artworkCopyTextAlign: string;
    formatJustifyContent: string;
    lyricTextAlign: { primary: string; translation: string; romanization: string };
    lyricsViewport: { clientHeight: number; scrollHeight: number; scrollTop: number; overflowY: string };
    playerControlIcons: Array<{ ariaHidden: string | null; width: number; height: number }>;
    playerControlLabels: Array<string | null>;
    layout: NowPlayingLayout;
  }

  interface LayoutHarnessApi {
    setLayout: (layout: NowPlayingLayout) => Promise<void>;
    setArtworkVariant: (variant: 'square' | 'portrait' | 'landscape') => Promise<void>;
    snapshot: () => LayoutSnapshot;
    scrollLyrics: () => Promise<boolean>;
    attemptOuterScroll: () => Promise<number>;
  }

  declare global {
    interface Window { nowPlayingLayoutHarness: LayoutHarnessApi; }
  }

  let layout = $state<NowPlayingLayout>('a');
  let artworkVariant = $state<'square' | 'portrait' | 'landscape'>('landscape');
  let coverFrame = $state<{ width: number; height: number } | null>(null);
  let coverStageElement: HTMLDivElement;

  const artworkDimensions = {
    square: [400, 400],
    portrait: [300, 400],
    landscape: [400, 300],
  } as const;

  function artworkDataUrl(variant: keyof typeof artworkDimensions): string {
    const [width, height] = artworkDimensions[variant];
    return `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><rect width="${width}" height="${height}" fill="#55d9ff"/></svg>`)}`;
  }

  function updateCoverFrame(): void {
    const image = coverStageElement?.querySelector('img');
    const pane = coverStageElement?.parentElement;
    if (!image?.naturalWidth || !image.naturalHeight || !pane) {
      coverFrame = null;
      return;
    }
    const maxDimension = window.innerHeight * 0.92;
    const frame = calculateArtworkFrame({
      sourceWidth: image.naturalWidth,
      sourceHeight: image.naturalHeight,
      availableWidth: pane.clientWidth,
      availableHeight: pane.clientHeight,
      maxWidth: maxDimension,
      maxHeight: maxDimension,
      border: 1,
    });
    coverFrame = frame ? { width: frame.width, height: frame.height } : null;
  }

  async function setArtworkVariant(variant: keyof typeof artworkDimensions): Promise<void> {
    artworkVariant = variant;
    await tick();
    const image = coverStageElement.querySelector('img');
    if (image && !image.complete) await new Promise<void>((resolve) => image.addEventListener('load', () => resolve(), { once: true }));
    updateCoverFrame();
  }

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
        translation: `測試譯文第 ${index + 1} 行`,
        romanization: `ce shi yi wen ${index + 1}`,
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

  $effect(() => {
    const stage = coverStageElement;
    const variant = artworkVariant;
    if (!stage) return;
    void variant;
    const image = stage.querySelector('img');
    const observer = new ResizeObserver(updateCoverFrame);
    if (stage.parentElement) observer.observe(stage.parentElement);
    image?.addEventListener('load', updateCoverFrame);
    updateCoverFrame();
    return () => {
      observer.disconnect();
      image?.removeEventListener('load', updateCoverFrame);
    };
  });

  function rect(element: Element | null): { top: number; bottom: number; left: number; right: number; width: number; height: number } {
    if (!element) return { top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0 };
    const value = element.getBoundingClientRect();
    return { top: value.top, bottom: value.bottom, left: value.left, right: value.right, width: value.width, height: value.height };
  }

  function snapshot(): LayoutSnapshot {
    const root = document.documentElement;
    const outer = document.querySelector<HTMLElement>('[data-testid="outer-page-scroll"]');
    const sidebar = document.querySelector<HTMLElement>('.sidebar');
    const overlay = document.querySelector<HTMLElement>('[data-testid="now-playing-overlay"]');
    const card = document.querySelector<HTMLElement>('.now-playing-card');
    const cover = document.querySelector<HTMLImageElement>('[data-testid="cover-image"]');
    const formatRow = document.querySelector<HTMLElement>('.now-playing-format');
    const title = document.querySelector<HTMLElement>('.now-playing-track-info [data-track-field="title"]');
    const artist = document.querySelector<HTMLElement>('.now-playing-track-info [data-track-field="artist"]');
    const album = document.querySelector<HTMLElement>('.now-playing-track-info [data-track-field="album"]');
    const lyrics = document.querySelector<HTMLElement>('[data-testid="timed-lyrics"]');
    const paneElements = [...document.querySelectorAll<HTMLElement>('.now-playing-card > [data-layout-pane]')];
    const artworkChildren = [...document.querySelectorAll<HTMLElement>('.now-playing-artwork > *')];
    const coverRect = rect(document.querySelector('[data-testid="cover-stage"]'));
    const topbarRect = rect(document.querySelector('[data-testid="layout-topbar"]'));
    const dockRect = rect(document.querySelector('[data-testid="layout-dock"]'));
    const nowPlayingRect = rect(document.querySelector('.now-playing-view'));
    const cardRect = rect(card);
    const returnRect = rect(document.querySelector('.now-playing-overlay-return'));
    const layoutSwitchRect = rect(document.querySelector('.now-playing-layout-switch'));
    const lyricsToolbarRect = rect(document.querySelector('.lyrics-topbar-toggles'));
    const dockControlsRect = rect(document.querySelector('.dock-controls'));
    const artworkCopy = document.querySelector<HTMLElement>('.now-playing-copy');
    const lyricPrimary = document.querySelector<HTMLElement>('.lyric-primary');
    const lyricTranslation = document.querySelector<HTMLElement>('.lyric-translation');
    const lyricRomanization = document.querySelector<HTMLElement>('.lyric-romanization');

    return {
      documentWidth: root.scrollWidth,
      documentHeight: root.scrollHeight,
      viewportWidth: root.clientWidth,
      viewportHeight: root.clientHeight,
      sidebarDisplay: sidebar ? getComputedStyle(sidebar).display : 'missing',
      sidebar: rect(sidebar),
      overlay: rect(overlay),
      outerOverflow: outer ? getComputedStyle(outer).overflowY : 'missing',
      outerScrollHeight: outer?.scrollHeight ?? 0,
      outerClientHeight: outer?.clientHeight ?? 0,
      outerScrollTop: outer?.scrollTop ?? -1,
      shellHeight: document.querySelector<HTMLElement>('.app-shell')?.getBoundingClientRect().height ?? 0,
      topbar: topbarRect,
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
      renderedImage: rect(cover),
      typography: {
        format: formatRow ? Number.parseFloat(getComputedStyle(formatRow).fontSize) : 0,
        title: title ? Number.parseFloat(getComputedStyle(title).fontSize) : 0,
        artist: artist ? Number.parseFloat(getComputedStyle(artist).fontSize) : 0,
        album: album ? Number.parseFloat(getComputedStyle(album).fontSize) : 0,
      },
      returnButton: returnRect,
      layoutSwitch: layoutSwitchRect,
      lyricsToolbar: lyricsToolbarRect,
      dockControls: dockControlsRect,
      artworkCopyOverflowY: artworkCopy ? getComputedStyle(artworkCopy).overflowY : 'missing',
      artworkCopyChildren: artworkCopy ? [...artworkCopy.children].map((child) => child.className.toString() || child.tagName.toLowerCase()) : [],
      artworkCopyTextAlign: artworkCopy ? getComputedStyle(artworkCopy).textAlign : 'missing',
      formatJustifyContent: formatRow ? getComputedStyle(formatRow).justifyContent : 'missing',
      lyricTextAlign: {
        primary: lyricPrimary ? getComputedStyle(lyricPrimary).textAlign : 'missing',
        translation: lyricTranslation ? getComputedStyle(lyricTranslation).textAlign : 'missing',
        romanization: lyricRomanization ? getComputedStyle(lyricRomanization).textAlign : 'missing',
      },
      lyricsViewport: {
        clientHeight: lyrics?.clientHeight ?? 0,
        scrollHeight: lyrics?.scrollHeight ?? 0,
        scrollTop: lyrics?.scrollTop ?? 0,
        overflowY: lyrics ? getComputedStyle(lyrics).overflowY : 'missing',
      },
      playerControlIcons: [...document.querySelectorAll<SVGElement>('.dock-controls button svg')].map((icon) => ({
        ariaHidden: icon.getAttribute('aria-hidden'),
        width: icon.getBoundingClientRect().width,
        height: icon.getBoundingClientRect().height,
      })),
      playerControlLabels: [...document.querySelectorAll<HTMLButtonElement>('.dock-controls button')]
        .map((button) => button.getAttribute('aria-label')),
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
      window.nowPlayingLayoutHarness = { setLayout, setArtworkVariant, snapshot, scrollLyrics, attemptOuterScroll };
  });
</script>

<div class="app-shell" data-active-view="library" data-testid="now-playing-shell">
  <div class="now-playing-backdrop" data-testid="now-playing-backdrop" aria-hidden="true"></div>
  <aside class="sidebar" aria-label="底層主要導覽"><span>側欄</span></aside>
  <main class="workspace">
    <header class="topbar"><strong>MOEMUSIC / LIBRARY</strong></header>
    <div class="page-scroll"><div class="page-content">底層曲庫狀態</div></div>
  </main>
  <section class="now-playing-overlay is-open" data-testid="now-playing-overlay" aria-label="正在播放">
    <header class="topbar now-playing-overlay-topbar" data-testid="layout-topbar">
      <button class="outline-button now-playing-overlay-return" type="button" aria-label="返回播放前頁面" title="返回播放前頁面">返回</button>
      <div class="now-playing-header-track">
        <nav class="now-playing-track-info" aria-label="曲目資訊與曲庫分類">
          <button data-track-field="title" type="button">測試曲名</button><span aria-hidden="true">．</span>
          <button data-track-field="artist" type="button">測試演出者</button><span aria-hidden="true">．</span>
          <button data-track-field="album" type="button">測試專輯</button>
        </nav>
        <p class="now-playing-format"><span>FLAC．48 kHz．24 bit</span><img src={hiResBadgeUrl} alt="Hi-Res" /></p>
      </div>
      <div class="now-playing-topbar-tools">
        <div class="lyrics-topbar-toggles" role="group" aria-label="歌詞副行顯示">
          <button type="button" class="lyrics-toggle" aria-pressed="true" aria-label="切換譯文顯示">譯</button>
          <button type="button" class="lyrics-toggle" aria-pressed="false" aria-label="切換羅馬拼音顯示">羅</button>
        </div>
      </div>
    </header>
    <div class="now-playing-overlay-body" data-testid="outer-page-scroll">
      <div class="now-playing-overlay-content">
        <section class="now-playing-view" aria-label="正在播放版面">
          <NowPlayingLayoutSwitch {layout} variant="compact" onChange={setLayout} />
          <NowPlayingArrangement {layout}>
            {#snippet artwork()}
              <div
                bind:this={coverStageElement}
                class="cover-stage"
                data-testid="cover-stage"
                style:width={coverFrame ? `${coverFrame.width}px` : undefined}
                style:height={coverFrame ? `${coverFrame.height}px` : undefined}
              >
                <img
                  class="cover-stage-image"
                  data-testid="cover-image"
                  src={artworkDataUrl(artworkVariant)}
                  alt="測試封面"
                />
              </div>
            {/snippet}
            {#snippet lyrics()}
              <LyricsView
                trackId="layout-probe"
                positionMs={50_000}
                isPlaying={true}
                playbackState="playing"
                lyricsPreferences={{ showTranslation: true, showRomanization: true, inactiveOpacityPercent: 70, primaryFontSizePx: 14, auxiliaryFontSizePx: 10 }}
                api={lyricsApi}
              />
            {/snippet}
          </NowPlayingArrangement>
        </section>
      </div>
    </div>
  </section>
  <footer class="player-dock" data-testid="layout-dock">
    <div class="dock-track"><button class="dock-art" type="button" aria-label="目前歌曲封面">M</button><div class="dock-track-copy"><strong>版面驗收曲目</strong><span>測試演出者</span></div></div>
    <div class="dock-center"><div class="dock-controls"><button class="control-button secondary-control" type="button" aria-label="隨機播放"><IconArrowsShuffle size={20} stroke={1.7} aria-hidden="true" /></button><button class="control-button" type="button" aria-label="上一首"><IconPlayerTrackPrev size={20} stroke={1.7} aria-hidden="true" /></button><button class="play-button" type="button" aria-label="播放"><IconPlayerPlay size={21} stroke={1.9} aria-hidden="true" /></button><button class="control-button" type="button" aria-label="下一首"><IconPlayerTrackNext size={20} stroke={1.7} aria-hidden="true" /></button><button class="control-button secondary-control" type="button" aria-label="循環播放"><IconRepeat size={20} stroke={1.7} aria-hidden="true" /></button></div><div class="progress-row"><span>0:50</span><input type="range" min="0" max="120" value="50" aria-label="播放進度"/><span>2:00</span></div></div>
    <div class="dock-volume"><span>音量</span><input class="volume-slider" type="range" min="0" max="1" step="0.01" value="0.5" aria-label="音量"/></div>
  </footer>
</div>
