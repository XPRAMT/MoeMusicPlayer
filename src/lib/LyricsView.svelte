<script lang="ts">
  import { tr } from './i18n.js';
  import { onDestroy, tick, untrack } from 'svelte';
  import { emit, listen } from '@tauri-apps/api/event';
  import { defaultLyricsApi, type LyricsApi } from './lyrics-api';
  import type {
    LyricLine,
    LyricsCandidate,
    LyricsPreferences,
    LyricsTrackResult,
    PlaybackState,
    TrackLyrics,
  } from './ipc';
  import { createLyricsController } from './lyrics-controller.js';
  import { lyricsForDisplay } from './lyrics-traditional.js';
  import { wordHighlightTimings } from './lyrics-word-highlight.js';
  import { estimatePlaybackPosition, msUntilPlaybackPosition, type PlaybackClock } from './playback-clock';
  import {
    LYRIC_CUE_LEAD_MS,
    LYRIC_OFFSET_GLIDE_MS,
    cueAnimationMs,
    displayCuePositions,
    findDisplayCueIndex,
    nextDisplayCuePosition,
  } from './lyrics-cue-timing';
  import {
    glidePosition,
    planTimedScroll,
    restingGlide,
    retargetGlide,
    snapToDevicePixel,
    timedNavigationKind,
    type ScrollGlide,
  } from './lyrics-scroll-glide';
  import { effectiveCandidateFilter, filterLyricsCandidates, getCandidatePresentation } from './lyrics-candidate-preview.js';
  import {
    DEFAULT_LYRICS_PREFERENCES,
    getDisplayActiveLyricIndex,
    getLyricLineOpacity,
    lyricsAuxiliaryFontSizePx,
    normalizeLyricsPreferences,
  } from './lyrics-preferences.js';
  import {
    buildLyricsLayout,
    buildTimedLyricTimeline,
    computeLyricsInkLineHeight,
    LYRICS_AUXILIARY_LINE_HEIGHT,
    LYRICS_INK_ASCENT_SAMPLE,
    LYRICS_INK_DESCENT_SAMPLE,
    LYRICS_PRIMARY_LINE_HEIGHT,
    findLyricsRowAtOffset,
    findActiveLyricIndex,
    getPlainLyricWindow,
    getTimedLyricWindow,
    sameLyricWindow,
    getLyricsOffset,
    updateLyricsRowHeight,
  } from './lyrics-window.js';
  import {
    LYRICS_TIMING_OFFSET_MAX_SEC,
    LYRICS_TIMING_OFFSET_MIN_SEC,
    LYRICS_TIMING_OFFSET_STEP_SEC,
    clampLyricsTimingOffsetSec,
    formatLyricsTimingOffsetSec,
    loadLyricsTimingOffsetSec,
    LYRICS_TIMING_OFFSET_EVENT,
    lyricsDisplayOffsetMs,
    saveLyricsTimingOffsetSec,
  } from './lyrics-timing-offset.js';

  type LyricsViewState = {
    trackId: string | null;
    phase: 'idle' | 'loading' | 'ready' | 'empty' | 'candidates' | 'searching' | 'error';
    lyrics: TrackLyrics | null;
    lyricsRemoved: boolean;
    candidates: LyricsCandidate[];
    error: string | null;
    isSearching: boolean;
    selectingCandidateId: string | null;
    selectionMode: boolean;
    failedStage: 'load' | 'search' | 'selection' | null;
    pickerClosed: boolean;
    hasMore: boolean;
    isLoadingMore: boolean;
  };

  export type LyricsTopbarStatus = {
    source: string;
    sync: string;
    canAdjustTiming: boolean;
    timingPanelOpen: boolean;
    openManualSelection: () => void;
    toggleTimingOffset: () => void;
  };

  interface Props {
    trackId: string | null;
    positionMs: number;
    /** The shell's playback clock. When given, the active line follows it: the view wakes once when the next line
     *  starts (and again when the clock is corrected), instead of recomputing on every position sample. */
    playbackClock?: PlaybackClock | null;
    isPlaying: boolean;
    playbackState?: PlaybackState;
    lyricsPreferences?: LyricsPreferences;
    onPreferencesChange?: (patch: Partial<LyricsPreferences>) => void;
    onLyricsRemovedChange?: (removed: boolean) => void;
    onStatusChange?: (status: LyricsTopbarStatus | null) => void;
    api?: LyricsApi;
    /** The app shell's lyrics controller and its state. When given, this view only shows them (the shell loads the
     *  lyrics once for every consumer, for example the taskbar lyrics); alone it creates its own. */
    sharedController?: ReturnType<typeof createLyricsController>;
    sharedState?: LyricsViewState;
  }

  let {
    trackId,
    positionMs = 0,
    playbackClock = null,
    isPlaying = false,
    playbackState,
    lyricsPreferences = DEFAULT_LYRICS_PREFERENCES,
    onPreferencesChange = () => {},
    onLyricsRemovedChange,
    onStatusChange,
    api = defaultLyricsApi,
    sharedController,
    sharedState,
  }: Props = $props();

  let ownState = $state<LyricsViewState>({
    trackId: null,
    phase: 'idle',
    lyrics: null,
    lyricsRemoved: false,
    candidates: [],
    error: null,
    isSearching: false,
    selectingCandidateId: null,
    selectionMode: false,
    failedStage: null,
    pickerClosed: false,
    hasMore: false,
    isLoadingMore: false,
  });
  let viewState = $derived(sharedState ?? ownState);
  let timedViewport = $state<HTMLDivElement | null>(null);
  let plainViewport = $state<HTMLDivElement | null>(null);
  let plainScrollTop = $state(0);
  let plainViewportHeight = $state(480);
  let timedViewportHeight = $state(480);
  let layoutWidth = $state(0);
  let lyricsRoot = $state<HTMLElement | null>(null);
  // Line-height = font-specific ink fit (ascenders + descenders, no extra leading); the row padding adds
  // the 3px first-shadow extent. Measured once per font so 2-line clamping never cuts descenders.
  let inkLineHeights = $state({ primary: LYRICS_PRIMARY_LINE_HEIGHT, auxiliary: LYRICS_AUXILIARY_LINE_HEIGHT });
  function measureInkLineHeight(context: CanvasRenderingContext2D, font: string, fallback: number): number {
    context.font = font;
    const ascent = context.measureText(LYRICS_INK_ASCENT_SAMPLE);
    const descent = context.measureText(LYRICS_INK_DESCENT_SAMPLE);
    return computeLyricsInkLineHeight({
      fontAscent: ascent.fontBoundingBoxAscent / 100,
      fontDescent: ascent.fontBoundingBoxDescent / 100,
      inkAscent: Math.max(ascent.actualBoundingBoxAscent, descent.actualBoundingBoxAscent) / 100,
      inkDescent: Math.max(ascent.actualBoundingBoxDescent, descent.actualBoundingBoxDescent) / 100,
    }, fallback);
  }
  function measureInkLineHeights(): void {
    const root = lyricsRoot;
    const context = root ? document.createElement('canvas').getContext('2d') : null;
    if (!root || !context) return;
    const family = getComputedStyle(root).fontFamily;
    const primary = measureInkLineHeight(context, `550 100px ${family}`, LYRICS_PRIMARY_LINE_HEIGHT);
    const auxiliary = measureInkLineHeight(context, `400 100px ${family}`, LYRICS_AUXILIARY_LINE_HEIGHT);
    if (primary !== inkLineHeights.primary || auxiliary !== inkLineHeights.auxiliary) inkLineHeights = { primary, auxiliary };
  }
  $effect(() => {
    if (!lyricsRoot) return;
    measureInkLineHeights();
    void document.fonts?.ready.then(() => measureInkLineHeights());
  });
  let layoutRevision = $state(0);
  let previousTrackId: string | null | undefined;
  let previousPlainLayout: ReturnType<typeof buildLyricsLayout> | null = null;
  let previousTimedNavigation: {
    trackId: string | null;
    generation: number;
    index: number;
    viewport: HTMLDivElement;
    height: number;
    jumpId: number | null;
    clockPlaying: boolean | null;
    offsetMs: number;
  } | null = null;
  // Cue scroll tween (ADR 0029): fixed length (the cue animation), ease-out, requestAnimationFrame only while it runs.
  // It replaces the browser's smooth scrollTo, whose length the browser picks, so the scroll can end on the line start.
  // Both the cue advance and the delay-change glide are one retargetable Hermite glide (lyrics-scroll-glide.ts), so a new
  // target taken mid-flight keeps the position and speed continuous. Positions are written in whole device pixels.
  let cueScrollFrame = 0;
  let cueGlide: ScrollGlide | null = null;
  function stopCueScroll(): void {
    if (cueScrollFrame) cancelAnimationFrame(cueScrollFrame);
    cueScrollFrame = 0;
    cueGlide = null;
    timedSmoothScrolling = false;
  }
  function runCueGlide(element: HTMLDivElement, glide: ScrollGlide): void {
    cueGlide = glide;
    timedSmoothScrolling = true;
    // Rows the glide brings in are rendered before it moves (union of the view and the glide's range).
    refreshTimedWindow();
    if (cueScrollFrame) return; // the running frame loop picks up the new glide
    const step = (now: number) => {
      const current = cueGlide;
      if (!current || timedUserScrolling) { stopCueScroll(); return; }
      const top = snapToDevicePixel(glidePosition(current, now), window.devicePixelRatio);
      if (Math.abs(element.scrollTop - top) >= 0.5 / (window.devicePixelRatio || 1)) element.scrollTop = top;
      if (now < current.startedAt + current.durationMs) { cueScrollFrame = requestAnimationFrame(step); return; }
      cueScrollFrame = 0;
      cueGlide = null;
      timedSmoothScrolling = false;
      correctTimedCenter();
    };
    cueScrollFrame = requestAnimationFrame(step);
  }
  function startCueScroll(element: HTMLDivElement, targetTop: number, durationMs: number): void {
    runCueGlide(element, restingGlide(element.scrollTop, targetTop, performance.now(), durationMs));
  }
  function glideCueScroll(element: HTMLDivElement, targetTop: number): void {
    runCueGlide(element, retargetGlide(cueGlide, element.scrollTop, targetTop, performance.now(), LYRIC_OFFSET_GLIDE_MS));
  }
  let timedSmoothScrolling = false;
  let timedUserScrolling = false;
  let userScrollResetTimer: ReturnType<typeof setTimeout> | null = null;
  let timingOffsetSec = $state(0);
  let timingPanelOpen = $state(false);
  let timingPanelElement = $state<HTMLDivElement | null>(null);
  let candidateQuery = $state('');
  /** The text of the search whose results are on screen (typed text equal to it is not filtered again). */
  let submittedQuery = $state('');

  // svelte-ignore state_referenced_locally
  const controller = sharedController ?? createLyricsController({
    api: {
      getTrack: (args) => api.getTrack(args),
      search: (args) => api.search(args),
      selectCandidate: (args) => api.selectCandidate(args),
      cancelSearch: (args) => api.cancelSearch(args),
      clearTrack: (args) => api.clearTrack(args),
      loadMore: (args) => api.loadMore?.(args) ?? Promise.resolve({ lyrics: null, candidates: [], status: 'empty', error: null, hasMore: false }),
    },
    onChange(nextState: LyricsViewState) {
      ownState = nextState;
    },
  });

  let preferences = $derived(normalizeLyricsPreferences(lyricsPreferences));

  $effect(() => {
    if (viewState.phase === 'loading' || viewState.phase === 'idle') return;
    onLyricsRemovedChange?.(viewState.lyricsRemoved && !selectionUiOpen);
  });

  // The Traditional spelling arrives with the lines (ADR 0030), so it can never belong to another track.
  let lines = $derived<LyricLine[]>(
    viewState.lyrics ? lyricsForDisplay(viewState.lyrics, preferences.simplifiedToTraditional).lines : [],
  );
  let effectivePlaybackState = $derived(playbackState ?? (isPlaying ? 'playing' : 'paused'));
  let timedLines = $derived(
    viewState.lyrics?.synced ? buildTimedLyricTimeline(lines) : [],
  );
  let timedOffsetMs = $derived(lyricsDisplayOffsetMs(viewState.lyrics?.offsetMs, timingOffsetSec));
  // Bumped by the cue timer below; the clock estimate is read again only then (or when the clock itself changes).
  let cueTick = $state(0);
  // The offset is read too: right after a delay step the index must come from a fresh position, not the estimate of
  // the last cue tick (a stale position picked a wrong line that the next timer then flipped back).
  let cuePositionMs = $derived.by(() => {
    cueTick;
    timedOffsetMs;
    return playbackClock ? estimatePlaybackPosition(playbackClock, performance.now()) : positionMs;
  });
  let timedStartTimes = $derived(timedLines.map((entry) => entry.startMs));
  // While playing, a line is shown LYRIC_CUE_LEAD_MS early (shorter when the gap is) so its change animation ends on
  // the line start (ADR 0029). Paused, a line is shown at its own start.
  let cueLeading = $derived(isPlaying && effectivePlaybackState === 'playing');
  let timedDisplayPositions = $derived(displayCuePositions(timedStartTimes, timedOffsetMs, cueLeading));
  $effect(() => {
    const clock = playbackClock;
    const shown = timedDisplayPositions;
    cueTick;
    if (!clock || !clock.playing || shown.length === 0) return;
    const now = performance.now();
    const next = nextDisplayCuePosition(shown, estimatePlaybackPosition(clock, now));
    const wait = next === null ? null : msUntilPlaybackPosition(clock, now, next);
    if (wait === null) return;
    const timer = setTimeout(() => { cueTick += 1; }, wait + 2);
    return () => clearTimeout(timer);
  });
  /** The line that is really playing (by its own start time); play counts and the like never lead. */
  let activeTimedIndex = $derived(findActiveLyricIndex(timedLines, cuePositionMs, timedOffsetMs));
  /** The line drawn as current: it leads activeTimedIndex by the cue animation. */
  let displayTimedIndex = $derived(findDisplayCueIndex(timedDisplayPositions, cuePositionMs));
  /** How long the change to the line now shown animates (the time left until it really starts). */
  let cueAnimationDurationMs = $derived(cueAnimationMs(timedStartTimes, timedOffsetMs, displayTimedIndex, cuePositionMs));
  let displayActiveTimedIndex = $derived(
    getDisplayActiveLyricIndex(displayTimedIndex, effectivePlaybackState),
  );
  let layoutPreferences = $derived({ ...preferences, primaryLineHeight: inkLineHeights.primary, auxiliaryLineHeight: inkLineHeights.auxiliary });
  let timedLayout = $derived(buildLyricsLayout(timedLines, layoutPreferences, preferences.lineGapPx, layoutWidth));
  let visibleActiveTimedIndex = $derived(
    displayActiveTimedIndex >= 0 && timedLayout.heights[displayActiveTimedIndex] > 0
      ? displayActiveTimedIndex
      : -1,
  );
  let plainLayout = $derived(buildLyricsLayout(lines, layoutPreferences, preferences.lineGapPx, layoutWidth));
  // Word highlight (ADR 0041): only the current line with word timing is split into words, each with one CSS
  // animation (delay = time until the word starts). Recomputed only on a line change, seek, play/pause or offset
  // change; the clock's regular samples do not touch it, so no style is written while a line plays.
  let wordsPlaying = $derived(effectivePlaybackState === 'playing' && (playbackClock ? playbackClock.playing : isPlaying));
  let currentLineWords = $derived.by(() => {
    const index = visibleActiveTimedIndex;
    const line = index >= 0 ? timedLines[index]?.line : undefined;
    const words = line?.words ?? [];
    // Romanization rows get word effects only with their own word timing; translation rows never do.
    const romanizationWords = line?.romanizationWords ?? [];
    if (words.length === 0 && romanizationWords.length === 0) return null;
    const jump = playbackClock?.jumpId ?? 0;
    const playing = wordsPlaying;
    const offset = timedOffsetMs;
    const position = untrack(() =>
      playbackClock ? estimatePlaybackPosition(playbackClock, performance.now()) : positionMs,
    );
    return {
      index,
      key: `${index}:${jump}:${playing}:${offset}:${timedLayout.generation}`,
      words: words.length > 0 ? wordHighlightTimings(words, null, position, offset) : null,
      romanizationWords: romanizationWords.length > 0 ? wordHighlightTimings(romanizationWords, null, position, offset) : null,
    };
  });
  let timedTotalHeight = $derived.by(() => { layoutRevision; return timedLayout.totalHeight; });
  let timedCanvasHeight = $derived(timedTotalHeight + timedViewportHeight);
  let timedEdgePadding = $derived(timedViewportHeight / 2);
  let plainTotalHeight = $derived.by(() => { layoutRevision; return plainLayout.totalHeight; });
  // Rendered by scroll position (plus overscan and a running glide's range), not by the active line, so hand
  // scrolling never shows blank spacers. Reassigned only when the rows or spacer heights actually change.
  let timedWindow = $state<ReturnType<typeof getTimedLyricWindow>>({ start: 0, end: 0, beforeHeight: 0, afterHeight: 0, rows: [] });
  function refreshTimedWindow(extraScrollTops: number[] = []): void {
    const element = timedViewport;
    const glide = cueGlide;
    const tops = glide ? [glide.from, glide.to, ...extraScrollTops] : extraScrollTops;
    const next = getTimedLyricWindow(
      timedLayout,
      element ? element.scrollTop : 0,
      element ? element.clientHeight || timedViewportHeight : timedViewportHeight,
      timedEdgePadding,
      tops,
    );
    const current = untrack(() => timedWindow);
    if (!sameLyricWindow(current, next) || current.rows.length !== next.rows.length) timedWindow = next;
  }
  $effect(() => {
    layoutRevision;
    timedLayout;
    timedViewportHeight;
    timedEdgePadding;
    timedViewport;
    untrack(() => refreshTimedWindow());
  });
  let timedRenderedRows = $derived(timedWindow.rows.filter((index) => timedLayout.heights[index] > 0));
  let plainWindow = $derived.by(() => {
    layoutRevision;
    return getPlainLyricWindow(plainLayout, plainScrollTop, plainViewportHeight);
  });
  let plainRenderedRows = $derived(plainWindow.rows.filter((index) => plainLayout.heights[index] > 0));
  let isTimed = $derived(timedLines.length > 0);
  let selectionUiOpen = $derived(
    viewState.selectionMode
      || viewState.isSearching
      || viewState.phase === 'candidates'
      || viewState.phase === 'searching',
  );
  let filteredCandidates = $derived(
    filterLyricsCandidates(viewState.candidates, effectiveCandidateFilter(candidateQuery, submittedQuery)),
  );
  let showCandidatePicker = $derived(
    !viewState.pickerClosed
      && !viewState.isSearching
      && (
        viewState.candidates.length > 0
        || viewState.selectionMode
        // Empty after a failed/empty search may open the picker; cleared tracks keep
        // pickerClosed=true so they stay on the empty playback UI until 「手動指定」.
        || (viewState.phase === 'empty' && !viewState.error && !viewState.lyrics)
      ),
  );

  function timedCueScrollTop(layout: ReturnType<typeof buildLyricsLayout>, index: number, viewportHeight: number, scrollportHeight = viewportHeight): number {
    if (index < 0) return 0;
    const visibleIndex = layout.heights[index] <= 0
      ? findLyricsRowAtOffset(layout, getLyricsOffset(layout, index))
      : index;
    if (visibleIndex < 0) return 0;
    return Math.max(0, viewportHeight / 2 + getLyricsOffset(layout, visibleIndex) + layout.heights[visibleIndex] / 2 - scrollportHeight / 2);
  }

  function correctTimedCenter(): void {
    const element = timedViewport;
    const activeIndex = displayActiveTimedIndex;
    if (!element || activeIndex < 0 || timedSmoothScrolling || timedUserScrolling) return;
    const targetTop = timedCueScrollTop(timedLayout, activeIndex, timedViewportHeight, element.clientHeight);
    if (Math.abs(element.scrollTop - targetTop) > 1) element.scrollTo({ top: targetTop, behavior: 'instant' });
  }

  /** Whether correctTimedCenter would recenter now; otherwise a measured row keeps the top row in place instead. */
  function timedCenterFollows(): boolean {
    return displayActiveTimedIndex >= 0 && !timedSmoothScrolling && !timedUserScrolling;
  }
  /** Row under the top edge and the distance into it, so re-measured rows above it do not shift the view. */
  function captureScrollAnchor(layout: ReturnType<typeof buildLyricsLayout>, viewport: HTMLDivElement, timed: boolean) {
    if (timed && timedCenterFollows()) return null;
    const pad = timed ? timedEdgePadding : 0;
    const top = viewport.scrollTop - pad;
    if (timed && top < 0) return null;
    const index = findLyricsRowAtOffset(layout, top);
    return index < 0 ? null : { index, offset: top - getLyricsOffset(layout, index), pad };
  }
  function restoreScrollAnchor(layout: ReturnType<typeof buildLyricsLayout>, viewport: HTMLDivElement, anchor: { index: number; offset: number; pad: number } | null): void {
    if (!anchor) return;
    viewport.scrollTop = anchor.pad + getLyricsOffset(layout, anchor.index) + anchor.offset;
  }

  const rowObserver = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver((entries) => {
    const layout = isTimed ? timedLayout : plainLayout;
    const viewport = isTimed ? timedViewport : plainViewport;
    if (!viewport) return;
    const anchor = captureScrollAnchor(layout, viewport, isTimed);
    let changed = false;
    for (const entry of entries) {
      const row = entry.target as HTMLElement;
      if (Number(row.dataset.layoutGeneration) !== layout.generation) continue;
      const index = Number(row.dataset.layoutIndex);
      if (updateLyricsRowHeight(layout, index, row.getBoundingClientRect().height)) changed = true;
    }
    if (!changed) return;
    layoutRevision += 1;
    if (isTimed) correctTimedCenter();
    restoreScrollAnchor(layout, viewport, anchor);
    if (!isTimed && anchor) plainScrollTop = viewport.scrollTop;
  });

  function observeLyricRow(node: HTMLElement, index: number) {
    const update = (nextIndex: number) => {
      node.dataset.layoutIndex = String(nextIndex);
      node.dataset.layoutGeneration = String((isTimed ? timedLayout : plainLayout).generation);
      rowObserver?.observe(node, { box: 'border-box' });
    };
    update(index);
    return {
      update,
      destroy() { rowObserver?.unobserve(node); },
    };
  }

  function measureVisibleRows(generation: number) {
    void tick().then(() => {
      const layout = isTimed ? timedLayout : plainLayout;
      const viewport = isTimed ? timedViewport : plainViewport;
      if (!viewport || generation !== layout.generation) return;
      const anchor = captureScrollAnchor(layout, viewport, isTimed);
      let changed = false;
      for (const row of viewport.querySelectorAll<HTMLElement>(`.lyric-line[data-layout-generation="${generation}"]`)) {
        changed = updateLyricsRowHeight(layout, Number(row.dataset.layoutIndex), row.getBoundingClientRect().height) || changed;
      }
      if (!changed) return;
      layoutRevision += 1;
      if (isTimed) correctTimedCenter();
      restoreScrollAnchor(layout, viewport, anchor);
      if (!isTimed && anchor) plainScrollTop = viewport.scrollTop;
    });
  }

  $effect(() => {
    const generation = isTimed ? timedLayout.generation : plainLayout.generation;
    measureVisibleRows(generation);
  });

  $effect(() => {
    const viewport = isTimed ? timedViewport : plainViewport;
    if (!viewport) return;
    const observer = new ResizeObserver(() => {
      const nextWidth = Math.round(viewport.clientWidth);
      if (nextWidth !== layoutWidth) layoutWidth = nextWidth;
      if (isTimed) timedViewportHeight = viewport.clientHeight;
      else plainViewportHeight = viewport.clientHeight;
    });
    observer.observe(viewport);
    return () => observer.disconnect();
  });

  $effect(() => {
    layoutRevision;
    timedLayout.generation;
    plainLayout.generation;
    timingPanelOpen;
    timingOffsetSec;
    const source = viewState.lyrics ? sourceLabel(viewState.lyrics.source) : tr('尚未載入');
    const sync = viewState.lyrics ? (isTimed ? tr('同步歌詞') : tr('純歌詞')) : tr('沒有歌詞');
    onStatusChange?.({
      source,
      sync,
      canAdjustTiming: isTimed,
      timingPanelOpen,
      openManualSelection: () => {
        if (!trackId) return;
        void controller.searchAgain();
      },
      toggleTimingOffset: () => {
        if (!isTimed) return;
        timingPanelOpen = !timingPanelOpen;
      },
    });
  });

  $effect(() => {
    const element = plainViewport;
    const nextLayout = plainLayout;
    if (!element) {
      previousPlainLayout = nextLayout;
      return;
    }

    if (previousPlainLayout && previousPlainLayout !== nextLayout && previousPlainLayout.heights.length > 0) {
      const firstVisibleIndex = findLyricsRowAtOffset(previousPlainLayout, element.scrollTop);
      if (firstVisibleIndex >= 0) {
        const intraRowOffset = element.scrollTop - getLyricsOffset(previousPlainLayout, firstVisibleIndex);
        element.scrollTop = getLyricsOffset(nextLayout, Math.min(firstVisibleIndex, nextLayout.heights.length)) + intraRowOffset;
      }
      plainScrollTop = element.scrollTop;
    }
    previousPlainLayout = nextLayout;
  });

  $effect(() => {
    const nextTrackId = trackId;
    if (nextTrackId === previousTrackId) return;
    previousTrackId = nextTrackId;
    plainScrollTop = 0;
    if (plainViewport) plainViewport.scrollTop = 0;
    if (timedViewport) timedViewport.scrollTop = 0;
    timingPanelOpen = false;
    candidateQuery = '';
    submittedQuery = '';
    timingOffsetSec = loadLyricsTimingOffsetSec(nextTrackId);
    if (!sharedController) void controller.setTrack(nextTrackId);
  });

  $effect(() => {
    const element = plainViewport;
    if (!element) return;

    const updateViewport = () => {
      plainScrollTop = element.scrollTop;
      plainViewportHeight = element.clientHeight;
    };
    element.addEventListener('scroll', updateViewport, { passive: true });
    const observer = typeof ResizeObserver === 'undefined'
      ? null
      : new ResizeObserver(updateViewport);
    observer?.observe(element);
    updateViewport();

    return () => {
      element.removeEventListener('scroll', updateViewport);
      observer?.disconnect();
    };
  });

  $effect(() => {
    const element = timedViewport;
    const activeIndex = displayTimedIndex;
    const animationMs = untrack(() => cueAnimationDurationMs);
    const jumpId = playbackClock?.jumpId ?? null;
    const clockPlaying = playbackClock?.playing ?? null;
    const layout = timedLayout;
    const currentTrackId = trackId;
    const currentlyPlaying = isPlaying && effectivePlaybackState === 'playing';
    const viewportHeight = timedViewportHeight;
    const offsetMs = timedOffsetMs;
    if (!element) return;

    const previous = previousTimedNavigation;
    const sameTrackAndLayout = previous?.trackId === currentTrackId && previous.generation === layout.generation;
    if (
      sameTrackAndLayout
      && previous?.index === activeIndex
      && previous.viewport === element
      && previous.height === viewportHeight
    ) {
      // Same line: remember the clock state so a later normal advance is not mistaken for a seek.
      previous.jumpId = jumpId;
      previous.clockPlaying = clockPlaying;
      previous.offsetMs = offsetMs;
      return;
    }
    // A seek while playing (the clock jumped without a play/pause change) lands at once, like a track change.
    const seeked = previous !== null && previous.jumpId !== jumpId && previous.clockPlaying === clockPlaying;
    const kind = timedNavigationKind({
      sameTrackAndLayout: Boolean(sameTrackAndLayout && previous?.viewport === element && previous.height === viewportHeight),
      seeked,
      offsetChanged: previous !== null && previous.offsetMs !== offsetMs,
      playing: currentlyPlaying,
      previousIndex: previous?.index ?? -1,
      index: activeIndex,
      glideRunning: cueGlide !== null,
    });
    previousTimedNavigation = { trackId: currentTrackId, generation: layout.generation, index: activeIndex, viewport: element, height: viewportHeight, jumpId, clockPlaying, offsetMs };

    const frame = requestAnimationFrame(() => {
      const reducedMotion = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
      const targetTop = timedCueScrollTop(layout, activeIndex, viewportHeight, element.clientHeight);
      const plan = planTimedScroll({ kind, glideRunning: cueGlide !== null, animationMs, reducedMotion, distancePx: targetTop - element.scrollTop });
      if (plan === 'retarget') glideCueScroll(element, targetTop);
      else if (plan === 'rest') startCueScroll(element, targetTop, animationMs);
      else if (plan === 'instant') {
        stopCueScroll();
        element.scrollTo({ top: targetTop, behavior: 'instant' });
        refreshTimedWindow();
      }
    });
    return () => cancelAnimationFrame(frame);
  });

  $effect(() => {
    const element = timedViewport;
    if (!element) return;
    const onScrollEnd = () => {
      if (cueScrollFrame) return; // the cue tween's own frames; it corrects the center when it ends
      const shouldCorrectCenter = timedSmoothScrolling && !timedUserScrolling;
      timedSmoothScrolling = false;
      timedUserScrolling = false;
      if (userScrollResetTimer) clearTimeout(userScrollResetTimer);
      userScrollResetTimer = null;
      if (shouldCorrectCenter) correctTimedCenter();
    };
    const onUserScroll = () => {
      stopCueScroll();
      timedUserScrolling = true;
      if (userScrollResetTimer) clearTimeout(userScrollResetTimer);
      userScrollResetTimer = setTimeout(() => { timedUserScrolling = false; userScrollResetTimer = null; }, 300);
    };
    const onScroll = () => refreshTimedWindow();
    element.addEventListener('scroll', onScroll, { passive: true });
    element.addEventListener('scrollend', onScrollEnd);
    element.addEventListener('wheel', onUserScroll, { passive: true });
    element.addEventListener('pointerdown', onUserScroll, { passive: true });
    element.addEventListener('touchstart', onUserScroll, { passive: true });
    return () => {
      element.removeEventListener('scroll', onScroll);
      element.removeEventListener('scrollend', onScrollEnd);
      element.removeEventListener('wheel', onUserScroll);
      element.removeEventListener('pointerdown', onUserScroll);
      element.removeEventListener('touchstart', onUserScroll);
      if (userScrollResetTimer) clearTimeout(userScrollResetTimer);
      userScrollResetTimer = null;
    };
  });

  // The per-track delay was changed elsewhere (the song details page resets it after saving imported lyrics
  // that already have it baked in): read it again for the shown track.
  $effect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void listen<{ trackId: string; delaySec: number }>(LYRICS_TIMING_OFFSET_EVENT, (event) => {
      const shown = untrack(() => trackId);
      if (shown && event.payload?.trackId === shown) timingOffsetSec = loadLyricsTimingOffsetSec(shown);
    })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  // Tags were saved for the playing track: read its (possibly embedded) lyrics again.
  $effect(() => {
    // With the shell's controller the shell reloads it.
    if (sharedController) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void listen<string | null>('current-track-refreshed', (event) => {
      if (event.payload && event.payload === viewState.trackId) void controller.reload();
    })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  onDestroy(() => { stopCueScroll(); onStatusChange?.(null); if (!sharedController) controller.dispose(); rowObserver?.disconnect(); });

  function sourceLabel(source: TrackLyrics['source']): string {
    switch (source) {
      case 'local': return tr('本機 LRC');
      case 'embedded': return tr('內嵌歌詞');
      case 'netease': return tr('網易雲');
      case 'qqmusic': return tr('QQ 音樂');
      case 'manual': return tr('手動指定');
    }
  }

  function providerLabel(provider: LyricsCandidate['provider']): string {
    if (provider === 'qqmusic') return tr('QQ 音樂');
    if (provider === 'local') return tr('本機 LRC');
    if (provider === 'embedded') return tr('內嵌歌詞');
    return tr('網易雲');
  }


  function formatCandidateDuration(durationMs: number | null): string {
    if (durationMs === null || !Number.isFinite(durationMs) || durationMs < 0) return tr('長度未知');
    const totalSeconds = Math.floor(durationMs / 1000);
    return `${Math.floor(totalSeconds / 60)}:${String(totalSeconds % 60).padStart(2, '0')}`;
  }

  function retry(): void {
    void controller.retry();
  }

  function searchAgain(): void {
    void controller.searchAgain();
  }

  function submitCandidateSearch(event?: Event): void {
    event?.preventDefault?.();
    // Typed text filters current results; submitting re-queries providers (optional query override), and the results
    // that come back are that query's answer, so the same text no longer filters them.
    submittedQuery = candidateQuery.trim();
    void controller.searchAgain({ query: candidateQuery.trim() || undefined });
  }

  function onCandidateQueryInput(event: Event): void {
    const target = event.currentTarget as HTMLInputElement;
    candidateQuery = target.value;
  }

  function cancelSearch(): void {
    controller.cancelSearch();
  }

  function dismissSelection(): void {
    candidateQuery = '';
    submittedQuery = '';
    controller.dismissSelection();
  }

  function removeLyrics(): void {
    candidateQuery = '';
    submittedQuery = '';
    void controller.removeLyrics();
  }

  function selectCandidate(candidateId: string): void {
    void controller.selectCandidate(candidateId);
  }

  function setTimingOffsetSec(next: number): void {
    timingOffsetSec = saveLyricsTimingOffsetSec(trackId, next);
    // Other lyric windows (taskbar lyrics) follow the change at once; outside Tauri there is nobody to tell.
    if (trackId) void Promise.resolve().then(() => emit(LYRICS_TIMING_OFFSET_EVENT, { trackId, delaySec: timingOffsetSec })).catch(() => {});
  }

  function onTimingOffsetInput(event: Event): void {
    const target = event.currentTarget as HTMLInputElement;
    setTimingOffsetSec(Number(target.value));
  }

  function nudgeTimingOffset(deltaSec: number): void {
    setTimingOffsetSec(timingOffsetSec + deltaSec);
  }

  function resetTimingOffset(): void {
    setTimingOffsetSec(0);
  }

  function handleTimingPanelOutsidePointerDown(event: PointerEvent): void {
    if (!timingPanelOpen || !(event.target instanceof Node)) return;
    if (timingPanelElement?.contains(event.target)) return;
    if (event.target instanceof Element && event.target.closest(`[aria-label="${tr('同步歌詞延遲調整')}"]`)) return;
    timingPanelOpen = false;
  }
</script>

<svelte:window onpointerdown={handleTimingPanelOutsidePointerDown} />

<div
  bind:this={lyricsRoot}
  class="lyrics-view"
  data-testid="lyrics-view"
  data-track-id={viewState.trackId ?? ''}
  data-phase={viewState.phase}
  data-playing={isPlaying ? 'true' : 'false'}
  data-playback-state={effectivePlaybackState}
  data-line-gap={preferences.lineGapPx}
  data-text-effect={preferences.textEffect}
  data-active-cue-index={displayTimedIndex}
  data-current-cue-index={activeTimedIndex}
  data-layout-revision={layoutRevision}
  style={`--lyric-cue-ms:${cueAnimationDurationMs > 0 ? cueAnimationDurationMs : LYRIC_CUE_LEAD_MS}ms;--lyric-primary-font-size:${preferences.primaryFontSizePx}px;--lyric-auxiliary-font-size:${lyricsAuxiliaryFontSizePx(preferences)}px;--lyric-primary-line-height:${inkLineHeights.primary};--lyric-auxiliary-line-height:${inkLineHeights.auxiliary}`}
>
  {#if !trackId}
    <p class="lyrics-placeholder" role="status">{tr("播放歌曲後會在此顯示歌詞。")}</p>
  {:else if viewState.phase === 'loading' && !viewState.lyrics}
    <p class="lyrics-placeholder" role="status">{tr("正在讀取本機與已保存的歌詞…")}</p>
  {:else}
    {#if timingPanelOpen && isTimed}
      <div bind:this={timingPanelElement} class="lyrics-timing-panel floating-surface" role="group" aria-label={tr("歌詞平移")} data-testid="lyrics-timing-panel">
        <div class="lyrics-timing-panel-header">
          <strong>{tr("歌詞平移")}</strong>
          <span class="lyrics-timing-value" data-testid="lyrics-timing-value">{formatLyricsTimingOffsetSec(timingOffsetSec)}</span>
          <button type="button" class="soft-button" onclick={resetTimingOffset}>{tr("重設")}</button>
        </div>
        <div class="lyrics-timing-controls">
          <button type="button" class="soft-button" aria-label={tr("延遲減少 0.1 秒")} onclick={() => nudgeTimingOffset(-LYRICS_TIMING_OFFSET_STEP_SEC)}>−0.1</button>
          <input
            class="lyrics-timing-slider"
            type="range"
            min={LYRICS_TIMING_OFFSET_MIN_SEC}
            max={LYRICS_TIMING_OFFSET_MAX_SEC}
            step={LYRICS_TIMING_OFFSET_STEP_SEC}
            value={timingOffsetSec}
            aria-label={tr("歌詞延遲秒數")}
            aria-valuemin={LYRICS_TIMING_OFFSET_MIN_SEC}
            aria-valuemax={LYRICS_TIMING_OFFSET_MAX_SEC}
            aria-valuenow={timingOffsetSec}
            oninput={onTimingOffsetInput}
          />
          <button type="button" class="soft-button" aria-label={tr("延遲增加 0.1 秒")} onclick={() => nudgeTimingOffset(LYRICS_TIMING_OFFSET_STEP_SEC)}>+0.1</button>
        </div>
      </div>
    {/if}
    {#if viewState.lyrics && isTimed && !selectionUiOpen}
      <div
        class="lyrics-lines-viewport timed-lyrics-viewport"
        role="region"
        aria-label={tr("同步歌詞")}
        aria-live="off"
        bind:this={timedViewport}
        data-testid="timed-lyrics"
        data-rendered-count={timedRenderedRows.length}
        data-window-start={timedWindow.start}
        data-window-end={timedWindow.end}
        data-total-height={timedCanvasHeight}
      >
        <div class="lyrics-spacer lyrics-edge-spacer" data-edge-spacer="head" style={`height:${timedEdgePadding}px`} aria-hidden="true"></div>
        <div class="lyrics-spacer" data-virtual-spacer="before" style={`height:${timedWindow.beforeHeight}px`} aria-hidden="true"></div>
        {#each timedRenderedRows as timelineIndex (`${timelineIndex}:${timedLayout.generation}`)}
          {@const row = timedLines[timelineIndex]}
          <div
            class="lyric-line"
            class:empty={timedLayout.heights[timelineIndex] === 0}
            class:active={timelineIndex === visibleActiveTimedIndex}
            data-lyric-index={row.index}
            data-timeline-index={timelineIndex}
            use:observeLyricRow={timelineIndex}
            style={`--lyric-text-opacity:${getLyricLineOpacity(timelineIndex, visibleActiveTimedIndex, preferences.inactiveOpacityPercent)};--lyric-gap-after:${timedLayout.gaps[timelineIndex]}px`}
            aria-current={timelineIndex === visibleActiveTimedIndex ? 'true' : undefined}
          >
            {#if row.line.text?.trim()}
              {#if currentLineWords?.words && currentLineWords.index === timelineIndex}
                {#key currentLineWords.key}
                  <p class="lyric-primary has-words" class:words-paused={!wordsPlaying} data-word-count={currentLineWords.words.length}>{#each currentLineWords.words as word, wordIndex (wordIndex)}<span class="lyric-word" style={`animation-delay:${word.delayMs}ms;animation-duration:${word.durationMs}ms`}>{word.text}</span>{/each}</p>
                {/key}
              {:else}<p class="lyric-primary">{row.line.text}</p>{/if}
            {/if}
            {#if preferences.showRomanization && row.line.romanization?.trim()}
              {#if currentLineWords?.romanizationWords && currentLineWords.index === timelineIndex}
                {#key currentLineWords.key}
                  <p class="lyric-romanization has-words" class:words-paused={!wordsPlaying}>{#each currentLineWords.romanizationWords as word, wordIndex (wordIndex)}<span class="lyric-word" style={`animation-delay:${word.delayMs}ms;animation-duration:${word.durationMs}ms`}>{word.text}</span>{/each}</p>
                {/key}
              {:else}<p class="lyric-romanization">{row.line.romanization}</p>{/if}
            {/if}
            {#if preferences.showTranslation && row.line.translation?.trim()}<p class="lyric-translation">{row.line.translation}</p>{/if}
          </div>
        {/each}
        <div class="lyrics-spacer" data-virtual-spacer="after" style={`height:${timedWindow.afterHeight}px`} aria-hidden="true"></div>
        <div class="lyrics-spacer lyrics-edge-spacer" data-edge-spacer="tail" style={`height:${timedEdgePadding}px`} aria-hidden="true"></div>
      </div>
    {:else if viewState.lyrics && lines.length > 0 && !selectionUiOpen}
      <div
        class="lyrics-lines-viewport plain-lyrics-viewport"
        role="region"
        aria-label={tr("純歌詞，可捲動瀏覽")}
        aria-live="off"
        bind:this={plainViewport}
        data-testid="plain-lyrics"
        data-rendered-count={plainRenderedRows.length}
        data-window-start={plainWindow.start}
        data-window-end={plainWindow.end}
        data-total-height={plainTotalHeight}
      >
        <div class="lyrics-spacer" style={`height:${plainWindow.beforeHeight}px`} aria-hidden="true"></div>
        {#each plainRenderedRows as rowIndex (`${rowIndex}:${plainLayout.generation}`)}
          {@const line = lines[rowIndex]}
          <div class="lyric-line plain-lyric-line" class:empty={plainLayout.heights[rowIndex] === 0} data-lyric-index={rowIndex} use:observeLyricRow={rowIndex} style={`--lyric-gap-after:${plainLayout.gaps[rowIndex]}px`}>
            {#if line.text?.trim()}<p class="lyric-primary">{line.text}</p>{/if}
            {#if preferences.showRomanization && line.romanization?.trim()}<p class="lyric-romanization">{line.romanization}</p>{/if}
            {#if preferences.showTranslation && line.translation?.trim()}<p class="lyric-translation">{line.translation}</p>{/if}
          </div>
        {/each}
        <div class="lyrics-spacer" style={`height:${plainWindow.afterHeight}px`} aria-hidden="true"></div>
      </div>
    {:else if viewState.lyrics && !selectionUiOpen}
      <p class="lyrics-placeholder" role="status">{tr("這份歌詞沒有可顯示的文字。")}</p>
    {/if}

    {#if viewState.error}
      <div class="lyrics-message lyrics-error" role="alert">
        <span>{viewState.error}</span>
        {#if viewState.failedStage === 'load'}
          <button type="button" class="soft-button" onclick={retry}>{tr("重試讀取")}</button>
          <button type="button" class="soft-button" onclick={searchAgain}>{tr("線上搜尋")}</button>
        {:else if viewState.failedStage === 'selection'}
          <button type="button" class="soft-button" onclick={retry}>{tr("重試指定")}</button>
        {:else}
          <button type="button" class="soft-button" onclick={searchAgain}>{tr("重試搜尋")}</button>
        {/if}
      </div>
    {/if}

    {#if viewState.isSearching}
      <div class="lyrics-message lyrics-searching" role="status">
        <span>{tr("正在搜尋網易雲與 QQ 音樂歌詞…")}</span>
        <button type="button" class="soft-button" onclick={cancelSearch}>{tr("取消搜尋")}</button>
      </div>
    {:else if showCandidatePicker}
      <section class="lyrics-candidates" aria-label={tr("歌詞候選")} data-testid="lyrics-candidates">
        <form class="lyrics-candidates-search" onsubmit={submitCandidateSearch}>
          <input
            class="lyrics-candidates-search-input"
            type="search"
            value={candidateQuery}
            oninput={onCandidateQueryInput}
            placeholder={tr("搜尋歌詞關鍵字…")}
            aria-label={tr("搜尋歌詞關鍵字")}
            autocomplete="off"
            spellcheck="false"
            data-testid="lyrics-candidates-search"
          />
          <div class="lyrics-candidates-actions">
            <button type="submit" class="soft-button">{tr("搜尋")}</button>
            <button type="button" class="soft-button" data-testid="lyrics-dismiss" onclick={dismissSelection}>{tr("關閉")}</button>
            <button type="button" class="soft-button" data-testid="lyrics-remove" onclick={removeLyrics}>{tr("移除歌詞")}</button>
          </div>
        </form>
        <ul>
          {#each filteredCandidates as candidate (candidate.id)}
            {@const presentation = getCandidatePresentation(candidate)}
            <li class="lyrics-candidate">
              <div class="candidate-copy">
                <strong>{candidate.title}</strong>
                <span>{candidate.artist}{candidate.album ? ` · ${candidate.album}` : ''}</span>
                <small>{providerLabel(candidate.provider)} {tr("· 分數")} {candidate.score.toFixed(2)} · {formatCandidateDuration(candidate.durationMs)} · {presentation.formatLabel}</small>
                {#if candidate.previewLines.length > 0}
                  <span class="candidate-preview">{candidate.previewLines.slice(0, 2).join(' / ')}</span>
                {:else if presentation.previewNotice}
                  <span class="candidate-preview-note" role="note">{presentation.previewNotice}</span>
                {/if}
                {#if candidate.reasons.length > 0}
                  <small>{candidate.reasons.join('；')}</small>
                {/if}
              </div>
              <button
                type="button"
                class="soft-button"
                disabled={viewState.selectingCandidateId !== null}
                onclick={() => selectCandidate(candidate.id)}
              >
                {viewState.selectingCandidateId === candidate.id ? tr('套用中…') : tr('選擇')}
              </button>
            </li>
          {:else}
            <li class="lyrics-candidate-empty" role="status">
              {#if viewState.candidates.length === 0}
                {tr("找不到符合的歌詞。可輸入關鍵字後再搜尋。")}
              {:else}
                {tr("沒有符合「{0}」的候選。", [candidateQuery.trim()])}
              {/if}
            </li>
          {/each}
          {#if viewState.hasMore}
            <li class="lyrics-load-more">
              <button
                type="button"
                class="soft-button"
                data-testid="lyrics-load-more"
                disabled={viewState.isLoadingMore}
                onclick={() => void controller.loadMore()}
              >
                {viewState.isLoadingMore ? tr('載入中…') : tr('載入更多歌詞')}
              </button>
            </li>
          {/if}
        </ul>
      </section>
    {/if}
  {/if}
</div>

<style>
  .lyrics-view {
    position: relative;
    display: flex;
    min-width: 0;
    min-height: 0;
    flex: 1 1 auto;
    flex-direction: column;
    gap: 12px;
  }

  .lyrics-placeholder {
    display: grid;
    min-height: 150px;
    flex: 1 1 auto;
    place-items: center;
    margin: 0;
    color: var(--muted);
    font-size: var(--font-body);
    line-height: 1.7;
    text-align: center;
  }

  .lyrics-lines-viewport {
    position: relative;
    min-height: 168px;
    max-height: min(56vh, 620px);
    flex: 1 1 auto;
    overflow: auto;
    overscroll-behavior: contain;
    -webkit-mask-image: linear-gradient(to bottom, transparent 0%, #000 24%, #000 76%, transparent 100%);
    mask-image: linear-gradient(to bottom, transparent 0%, #000 24%, #000 76%, transparent 100%);
    scrollbar-color: color-mix(in srgb, var(--accent) 42%, transparent) transparent;
    scrollbar-width: thin;
  }

  .lyrics-spacer {
    width: 1px;
    pointer-events: none;
  }

  .lyrics-edge-spacer {
    flex: 0 0 auto;
  }

  /* Shadow geometry shared by text-shadow/filter and row padding: each row's vertical padding equals the
     first (tight) shadow's blur radius, so at line spacing 0 neighbouring tight shadows just touch; the wider
     glow may overlap neighbours. The spacing setting adds on top.
     Keep LYRICS_ROW_VERTICAL_PADDING_PX (lyrics-window.js) = 2 x --lyric-shadow-blur. */
  .lyrics-view {
    --lyric-shadow-blur: 3px;
    --lyric-glow-blur: 8px;
  }

  .lyric-line {
    display: flex;
    box-sizing: border-box;
    width: 100%;
    height: auto;
    flex: 0 0 auto;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    /* Visible so the glow is never hard-cut at the row edge (the viewport still clips/masks). */
    overflow: visible;
    padding: var(--lyric-shadow-blur) 9px;
    margin-bottom: var(--lyric-gap-after, 0px);
    border-bottom: 0;
    color: var(--muted);
    transition: color var(--lyric-cue-ms) ease;
  }

  .lyric-line.active {
    color: var(--text);
    background: transparent;
  }

  .lyric-line.empty {
    height: 0;
    min-height: 0;
    padding: 0;
    margin: 0;
    border: 0;
  }


  .lyrics-view[data-text-effect='shadow'] .lyric-translation,
  .lyrics-view[data-text-effect='shadow'] .lyric-romanization {
    text-shadow:
      0 0 var(--lyric-shadow-blur) rgba(var(--text-inverse-rgb), 0.6),
      0 0 var(--lyric-glow-blur) rgba(var(--text-inverse-rgb), 0.38);
  }

  /* The primary line must clip (2-line clamp; later lines are still painted), and a text-shadow would be
     clipped with it. A filter is applied after the element's own overflow clip, so the same shadow drawn
     as drop-shadow fades out naturally past the clamped box. */
  .lyrics-view[data-text-effect='shadow'] .lyric-primary {
    text-shadow: none;
    filter:
      drop-shadow(0 0 var(--lyric-shadow-blur) rgba(var(--text-inverse-rgb), 0.6))
      drop-shadow(0 0 var(--lyric-glow-blur) rgba(var(--text-inverse-rgb), 0.38));
  }

  .lyrics-view[data-text-effect='stroke'] .lyric-primary,
  .lyrics-view[data-text-effect='stroke'] .lyric-translation,
  .lyrics-view[data-text-effect='stroke'] .lyric-romanization {
    text-shadow: none;
    -webkit-text-stroke: 0.08em rgba(var(--text-inverse-rgb), 0.5);
    paint-order: stroke fill;
  }

  /* The primary line is clipped to 2 lines at the measured ink height, with no room beyond the descenders; the outline
     reaches half its width (0.04em) past the glyphs and the bottoms of y, j, g and q were cut. Give the clip box that
     room as padding and take it back with a negative margin, so the layout does not move. */
  .lyrics-view[data-text-effect='stroke'] .lyric-primary {
    padding-block: 0.06em;
    margin-block: -0.06em;
    max-height: calc(var(--lyric-primary-font-size, 14px) * var(--lyric-primary-line-height, 1.35) * 2 + 0.12em);
  }

  .lyrics-view[data-text-effect='none'] .lyric-primary,
  .lyrics-view[data-text-effect='none'] .lyric-translation,
  .lyrics-view[data-text-effect='none'] .lyric-romanization {
    text-shadow: none;
    -webkit-text-stroke: 0;
  }

  .lyric-primary {
    display: -webkit-box;
    box-sizing: border-box;
    min-width: 0;
    max-height: calc(var(--lyric-primary-font-size, 14px) * var(--lyric-primary-line-height, 1.35) * 2);
    flex: 0 1 auto;
    margin: 0;
    overflow: hidden;
    color: inherit;
    font-size: var(--lyric-primary-font-size, 14px);
    font-weight: 550;
    /* Measured ink fit (ascenders + descenders, no extra leading); vertical room comes from the shadow padding. */
    line-height: var(--lyric-primary-line-height, 1.35);
    line-clamp: 2;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    text-overflow: ellipsis;
    text-align: center;
    opacity: var(--lyric-text-opacity, 1);
    transition: opacity var(--lyric-cue-ms) ease;
  }

  /* Word highlight (ADR 0041), karaoke style: the word being sung fills with the theme accent from left to right
     with a soft edge, words already sung stay accent until the line ends, words not yet sung keep the line's own
     colour. One CSS animation per word (delay = time until the word starts, negative once started; duration = its
     length; fill mode forwards), so no style is written per frame and a word holds the full fill after it ends,
     including words already past when the line is rebuilt after a seek, pause or offset change. The fill is the span's background clipped to its
     glyphs (`background-clip: text`) with the text fill transparent while the animation runs; the gradient position
     is the registered `--lyric-word-fill` (app.css), animated from before the word's left edge to past its right
     edge. Before the animation starts the opaque text fill covers the background. No filter on the span: the shadow mode's
     drop-shadow stays on the line, and the outline (`-webkit-text-stroke`) is drawn over the fill as before.
     `box-decoration-break: clone` gives each fragment of a word that wraps its own wipe. */
  .lyric-word {
    background-image: linear-gradient(
      90deg,
      var(--accent) calc(var(--lyric-word-fill) - var(--lyric-word-edge)),
      var(--text) calc(var(--lyric-word-fill) + var(--lyric-word-edge))
    );
    -webkit-background-clip: text;
    background-clip: text;
    -webkit-box-decoration-break: clone;
    box-decoration-break: clone;
    --lyric-word-edge: 12%;
    animation-name: lyric-word-wipe;
    animation-timing-function: linear;
    animation-iteration-count: 1;
    animation-fill-mode: forwards;
  }

  .words-paused .lyric-word {
    animation-play-state: paused;
  }

  @keyframes lyric-word-wipe {
    from {
      --lyric-word-fill: -12%;
      color: transparent;
    }
    to {
      --lyric-word-fill: 112%;
      color: transparent;
    }
  }

  /* Reduced motion: no wipe, the whole word turns accent when it starts and stays accent (forwards). */
  @media (prefers-reduced-motion: reduce) {
    .lyric-word {
      animation-name: lyric-word-sing;
    }
  }

  @keyframes lyric-word-sing {
    from,
    to {
      color: var(--accent);
    }
  }

  .lyric-translation,
  .lyric-romanization {
    display: block;
    box-sizing: border-box;
    min-width: 0;
    max-height: none;
    flex: 0 1 auto;
    margin: 0;
    overflow: visible;
    color: var(--muted);
    font-size: var(--lyric-auxiliary-font-size, 10px);
    line-height: var(--lyric-auxiliary-line-height, 1.35);
    white-space: normal;
    overflow-wrap: anywhere;
    word-break: break-word;
    text-overflow: clip;
    text-align: center;
    opacity: var(--lyric-text-opacity, 1);
    transition: opacity var(--lyric-cue-ms) ease;
  }

  .lyrics-message {
    display: flex;
    min-width: 0;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    color: var(--muted);
    font-size: var(--font-caption);
  }

  .lyrics-error {
    color: var(--status-danger, #ff8b88);
  }

  .lyrics-view :global(.soft-button) {
    flex: 0 0 auto;
  }

  .lyrics-candidates {
    display: flex;
    box-sizing: border-box;
    min-height: 0;
    flex: 1 1 auto;
    flex-direction: column;
    gap: 8px;
    overflow: hidden;
    max-height: min(50vh, 480px);
    border: 0;
    background: transparent;
  }

  .lyrics-candidates-search {
    display: flex;
    min-width: 0;
    flex: 0 0 auto;
    flex-wrap: wrap;
    align-items: center;
    gap: 7px;
  }

  .lyrics-candidates-actions {
    display: flex;
    flex: 0 0 auto;
    align-items: center;
    justify-content: flex-end;
    margin-left: auto;
    gap: 6px;
  }

  .lyrics-candidates-search-input {
    box-sizing: border-box;
    min-width: 0;
    flex: 1 1 180px;
    min-height: 34px;
    padding: 0 11px;
    border: 1px solid rgba(var(--text-rgb), 0.18);
    border-radius: 10px;
    color: var(--text);
    background: transparent;
    font: inherit;
    font-size: var(--font-caption);
  }

  .lyrics-candidates-search-input::placeholder {
    color: var(--muted);
  }

  /*
   * .lyrics-candidates (and .now-playing-lyrics) clip overflow so the list can scroll inside a
   * cover-height panel. An outset focus ring would be cut off at the panel edge, so draw it inset.
   */
  .lyrics-candidates-search-input:focus-visible {
    border-color: var(--accent);
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .lyrics-candidates :global(.soft-button:focus-visible) {
    outline-offset: -2px;
  }

  .lyrics-candidates ul {
    display: grid;
    min-height: 0;
    flex: 1 1 auto;
    gap: 7px;
    margin: 0;
    padding: 0 0 4px;
    overflow: auto;
    overscroll-behavior: contain;
    list-style: none;
    scrollbar-color: color-mix(in srgb, var(--accent) 42%, transparent) transparent;
    scrollbar-width: thin;
  }

  .lyrics-load-more {
    display: flex;
    justify-content: center;
    padding: 8px 0 4px;
    list-style: none;
  }

  .lyrics-candidate-empty {
    padding: 18px 10px;
    color: var(--muted);
    font-size: var(--font-title);
    line-height: 1.6;
    text-align: center;
    list-style: none;
  }

  .lyrics-candidate {
    display: flex;
    min-width: 0;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 9px;
    border: 1px solid rgba(var(--text-rgb), 0.14);
    border-radius: 8px;
    background: transparent;
  }

  .candidate-copy {
    display: grid;
    min-width: 0;
    gap: 4px;
  }

  .candidate-copy strong,
  .candidate-copy span,
  .candidate-copy small {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .candidate-copy strong {
    color: var(--text);
    font-size: var(--font-title);
    line-height: 1.35;
  }

  .candidate-copy span,
  .candidate-copy small {
    color: var(--muted);
    font-size: var(--font-body);
    line-height: 1.4;
  }

  .candidate-preview {
    color: var(--text-soft) !important;
  }

  .candidate-preview-note {
    color: var(--muted);
    font-size: var(--font-body);
    line-height: 1.5;
  }

  .lyrics-timing-panel {
    position: absolute;
    z-index: 3;
    top: 0;
    right: 0;
    left: 0;
    display: grid;
    gap: 8px;
    padding: 10px 10px 8px;
  }

  .lyrics-timing-panel-header {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 8px;
  }

  .lyrics-timing-panel-header strong {
    color: var(--text-soft);
    font-size: var(--font-caption);
    font-weight: 650;
  }

  .lyrics-timing-value {
    margin-right: auto;
    color: var(--text);
    font-size: var(--font-body);
    font-variant-numeric: tabular-nums;
  }

  .lyrics-timing-controls {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 8px;
  }

  .lyrics-timing-slider {
    min-width: 0;
    flex: 1 1 auto;
    accent-color: var(--accent);
  }

  @media (max-width: 560px) {
    .lyrics-candidate {
      align-items: flex-start;
    }

    .lyrics-timing-controls {
      flex-wrap: wrap;
    }
  }
</style>
