<script lang="ts" generics="Item">
  import { onMount } from 'svelte';
  import type { Snippet } from 'svelte';
  import {
    buildVirtualRows,
    getNextActiveIndex,
    getVirtualRange,
    type VirtualRange,
  } from './track-list-data.js';

  export interface VirtualListRow<Item> {
    item: Item | null;
    index: number;
    key: string | number;
    id: string;
    active: boolean;
    selected: boolean;
    play: () => void;
  }

  interface Props<Item> {
    totalCount: number | null;
    pending?: boolean;
    revision: number;
    generation: number;
    itemAt: (index: number) => Item | null;
    getKey: (item: Item, index: number) => string | number;
    isSelected?: (item: Item, index: number) => boolean;
    rowHeight: number;
    compactRowHeight?: number;
    shortDesktopRowHeight?: number;
    headerHeight: number;
    compactHeaderHeight?: number;
    maxViewportHeight: number;
    overscan?: number;
    listId: string;
    ariaLabel: string;
    columnCount: number;
    className?: string;
    rowClassName?: string;
    listName: string;
    onRange: (range: VirtualRange) => void;
    onPlay: (item: Item, index: number) => void;
    header?: Snippet;
    row: Snippet<[VirtualListRow<Item>]>;
  }

  let {
    totalCount,
    pending = false,
    revision,
    generation,
    itemAt,
    getKey,
    isSelected = () => false,
    rowHeight: baseRowHeight,
    compactRowHeight = baseRowHeight,
    shortDesktopRowHeight = baseRowHeight,
    headerHeight: baseHeaderHeight,
    compactHeaderHeight = baseHeaderHeight,
    maxViewportHeight: maxHeight,
    overscan = 8,
    listId,
    ariaLabel,
    columnCount,
    className = '',
    rowClassName = '',
    listName,
    onRange,
    onPlay,
    header,
    row,
  }: Props<Item> = $props();

  let viewport: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let viewportHeight = $state(480);
  let activeIndex = $state(0);
  let rowHeight = $state(57);
  let headerHeight = $state(37);
  let maxViewportHeight = $state(680);
  let seenGeneration = $state(0);

  const virtualTotalCount = $derived(totalCount ?? 40);
  const viewportPixelHeight = $derived(Math.max(
    headerHeight + rowHeight,
    Math.min(maxViewportHeight, headerHeight + virtualTotalCount * rowHeight),
  ));
  const range = $derived(getVirtualRange(
    scrollTop,
    viewportHeight,
    virtualTotalCount,
    rowHeight,
    overscan,
    headerHeight,
  ));
  const virtualRows = $derived.by(() => buildVirtualRows(
    { itemAt },
    range,
    revision,
    getKey,
  ).rows);
  const activeDescendantId = $derived(`${listId}-row-${activeIndex}`);

  $effect(() => {
    const currentGeneration = generation;
    if (currentGeneration !== seenGeneration) {
      seenGeneration = currentGeneration;
      activeIndex = 0;
      scrollTop = 0;
      viewport?.scrollTo({ top: 0 });
    }
    const currentRange = range;
    onRange(currentRange);
  });

  onMount(() => {
    const updateMetrics = () => {
      const narrow = window.matchMedia('(max-width: 620px)').matches;
      const shortDesktop = window.matchMedia('(max-height: 680px) and (min-width: 621px)').matches;
      rowHeight = narrow ? compactRowHeight : shortDesktop ? shortDesktopRowHeight : baseRowHeight;
      headerHeight = narrow ? compactHeaderHeight : baseHeaderHeight;
      maxViewportHeight = Math.max(160, Math.min(maxHeight, Math.floor(window.innerHeight * 0.65)));
      viewportHeight = viewport?.clientHeight ?? viewportHeight;
    };
    updateMetrics();
    window.addEventListener('resize', updateMetrics);
    return () => window.removeEventListener('resize', updateMetrics);
  });

  function updateViewport(): void {
    if (!viewport) return;
    scrollTop = viewport.scrollTop;
    viewportHeight = viewport.clientHeight;
  }

  function setActiveIndex(index: number): void {
    const maxIndex = Math.max(0, virtualTotalCount - 1);
    activeIndex = Math.max(0, Math.min(maxIndex, index));
    if (!viewport) return;
    const rowTop = headerHeight + activeIndex * rowHeight;
    const rowBottom = rowTop + rowHeight;
    let nextScrollTop = viewport.scrollTop;
    if (rowTop < nextScrollTop + headerHeight) nextScrollTop = Math.max(0, rowTop - headerHeight);
    else if (rowBottom > nextScrollTop + viewport.clientHeight) {
      nextScrollTop = rowBottom - viewport.clientHeight;
    }
    if (nextScrollTop !== viewport.scrollTop) viewport.scrollTo({ top: nextScrollTop });
  }

  function playRow(item: Item | null, index: number): void {
    if (item === null) return;
    activeIndex = index;
    onPlay(item, index);
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      playRow(itemAt(activeIndex), activeIndex);
      return;
    }
    const nextIndex = getNextActiveIndex(
      event.key,
      activeIndex,
      totalCount ?? 40,
      viewportHeight,
      rowHeight,
    );
    if (nextIndex === null) return;
    event.preventDefault();
    setActiveIndex(nextIndex);
  }
</script>

<div
  class={`paged-virtual-viewport ${className}`}
  bind:this={viewport}
  role="grid"
  aria-label={ariaLabel}
  aria-rowcount={totalCount === null ? -1 : totalCount + 1}
  aria-colcount={columnCount}
  aria-activedescendant={activeDescendantId}
  aria-busy={pending || totalCount === null}
  tabindex="0"
  style={`height:${viewportPixelHeight}px;--paged-header-height:${headerHeight}px;--paged-row-height:${rowHeight}px`}
  onscroll={updateViewport}
  onkeydown={handleKeydown}
>
  <div class="paged-virtual-header" role="presentation" style={`height:${headerHeight}px`}>
    {#if header}{@render header()}{/if}
  </div>
  <div
    class="paged-virtual-spacer"
    role="presentation"
    aria-hidden="true"
    style={`height:${virtualTotalCount * rowHeight}px`}
  ></div>
  {#each virtualRows as virtualRow (virtualRow.key)}
    {@const id = `${listId}-row-${virtualRow.index}`}
    {@const selected = virtualRow.item !== null && isSelected(virtualRow.item, virtualRow.index)}
    <div
      id={id}
      class={`paged-virtual-row ${rowClassName}`}
      class:selected
      class:active={activeIndex === virtualRow.index}
      role="row"
      aria-rowindex={virtualRow.index + 2}
      aria-selected={selected}
      aria-label={virtualRow.item === null ? `第 ${virtualRow.index + 1} 項，正在載入` : undefined}
      style={`height:${rowHeight}px;transform:translateY(${headerHeight + virtualRow.index * rowHeight}px)`}
    >
      {@render row({
        item: virtualRow.item,
        index: virtualRow.index,
        key: virtualRow.key,
        id,
        active: activeIndex === virtualRow.index,
        selected,
        play: () => playRow(virtualRow.item, virtualRow.index),
      })}
    </div>
  {/each}
</div>

<style>
  .paged-virtual-viewport {
    position: relative;
    width: 100%;
    max-height: min(65vh, 680px);
    overflow: auto;
    overscroll-behavior: contain;
    scrollbar-gutter: stable;
    outline: none;
  }

  .paged-virtual-viewport:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }

  .paged-virtual-header {
    position: sticky;
    z-index: 2;
    top: 0;
    box-sizing: border-box;
  }

  .paged-virtual-spacer {
    width: 1px;
    pointer-events: none;
  }

  .paged-virtual-row {
    position: absolute;
    z-index: 1;
    top: 0;
    right: 0;
    left: 0;
    box-sizing: border-box;
  }

  .paged-virtual-row.active {
    background: color-mix(in srgb, var(--accent) 11%, transparent);
  }
</style>
