<script lang="ts">
  import { tick } from 'svelte';
  import LyricsView from '../src/lib/LyricsView.svelte';
  import type {
    LyricsCandidate,
    LyricsPreferences,
    LyricsTrackResult,
    TrackLyrics,
  } from '../src/lib/ipc';
  import { DEFAULT_LYRICS_PREFERENCES, normalizeLyricsPreferences } from '../src/lib/lyrics-preferences.js';
  import { buildLyricsLayout } from '../src/lib/lyrics-window.js';

  interface HarnessApi {
    setPosition: (positionMs: number) => Promise<void>;
    setPlaying: (isPlaying: boolean) => Promise<void>;
    setTrack: (trackId: string | null) => Promise<void>;
    setPlaybackState: (state: 'playing' | 'paused' | 'stopped' | 'ready') => Promise<void>;
    setPreferences: (patch: Partial<LyricsPreferences>) => Promise<void>;
    scrollPlainTo: (lineIndex: number) => Promise<void>;
    snapshot: () => {
      trackId: string;
      phase: string;
      activeIndex: number | null;
      activeCueIndex: number | null;
      renderedTimedRows: number;
      renderedPlainRows: number;
      plainFirstIndex: number | null;
      getCount: number;
      searchCount: number;
      cancelCount: number;
      selectedSource: string | null;
      qrcNoticeVisible: boolean;
      candidateActionEnabled: boolean;
      documentWidth: number;
      viewportWidth: number;
      isPlaying: string | null;
      playbackState: string | null;
      totalHeight: number;
      lineGapPx: number;
      timedWindowStart: number | null;
      timedWindowEnd: number | null;
      beforeSpacerHeight: number;
      afterSpacerHeight: number;
      actualRowHeights: number[];
      firstRowTranslationCount: number;
      firstRowRomanizationCount: number;
      firstRowOpacity: number | null;
      firstRowTextOpacity: number | null;
      activeRowOpacity: number | null;
      activeRowTextOpacity: number | null;
      activeRowContentHeight: number | null;
      activeRowBoxHeight: number | null;
      bothAuxiliaryRowContentHeight: number | null;
      firstPrimaryTextHeight: number | null;
      firstAuxiliaryTop: number | null;
      firstPrimaryBottom: number | null;
      longPrimaryHeight: number | null;
      longPrimaryLineHeight: number | null;
      timedScrollTop: number | null;
      timedViewportHeight: number | null;
      preferences: LyricsPreferences;
      toolbarAvailable: boolean;
    };
  }

  declare global {
    interface Window {
      lyricsViewHarness: HarnessApi;
    }
  }

  let trackId = $state<string | null>('timed-track');
  let positionMs = $state(0);
  let isPlaying = $state(true);
  let playbackState = $state<'playing' | 'paused' | 'stopped' | 'ready'>('playing');
  let lyricsPreferences = $state<LyricsPreferences>(normalizeLyricsPreferences(DEFAULT_LYRICS_PREFERENCES));
  let getCount = 0;
  let searchCount = 0;
  let cancelCount = 0;
  let selectedSource: string | null = null;

  function makeLyrics(id: string, source: TrackLyrics['source'], synced: boolean): TrackLyrics {
    return {
      trackId: id,
      source,
      title: `曲目 ${id}`,
      artist: '測試演出者',
      album: '測試專輯',
      durationMs: 120_000,
      offsetMs: 0,
      synced,
      lines: Array.from({ length: 120 }, (_, index) => ({
        startMs: synced ? index * 1_000 : null,
        text: synced && index === 90 ? '這段原文用來確認實際換行高度會隨可用歌詞寬度一起改變而更新' : index === 1 ? '  ' : `${id} 歌詞 ${index}`,
        translation: index % 3 === 0 ? `翻譯 ${index}` : index % 3 === 1 ? '  ' : null,
        romanization: index % 5 === 0 ? `拼音 ${index}` : index % 5 === 1 ? '' : null,
      })),
    };
  }

  const candidate: LyricsCandidate = {
    id: 'candidate-hanser',
    provider: 'qqmusic',
    title: '候選歌詞',
    artist: '測試演出者',
    album: null,
    durationMs: 121_000,
    score: 0.88,
    confidence: 'medium',
    reasons: ['標題與演出者相符'],
    previewLines: [],
    hasSyncedLyrics: false,
  };

  const api = {
    async getTrack({ trackId: requestedTrackId }: { trackId: string }): Promise<LyricsTrackResult> {
      getCount += 1;
      if (requestedTrackId === 'candidate-track') {
        return { lyrics: null, candidates: [], status: 'empty', error: null };
      }
      const synced = requestedTrackId !== 'plain-track';
      return {
        lyrics: makeLyrics(requestedTrackId, 'local', synced),
        candidates: [],
        status: 'ready',
        error: null,
      };
    },
    async search({ trackId: requestedTrackId }: { trackId: string; requestId: string }): Promise<LyricsTrackResult> {
      searchCount += 1;
      return requestedTrackId === 'candidate-track'
        ? { lyrics: null, candidates: [candidate], status: 'candidates', error: null }
        : { lyrics: null, candidates: [], status: 'empty', error: null };
    },
    async selectCandidate({ trackId: requestedTrackId }: { trackId: string; candidateId: string }): Promise<TrackLyrics> {
      selectedSource = 'manual';
      const selected = makeLyrics(requestedTrackId, 'manual', true);
      selected.lines = selected.lines.slice(0, 4);
      return selected;
    },
    async cancelSearch(): Promise<void> {
      cancelCount += 1;
    },
  };

  async function settle(): Promise<void> {
    await tick();
    await Promise.resolve();
    await tick();
    await new Promise((resolve) => setTimeout(resolve, 0));
    await tick();
  }

  function updatePreferences(patch: Partial<LyricsPreferences>): void {
    lyricsPreferences = normalizeLyricsPreferences({ ...lyricsPreferences, ...patch });
  }

  function snapshot() {
    const timed = document.querySelector<HTMLElement>('[data-testid="timed-lyrics"]');
    const plain = document.querySelector<HTMLElement>('[data-testid="plain-lyrics"]');
    const viewport = timed ?? plain;
    const active = timed?.querySelector<HTMLElement>('[aria-current="true"]');
    const firstPlain = plain?.querySelector<HTMLElement>('[data-lyric-index]');
    const rows = [...(viewport?.querySelectorAll<HTMLElement>('.lyric-line') ?? [])];
    const spacers = [...(viewport?.querySelectorAll<HTMLElement>('.lyrics-spacer') ?? [])];
    const rowContentHeight = (row: HTMLElement | null): number | null => {
      if (!row) return null;
      const paragraphs = [...row.querySelectorAll<HTMLElement>('p')];
      const style = getComputedStyle(row);
      return paragraphs.reduce((height, paragraph) => height + paragraph.getBoundingClientRect().height, 0)
        + Math.max(0, paragraphs.length - 1) * Number.parseFloat(style.gap)
        + Number.parseFloat(style.paddingTop)
        + Number.parseFloat(style.paddingBottom)
        + Number.parseFloat(style.borderBottomWidth);
    };
    const firstBothAuxiliary = rows.find((row) => row.querySelector('.lyric-translation') && row.querySelector('.lyric-romanization')) ?? null;
    const activePrimary = active?.querySelector<HTMLElement>('.lyric-primary') ?? null;
    const firstPrimary = rows[0]?.querySelector<HTMLElement>('.lyric-primary') ?? null;
    const longPrimary = rows.find((row) => row.querySelector('.lyric-primary')?.textContent?.startsWith('這段原文'))?.querySelector<HTMLElement>('.lyric-primary') ?? null;
    const firstAuxiliary = rows[0]?.querySelector<HTMLElement>('.lyric-translation, .lyric-romanization') ?? null;
    const lyricsRoot = document.querySelector<HTMLElement>('[data-testid="lyrics-view"]');

    return {
      trackId,
      phase: document.querySelector<HTMLElement>('[data-testid="lyrics-view"]')?.dataset.phase ?? '',
      activeIndex: active ? Number(active.dataset.lyricIndex) : null,
      activeCueIndex: lyricsRoot?.dataset.activeCueIndex ? Number(lyricsRoot.dataset.activeCueIndex) : null,
      renderedTimedRows: timed?.querySelectorAll('.lyric-line').length ?? 0,
      renderedPlainRows: plain?.querySelectorAll('.lyric-line').length ?? 0,
      plainFirstIndex: firstPlain ? Number(firstPlain.dataset.lyricIndex) : null,
      getCount,
      searchCount,
      cancelCount,
      selectedSource,
      qrcNoticeVisible: document.querySelector('.candidate-preview-note') !== null,
      candidateActionEnabled: [...document.querySelectorAll('button')].some((button) => button.textContent?.trim() === '使用這份' && !button.disabled),
      documentWidth: document.documentElement.scrollWidth,
      viewportWidth: window.innerWidth,
      isPlaying: lyricsRoot?.dataset.playing ?? null,
      playbackState: lyricsRoot?.dataset.playbackState ?? null,
      totalHeight: Number(viewport?.dataset.totalHeight ?? 0),
      lineGapPx: Number(lyricsRoot?.dataset.lineGap ?? 24),
      timedWindowStart: timed ? Number(timed.dataset.windowStart) : null,
      timedWindowEnd: timed ? Number(timed.dataset.windowEnd) : null,
      beforeSpacerHeight: spacers[0]?.getBoundingClientRect().height ?? 0,
      afterSpacerHeight: spacers.at(-1)?.getBoundingClientRect().height ?? 0,
      actualRowHeights: rows.map((row) => row.getBoundingClientRect().height),
      expectedRowHeights: rows.map((row) => Number.parseFloat(getComputedStyle(row).height)),
      rowMargins: rows.map((row) => Number.parseFloat(getComputedStyle(row).marginBottom)),
      firstRowTranslationCount: rows[0]?.querySelectorAll('.lyric-translation').length ?? 0,
      firstRowRomanizationCount: rows[0]?.querySelectorAll('.lyric-romanization').length ?? 0,
      firstRowOpacity: rows[0] ? Number(getComputedStyle(rows[0]).opacity) : null,
      firstRowTextOpacity: firstPrimary ? Number(getComputedStyle(firstPrimary).opacity) : null,
      activeRowOpacity: active ? Number(getComputedStyle(active).opacity) : null,
      activeRowTextOpacity: activePrimary ? Number(getComputedStyle(activePrimary).opacity) : null,
      activeRowContentHeight: rowContentHeight(active),
      activeRowBoxHeight: active?.getBoundingClientRect().height ?? null,
      bothAuxiliaryRowContentHeight: rowContentHeight(firstBothAuxiliary),
      firstPrimaryTextHeight: firstPrimary?.getBoundingClientRect().height ?? null,
      firstAuxiliaryTop: firstAuxiliary?.getBoundingClientRect().top ?? null,
      firstPrimaryBottom: firstPrimary?.getBoundingClientRect().bottom ?? null,
      longPrimaryHeight: longPrimary?.getBoundingClientRect().height ?? null,
      longPrimaryLineHeight: longPrimary ? Number.parseFloat(getComputedStyle(longPrimary).lineHeight) : null,
      timedScrollTop: timed?.scrollTop ?? null,
      timedViewportHeight: timed?.clientHeight ?? null,
      preferences: { ...lyricsPreferences },
      toolbarAvailable: [...document.querySelectorAll<HTMLButtonElement>('.lyrics-toggle')].every((button) => !button.disabled),
    };
  }

  window.lyricsViewHarness = {
    async setPosition(nextPositionMs) {
      positionMs = nextPositionMs;
      await tick();
    },
    async setPlaying(nextIsPlaying) {
      isPlaying = nextIsPlaying;
      playbackState = nextIsPlaying ? 'playing' : 'paused';
      await tick();
    },
    async setPlaybackState(nextState) {
      playbackState = nextState;
      isPlaying = nextState === 'playing';
      await tick();
    },
    async setPreferences(patch) {
      updatePreferences(patch);
      await tick();
    },
    async setTrack(nextTrackId) {
      trackId = nextTrackId;
      positionMs = 0;
      await settle();
    },
    async scrollPlainTo(lineIndex) {
      const plain = document.querySelector<HTMLElement>('[data-testid="plain-lyrics"]');
      if (plain) {
        const lyrics = makeLyrics(trackId ?? 'plain-track', 'local', false);
        const layout = buildLyricsLayout(lyrics.lines, lyricsPreferences, lyricsPreferences.lineGapPx);
        plain.scrollTop = layout.offsets[Math.min(lineIndex, layout.heights.length)];
        plain.dispatchEvent(new Event('scroll'));
        await tick();
      }
    },
    snapshot,
  };

  void settle();
</script>

<main>
  <div class="harness-lyrics-toolbar" role="group" aria-label="歌詞副行顯示">
    <button type="button" class="lyrics-toggle" aria-label="切換譯文顯示" aria-pressed={lyricsPreferences.showTranslation} onclick={() => updatePreferences({ showTranslation: !lyricsPreferences.showTranslation })}>譯</button>
    <button type="button" class="lyrics-toggle" aria-label="切換羅馬拼音顯示" aria-pressed={lyricsPreferences.showRomanization} onclick={() => updatePreferences({ showRomanization: !lyricsPreferences.showRomanization })}>羅</button>
  </div>
  <LyricsView
    {trackId}
    {positionMs}
    {isPlaying}
    {playbackState}
    {lyricsPreferences}
    onPreferencesChange={updatePreferences}
    {api}
  />
</main>

<style>
  :global(html),
  :global(body),
  :global(#app) {
    width: 100%;
    min-height: 100%;
    margin: 0;
  }

  main {
    box-sizing: border-box;
    width: min(100%, 960px);
    min-height: 760px;
    padding: 20px;
    color: var(--text);
    background: var(--app-bg);
  }

  .harness-lyrics-toolbar { display: flex; justify-content: flex-end; gap: 6px; padding: 8px; }
  .lyrics-toggle { min-width: 34px; min-height: 32px; border: 1px solid var(--line); border-radius: 8px; color: var(--muted); background: var(--panel); cursor: pointer; }
</style>
