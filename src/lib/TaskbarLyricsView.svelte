<script lang="ts">
  // 工作列歌詞視窗（ADR 0018）：透明、可穿透滑鼠、置頂的獨立視窗，由 Rust 的監看執行緒擺在工作列空白處。
  // 歌詞不在這裡載入：主程式層載入一次（和播放頁同一份、已套用簡轉繁），整理好後推送過來（`taskbar-lyrics-data`）。
  // 這個頁面只輪詢播放位置，並畫出目前這一行，所以不會和播放頁的歌詞不一致。
  import { onDestroy, onMount, untrack } from 'svelte';
  import { emit, listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { followLanguageChange } from './language-sync.js';
  import { invokeCommand } from './ipc';
  import {
    correctPlaybackClock,
    estimatePlaybackPosition,
    msUntilPlaybackPosition,
    nextCuePosition,
    type PlaybackClock,
    type TaskbarLyricsPlayback,
  } from './playback-clock';
  import type { LyricsLookSettings, PlaybackSnapshot } from './ipc';
  import { FLOATING_LYRICS_LAYOUT, normalizeFloatingLyricsSettings } from './floating-lyrics-settings.js';
  import { getDisplayActiveLyricIndex } from './lyrics-preferences.js';
  import { buildTimedLyricTimeline, findActiveLyricIndex } from './lyrics-window.js';
  import { LYRICS_TIMING_OFFSET_EVENT, loadLyricsTimingOffsetSec, lyricsDisplayOffsetMs } from './lyrics-timing-offset.js';
  import type { TaskbarLyricsData } from './taskbar-lyrics-data.js';
  import {
    DEFAULT_TASKBAR_LYRICS_SETTINGS,
    fitTaskbarLyricsFontSize,
    getLyricScrollProgress,
    getTaskbarLyricsColor,
    getTaskbarLyricsEffectRgb,
    normalizeTaskbarLyricsSettings,
  } from './taskbar-lyrics-settings.js';

  // This window does not poll the player (ADR 0028): the main window forwards every playback sample it takes
  // (`taskbar-lyrics-playback`, every 500 ms and right after a command) and both run the same playback clock, so a
  // line changes at the same moment in both. A timer wakes the view when the next line starts.
  /** How long the last line of a track is assumed to stay when the track length is unknown. */
  const LAST_LINE_FALLBACK_MS = 5000;

  // The floating lyrics window (ADR 0032) draws with this same component: only where its settings come from, the
  // theme (the taskbar's light/dark one does not apply) and the room around the text differ.
  let { variant = 'taskbar' }: { variant?: 'taskbar' | 'floating' } = $props();
  const floating = untrack(() => variant) === 'floating';
  const normalizeSettings = (value: unknown): LyricsLookSettings => (floating ? normalizeFloatingLyricsSettings(value) : normalizeTaskbarLyricsSettings(value));
  const reservedHeight = floating ? FLOATING_LYRICS_LAYOUT.paddingPx * 2 : 0;
  const sidePadding = floating ? FLOATING_LYRICS_LAYOUT.buttonPx + FLOATING_LYRICS_LAYOUT.buttonInsetPx * 2 : 4;

  let settings = $state<LyricsLookSettings>(normalizeSettings(floating ? {} : DEFAULT_TASKBAR_LYRICS_SETTINGS));
  let taskbarIsLight = $state(false);
  let data = $state<TaskbarLyricsData | null>(null);
  let playbackState = $state<PlaybackSnapshot['state']>('stopped');
  let trackId = $state<string | null>(null);
  let clock = $state<PlaybackClock | null>(null);
  let cueTick = $state(0);
  let positionMs = $derived.by(() => {
    cueTick;
    return clock ? estimatePlaybackPosition(clock, performance.now()) : 0;
  });
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
  // The per-track delay lives in localStorage, shared with the Now Playing page. It is loaded when the track changes and
  // updated from LYRICS_TIMING_OFFSET_EVENT (and the storage event) when the user changes it; the offset itself is the
  // same lyricsDisplayOffsetMs the page uses, so the cue timer and the scroll below re-plan at once.
  let timingDelaySec = $state(0);
  $effect(() => {
    timingDelaySec = loadLyricsTimingOffsetSec(trackId);
  });
  let timingOffsetMs = $derived(lyricsDisplayOffsetMs(data?.offsetMs, timingDelaySec));
  let activeIndex = $derived(getDisplayActiveLyricIndex(findActiveLyricIndex(timeline, positionMs, timingOffsetMs), playbackState));
  // A line scrolls over the time it is shown: from its own start to the start of the next line (the end of the track
  // for the last one).
  // The scroll runs as a linear CSS animation from where the line is now to its end position (reached at 90% of the
  // line's time); it restarts when the line changes or the clock is corrected, never per frame.
  let scrollRun = $derived.by(() => {
    if (activeIndex < 0) return { from: 0, remainingMs: null as number | null };
    const start = timeline[activeIndex].startMs;
    const next = timeline[activeIndex + 1]?.startMs ?? (durationMs !== null ? durationMs - timingOffsetMs : start + LAST_LINE_FALLBACK_MS);
    const lineMs = Math.max(1000, next - start);
    const from = getLyricScrollProgress(positionMs + timingOffsetMs - start, lineMs);
    const wait = from < 1 ? msUntilPlaybackPosition(clock, performance.now(), start - timingOffsetMs + lineMs * 0.9) : null;
    return { from, remainingMs: wait !== null && wait > 0 ? wait : null };
  });
  let scrollName = $state('taskbar-lyrics-scroll-a');
  $effect(() => {
    scrollRun;
    scrollName = untrack(() => scrollName) === 'taskbar-lyrics-scroll-a' ? 'taskbar-lyrics-scroll-b' : 'taskbar-lyrics-scroll-a';
  });
  function scrollStyle(overflow: number): string {
    const shift = `--scroll-shift: ${-overflow}px; --scroll-from: ${scrollRun.from};`;
    return scrollRun.remainingMs === null || overflow <= 0
      ? `${shift} transform: translateX(${-overflow * scrollRun.from}px)`
      : `${shift} animation: ${scrollName} ${Math.round(scrollRun.remainingMs)}ms linear forwards`;
  }
  $effect(() => {
    const current = clock;
    const starts = timeline.map((entry) => entry.startMs);
    const offset = timingOffsetMs;
    cueTick;
    if (!current?.playing || starts.length === 0) return;
    const now = performance.now();
    const next = nextCuePosition(starts, offset, estimatePlaybackPosition(current, now));
    const wait = next === null ? null : msUntilPlaybackPosition(current, now, next);
    if (wait === null) return;
    const timer = setTimeout(() => { cueTick += 1; }, wait + 2);
    return () => clearTimeout(timer);
  });
  let activeLine = $derived(activeIndex >= 0 ? timeline[activeIndex]?.line : undefined);
  let text = $derived(activeLine?.text?.trim() ?? '');
  let translation = $derived(settings.showTranslation ? (activeLine?.translation?.trim() ?? '') : '');
  let fontPx = $derived(fitTaskbarLyricsFontSize(settings.fontSizePx, viewportHeight - reservedHeight, translation !== '' ? settings.translationScalePercent : 0));
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

  function applyPlayback(sample: TaskbarLyricsPlayback): void {
    clock = correctPlaybackClock(clock, {
      trackId: sample.trackId,
      positionMs: sample.positionMs,
      playing: sample.playing,
      durationMs: sample.durationMs,
    }, performance.now());
    durationMs = sample.durationMs;
    playbackState = sample.state;
    trackId = sample.trackId;
  }

  const unlisteners: UnlistenFn[] = [];
  const onStorage = () => {
    timingDelaySec = loadLyricsTimingOffsetSec(trackId);
  };
  const onResize = () => {
    viewportHeight = window.innerHeight;
    viewportWidth = window.innerWidth;
  };

  onMount(() => {
    document.documentElement.classList.add('taskbar-lyrics-window');
    window.addEventListener('resize', onResize);
    window.addEventListener('storage', onStorage);
    onResize();
    void listen<{ trackId: string; delaySec: number }>(LYRICS_TIMING_OFFSET_EVENT, (event) => {
      if (event.payload?.trackId === trackId) timingDelaySec = loadLyricsTimingOffsetSec(trackId);
    }).then((unlisten) => unlisteners.push(unlisten));
    void invokeCommand(floating ? 'settings_get_floating_lyrics' : 'settings_get_taskbar_lyrics', {}).then((stored) => {
      settings = normalizeSettings(stored);
    });
    if (!floating) {
      void invokeCommand('taskbar_lyrics_theme', {}).then((light) => {
        taskbarIsLight = light;
      });
    }
    void listen<unknown>(floating ? 'floating-lyrics-settings' : 'taskbar-lyrics-settings', (event) => {
      settings = normalizeSettings(event.payload);
    }).then((unlisten) => unlisteners.push(unlisten));
    void listen<TaskbarLyricsData>('taskbar-lyrics-data', (event) => {
      data = event.payload;
    }).then((unlisten) => {
      unlisteners.push(unlisten);
      // Ask for the current lyrics now that this window is listening (the shell answers with the same message).
      void emit('taskbar-lyrics-request');
    });
    void listen<string>('app-language-changed', (event) => followLanguageChange(event.payload)).then((unlisten) => unlisteners.push(unlisten));
    if (!floating) {
      void listen<boolean>('taskbar-lyrics-theme', (event) => {
        taskbarIsLight = event.payload;
      }).then((unlisten) => unlisteners.push(unlisten));
    }
    void listen<TaskbarLyricsPlayback>('taskbar-lyrics-playback', (event) => applyPlayback(event.payload)).then((unlisten) => {
      unlisteners.push(unlisten);
      // Opened before (or after) the main window: ask it for the current playback now that this window listens.
      void emit('taskbar-lyrics-playback-request');
    });
  });

  onDestroy(() => {
    window.removeEventListener('resize', onResize);
    window.removeEventListener('storage', onStorage);
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
  style:padding={`${reservedHeight / 2}px ${sidePadding}px`}
>
  {#key activeIndex}
    {#if text}
      <div class="taskbar-lyrics-line">
        <span class="taskbar-lyrics-main" bind:this={mainClip}>
          <span class="taskbar-lyrics-track" bind:this={mainTrack} style={scrollStyle(mainOverflow)}>{text}</span>
        </span>
        {#if translation}
          <span class="taskbar-lyrics-translation" bind:this={translationClip}>
            <span class="taskbar-lyrics-track" bind:this={translationTrack} style={scrollStyle(translationOverflow)}>{translation}</span>
          </span>
        {/if}
      </div>
    {/if}
  {/key}
</div>

<style>
  @keyframes -global-taskbar-lyrics-scroll-a {
    from { transform: translateX(calc(var(--scroll-shift) * var(--scroll-from))); }
    to { transform: translateX(var(--scroll-shift)); }
  }
  @keyframes -global-taskbar-lyrics-scroll-b {
    from { transform: translateX(calc(var(--scroll-shift) * var(--scroll-from))); }
    to { transform: translateX(var(--scroll-shift)); }
  }
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

  /* The lines are clipped (for the scrolling); the outline reaches 0.04em past the glyphs, so give the clip room above and
     below for it (a negative margin takes it back, the layout does not move). */
  .taskbar-lyrics[data-text-effect='stroke'] .taskbar-lyrics-main,
  .taskbar-lyrics[data-text-effect='stroke'] .taskbar-lyrics-translation {
    padding-block: 0.06em;
    margin-block: -0.06em;
  }

  .taskbar-lyrics[data-text-effect='stroke'] .taskbar-lyrics-track {
    -webkit-text-stroke: 0.08em rgba(var(--taskbar-lyrics-effect-rgb), 0.5);
    paint-order: stroke fill;
  }
</style>
