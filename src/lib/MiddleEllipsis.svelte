<script lang="ts">
  import { onMount } from 'svelte';
  import {
    ellipsisStats,
    observeHostWidth,
    onEllipsisFontsInvalidated,
    resolveHostFont,
    sharedFitter,
  } from './ellipsis-fit.js';
  import type { TrackTextOverflowMode } from './track-text-overflow-context';

  interface Props {
    text: string;
    mode?: TrackTextOverflowMode;
    /** Available width in CSS px, when the caller knows it; otherwise the width comes from the shared ResizeObserver. */
    maxWidth?: number;
    /** Cells that draw with the same font share one font lookup; defaults to the parent/grandparent class names. */
    fontContext?: string;
    /**
     * Set when the host is sized by its content (an inline-flex/shrink-to-fit parent). The host then gets the full
     * text's width as its preferred width, so the observed width is the room really available and never the width of
     * the already shortened text (which would keep it shortened forever once the room grows).
     */
    contentSized?: boolean;
  }

  let { text, mode = 'middle', maxWidth, fontContext, contentSized = false }: Props = $props();
  let fullWidth = $state<number | null>(null);
  let host = $state<HTMLElement | null>(null);
  let shown = $state('');

  // None of these is reactive: the font comes from a per-context cache, the width from the shared ResizeObserver, and
  // layout() only fits again when text, mode, width or font actually differ from the last fit.
  let observedWidth: number | null = null;
  let lastText: string | null = null;
  let lastMode = '';
  let lastWidth = -1;
  let lastFont: string | null = null;

  function layout(): void {
    if (!host) return;
    const font = resolveHostFont(host, fontContext);
    if (contentSized) {
      const next = text ? Math.ceil(sharedFitter.measureWidth(font, text)) : null;
      if (next !== fullWidth) fullWidth = next;
    }
    let width = maxWidth ?? observedWidth;
    if (width === null || width === undefined) {
      if (typeof ResizeObserver !== 'undefined') return; // the observer's first callback supplies it
      width = host.clientWidth;
      if (ellipsisStats) ellipsisStats.layoutReads++;
    }
    if (text === lastText && mode === lastMode && width === lastWidth && font === lastFont) return;
    lastText = text;
    lastMode = mode;
    lastWidth = width;
    lastFont = font;
    if (ellipsisStats) ellipsisStats.layoutRuns++;
    shown = sharedFitter.fit(text, mode, font, width);
    scheduleDomCheck();
  }

  // Sanity guard: canvas and DOM can still disagree (font fallback per glyph, a face still loading). After the text
  // is drawn, trust the DOM: a content-sized host takes the real width; any other host fits again in the real room.
  let domCheckScheduled = false;
  function scheduleDomCheck(): void {
    if (domCheckScheduled || typeof requestAnimationFrame === 'undefined') return;
    domCheckScheduled = true;
    requestAnimationFrame(() => {
      domCheckScheduled = false;
      if (!host) return;
      const overflow = host.scrollWidth - host.clientWidth;
      if (overflow <= 1) return;
      if (contentSized && shown === text) {
        if (fullWidth === null || host.scrollWidth > fullWidth) fullWidth = host.scrollWidth;
        return;
      }
      const room = host.clientWidth - overflow;
      if (room > 0 && lastFont !== null) shown = sharedFitter.fit(text, mode, lastFont, room);
    });
  }

  $effect(() => {
    text;
    mode;
    maxWidth;
    layout();
  });

  onMount(() => {
    if (!host) return;
    if (ellipsisStats) ellipsisStats.mounts++;
    const stopFonts = onEllipsisFontsInvalidated(layout);
    if (typeof ResizeObserver === 'undefined') {
      layout();
      return stopFonts;
    }
    const stopObserving = observeHostWidth(host, (width) => {
      observedWidth = width;
      layout();
    });
    return () => {
      if (ellipsisStats) ellipsisStats.destroys++;
      stopObserving();
      stopFonts();
    };
  });
</script>

<span class="middle-ellipsis" bind:this={host} style:width={contentSized && fullWidth !== null ? `${fullWidth}px` : undefined}>{shown || text}</span>

<style>
  .middle-ellipsis {
    display: block;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }
</style>
