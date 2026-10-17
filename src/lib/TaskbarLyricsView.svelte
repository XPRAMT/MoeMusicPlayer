<script lang="ts">
  // 工作列歌詞視窗（ADR 0018）：透明、可穿透滑鼠、置頂的獨立視窗，由 Rust 的監看執行緒擺在工作列空白處。
  // 歌詞不在這裡載入：主程式層載入一次（和播放頁同一份、已套用簡轉繁），整理好後推送過來（`taskbar-lyrics-data`）。
  // 這個頁面只輪詢播放位置，並畫出目前這一行，所以不會和播放頁的歌詞不一致。
  import { onDestroy, onMount } from 'svelte';
  import { emit, listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { fade } from 'svelte/transition';
  import { followLanguageChange } from './language-sync.js';
  import { invokeCommand } from './ipc';
  import type { PlaybackSnapshot, TaskbarLyricsSettings } from './ipc';
  import { getDisplayActiveLyricIndex } from './lyrics-preferences.js';
  import { buildTimedLyricTimeline, findActiveLyricIndex } from './lyrics-window.js';
  import { loadLyricsTimingOffsetSec, lyricsTimingDelaySecToOffsetMs } from './lyrics-timing-offset.js';
  import type { TaskbarLyricsData } from './taskbar-lyrics-data.js';
  import {
    DEFAULT_TASKBAR_LYRICS_SETTINGS,
    fitTaskbarLyricsFontSize,
    getTaskbarLyricsColor,
    normalizeTaskbarLyricsSettings,
  } from './taskbar-lyrics-settings.js';

  // The Now Playing page reads the playback snapshot every 100 ms; so does this window, so a line changes at the same
  // moment in both.
  const SNAPSHOT_POLL_MS = 100;
  const MIN_FIT_SCALE = 0.7;

  let settings = $state<TaskbarLyricsSettings>({ ...DEFAULT_TASKBAR_LYRICS_SETTINGS });
  let taskbarIsLight = $state(false);
  let data = $state<TaskbarLyricsData | null>(null);
  let playbackState = $state<PlaybackSnapshot['state']>('stopped');
  let trackId = $state<string | null>(null);
  let positionMs = $state(0);
  let viewportHeight = $state(typeof window === 'undefined' ? 0 : window.innerHeight);
  let fontScale = $state(1);
  let boxElement = $state<HTMLElement | null>(null);
  let lineElement = $state<HTMLElement | null>(null);

  // Lines of another track (the data arrives a moment after a track change) are never drawn.
  let timeline = $derived(data && data.trackId === trackId ? buildTimedLyricTimeline(data.lines) : []);
  let activeIndex = $derived.by(() => {
    // The per-track delay lives in localStorage, shared with the Now Playing page, and is read live.
    const offsetMs = (data?.offsetMs ?? 0) + lyricsTimingDelaySecToOffsetMs(loadLyricsTimingOffsetSec(trackId));
    return getDisplayActiveLyricIndex(findActiveLyricIndex(timeline, positionMs, offsetMs), playbackState);
  });
  let activeLine = $derived(activeIndex >= 0 ? timeline[activeIndex]?.line : undefined);
  let text = $derived(activeLine?.text?.trim() ?? '');
  let translation = $derived(settings.showTranslation ? (activeLine?.translation?.trim() ?? '') : '');
  let baseFontPx = $derived(fitTaskbarLyricsFontSize(settings.fontSizePx, viewportHeight, translation !== ''));
  let color = $derived(getTaskbarLyricsColor(settings, taskbarIsLight));
  let alignItems = $derived(settings.align === 'center' ? 'center' : settings.align === 'right' ? 'flex-end' : 'flex-start');

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
      positionMs = snapshot.positionMs;
      playbackState = snapshot.state;
      trackId = snapshot.currentTrack?.id ?? null;
    } catch {
      // The player is shutting down or not ready yet; the next poll tries again.
    } finally {
      snapshotBusy = false;
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
    void listen<TaskbarLyricsData>('taskbar-lyrics-data', (event) => {
      data = event.payload;
    }).then((unlisten) => {
      unlisteners.push(unlisten);
      // Ask for the current lyrics now that this window is listening (the shell answers with the same message).
      void emit('taskbar-lyrics-request');
    });
    void listen<string>('app-language-changed', (event) => followLanguageChange(event.payload)).then((unlisten) => unlisteners.push(unlisten));
    void listen<boolean>('taskbar-lyrics-theme', (event) => {
      taskbarIsLight = event.payload;
    }).then((unlisten) => unlisteners.push(unlisten));
    void pollSnapshot();
    timers.push(setInterval(pollSnapshot, SNAPSHOT_POLL_MS));
  });

  onDestroy(() => {
    window.removeEventListener('resize', onResize);
    for (const timer of timers) clearInterval(timer);
    for (const unlisten of unlisteners) unlisten();
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
    /* The font's own content box is 1.33 em (Segoe UI): a shorter line box clips the bottoms of g, y and j. */
    line-height: 1.4;
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
