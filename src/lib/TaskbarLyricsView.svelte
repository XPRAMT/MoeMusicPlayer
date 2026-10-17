<script lang="ts">
  // 工作列歌詞視窗（ADR 0018）：透明、可穿透滑鼠、置頂的獨立視窗，由 Rust 的監看執行緒擺在工作列空白處。
  // 這個頁面自己持有歌詞狀態：輪詢播放快照、用歌詞控制器載入目前曲目的歌詞，只顯示目前這一行。
  import { onDestroy, onMount } from 'svelte';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { fade } from 'svelte/transition';
  import { followLanguageChange } from './language-sync.js';
  import { invokeCommand } from './ipc';
  import type { LyricLine, LyricsPreferences, PlaybackSnapshot, TaskbarLyricsSettings, TrackLyrics } from './ipc';
  import { createLyricsController } from './lyrics-controller.js';
  import { convertTrackLyrics } from './lyrics-traditional.js';
  import { getDisplayActiveLyricIndex, normalizeLyricsPreferences } from './lyrics-preferences.js';
  import { buildTimedLyricTimeline, findActiveLyricIndex } from './lyrics-window.js';
  import { loadLyricsTimingOffsetSec, lyricsTimingDelaySecToOffsetMs } from './lyrics-timing-offset.js';
  import {
    DEFAULT_TASKBAR_LYRICS_SETTINGS,
    fitTaskbarLyricsFontSize,
    getTaskbarLyricsColor,
    normalizeTaskbarLyricsSettings,
  } from './taskbar-lyrics-settings.js';

  const SNAPSHOT_POLL_MS = 200;
  const CLOCK_MS = 50;
  const PREFERENCES_POLL_MS = 3000;
  const MIN_FIT_SCALE = 0.7;

  const lyricsApi = {
    getTrack: (args: { trackId: string }) => invokeCommand('lyrics_get_track', args),
    search: (args: { trackId: string; requestId: string; manual?: boolean; query?: string; queryAlternates?: string[] }) => invokeCommand('lyrics_search', args),
    selectCandidate: (args: { trackId: string; candidateId: string }) => invokeCommand('lyrics_select_candidate', args),
    cancelSearch: (args: { requestId: string }) => invokeCommand('lyrics_cancel_search', args),
    clearTrack: (args: { trackId: string }) => invokeCommand('lyrics_clear_track', args),
    loadMore: (args: { trackId: string }) => invokeCommand('lyrics_load_more', args),
  };

  let settings = $state<TaskbarLyricsSettings>({ ...DEFAULT_TASKBAR_LYRICS_SETTINGS });
  let taskbarIsLight = $state(false);
  let lyricsPreferences = $state<LyricsPreferences>(normalizeLyricsPreferences(null));
  let lyrics = $state<TrackLyrics | null>(null);
  let traditional = $state<{ trackId: string; lines: LyricLine[] } | null>(null);
  let playbackState = $state<PlaybackSnapshot['state']>('stopped');
  let isPlaying = $state(false);
  let trackId = $state<string | null>(null);
  let basePositionMs = 0;
  let baseAt = 0;
  let now = $state(0);
  let viewportHeight = $state(typeof window === 'undefined' ? 0 : window.innerHeight);
  let fontScale = $state(1);
  let boxElement = $state<HTMLElement | null>(null);
  let lineElement = $state<HTMLElement | null>(null);

  const controller = createLyricsController({
    api: lyricsApi,
    onChange: (state: { lyrics: TrackLyrics | null }) => {
      lyrics = state.lyrics;
    },
  });

  let lines = $derived<LyricLine[]>(
    lyricsPreferences.simplifiedToTraditional && traditional && traditional.trackId === lyrics?.trackId
      ? traditional.lines
      : (lyrics?.lines ?? []),
  );
  let timeline = $derived(lyrics?.synced ? buildTimedLyricTimeline(lines) : []);
  let positionMs = $derived.by(() => {
    now;
    return basePositionMs + (isPlaying ? Math.max(0, performance.now() - baseAt) : 0);
  });
  let activeIndex = $derived.by(() => {
    now;
    const offsetMs = (lyrics?.offsetMs ?? 0) + lyricsTimingDelaySecToOffsetMs(loadLyricsTimingOffsetSec(trackId));
    return getDisplayActiveLyricIndex(findActiveLyricIndex(timeline, positionMs, offsetMs), playbackState);
  });
  let activeLine = $derived(activeIndex >= 0 ? timeline[activeIndex]?.line : undefined);
  let text = $derived(activeLine?.text?.trim() ?? '');
  let translation = $derived(settings.showTranslation ? (activeLine?.translation?.trim() ?? '') : '');
  let baseFontPx = $derived(fitTaskbarLyricsFontSize(settings.fontSizePx, viewportHeight, translation !== ''));
  let color = $derived(getTaskbarLyricsColor(settings, taskbarIsLight));
  let alignItems = $derived(settings.align === 'center' ? 'center' : settings.align === 'right' ? 'flex-end' : 'flex-start');

  $effect(() => {
    const current = lyrics;
    if (!current || !lyricsPreferences.simplifiedToTraditional) return;
    let cancelled = false;
    void convertTrackLyrics(current).then((converted: TrackLyrics) => {
      if (!cancelled) traditional = { trackId: current.trackId, lines: converted.lines };
    });
    return () => {
      cancelled = true;
    };
  });

  // A line wider than the window shrinks (down to MIN_FIT_SCALE) before it is cut off with an ellipsis.
  $effect(() => {
    text;
    translation;
    baseFontPx;
    const box = boxElement;
    const line = lineElement;
    if (!box || !line) return;
    requestAnimationFrame(() => {
      const natural = line.scrollWidth / fontScale;
      const available = box.clientWidth;
      fontScale = natural > available && natural > 0 ? Math.max(MIN_FIT_SCALE, available / natural) : 1;
    });
  });

  let snapshotBusy = false;
  async function pollSnapshot(): Promise<void> {
    if (snapshotBusy) return;
    snapshotBusy = true;
    try {
      const snapshot = await invokeCommand('playback_get_snapshot', {});
      basePositionMs = snapshot.positionMs;
      baseAt = performance.now();
      isPlaying = snapshot.isPlaying && snapshot.state === 'playing';
      playbackState = snapshot.state;
      const nextId = snapshot.currentTrack?.id ?? null;
      if (nextId !== trackId) {
        trackId = nextId;
        lyrics = null;
        traditional = null;
        void controller.setTrack(nextId, { title: snapshot.currentTrack?.title, artist: snapshot.currentTrack?.artist });
      }
    } catch {
      // The player is shutting down or not ready yet; the next poll tries again.
    } finally {
      snapshotBusy = false;
    }
  }

  async function pollPreferences(): Promise<void> {
    try {
      lyricsPreferences = normalizeLyricsPreferences(await invokeCommand('settings_get_lyrics_preferences', {}));
    } catch {
      // Keep the previous preferences.
    }
  }

  const timers: ReturnType<typeof setInterval>[] = [];
  const unlisteners: UnlistenFn[] = [];
  const onResize = () => {
    viewportHeight = window.innerHeight;
  };

  onMount(() => {
    document.documentElement.classList.add('taskbar-lyrics-window');
    window.addEventListener('resize', onResize);
    onResize();
    void invokeCommand('settings_get_taskbar_lyrics', {}).then((stored) => {
      settings = normalizeTaskbarLyricsSettings(stored);
    });
    void invokeCommand('taskbar_lyrics_theme', {}).then((light) => {
      taskbarIsLight = light;
    });
    void listen<TaskbarLyricsSettings>('taskbar-lyrics-settings', (event) => {
      settings = normalizeTaskbarLyricsSettings(event.payload);
    }).then((unlisten) => unlisteners.push(unlisten));
    void listen<string | null>('current-track-refreshed', (event) => {
      if (event.payload && event.payload === trackId) void controller.reload();
    }).then((unlisten) => unlisteners.push(unlisten));
    void listen<string>('app-language-changed', (event) => followLanguageChange(event.payload)).then((unlisten) => unlisteners.push(unlisten));
    void listen<boolean>('taskbar-lyrics-theme', (event) => {
      taskbarIsLight = event.payload;
    }).then((unlisten) => unlisteners.push(unlisten));
    void pollPreferences();
    void pollSnapshot();
    timers.push(setInterval(pollSnapshot, SNAPSHOT_POLL_MS));
    timers.push(setInterval(() => (now = performance.now()), CLOCK_MS));
    timers.push(setInterval(pollPreferences, PREFERENCES_POLL_MS));
  });

  onDestroy(() => {
    window.removeEventListener('resize', onResize);
    for (const timer of timers) clearInterval(timer);
    for (const unlisten of unlisteners) unlisten();
    controller.dispose();
  });
</script>

<div
  class="taskbar-lyrics"
  bind:this={boxElement}
  style:align-items={alignItems}
  style:text-align={settings.align}
  style:color
  style:--taskbar-lyrics-size={`${baseFontPx * fontScale}px`}
>
  {#key activeIndex}
    {#if text}
      <div class="taskbar-lyrics-line" bind:this={lineElement} in:fade={{ duration: 120 }}>
        <span class="taskbar-lyrics-main">{text}</span>
        {#if translation}
          <span class="taskbar-lyrics-translation">{translation}</span>
        {/if}
      </div>
    {/if}
  {/key}
</div>

<style>
  :global(html.taskbar-lyrics-window),
  :global(html.taskbar-lyrics-window body),
  :global(html.taskbar-lyrics-window #app) {
    min-width: 0;
    overflow: hidden;
    background: transparent;
    user-select: none;
    cursor: default;
  }

  .taskbar-lyrics {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
    padding: 0 4px;
    overflow: hidden;
    font-size: var(--taskbar-lyrics-size, 16px);
    font-weight: 500;
    line-height: 1.25;
    white-space: nowrap;
    pointer-events: none;
  }

  .taskbar-lyrics-line {
    display: flex;
    flex-direction: column;
    max-width: 100%;
  }

  .taskbar-lyrics-main,
  .taskbar-lyrics-translation {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .taskbar-lyrics-translation {
    font-size: 0.75em;
    opacity: 0.8;
  }
</style>
