import { mount, tick } from 'svelte';
import PlaylistEntryList from '../src/lib/PlaylistEntryList.svelte';
import '../src/app.css';
import type { PlaylistEntrySummary, PlaylistPage } from '../src/lib/ipc';

interface HarnessState {
  requestCount: number;
  generatedRecords: number;
  playedPlaylistId: string | null;
  playedEntryPosition: number | null;
}

interface HarnessApi {
  state: HarnessState;
  scrollSweep: (stepRows?: number) => Promise<{
    steps: number;
    maxDomRows: number;
    finalDomRows: number;
    scrollHeight: number;
    elapsedMs: number;
  }>;
  keyboardPlaySecondDuplicate: () => Promise<{ trackIds: Array<string | null>; playedEntryPosition: number | null }>;
}

declare global {
  interface Window {
    playlistListHarness: HarnessApi;
  }
}

const totalCount = 100_000;
const playlistId = 'synthetic-playlist';
const state: HarnessState = {
  requestCount: 0,
  generatedRecords: 0,
  playedPlaylistId: null,
  playedEntryPosition: null,
};

function makeEntry(position: number): PlaylistEntrySummary {
  if (position === 2) {
    return {
      position,
      trackId: null,
      title: 'Unmatched synthetic entry',
      artist: null,
      album: null,
      durationMs: null,
      hasEnabledMapping: false,
    };
  }
  return {
    position,
    trackId: position < 2 ? 'duplicate-track' : `track-${position}`,
    title: `Synthetic entry ${position}`,
    artist: 'Virtual playlist test',
    album: 'Synthetic album',
    durationMs: 180_000,
    hasEnabledMapping: true,
  };
}

async function fetchPage(request: { playlistId: string; offset: number; limit: number }): Promise<PlaylistPage> {
  state.requestCount += 1;
  const count = Math.min(request.limit, Math.max(0, totalCount - request.offset));
  const items = Array.from({ length: count }, (_, index) => makeEntry(request.offset + index));
  state.generatedRecords += items.length;
  return { items, offset: request.offset, limit: request.limit, totalCount };
}

mount(PlaylistEntryList, {
  target: document.querySelector('#playlist-list-test')!,
  props: {
    playlistId,
    resetKey: 0,
    playbackReady: true,
    isSendingPlaybackCommand: false,
    fetchPage,
    onPlay: (entry: PlaylistEntrySummary, sourcePlaylistId: string) => {
      state.playedPlaylistId = sourcePlaylistId;
      state.playedEntryPosition = entry.position;
    },
  },
});

window.playlistListHarness = {
  state,
  async scrollSweep(stepRows = 1_000) {
    const startedAt = performance.now();
    const viewport = document.querySelector<HTMLDivElement>('.playlist-entry-table');
    if (!viewport) throw new Error('Playlist entry grid did not mount.');

    const maxScrollTop = viewport.scrollHeight - viewport.clientHeight;
    const rowHeight = viewport.querySelector<HTMLElement>('.playlist-entry-row')?.clientHeight ?? 56;
    let maxDomRows = 0;
    let steps = 0;
    const increment = Math.max(1, stepRows) * rowHeight;
    for (let scrollTop = 0; scrollTop <= maxScrollTop; scrollTop += increment) {
      viewport.scrollTop = scrollTop;
      viewport.dispatchEvent(new Event('scroll'));
      await tick();
      await new Promise((resolve) => setTimeout(resolve, 0));
      maxDomRows = Math.max(maxDomRows, document.querySelectorAll('.playlist-entry-row').length);
      steps += 1;
    }
    viewport.scrollTop = maxScrollTop;
    viewport.dispatchEvent(new Event('scroll'));
    await tick();
    await new Promise((resolve) => setTimeout(resolve, 25));

    return {
      steps,
      maxDomRows,
      finalDomRows: document.querySelectorAll('.playlist-entry-row').length,
      scrollHeight: viewport.scrollHeight,
      elapsedMs: Number((performance.now() - startedAt).toFixed(2)),
    };
  },
  async keyboardPlaySecondDuplicate() {
    const viewport = document.querySelector<HTMLDivElement>('.playlist-entry-table');
    if (!viewport) throw new Error('Playlist entry grid did not mount.');
    viewport.scrollTop = 0;
    viewport.dispatchEvent(new Event('scroll'));
    await tick();
    await new Promise((resolve) => setTimeout(resolve, 10));
    const firstRows = [...document.querySelectorAll<HTMLElement>('.playlist-entry-row')]
      .slice(0, 2)
      .map((row) => ({
        position: row.querySelector('.playlist-entry-index')?.textContent?.trim() ?? null,
        title: row.querySelector('.playlist-entry-title strong')?.textContent?.trim() ?? null,
      }));
    viewport.focus();
    viewport.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
    await tick();
    await new Promise((resolve) => setTimeout(resolve, 10));
    viewport.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    await tick();
    return { firstRows, playedPlaylistId: state.playedPlaylistId, playedEntryPosition: state.playedEntryPosition };
  },
};
