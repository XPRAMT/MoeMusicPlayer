import { mount, tick } from 'svelte';
import TrackList from '../src/lib/TrackList.svelte';
import '../src/app.css';
import type { TrackPage, TrackPageRequest, TrackSummary } from '../src/lib/ipc';

interface TrackListHarnessState {
  requestCount: number;
  generatedRecords: number;
  totalCount: number | null;
  playedTrackId: string | null;
}

interface TrackListHarnessApi {
  state: TrackListHarnessState;
  scrollSweep: (stepRows?: number) => Promise<{
    steps: number;
    maxDomRows: number;
    finalDomRows: number;
    scrollHeight: number;
    elapsedMs: number;
  }>;
  keyboardPlay: () => Promise<string | null>;
}

declare global {
  interface Window {
    trackListHarness: TrackListHarnessApi;
  }
}

const totalCount = 100_000;
const state: TrackListHarnessState = {
  requestCount: 0,
  generatedRecords: 0,
  totalCount: null,
  playedTrackId: null,
};

function makeTrack(index: number): TrackSummary {
  return {
    id: `synthetic-${index}`,
    title: `Synthetic track ${index}`,
    artist: 'Virtual library test',
    album: `Album ${Math.floor(index / 12)}`,
    albumArtist: null,
    trackNumber: (index % 12) + 1,
    discNumber: 1,
    durationMs: 180_000,
    codec: 'test',
    bitrateBps: null,
    sampleRateHz: null,
  };
}

async function fetchPage(request: TrackPageRequest): Promise<TrackPage> {
  state.requestCount += 1;
  const count = Math.min(request.limit, Math.max(0, totalCount - request.offset));
  const items = Array.from({ length: count }, (_, index) => makeTrack(request.offset + index));
  state.generatedRecords += items.length;
  return { items, offset: request.offset, limit: request.limit, totalCount };
}

mount(TrackList, {
  target: document.querySelector('#track-list-test')!,
  props: {
    query: '',
    resetKey: 'synthetic-100k',
    selectedTrackId: null,
    playbackReady: true,
    isSendingPlaybackCommand: false,
    fetchPage,
    onPlay: (track: TrackSummary) => { state.playedTrackId = track.id; },
    onTotalCount: (count: number | null) => { state.totalCount = count; },
  },
});

window.trackListHarness = {
  state,
  async scrollSweep(stepRows = 12) {
    const startedAt = performance.now();
    const viewport = document.querySelector<HTMLDivElement>('.track-list-viewport');
    if (!viewport) throw new Error('TrackList grid did not mount.');

    const maxScrollTop = viewport.scrollHeight - viewport.clientHeight;
    let maxDomRows = 0;
    let steps = 0;
    const increment = Math.max(1, stepRows) * 57;
    for (let scrollTop = 0; scrollTop <= maxScrollTop; scrollTop += increment) {
      viewport.scrollTop = scrollTop;
      viewport.dispatchEvent(new Event('scroll'));
      await tick();
      maxDomRows = Math.max(maxDomRows, document.querySelectorAll('.virtual-track-row').length);
      steps += 1;
    }
    viewport.scrollTop = maxScrollTop;
    viewport.dispatchEvent(new Event('scroll'));
    await tick();
    await new Promise((resolve) => setTimeout(resolve, 20));

    return {
      steps,
      maxDomRows,
      finalDomRows: document.querySelectorAll('.virtual-track-row').length,
      scrollHeight: viewport.scrollHeight,
      elapsedMs: Number((performance.now() - startedAt).toFixed(2)),
    };
  },
  async keyboardPlay() {
    const viewport = document.querySelector<HTMLDivElement>('.track-list-viewport');
    if (!viewport) throw new Error('TrackList grid did not mount.');
    viewport.focus();
    viewport.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
    await tick();
    viewport.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    await tick();
    return state.playedTrackId;
  },
};
