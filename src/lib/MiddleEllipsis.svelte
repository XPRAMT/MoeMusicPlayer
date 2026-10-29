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
  }

  let { text, mode = 'middle', maxWidth, fontContext }: Props = $props();
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

<span class="middle-ellipsis" bind:this={host}>{shown || text}</span>

<style>
  .middle-ellipsis {
    display: block;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }
</style>
