<script lang="ts">
  import { onDestroy } from 'svelte';
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
  } from './lyrics-window.js';

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

  interface Props {
    trackId: string | null;
    positionMs: number;
    isPlaying: boolean;
    playbackState?: PlaybackState;
    lyricsPreferences?: LyricsPreferences;
    onPreferencesChange?: (patch: Partial<LyricsPreferences>) => void;
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
  let previousTrackId: string | null | undefined;
  let previousPlainLayout: ReturnType<typeof buildLyricsLayout> | null = null;

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
    findActiveLyricIndex(timedLines, positionMs, viewState.lyrics?.offsetMs ?? 0),
  );
  let displayActiveTimedIndex = $derived(
    getDisplayActiveLyricIndex(activeTimedIndex, effectivePlaybackState),
  );
  let timedLayout = $derived(buildLyricsLayout(timedLines, preferences, preferences.lineGapPx));
  let visibleActiveTimedIndex = $derived(
    displayActiveTimedIndex >= 0 && timedLayout.heights[displayActiveTimedIndex] > 0
      ? displayActiveTimedIndex
      : -1,
  );
  let plainLayout = $derived(buildLyricsLayout(lines, preferences, preferences.lineGapPx));
  let timedWindow = $derived(getTimedLyricWindow(timedLayout, activeTimedIndex));
  let timedRenderedRows = $derived(timedWindow.rows.filter((index) => timedLayout.heights[index] > 0));
  let plainWindow = $derived(
    getPlainLyricWindow(plainLayout, plainScrollTop, plainViewportHeight),
  );
  let plainRenderedRows = $derived(plainWindow.rows.filter((index) => plainLayout.heights[index] > 0));
  let isTimed = $derived(timedLines.length > 0);

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
        const intraRowOffset = element.scrollTop - previousPlainLayout.offsets[firstVisibleIndex];
        element.scrollTop = nextLayout.offsets[Math.min(firstVisibleIndex, nextLayout.heights.length)] + intraRowOffset;
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
    if (!element) return;

    const frame = requestAnimationFrame(() => {
      const visibleIndex = activeIndex >= 0 && layout.heights[activeIndex] <= 0
        ? findLyricsRowAtOffset(layout, layout.offsets[activeIndex])
        : activeIndex;
      const targetTop = visibleIndex < 0
        ? 0
        : Math.max(0, layout.offsets[visibleIndex] + layout.heights[visibleIndex] / 2 - element.clientHeight / 2);
      element.scrollTop = targetTop;
    });
    return () => cancelAnimationFrame(frame);
  });

  onDestroy(() => controller.dispose());

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
  style={`--lyric-primary-font-size:${preferences.primaryFontSizePx}px;--lyric-auxiliary-font-size:${preferences.auxiliaryFontSizePx}px`}
>
  <div class="lyrics-panel-heading">
    <div>
      <p class="section-kicker">LYRICS</p>
      <h3 id="lyrics-heading">歌詞</h3>
    </div>
    {#if viewState.lyrics}
      <div class="lyrics-source">
        <span>{sourceLabel(viewState.lyrics.source)}</span>
        <span>{isTimed ? '同步歌詞' : '純歌詞'}</span>
      </div>
    {/if}
  </div>

  <div class="lyrics-display-controls" role="group" aria-label="歌詞副行顯示">
    <button
      type="button"
      class="lyrics-toggle"
      aria-pressed={preferences.showTranslation}
      aria-label="切換譯文顯示"
      onclick={() => onPreferencesChange({ showTranslation: !preferences.showTranslation })}
    >譯</button>
    <button
      type="button"
      class="lyrics-toggle"
      aria-pressed={preferences.showRomanization}
      aria-label="切換羅馬拼音顯示"
      onclick={() => onPreferencesChange({ showRomanization: !preferences.showRomanization })}
    >羅</button>
  </div>

  {#if !trackId}
    <p class="lyrics-placeholder" role="status">播放歌曲後會在此顯示歌詞。</p>
  {:else if viewState.phase === 'loading' && !viewState.lyrics}
    <p class="lyrics-placeholder" role="status">正在讀取本機與已保存的歌詞…</p>
  {:else}
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
        data-total-height={timedLayout.totalHeight}
      >
        <div class="lyrics-spacer" style={`height:${timedWindow.beforeHeight}px`} aria-hidden="true"></div>
        {#each timedRenderedRows as timelineIndex (timelineIndex)}
          {@const row = timedLines[timelineIndex]}
          <div
            class="lyric-line"
            class:empty={timedLayout.heights[timelineIndex] === 0}
            class:active={timelineIndex === visibleActiveTimedIndex}
            data-lyric-index={row.index}
            data-timeline-index={timelineIndex}
            style={`--lyric-text-opacity:${getLyricLineOpacity(timelineIndex, visibleActiveTimedIndex, preferences.inactiveOpacityPercent)};--lyric-row-height:${timedLayout.heights[timelineIndex]}px;--lyric-gap-after:${timedLayout.gaps[timelineIndex]}px`}
            aria-current={timelineIndex === visibleActiveTimedIndex ? 'true' : undefined}
          >
            {#if row.line.text?.trim()}<p class="lyric-primary">{row.line.text}</p>{/if}
            {#if preferences.showTranslation && row.line.translation?.trim()}<p class="lyric-translation">{row.line.translation}</p>{/if}
            {#if preferences.showRomanization && row.line.romanization?.trim()}<p class="lyric-romanization">{row.line.romanization}</p>{/if}
          </div>
        {/each}
        <div class="lyrics-spacer" style={`height:${timedWindow.afterHeight}px`} aria-hidden="true"></div>
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
        data-total-height={plainLayout.totalHeight}
      >
        <div class="lyrics-spacer" style={`height:${plainWindow.beforeHeight}px`} aria-hidden="true"></div>
        {#each plainRenderedRows as rowIndex (rowIndex)}
          {@const line = lines[rowIndex]}
          <div class="lyric-line plain-lyric-line" class:empty={plainLayout.heights[rowIndex] === 0} data-lyric-index={rowIndex} style={`--lyric-row-height:${plainLayout.heights[rowIndex]}px;--lyric-gap-after:${plainLayout.gaps[rowIndex]}px`}>
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

  .lyrics-panel-heading {
    display: flex;
    min-width: 0;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .lyrics-panel-heading h3 {
    margin: 0;
    color: var(--text);
    font-size: 16px;
    font-weight: 600;
  }

  .lyrics-source {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 5px;
    color: var(--muted);
    font-size: 10px;
  }

  .lyrics-source span {
    padding: 4px 7px;
    border: 1px solid var(--line);
    border-radius: 99px;
  }

  .lyrics-display-controls {
    display: flex;
    flex: 0 0 auto;
    gap: 6px;
  }

  .lyrics-toggle {
    min-width: 36px;
    min-height: 32px;
    padding: 0 9px;
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--muted);
    background: var(--panel, #111);
    font: inherit;
    font-size: 11px;
    font-weight: 650;
    cursor: pointer;
  }

  .lyrics-toggle[aria-pressed='true'] {
    border-color: color-mix(in srgb, var(--accent) 48%, var(--line));
    color: var(--accent-text);
    background: color-mix(in srgb, var(--accent) 12%, var(--panel, #111));
  }

  .lyrics-toggle:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
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
    scrollbar-color: color-mix(in srgb, var(--accent) 42%, transparent) transparent;
    scrollbar-width: thin;
  }

  .lyrics-spacer {
    width: 1px;
    pointer-events: none;
  }

  .lyric-line {
    display: flex;
    box-sizing: border-box;
    width: 100%;
    height: var(--lyric-row-height, auto);
    flex: 0 0 auto;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    overflow: hidden;
    padding: 7px 9px;
    margin-bottom: var(--lyric-gap-after, 0px);
    border-bottom: 1px solid rgba(var(--text-rgb), 0.045);
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
    opacity: var(--lyric-text-opacity, 1);
    transition: opacity 140ms ease;
  }

  .lyric-primary {
    box-sizing: border-box;
    height: calc(var(--lyric-primary-font-size, 14px) * 1.35 * 2);
    flex: 0 0 auto;
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
    height: calc(var(--lyric-auxiliary-font-size, 10px) * 1.25);
    flex: 0 0 auto;
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
    max-height: min(31vh, 260px);
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

  @media (max-width: 420px) {
    .lyrics-panel-heading { align-items: flex-start; }
    .lyrics-display-controls { align-self: flex-start; }
  }

  @media (max-width: 560px) {
    .lyrics-candidate {
      align-items: flex-start;
    }

    .lyrics-action {
      padding: 6px 8px;
    }
  }
</style>
