<script lang="ts">
  import { onMount } from 'svelte';
  import {
    ellipsisStats,
    hostWidthFromResizeEntry,
    onEllipsisFontsInvalidated,
    sharedFitter,
  } from './ellipsis-fit.js';
  import type { TrackTextOverflowMode } from './track-text-overflow-context';

  interface Props {
    text: string;
    mode?: TrackTextOverflowMode;
    /** Available width in CSS px, when the caller knows it; otherwise the width comes from the ResizeObserver. */
    maxWidth?: number;
  }

  let { text, mode = 'middle', maxWidth }: Props = $props();
  let host = $state<HTMLElement | null>(null);
  let shown = $state('');

  // Neither value is reactive: the font is read once per host (and again only when fonts are invalidated), the width
  // comes from the ResizeObserver entry, so scrolling and recycling rows never force a style recalc or a layout.
  let font = '';
  let observedWidth: number | null = null;

  function layout(): void {
    if (!host) return;
    if (!font) {
      font = getComputedStyle(host).font;
      if (ellipsisStats) ellipsisStats.fontReads++;
    }
    let width = maxWidth ?? observedWidth;
    if (width === null || width === undefined) {
      if (typeof ResizeObserver !== 'undefined') return; // the observer's first callback supplies it
      width = host.clientWidth;
      if (ellipsisStats) ellipsisStats.layoutReads++;
    }
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
    const stopFonts = onEllipsisFontsInvalidated(() => {
      font = '';
      layout();
    });
    if (typeof ResizeObserver === 'undefined') {
      layout();
      return stopFonts;
    }
    const observer = new ResizeObserver((entries) => {
      const entry = entries[entries.length - 1];
      if (entry) observedWidth = hostWidthFromResizeEntry(entry);
      layout();
    });
    observer.observe(host);
    return () => {
      observer.disconnect();
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
