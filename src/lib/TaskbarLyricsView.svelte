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
    getLyricScrollProgress,
    getTaskbarLyricsColor,
    getTaskbarLyricsEffectRgb,
    normalizeTaskbarLyricsSettings,
  } from './taskbar-lyrics-settings.js';

  // The Now Playing page reads the playback snapshot every 100 ms; so does this window, so a line changes at the same
  // moment in both.
  const SNAPSHOT_POLL_MS = 100;
  /** How long the last line of a track is assumed to stay when the track length is unknown. */
  const LAST_LINE_FALLBACK_MS = 5000;

  let settings = $state<TaskbarLyricsSettings>({ ...DEFAULT_TASKBAR_LYRICS_SETTINGS });
  let taskbarIsLight = $state(false);
  let data = $state<TaskbarLyricsData | null>(null);
  let playbackState = $state<PlaybackSnapshot['state']>('stopped');
  let trackId = $state<string | null>(null);
  let positionMs = $state(0);
  let durationMs = $state<number | null>(null);
  let viewportHeight = $state(typeof window === 'undefined' ? 0 : window.innerHeight);
  let viewportWidth = $state(typeof window === 'undefined' ? 0 : window.innerWidth);
  // How far past the window each line reaches (px); a line wider than the window scrolls by that much.
  let mainOverflow = $state(0);
  let translationOverflow = $state(0);
  let mainClip = $state<HTMLElement | null>(null);
  let mainTrack = $state<HTMLElement | null>(null);
  let translationClip = $state<HTMLElement | null>(null);
  let translationTrack = $state<HTMLElement | null>(null);

  // Lines of another track (the data arrives a moment after a track change) are never drawn.
  let timeline = $derived(data && data.trackId === trackId ? buildTimedLyricTimeline(data.lines) : []);
  // The per-track delay lives in localStorage, shared with the Now Playing page, and is read live.
  let timingOffsetMs = $derived((data?.offsetMs ?? 0) + lyricsTimingDelaySecToOffsetMs(loadLyricsTimingOffsetSec(trackId)));
  let activeIndex = $derived(getDisplayActiveLyricIndex(findActiveLyricIndex(timeline, positionMs, timingOffsetMs), playbackState));
  // A line scrolls over the time it is shown: from its own start to the start of the next line (the end of the track
  // for the last one).
  let scrollProgress = $derived.by(() => {
    if (activeIndex < 0) return 0;
    const start = timeline[activeIndex].startMs;
    const next = timeline[activeIndex + 1]?.startMs ?? (durationMs !== null ? durationMs - timingOffsetMs : start + LAST_LINE_FALLBACK_MS);
    return getLyricScrollProgress(positionMs + timingOffsetMs - start, Math.max(1000, next - start));
  });
  let activeLine = $derived(activeIndex >= 0 ? timeline[activeIndex]?.line : undefined);
  let text = $derived(activeLine?.text?.trim() ?? '');
  let translation = $derived(settings.showTranslation ? (activeLine?.translation?.trim() ?? '') : '');
  let fontPx = $derived(fitTaskbarLyricsFontSize(settings.fontSizePx, viewportHeight, translation !== '' ? settings.translationScalePercent : 0));
  let color = $derived(getTaskbarLyricsColor(settings, taskbarIsLight));
  let effectRgb = $derived(getTaskbarLyricsEffectRgb(settings, taskbarIsLight));
  let alignItems = $derived(settings.align === 'center' ? 'center' : settings.align === 'right' ? 'flex-end' : 'flex-start');

  // Measure how far each line reaches past the window (after the text, the font size or the window size changed).
  $effect(() => {
    text;
    translation;
    fontPx;
    settings.translationScalePercent;
    viewportWidth;
    const main = [mainClip, mainTrack] as const;
    const second = [translationClip, translationTrack] as const;
    requestAnimationFrame(() => {
      mainOverflow = main[0] && main[1] ? Math.max(0, Math.ceil(main[1].offsetWidth - main[0].clientWidth)) : 0;
      translationOverflow = second[0] && second[1] ? Math.max(0, Math.ceil(second[1].offsetWidth - second[0].clientWidth)) : 0;
    });
  });

  let snapshotBusy = false;
  async function pollSnapshot(): Promise<void> {
    if (snapshotBusy) return;
    snapshotBusy = true;
    try {
      const snapshot = await invokeCommand('playback_get_snapshot', {});
      positionMs = snapshot.positionMs;
      durationMs = snapshot.durationMs;
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
    viewportWidth = window.innerWidth;
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
  style:align-items={alignItems}
  style:text-align={settings.align}
  style:color
  data-text-effect={settings.textEffect}
  style:--taskbar-lyrics-effect-rgb={effectRgb}
  style:--taskbar-lyrics-size={`${fontPx}px`}
  style:--taskbar-lyrics-translation-scale={settings.translationScalePercent / 100}
>
  {#key activeIndex}
    {#if text}
      <div class="taskbar-lyrics-line" in:fade={{ duration: 120 }}>
        <span class="taskbar-lyrics-main" bind:this={mainClip}>
          <span class="taskbar-lyrics-track" bind:this={mainTrack} style:transform={`translateX(${-mainOverflow * scrollProgress}px)`}>{text}</span>
        </span>
        {#if translation}
          <span class="taskbar-lyrics-translation" bind:this={translationClip}>
            <span class="taskbar-lyrics-track" bind:this={translationTrack} style:transform={`translateX(${-translationOverflow * scrollProgress}px)`}>{translation}</span>
          </span>
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

  /* A line wider than the window is clipped here and its track scrolls inside (see scrollProgress). */
  .taskbar-lyrics-main,
  .taskbar-lyrics-translation {
    display: block;
    max-width: 100%;
    overflow: hidden;
  }

  .taskbar-lyrics-track {
    display: inline-block;
    white-space: nowrap;
    /* The page reads the position every 100 ms: glide between readings. */
    transition: transform 100ms linear;
  }

  .taskbar-lyrics-translation {
    font-size: calc(var(--taskbar-lyrics-translation-scale, 0.75) * 1em);
    opacity: 0.8;
  }

  /* The same two looks as the Now Playing lyrics (a tight shadow plus a wider glow, or a thin outline), in the colour
     opposite to the text. The clipped lines get the shadow as a filter: a text-shadow would be cut by the clip. */
  .taskbar-lyrics {
    --taskbar-lyrics-shadow-blur: 0.19em;
    --taskbar-lyrics-glow-blur: 0.5em;
  }

  .taskbar-lyrics[data-text-effect='shadow'] .taskbar-lyrics-main,
  .taskbar-lyrics[data-text-effect='shadow'] .taskbar-lyrics-translation {
    filter:
      drop-shadow(0 0 var(--taskbar-lyrics-shadow-blur) rgba(var(--taskbar-lyrics-effect-rgb), 0.6))
      drop-shadow(0 0 var(--taskbar-lyrics-glow-blur) rgba(var(--taskbar-lyrics-effect-rgb), 0.38));
  }

  .taskbar-lyrics[data-text-effect='stroke'] .taskbar-lyrics-track {
    -webkit-text-stroke: 0.1em rgba(var(--taskbar-lyrics-effect-rgb), 0.88);
    paint-order: stroke fill;
  }
</style>
