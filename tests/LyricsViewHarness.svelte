<script lang="ts">
  import { tick } from 'svelte';
  import LyricsView from '../src/lib/LyricsView.svelte';
  import type {
    LyricsCandidate,
    LyricsTrackResult,
    TrackLyrics,
  } from '../src/lib/ipc';

  interface HarnessApi {
    setPosition: (positionMs: number) => Promise<void>;
    setPlaying: (isPlaying: boolean) => Promise<void>;
    setTrack: (trackId: string) => Promise<void>;
    scrollPlainTo: (lineIndex: number) => Promise<void>;
    snapshot: () => {
      trackId: string;
      phase: string;
      activeIndex: number | null;
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
    };
  }

  declare global {
    interface Window {
      lyricsViewHarness: HarnessApi;
    }
  }

  let trackId = $state('timed-track');
  let positionMs = $state(0);
  let isPlaying = $state(true);
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
        text: `${id} 歌詞 ${index}`,
        translation: index % 3 === 0 ? `翻譯 ${index}` : null,
        romanization: index % 5 === 0 ? `拼音 ${index}` : null,
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

  function snapshot() {
    const timed = document.querySelector<HTMLElement>('[data-testid="timed-lyrics"]');
    const plain = document.querySelector<HTMLElement>('[data-testid="plain-lyrics"]');
    const active = timed?.querySelector<HTMLElement>('[aria-current="true"]');
    const firstPlain = plain?.querySelector<HTMLElement>('[data-lyric-index]');

    return {
      trackId,
      phase: document.querySelector<HTMLElement>('[data-testid="lyrics-view"]')?.dataset.phase ?? '',
      activeIndex: active ? Number(active.dataset.lyricIndex) : null,
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
      isPlaying: document.querySelector<HTMLElement>('[data-testid="lyrics-view"]')?.dataset.playing ?? null,
    };
  }

  window.lyricsViewHarness = {
    async setPosition(nextPositionMs) {
      positionMs = nextPositionMs;
      await tick();
    },
    async setPlaying(nextIsPlaying) {
      isPlaying = nextIsPlaying;
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
        plain.scrollTop = lineIndex * 76;
        plain.dispatchEvent(new Event('scroll'));
        await tick();
      }
    },
    snapshot,
  };

  void settle();
</script>

<main>
  <LyricsView {trackId} {positionMs} {isPlaying} {api} />
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
    width: min(100%, 360px);
    min-height: 760px;
    padding: 20px;
    color: var(--text);
    background: var(--app-bg);
  }
</style>
