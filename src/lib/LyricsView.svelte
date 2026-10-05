<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { invokeCommand } from './ipc';
  import type {
    LyricsCandidate,
    LyricsPreferences,
    LyricsTrackResult,
    PlaybackState,
    TrackLyrics,
  } from './ipc';
  import { createLyricsController } from './lyrics-controller.js';
  import { getCandidatePresentation } from './lyrics-candidate-preview.js';
  import {
    DEFAULT_LYRICS_PREFERENCES,
    getDisplayActiveLyricIndex,
    getLyricLineOpacity,
    normalizeLyricsPreferences,
  } from './lyrics-preferences.js';
  import {
    buildLyricsLayout,
    buildTimedLyricTimeline,
    findLyricsRowAtOffset,
    findActiveLyricIndex,
    getPlainLyricWindow,
    getTimedLyricWindow,
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
    lyricsTimingDelaySecToOffsetMs,
    saveLyricsTimingOffsetSec,
  } from './lyrics-timing-offset.js';

  type LyricsApi = {
    getTrack: (args: { trackId: string }) => Promise<LyricsTrackResult>;
    search: (args: { trackId: string; requestId: string }) => Promise<LyricsTrackResult>;
    selectCandidate: (args: { trackId: string; candidateId: string }) => Promise<TrackLyrics>;
    cancelSearch: (args: { requestId: string }) => Promise<void>;
  };

  type LyricsViewState = {
    trackId: string | null;
    phase: 'idle' | 'loading' | 'ready' | 'empty' | 'candidates' | 'searching' | 'error';
    lyrics: TrackLyrics | null;
    candidates: LyricsCandidate[];
    error: string | null;
    isSearching: boolean;
    selectingCandidateId: string | null;
    failedStage: 'load' | 'search' | 'selection' | null;
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
    isPlaying: boolean;
    playbackState?: PlaybackState;
    lyricsPreferences?: LyricsPreferences;
    onPreferencesChange?: (patch: Partial<LyricsPreferences>) => void;
    onStatusChange?: (status: LyricsTopbarStatus | null) => void;
    api?: LyricsApi;
  }

  const defaultLyricsApi: LyricsApi = {
    getTrack: (args) => invokeCommand('lyrics_get_track', args),
    search: (args) => invokeCommand('lyrics_search', args),
    selectCandidate: (args) => invokeCommand('lyrics_select_candidate', args),
    cancelSearch: (args) => invokeCommand('lyrics_cancel_search', args),
  };

  let {
    trackId,
    positionMs = 0,
    isPlaying = false,
    playbackState,
    lyricsPreferences = DEFAULT_LYRICS_PREFERENCES,
    onPreferencesChange = () => {},
    onStatusChange,
    api = defaultLyricsApi,
  }: Props = $props();

  let viewState = $state<LyricsViewState>({
    trackId: null,
    phase: 'idle',
    lyrics: null,
    candidates: [],
    error: null,
    isSearching: false,
    selectingCandidateId: null,
    failedStage: null,
  });
  let timedViewport = $state<HTMLDivElement | null>(null);
  let plainViewport = $state<HTMLDivElement | null>(null);
  let plainScrollTop = $state(0);
  let plainViewportHeight = $state(480);
  let timedViewportHeight = $state(480);
  let layoutWidth = $state(0);
  let layoutRevision = $state(0);
  let previousTrackId: string | null | undefined;
  let previousPlainLayout: ReturnType<typeof buildLyricsLayout> | null = null;
  let previousTimedNavigation: { trackId: string | null; generation: number; index: number; viewport: HTMLDivElement; height: number } | null = null;
  let timedSmoothScrolling = false;
  let timedUserScrolling = false;
  let userScrollResetTimer: ReturnType<typeof setTimeout> | null = null;
  let timingOffsetSec = $state(0);
  let timingPanelOpen = $state(false);

  const controller = createLyricsController({
    api: {
      getTrack: (args) => api.getTrack(args),
      search: (args) => api.search(args),
      selectCandidate: (args) => api.selectCandidate(args),
      cancelSearch: (args) => api.cancelSearch(args),
    },
    onChange(nextState: LyricsViewState) {
      viewState = nextState;
    },
  });

  let lines = $derived(viewState.lyrics?.lines ?? []);
  let preferences = $derived(normalizeLyricsPreferences(lyricsPreferences));
  let effectivePlaybackState = $derived(playbackState ?? (isPlaying ? 'playing' : 'paused'));
  let timedLines = $derived(
    viewState.lyrics?.synced ? buildTimedLyricTimeline(lines) : [],
  );
  let activeTimedIndex = $derived(
    findActiveLyricIndex(
      timedLines,
      positionMs,
      (viewState.lyrics?.offsetMs ?? 0) + lyricsTimingDelaySecToOffsetMs(timingOffsetSec),
    ),
  );
  let displayActiveTimedIndex = $derived(
    getDisplayActiveLyricIndex(activeTimedIndex, effectivePlaybackState),
  );
  let timedLayout = $derived(buildLyricsLayout(timedLines, preferences, preferences.lineGapPx, layoutWidth));
  let visibleActiveTimedIndex = $derived(
    displayActiveTimedIndex >= 0 && timedLayout.heights[displayActiveTimedIndex] > 0
      ? displayActiveTimedIndex
      : -1,
  );
  let plainLayout = $derived(buildLyricsLayout(lines, preferences, preferences.lineGapPx, layoutWidth));
  let timedTotalHeight = $derived.by(() => { layoutRevision; return timedLayout.totalHeight; });
  let timedCanvasHeight = $derived(timedTotalHeight + timedViewportHeight);
  let timedEdgePadding = $derived(timedViewportHeight / 2);
  let plainTotalHeight = $derived.by(() => { layoutRevision; return plainLayout.totalHeight; });
  let timedWindow = $derived.by(() => {
    layoutRevision;
    return getTimedLyricWindow(timedLayout, activeTimedIndex);
  });
  let timedRenderedRows = $derived(timedWindow.rows.filter((index) => timedLayout.heights[index] > 0));
  let plainWindow = $derived.by(() => {
    layoutRevision;
    return getPlainLyricWindow(plainLayout, plainScrollTop, plainViewportHeight);
  });
  let plainRenderedRows = $derived(plainWindow.rows.filter((index) => plainLayout.heights[index] > 0));
  let isTimed = $derived(timedLines.length > 0);

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

  const rowObserver = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver((entries) => {
    const layout = isTimed ? timedLayout : plainLayout;
    const viewport = isTimed ? timedViewport : plainViewport;
    if (!viewport) return;
    const anchorIndex = isTimed ? -1 : findLyricsRowAtOffset(layout, viewport.scrollTop);
    const anchorOffset = anchorIndex < 0 ? 0 : viewport.scrollTop - getLyricsOffset(layout, anchorIndex);
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
    if (anchorIndex >= 0 && !isTimed) {
      viewport.scrollTop = getLyricsOffset(layout, anchorIndex) + anchorOffset;
      plainScrollTop = viewport.scrollTop;
    }
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
      const anchorIndex = isTimed ? -1 : findLyricsRowAtOffset(layout, viewport.scrollTop);
      const anchorOffset = anchorIndex < 0 ? 0 : viewport.scrollTop - getLyricsOffset(layout, anchorIndex);
      let changed = false;
      for (const row of viewport.querySelectorAll<HTMLElement>(`.lyric-line[data-layout-generation="${generation}"]`)) {
        changed = updateLyricsRowHeight(layout, Number(row.dataset.layoutIndex), row.getBoundingClientRect().height) || changed;
      }
      if (!changed) return;
      layoutRevision += 1;
      if (isTimed) correctTimedCenter();
      if (anchorIndex >= 0 && !isTimed) {
        viewport.scrollTop = getLyricsOffset(layout, anchorIndex) + anchorOffset;
        plainScrollTop = viewport.scrollTop;
      }
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
    const source = viewState.lyrics ? sourceLabel(viewState.lyrics.source) : '尚未載入';
    const sync = viewState.lyrics ? (isTimed ? '同步歌詞' : '純歌詞') : '沒有歌詞';
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
    timingOffsetSec = loadLyricsTimingOffsetSec(nextTrackId);
    void controller.setTrack(nextTrackId);
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
    const activeIndex = activeTimedIndex;
    const layout = timedLayout;
    const currentTrackId = trackId;
    const currentlyPlaying = isPlaying && effectivePlaybackState === 'playing';
    const viewportHeight = timedViewportHeight;
    if (!element) return;

    const previous = previousTimedNavigation;
    const sameTrackAndLayout = previous?.trackId === currentTrackId && previous.generation === layout.generation;
    if (
      sameTrackAndLayout
      && previous?.index === activeIndex
      && previous.viewport === element
      && previous.height === viewportHeight
    ) return;
    const isNormalCueAdvance = Boolean(currentlyPlaying && sameTrackAndLayout && previous?.index >= 0 && activeIndex === previous.index + 1);
    previousTimedNavigation = { trackId: currentTrackId, generation: layout.generation, index: activeIndex, viewport: element, height: viewportHeight };

    const frame = requestAnimationFrame(() => {
      const reducedMotion = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
      const behavior: ScrollBehavior = isNormalCueAdvance && !reducedMotion ? 'smooth' : 'instant';
      const targetTop = timedCueScrollTop(layout, activeIndex, viewportHeight, element.clientHeight);
      timedSmoothScrolling = behavior === 'smooth' && Math.abs(element.scrollTop - targetTop) > 1;
      element.scrollTo({ top: targetTop, behavior });
    });
    return () => cancelAnimationFrame(frame);
  });

  $effect(() => {
    const element = timedViewport;
    if (!element) return;
    const onScrollEnd = () => {
      const shouldCorrectCenter = timedSmoothScrolling && !timedUserScrolling;
      timedSmoothScrolling = false;
      timedUserScrolling = false;
      if (userScrollResetTimer) clearTimeout(userScrollResetTimer);
      userScrollResetTimer = null;
      if (shouldCorrectCenter) correctTimedCenter();
    };
    const onUserScroll = () => {
      timedSmoothScrolling = false;
      timedUserScrolling = true;
      if (userScrollResetTimer) clearTimeout(userScrollResetTimer);
      userScrollResetTimer = setTimeout(() => { timedUserScrolling = false; userScrollResetTimer = null; }, 300);
    };
    element.addEventListener('scrollend', onScrollEnd);
    element.addEventListener('wheel', onUserScroll, { passive: true });
    element.addEventListener('pointerdown', onUserScroll, { passive: true });
    element.addEventListener('touchstart', onUserScroll, { passive: true });
    return () => {
      element.removeEventListener('scrollend', onScrollEnd);
      element.removeEventListener('wheel', onUserScroll);
      element.removeEventListener('pointerdown', onUserScroll);
      element.removeEventListener('touchstart', onUserScroll);
      if (userScrollResetTimer) clearTimeout(userScrollResetTimer);
      userScrollResetTimer = null;
    };
  });

  onDestroy(() => { onStatusChange?.(null); controller.dispose(); rowObserver?.disconnect(); });

  function sourceLabel(source: TrackLyrics['source']): string {
    switch (source) {
      case 'local': return '本機 LRC';
      case 'embedded': return '內嵌歌詞';
      case 'netease': return '網易雲';
      case 'qqmusic': return 'QQ 音樂';
      case 'manual': return '手動指定';
    }
  }

  function providerLabel(provider: LyricsCandidate['provider']): string {
    return provider === 'netease' ? '網易雲' : 'QQ 音樂';
  }

  function confidenceLabel(confidence: LyricsCandidate['confidence']): string {
    switch (confidence) {
      case 'high': return '高信心';
      case 'medium': return '中信心';
      case 'low': return '低信心';
    }
  }

  function formatCandidateDuration(durationMs: number | null): string {
    if (durationMs === null || !Number.isFinite(durationMs) || durationMs < 0) return '長度未知';
    const totalSeconds = Math.floor(durationMs / 1000);
    return `${Math.floor(totalSeconds / 60)}:${String(totalSeconds % 60).padStart(2, '0')}`;
  }

  function retry(): void {
    void controller.retry();
  }

  function searchAgain(): void {
    void controller.searchAgain();
  }

  function cancelSearch(): void {
    controller.cancelSearch();
  }

  function selectCandidate(candidateId: string): void {
    void controller.selectCandidate(candidateId);
  }

  function setTimingOffsetSec(next: number): void {
    timingOffsetSec = saveLyricsTimingOffsetSec(trackId, next);
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
</script>

<div
  class="lyrics-view"
  data-testid="lyrics-view"
  data-track-id={viewState.trackId ?? ''}
  data-phase={viewState.phase}
  data-playing={isPlaying ? 'true' : 'false'}
  data-playback-state={effectivePlaybackState}
  data-line-gap={preferences.lineGapPx}
  data-active-cue-index={activeTimedIndex}
  data-layout-revision={layoutRevision}
  style={`--lyric-primary-font-size:${preferences.primaryFontSizePx}px;--lyric-auxiliary-font-size:${preferences.auxiliaryFontSizePx}px`}
>
  {#if !trackId}
    <p class="lyrics-placeholder" role="status">播放歌曲後會在此顯示歌詞。</p>
  {:else if viewState.phase === 'loading' && !viewState.lyrics}
    <p class="lyrics-placeholder" role="status">正在讀取本機與已保存的歌詞…</p>
  {:else}
    {#if timingPanelOpen && isTimed}
      <div class="lyrics-timing-panel" role="group" aria-label="歌詞同步延遲" data-testid="lyrics-timing-panel">
        <div class="lyrics-timing-panel-header">
          <strong>同步歌詞延遲</strong>
          <span data-testid="lyrics-timing-value">{formatLyricsTimingOffsetSec(timingOffsetSec)}</span>
          <button type="button" class="lyrics-action secondary" onclick={() => { timingPanelOpen = false; }}>關閉</button>
        </div>
        <div class="lyrics-timing-controls">
          <button type="button" class="lyrics-action secondary" aria-label="延遲減少 0.1 秒" onclick={() => nudgeTimingOffset(-LYRICS_TIMING_OFFSET_STEP_SEC)}>−0.1</button>
          <input
            class="lyrics-timing-slider"
            type="range"
            min={LYRICS_TIMING_OFFSET_MIN_SEC}
            max={LYRICS_TIMING_OFFSET_MAX_SEC}
            step={LYRICS_TIMING_OFFSET_STEP_SEC}
            value={timingOffsetSec}
            aria-label="歌詞延遲秒數"
            aria-valuemin={LYRICS_TIMING_OFFSET_MIN_SEC}
            aria-valuemax={LYRICS_TIMING_OFFSET_MAX_SEC}
            aria-valuenow={timingOffsetSec}
            oninput={onTimingOffsetInput}
          />
          <button type="button" class="lyrics-action secondary" aria-label="延遲增加 0.1 秒" onclick={() => nudgeTimingOffset(LYRICS_TIMING_OFFSET_STEP_SEC)}>+0.1</button>
          <button type="button" class="lyrics-action secondary" onclick={resetTimingOffset}>重設</button>
        </div>
        <p class="lyrics-timing-hint">正值延後歌詞，負值提前歌詞；範圍 ±5 秒，步進 0.1 秒。</p>
      </div>
    {/if}
    {#if viewState.lyrics && isTimed}
      <div
        class="lyrics-lines-viewport timed-lyrics-viewport"
        role="region"
        aria-label="同步歌詞"
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
            {#if row.line.text?.trim()}<p class="lyric-primary">{row.line.text}</p>{/if}
            {#if preferences.showTranslation && row.line.translation?.trim()}<p class="lyric-translation">{row.line.translation}</p>{/if}
            {#if preferences.showRomanization && row.line.romanization?.trim()}<p class="lyric-romanization">{row.line.romanization}</p>{/if}
          </div>
        {/each}
        <div class="lyrics-spacer" data-virtual-spacer="after" style={`height:${timedWindow.afterHeight}px`} aria-hidden="true"></div>
        <div class="lyrics-spacer lyrics-edge-spacer" data-edge-spacer="tail" style={`height:${timedEdgePadding}px`} aria-hidden="true"></div>
      </div>
    {:else if viewState.lyrics && lines.length > 0}
      <div
        class="lyrics-lines-viewport plain-lyrics-viewport"
        role="region"
        aria-label="純歌詞，可捲動瀏覽"
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
            {#if preferences.showTranslation && line.translation?.trim()}<p class="lyric-translation">{line.translation}</p>{/if}
            {#if preferences.showRomanization && line.romanization?.trim()}<p class="lyric-romanization">{line.romanization}</p>{/if}
          </div>
        {/each}
        <div class="lyrics-spacer" style={`height:${plainWindow.afterHeight}px`} aria-hidden="true"></div>
      </div>
    {:else if viewState.lyrics}
      <p class="lyrics-placeholder" role="status">這份歌詞沒有可顯示的文字。</p>
    {/if}

    {#if viewState.error}
      <div class="lyrics-message lyrics-error" role="alert">
        <span>{viewState.error}</span>
        {#if viewState.failedStage === 'load'}
          <button type="button" class="lyrics-action" onclick={retry}>重試讀取</button>
          <button type="button" class="lyrics-action secondary" onclick={searchAgain}>線上搜尋</button>
        {:else if viewState.failedStage === 'selection'}
          <button type="button" class="lyrics-action" onclick={retry}>重試指定</button>
        {:else}
          <button type="button" class="lyrics-action" onclick={searchAgain}>重試搜尋</button>
        {/if}
      </div>
    {/if}

    {#if viewState.isSearching}
      <div class="lyrics-message lyrics-searching" role="status">
        <span>正在搜尋網易雲與 QQ 音樂歌詞…</span>
        <button type="button" class="lyrics-action secondary" onclick={cancelSearch}>取消搜尋</button>
      </div>
    {:else if viewState.candidates.length > 0}
      <section class="lyrics-candidates" aria-label="歌詞候選">
        <h4>選擇歌詞</h4>
        <ul>
          {#each viewState.candidates as candidate (candidate.id)}
            {@const presentation = getCandidatePresentation(candidate)}
            <li class="lyrics-candidate">
              <div class="candidate-copy">
                <strong>{candidate.title}</strong>
                <span>{candidate.artist}{candidate.album ? ` · ${candidate.album}` : ''}</span>
                <small>{providerLabel(candidate.provider)} · {confidenceLabel(candidate.confidence)} · 分數 {candidate.score.toFixed(2)} · {formatCandidateDuration(candidate.durationMs)} · {presentation.formatLabel}</small>
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
                class="lyrics-action"
                disabled={viewState.selectingCandidateId !== null}
                onclick={() => selectCandidate(candidate.id)}
              >
                {viewState.selectingCandidateId === candidate.id ? '套用中…' : '使用這份'}
              </button>
            </li>
          {/each}
        </ul>
      </section>
    {:else if !viewState.lyrics && !viewState.error && viewState.phase === 'empty'}
      <div class="lyrics-message lyrics-empty" role="status">
        <span>找不到符合的歌詞。</span>
        <button type="button" class="lyrics-action secondary" onclick={searchAgain}>再次搜尋</button>
      </div>
    {/if}
  {/if}
</div>

<style>
  .lyrics-view {
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
    font-size: 12px;
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

  .lyric-line {
    display: flex;
    box-sizing: border-box;
    width: 100%;
    height: auto;
    flex: 0 0 auto;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    overflow: hidden;
    padding: 7px 9px;
    margin-bottom: var(--lyric-gap-after, 0px);
    border-bottom: 0;
    color: var(--muted);
    transition: color 140ms ease;
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

  .lyric-primary,
  .lyric-translation,
  .lyric-romanization {
    display: -webkit-box;
    min-width: 0;
    margin: 0;
    overflow: hidden;
    -webkit-box-orient: vertical;
    text-overflow: ellipsis;
    text-align: center;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.82), 0 0 8px rgba(0, 0, 0, 0.38);
    opacity: var(--lyric-text-opacity, 1);
    transition: opacity 140ms ease;
  }

  .lyric-primary {
    box-sizing: border-box;
    max-height: calc(var(--lyric-primary-font-size, 14px) * 1.35 * 2);
    flex: 0 1 auto;
    color: inherit;
    font-size: var(--lyric-primary-font-size, 14px);
    font-weight: 550;
    line-height: 1.35;
    line-clamp: 2;
    -webkit-line-clamp: 2;
  }

  .lyric-translation,
  .lyric-romanization {
    box-sizing: border-box;
    max-height: calc(var(--lyric-auxiliary-font-size, 10px) * 1.25);
    flex: 0 1 auto;
    color: var(--muted);
    font-size: var(--lyric-auxiliary-font-size, 10px);
    line-height: 1.25;
    line-clamp: 1;
    -webkit-line-clamp: 1;
  }

  .lyrics-message {
    display: flex;
    min-width: 0;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    color: var(--muted);
    font-size: 11px;
  }

  .lyrics-error {
    color: var(--status-danger, #ff8b88);
  }

  .lyrics-action {
    flex: 0 0 auto;
    padding: 6px 10px;
    border: 1px solid color-mix(in srgb, var(--accent) 45%, var(--line));
    border-radius: 7px;
    color: var(--accent-text);
    background: color-mix(in srgb, var(--accent) 9%, transparent);
    font: inherit;
    cursor: pointer;
  }

  .lyrics-action.secondary {
    border-color: var(--line);
    color: var(--text-soft);
    background: transparent;
  }

  .lyrics-action:disabled {
    cursor: wait;
    opacity: 0.65;
  }

  .lyrics-candidates {
    min-height: 0;
    max-height: min(50vh, 480px);
    overflow: auto;
    border-top: 1px solid var(--line);
  }

  .lyrics-candidates h4 {
    position: sticky;
    top: 0;
    margin: 0;
    padding: 8px 0;
    color: var(--text-soft);
    background: var(--panel, #111);
    font-size: 11px;
    font-weight: 600;
  }

  .lyrics-candidates ul {
    display: grid;
    gap: 7px;
    margin: 0;
    padding: 0 0 4px;
    list-style: none;
  }

  .lyrics-candidate {
    display: flex;
    min-width: 0;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 9px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: color-mix(in srgb, var(--text) 2%, transparent);
  }

  .candidate-copy {
    display: grid;
    min-width: 0;
    gap: 3px;
  }

  .candidate-copy strong,
  .candidate-copy span,
  .candidate-copy small {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .candidate-copy strong {
    color: var(--text);
    font-size: 11px;
  }

  .candidate-copy span,
  .candidate-copy small {
    color: var(--muted);
    font-size: 9px;
  }

  .candidate-preview {
    color: var(--text-soft) !important;
  }

  .candidate-preview-note {
    color: var(--muted);
    font-size: 10px;
    line-height: 1.5;
  }

  .lyrics-timing-panel {
    display: grid;
    gap: 8px;
    padding: 10px 10px 8px;
    border: 1px solid var(--line);
    border-radius: 10px;
    background: color-mix(in srgb, var(--panel, #111) 88%, transparent);
  }

  .lyrics-timing-panel-header {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 8px;
  }

  .lyrics-timing-panel-header strong {
    color: var(--text-soft);
    font-size: 11px;
    font-weight: 650;
  }

  .lyrics-timing-panel-header span {
    margin-right: auto;
    color: var(--text);
    font-size: 12px;
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

  .lyrics-timing-hint {
    margin: 0;
    color: var(--muted);
    font-size: 10px;
    line-height: 1.5;
  }

  @media (max-width: 560px) {
    .lyrics-candidate {
      align-items: flex-start;
    }

    .lyrics-action {
      padding: 6px 8px;
    }

    .lyrics-timing-controls {
      flex-wrap: wrap;
    }
  }
</style>
