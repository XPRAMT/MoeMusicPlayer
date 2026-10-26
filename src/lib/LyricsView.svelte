<script lang="ts">
  import { tr } from './i18n.js';
  import { onDestroy, tick, untrack } from 'svelte';
  import { emit, listen } from '@tauri-apps/api/event';
  import { IconPlayerPlay } from '@tabler/icons-svelte-runes';
  import { defaultLyricsApi, type LyricsApi } from './lyrics-api';
  import { seekPlayback } from './ipc';
  import { formatDuration } from './format';
  import { appTooltip } from './app-tooltip.js';
  import { browseButtonIndex, browseReturnAction, browseSeekPositionMs, findBrowseCentreRow } from './lyrics-browse.js';
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
  import { karaokeLineAnimation, karaokeWordsOf, LYRIC_LINE_ANIMATION, wordTexts } from './lyrics-word-highlight.js';
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
  import { frozenRowWidth } from './lyrics-layout-freeze.js';
  import { APP_FONT_STACK_CSS, fontFamilyCss } from './font-picker.js';
  import { isAndroidWebView } from './font-faces.js';
  import { effectiveCandidateFilter, filterLyricsCandidates, getCandidatePresentation } from './lyrics-candidate-preview.js';
  import {
    DEFAULT_LYRICS_PREFERENCES,
    getDisplayActiveLyricIndex,
    getLyricLineOpacity,
    lyricsAuxiliaryFontSizePx,
    lyricsAuxiliaryWeight,
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
    lyricsShiftShownSec,
    lyricsShiftStoredSec,
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
    /** True while the cover is being zoomed: the text layout stays at the last settled size (the column only clips it) and
     *  is rebuilt once, instantly, when this turns false. */
    layoutFrozen?: boolean;
    /** Seeks for the browse-and-seek play button (the shell's own seek, which goes through `seekPlayback`); alone the
     *  view calls `seekPlayback` itself. */
    onSeek?: (positionMs: number) => Promise<unknown>;
    /** The track's length: a line whose seek target is at or past it gets no play button. Unknown: not limited. */
    durationMs?: number | null;
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
    layoutFrozen = false,
    onSeek = (positionMs: number) => seekPlayback(positionMs),
    durationMs = null,
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
    const primary = measureInkLineHeight(context, `${preferences.fontWeight} 100px ${family}`, LYRICS_PRIMARY_LINE_HEIGHT);
    const auxiliary = measureInkLineHeight(context, `${lyricsAuxiliaryWeight(preferences.fontWeight)} 100px ${family}`, LYRICS_AUXILIARY_LINE_HEIGHT);
    if (primary !== inkLineHeights.primary || auxiliary !== inkLineHeights.auxiliary) inkLineHeights = { primary, auxiliary };
  }
  $effect(() => {
    if (!lyricsRoot) return;
    void preferences.fontWeight; // re-measure when the weight changes
    void lyricFontFamily; // or the font
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
  // Browse and seek (ADR 0029): a scroll by the user starts browsing; the line at the viewport centre (when it is not
  // the current line) shows a play button that seeks to it. Browsing ends when the current line changes (or, if the
  // user is still scrolling then, when that scroll ends) and the view returns to the current line. No timeout.
  let browsing = $state(false);
  let browseCentreIndex = $state(-1);
  let browsePendingReturn = false;
  /** A scroll (the user's or its momentum) is running while browsing; cleared by `scrollend`. */
  let browseScrolling = false;
  let pointerHeld = false;
  let touchHeld = false;
  let timingOffsetSec = $state(0);
  let timingPanelOpen = $state(false);
  let timingShownSec = $derived(lyricsShiftShownSec(timingOffsetSec));
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
  // The playing page's own font (ADR 0042); unset: the interface font. A missing font falls through to the stack.
  const lyricFontFamily = $derived(fontFamilyCss(preferences.fontFamily, APP_FONT_STACK_CSS, { android: isAndroidWebView() }));

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
  /** The row showing the browse-and-seek play button, or -1. */
  let browseButtonRow = $derived(browseButtonIndex({
    browsing,
    centreIndex: browseCentreIndex,
    currentIndex: visibleActiveTimedIndex,
    seekPositionMs: timedLines[browseCentreIndex] ? browseSeekPositionMs(timedLines[browseCentreIndex].startMs, timedOffsetMs) : undefined,
    durationMs,
  }));
  let plainLayout = $derived(buildLyricsLayout(lines, layoutPreferences, preferences.lineGapPx, layoutWidth));
  // Word highlight (ADR 0041 D10): only the current line with word timing (or its own end time) is split into words, and one CSS
  // animation moves a single accent edge over it (`karaokeLine`). Recomputed only on a line change, seek, play/pause
  // or offset change; the clock's regular samples do not touch it, so no style is written while a line plays.
  let wordsPlaying = $derived(effectivePlaybackState === 'playing' && (playbackClock ? playbackClock.playing : isPlaying));
  let currentLineWords = $derived.by(() => {
    const index = visibleActiveTimedIndex;
    const line = index >= 0 ? timedLines[index]?.line : undefined;
    // A line with its own end but no word times is swept as one word spanning it (`karaokeWordsOf`).
    const words = karaokeWordsOf(line);
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
      position,
      offset,
      words: words.length > 0 ? words : null,
      romanizationWords: romanizationWords.length > 0 ? romanizationWords : null,
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
    if (!element || activeIndex < 0 || timedSmoothScrolling || timedUserScrolling || browsing) return;
    const targetTop = timedCueScrollTop(timedLayout, activeIndex, timedViewportHeight, element.clientHeight);
    if (Math.abs(element.scrollTop - targetTop) > 1) element.scrollTo({ top: targetTop, behavior: 'instant' });
  }

  /** Whether correctTimedCenter would recenter now; otherwise a measured row keeps the top row in place instead. */
  function timedCenterFollows(): boolean {
    return displayActiveTimedIndex >= 0 && !timedSmoothScrolling && !timedUserScrolling && !browsing;
  }

  function userScrollActive(): boolean {
    return pointerHeld || touchHeld || timedUserScrolling;
  }
  /** Browsing: the row under the viewport centre, from the layout the virtual window already uses (no DOM reads
   *  beyond the scroll position and height the window reads too). */
  function updateBrowseCentre(element: HTMLDivElement): void {
    const index = findBrowseCentreRow(timedLayout, element.scrollTop, element.clientHeight || timedViewportHeight, timedEdgePadding);
    if (index !== browseCentreIndex) browseCentreIndex = index;
  }
  function endBrowse(): void {
    browsing = false;
    browseCentreIndex = -1;
    browsePendingReturn = false;
    browseScrolling = false;
  }
  /** The current line changed while the user was still scrolling: once the scroll has ended, return to it with the
   *  cue scroll (instant with reduced motion). */
  function finishDeferredBrowse(): void {
    const action = browseReturnAction({ browsing, cueChanged: false, pendingReturn: browsePendingReturn, userActive: userScrollActive() || browseScrolling });
    if (action !== 'return') return;
    endBrowse();
    const element = timedViewport;
    const activeIndex = displayActiveTimedIndex;
    if (!element || activeIndex < 0) return;
    const reducedMotion = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
    const targetTop = timedCueScrollTop(timedLayout, activeIndex, timedViewportHeight, element.clientHeight);
    const plan = planTimedScroll({ kind: 'cue', glideRunning: cueGlide !== null, animationMs: LYRIC_CUE_LEAD_MS, reducedMotion, distancePx: targetTop - element.scrollTop });
    if (plan === 'retarget') glideCueScroll(element, targetTop);
    else if (plan === 'rest') startCueScroll(element, targetTop, LYRIC_CUE_LEAD_MS);
    else if (plan === 'instant') {
      stopCueScroll();
      element.scrollTo({ top: targetTop, behavior: 'instant' });
      refreshTimedWindow();
    }
  }
  /** The play button: seek to where the browsed line becomes current (its start and the display offset, no lead).
   *  The seek is a jump, so the view lands on the line at once (the navigation effect below). */
  function seekToBrowsedLine(timelineIndex: number): void {
    const entry = timedLines[timelineIndex];
    if (!entry) return;
    endBrowse();
    void Promise.resolve(onSeek(browseSeekPositionMs(entry.startMs, timedOffsetMs)))
      .catch(() => {})
      .finally(() => requestAnimationFrame(() => correctTimedCenter()));
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

  // The karaoke edge of the current line (ADR 0041 D10): measures the word spans once (their total drawn width on
  // every wrapped row) and sets one animation on the line span; nothing runs per frame in script.
  function karaokeLine(node: HTMLElement, params: { words: { s: number; e: number }[]; position: number; offset: number }) {
    const widths = Array.from(node.children, (child) =>
      Array.from(child.getClientRects()).reduce((sum, rect) => sum + rect.width, 0),
    );
    const reduced = typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
    const animation = karaokeLineAnimation(params.words, widths, params.position, params.offset, reduced);
    if (!animation) return;
    node.style.animationName = LYRIC_LINE_ANIMATION;
    node.style.animationDelay = `${animation.delayMs}ms`;
    node.style.animationDuration = `${animation.durationMs}ms`;
    node.style.animationTimingFunction = animation.easing;
  }

  // Rows are keyed by their line index only (not by layout generation): a re-wrap (the cover zoom settles, the column
  // width changes) updates the existing row elements in place, so the browser keeps their layers and nothing blanks
  // while new rows would be rasterised. The generation is passed in so the row is re-tagged and measured again.
  function observeLyricRow(node: HTMLElement, row: { index: number; generation: number }) {
    const update = (next: { index: number; generation: number }) => {
      node.dataset.layoutIndex = String(next.index);
      node.dataset.layoutGeneration = String(next.generation);
      rowObserver?.observe(node, { box: 'border-box' });
    };
    update(row);
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
      if (layoutFrozen) return; // re-measured once when the freeze ends (below)
      const nextWidth = Math.round(viewport.clientWidth);
      if (nextWidth !== layoutWidth) layoutWidth = nextWidth;
      if (isTimed) timedViewportHeight = viewport.clientHeight;
      else plainViewportHeight = viewport.clientHeight;
    });
    observer.observe(viewport);
    return () => observer.disconnect();
  });

  // The freeze ended: take the settled column size once (one re-wrap and one instant re-centre).
  $effect(() => {
    if (layoutFrozen) return;
    untrack(() => {
      const viewport = isTimed ? timedViewport : plainViewport;
      if (!viewport) return;
      const nextWidth = Math.round(viewport.clientWidth);
      if (nextWidth !== layoutWidth) layoutWidth = nextWidth;
      if (isTimed) { if (viewport.clientHeight !== timedViewportHeight) timedViewportHeight = viewport.clientHeight; }
      else if (viewport.clientHeight !== plainViewportHeight) plainViewportHeight = viewport.clientHeight;
    });
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
    untrack(() => endBrowse());
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
      if (!layoutFrozen) plainViewportHeight = element.clientHeight;
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
    const cueChanged = previous === null || previous.index !== activeIndex || previous.trackId !== currentTrackId;
    previousTimedNavigation = { trackId: currentTrackId, generation: layout.generation, index: activeIndex, viewport: element, height: viewportHeight, jumpId, clockPlaying, offsetMs };

    const frame = requestAnimationFrame(() => {
      // Browsing keeps the user's position until the current line changes; a change while the user is still
      // scrolling waits for that scroll to end (`finishDeferredBrowse`).
      const browse = browseReturnAction({ browsing, cueChanged, pendingReturn: browsePendingReturn, userActive: userScrollActive() || browseScrolling });
      if (browse === 'keep') return;
      if (browse === 'defer') { browsePendingReturn = true; return; }
      if (browse === 'return') endBrowse();
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
      browseScrolling = false;
      finishDeferredBrowse();
    };
    // The browse play button is not a scroll: pressing it must neither stop the cue scroll nor start browsing.
    const fromBrowseButton = (event: Event) => event.target instanceof Element && event.target.closest('.lyric-browse-seek') !== null;
    const onUserScroll = (event: Event) => {
      if (fromBrowseButton(event)) return;
      if (event.type === 'pointerdown' && (event as PointerEvent).pointerType !== 'touch') pointerHeld = true;
      else if (event.type === 'touchstart') touchHeld = true;
      stopCueScroll();
      timedUserScrolling = true;
      if (userScrollResetTimer) clearTimeout(userScrollResetTimer);
      userScrollResetTimer = setTimeout(() => { timedUserScrolling = false; userScrollResetTimer = null; finishDeferredBrowse(); }, 300);
    };
    const onPointerRelease = () => {
      if (!pointerHeld) return;
      pointerHeld = false;
      finishDeferredBrowse();
    };
    const onTouchRelease = (event: TouchEvent) => {
      if (!touchHeld || event.touches.length > 0) return;
      touchHeld = false;
      finishDeferredBrowse();
    };
    // Without `scrollend` a momentum scroll cannot be told apart, so browsing then waits only for the input guard.
    const tracksScrollEnd = 'onscrollend' in element;
    const onScroll = () => {
      refreshTimedWindow();
      // Every scroll while browsing is the user's (or its momentum): the view's own scrolls run only when not browsing.
      if (!browsing && userScrollActive() && !cueGlide) browsing = true;
      if (!browsing) return;
      if (tracksScrollEnd) browseScrolling = true;
      updateBrowseCentre(element);
    };
    element.addEventListener('scroll', onScroll, { passive: true });
    element.addEventListener('scrollend', onScrollEnd);
    element.addEventListener('wheel', onUserScroll, { passive: true });
    element.addEventListener('pointerdown', onUserScroll, { passive: true });
    element.addEventListener('touchstart', onUserScroll, { passive: true });
    window.addEventListener('pointerup', onPointerRelease, { passive: true });
    window.addEventListener('pointercancel', onPointerRelease, { passive: true });
    window.addEventListener('touchend', onTouchRelease, { passive: true });
    window.addEventListener('touchcancel', onTouchRelease, { passive: true });
    return () => {
      element.removeEventListener('scroll', onScroll);
      element.removeEventListener('scrollend', onScrollEnd);
      element.removeEventListener('wheel', onUserScroll);
      element.removeEventListener('pointerdown', onUserScroll);
      element.removeEventListener('touchstart', onUserScroll);
      window.removeEventListener('pointerup', onPointerRelease);
      window.removeEventListener('pointercancel', onPointerRelease);
      window.removeEventListener('touchend', onTouchRelease);
      window.removeEventListener('touchcancel', onTouchRelease);
      if (userScrollResetTimer) clearTimeout(userScrollResetTimer);
      userScrollResetTimer = null;
      pointerHeld = false;
      touchHeld = false;
      untrack(() => endBrowse());
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
    setTimingOffsetSec(lyricsShiftStoredSec(Number(target.value)));
  }

  function nudgeTimingOffset(deltaSec: number): void {
    // The panel works on the shown value (shown = −stored).
    setTimingOffsetSec(lyricsShiftStoredSec(timingShownSec + deltaSec));
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
  style={`--lyric-cue-ms:${cueAnimationDurationMs > 0 ? cueAnimationDurationMs : LYRIC_CUE_LEAD_MS}ms;--lyric-primary-font-size:${preferences.primaryFontSizePx}px;--lyric-auxiliary-font-size:${lyricsAuxiliaryFontSizePx(preferences)}px;--lyric-font-family:${lyricFontFamily};--lyric-primary-weight:${preferences.fontWeight};--lyric-auxiliary-weight:${lyricsAuxiliaryWeight(preferences.fontWeight)};--lyric-primary-line-height:${inkLineHeights.primary};--lyric-auxiliary-line-height:${inkLineHeights.auxiliary}`}
>
  {#if !trackId}
    <p class="lyrics-placeholder" role="status">{tr("播放歌曲後會在此顯示歌詞。")}</p>
  {:else if viewState.phase === 'loading' && !viewState.lyrics}
    <p class="lyrics-placeholder" role="status">{tr("正在讀取本機與已保存的歌詞…")}</p>
  {:else}
    {#if timingPanelOpen && isTimed}
      <div bind:this={timingPanelElement} class="lyrics-timing-panel floating-surface" role="group" aria-label={tr("歌詞同步")} data-testid="lyrics-timing-panel">
        <div class="lyrics-timing-panel-header">
          <strong>{tr("歌詞同步")}</strong>
          <span class="lyrics-timing-value" data-testid="lyrics-timing-value">{formatLyricsTimingOffsetSec(timingShownSec)}</span>
          <button type="button" class="soft-button" onclick={resetTimingOffset}>{tr("重設")}</button>
        </div>
        <div class="lyrics-timing-controls">
          <button type="button" class="soft-button" aria-label={tr("歌詞同步減少 0.1 秒")} onclick={() => nudgeTimingOffset(-LYRICS_TIMING_OFFSET_STEP_SEC)}>−0.1</button>
          <input
            class="lyrics-timing-slider"
            type="range"
            min={LYRICS_TIMING_OFFSET_MIN_SEC}
            max={LYRICS_TIMING_OFFSET_MAX_SEC}
            step={LYRICS_TIMING_OFFSET_STEP_SEC}
            value={timingShownSec}
            aria-label={tr("歌詞同步秒數")}
            aria-valuemin={LYRICS_TIMING_OFFSET_MIN_SEC}
            aria-valuemax={LYRICS_TIMING_OFFSET_MAX_SEC}
            aria-valuenow={timingShownSec}
            oninput={onTimingOffsetInput}
          />
          <button type="button" class="soft-button" aria-label={tr("歌詞同步增加 0.1 秒")} onclick={() => nudgeTimingOffset(LYRICS_TIMING_OFFSET_STEP_SEC)}>+0.1</button>
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
        data-layout-frozen={layoutFrozen ? '' : undefined}
        style:--lyrics-frozen-width={frozenRowWidth(layoutFrozen, layoutWidth)}
        data-testid="timed-lyrics"
        data-rendered-count={timedRenderedRows.length}
        data-window-start={timedWindow.start}
        data-window-end={timedWindow.end}
        data-total-height={timedCanvasHeight}
      >
        <div class="lyrics-spacer lyrics-edge-spacer" data-edge-spacer="head" style={`height:${timedEdgePadding}px`} aria-hidden="true"></div>
        <div class="lyrics-spacer" data-virtual-spacer="before" style={`height:${timedWindow.beforeHeight}px`} aria-hidden="true"></div>
        {#each timedRenderedRows as timelineIndex (timelineIndex)}
          {@const row = timedLines[timelineIndex]}
          <div
            class="lyric-line"
            class:empty={timedLayout.heights[timelineIndex] === 0}
            class:active={timelineIndex === visibleActiveTimedIndex}
            data-lyric-index={row.index}
            data-timeline-index={timelineIndex}
            use:observeLyricRow={{ index: timelineIndex, generation: timedLayout.generation }}
            style={`--lyric-text-opacity:${getLyricLineOpacity(timelineIndex, visibleActiveTimedIndex, preferences.mainOpacityPercent, preferences.secondaryOpacityPercent)};--lyric-gap-after:${timedLayout.gaps[timelineIndex]}px`}
            aria-current={timelineIndex === visibleActiveTimedIndex ? 'true' : undefined}
          >
            {#if timelineIndex === browseButtonRow}
              {@const seekLabel = formatDuration(browseSeekPositionMs(row.startMs, timedOffsetMs))}
              <button
                type="button"
                class="lyric-browse-seek"
                data-swipe-ignore
                data-testid="lyric-browse-seek"
                aria-label={tr('從這句開始播放（{0}）', [seekLabel])}
                use:appTooltip={tr('從這句開始播放（{0}）', [seekLabel])}
                onclick={() => seekToBrowsedLine(timelineIndex)}
              >
                <IconPlayerPlay size={22} stroke={2} aria-hidden="true" />
              </button>
            {/if}
            {#if row.line.text?.trim()}
              {#if currentLineWords?.words && currentLineWords.index === timelineIndex}
                {#key currentLineWords.key}
                  <p class="lyric-primary has-words" class:words-paused={!wordsPlaying} data-word-count={currentLineWords.words.length}><span class="lyric-karaoke" use:karaokeLine={{ words: currentLineWords.words, position: currentLineWords.position, offset: currentLineWords.offset }}>{#each wordTexts(currentLineWords.words, null) as text, wordIndex (wordIndex)}<span class="lyric-word">{text}</span>{/each}</span></p>
                {/key}
              {:else}<p class="lyric-primary">{row.line.text}</p>{/if}
            {/if}
            {#if preferences.showRomanization && row.line.romanization?.trim()}
              {#if currentLineWords?.romanizationWords && currentLineWords.index === timelineIndex}
                {#key currentLineWords.key}
                  <p class="lyric-romanization has-words" class:words-paused={!wordsPlaying}><span class="lyric-karaoke" use:karaokeLine={{ words: currentLineWords.romanizationWords, position: currentLineWords.position, offset: currentLineWords.offset }}>{#each wordTexts(currentLineWords.romanizationWords, null) as text, wordIndex (wordIndex)}<span class="lyric-word">{text}</span>{/each}</span></p>
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
        data-layout-frozen={layoutFrozen ? '' : undefined}
        style:--lyrics-frozen-width={frozenRowWidth(layoutFrozen, layoutWidth)}
        data-testid="plain-lyrics"
        data-rendered-count={plainRenderedRows.length}
        data-window-start={plainWindow.start}
        data-window-end={plainWindow.end}
        data-total-height={plainTotalHeight}
      >
        <div class="lyrics-spacer" style={`height:${plainWindow.beforeHeight}px`} aria-hidden="true"></div>
        {#each plainRenderedRows as rowIndex (rowIndex)}
          {@const line = lines[rowIndex]}
          <div class="lyric-line plain-lyric-line" class:empty={plainLayout.heights[rowIndex] === 0} data-lyric-index={rowIndex} use:observeLyricRow={{ index: rowIndex, generation: plainLayout.generation }} style={`--lyric-gap-after:${plainLayout.gaps[rowIndex]}px`}>
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
            <button type="button" class="soft-button" data-testid="lyrics-remove" onclick={removeLyrics}>{tr("不再搜尋")}</button>
            <button type="button" class="soft-button" data-testid="lyrics-dismiss" onclick={dismissSelection}>{tr("關閉")}</button>
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
    font-family: var(--lyric-font-family, inherit);
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
    /* Rows and the column's other contents cannot invalidate (or be invalidated by) the rest of the page. */
    contain: layout paint;
    min-height: 168px;
    max-height: min(56vh, 620px);
    flex: 1 1 auto;
    /* Never scrolls sideways (rows wrap to the width). Hidden at all times, not only while frozen, so the freeze ending
       changes no scroller property (the scroll layer stays as it is; ADR 0036). */
    overflow-x: hidden;
    overflow-y: auto;
    overscroll-behavior: contain;
    -webkit-mask-image: linear-gradient(to bottom, transparent 0%, #000 24%, #000 76%, transparent 100%);
    mask-image: linear-gradient(to bottom, transparent 0%, #000 24%, #000 76%, transparent 100%);
    scrollbar-color: color-mix(in srgb, var(--accent) 42%, transparent) transparent;
    scrollbar-width: thin;
  }

  /* The cover is being zoomed: the rows keep the text layout of the last settled width and the narrowing column clips them
     (layout only; the viewport already clips sideways). */
  .lyrics-lines-viewport[data-layout-frozen] .lyric-line {
    width: var(--lyrics-frozen-width, 100%);
    max-width: none;
    flex-shrink: 0;
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

  /* Browse and seek (ADR 0029): the play button of the centre line while browsing sits in a gutter that every timed
     row always reserves on BOTH sides (symmetric, so centred text stays centred), so the button never covers text and
     rows never reflow when it appears. The rows are border-box, so their measured heights, the karaoke word widths and
     the cover-zoom frozen width all include the gutter as they are. */
  .timed-lyrics-viewport {
    --lyric-browse-button: 22px;
    --lyric-browse-inset: 1px;
    --lyric-browse-gutter: 24px;
  }

  .timed-lyrics-viewport .lyric-line {
    position: relative;
    padding-inline: var(--lyric-browse-gutter);
  }

  .lyric-browse-seek {
    position: absolute;
    z-index: 1;
    top: 50%;
    left: var(--lyric-browse-inset);
    display: inline-flex;
    box-sizing: border-box;
    width: var(--lyric-browse-button);
    height: var(--lyric-browse-button);
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 0;
    border-radius: 6px;
    color: var(--accent);
    background: transparent;
    line-height: 1;
    cursor: pointer;
    transform: translateY(-50%);
    animation: lyric-browse-seek-in 200ms ease-out;
  }

  .lyric-browse-seek:hover {
    background: rgba(var(--text-rgb), 0.1);
  }

  .lyric-browse-seek:focus-visible {
    outline-offset: -1px;
  }

  @keyframes lyric-browse-seek-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @media (prefers-reduced-motion: reduce) {
    .lyric-browse-seek {
      animation: none;
    }
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
    font-weight: var(--lyric-primary-weight, 500);
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

  /* Word highlight (ADR 0041 D10), karaoke style: ONE accent edge per line. The line's words sit in one inline span
     whose background (accent up to `--lyric-line-fill`, the line's own colour after it, a soft edge of
     `--lyric-line-edge` on each side) is clipped to the glyphs, with the text fill transparent while the animation
     runs. `box-decoration-break: slice` lays the fragments of a wrapped line side by side in one background strip,
     so the edge runs over the rows in reading order. The registered `--lyric-line-fill` (app.css) goes 0% -> 100% in
     one animation whose `linear()` timing function is the karaoke glide (lyrics-word-highlight.js, set by
     `karaokeLine`), fill mode forwards: sung text stays accent until the line changes, text not yet sung keeps the
     line colour (before the animation starts the opaque text fill covers the background). No filter on the span: the
     shadow mode's drop-shadow stays on the line, and the outline (`-webkit-text-stroke`) is drawn over the fill.
     The word spans inside only serve to measure the words. */
  .lyric-karaoke {
    background-image: linear-gradient(
      90deg,
      var(--accent) calc(var(--lyric-line-fill) - var(--lyric-line-edge)),
      var(--text) calc(var(--lyric-line-fill) + var(--lyric-line-edge))
    );
    -webkit-background-clip: text;
    background-clip: text;
    -webkit-box-decoration-break: slice;
    box-decoration-break: slice;
    --lyric-line-edge: 0.15em;
    animation-timing-function: linear;
    animation-iteration-count: 1;
    animation-fill-mode: forwards;
  }

  .words-paused .lyric-karaoke {
    animation-play-state: paused;
  }

  /* Global: the name is written inline by `karaokeLineAnimation`, so it must not be Svelte-scoped. */
  @keyframes -global-lyric-line-wipe {
    from {
      --lyric-line-fill: 0%;
      color: transparent;
    }
    to {
      --lyric-line-fill: 100%;
      color: transparent;
    }
  }

  /* Reduced motion: no sweep (the timing function steps at each word start, `karaokeLineAnimation`) and a hard edge,
     so each word turns wholly accent when it starts and stays accent. */
  @media (prefers-reduced-motion: reduce) {
    .lyric-karaoke {
      --lyric-line-edge: 0em;
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
    font-weight: var(--lyric-auxiliary-weight, 500);
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
    outline: var(--focus-ring);
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
