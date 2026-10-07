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
    maxViewportHeight?: number;
    fillAvailable?: boolean;
    overscan?: number;
    listId: string;
    ariaLabel: string;
    columnCount: number;
    gridTemplate?: string;
    minContentWidth?: number;
    className?: string;
    rowClassName?: string;
    listName: string;
    onRange: (range: VirtualRange) => void;
    onPlay: (item: Item, index: number) => void;
    rowActivates?: boolean;
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
    maxViewportHeight: maxHeight = 680,
    fillAvailable = false,
    overscan = 8,
    listId,
    ariaLabel,
    columnCount,
    gridTemplate = '42px minmax(0,1fr) 42px',
    minContentWidth = 0,
    className = '',
    rowClassName = '',
    listName,
    onRange,
    onPlay,
    rowActivates = true,
    header,
    row,
  }: Props<Item> = $props();

  let frame: HTMLDivElement | undefined = $state();
  let viewport: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let headerShift = $state(0);
  let headerGutter = $state(0);
  let viewportHeight = $state(480);
  let activeIndex = $state(0);
  let rowHeight = $state(57);
  let headerHeight = $state(37);
  let maxViewportHeight = $state(680);
  let seenGeneration = $state(0);

  const virtualTotalCount = $derived(totalCount ?? 40);
  const rowViewportHeight = $derived(
    fillAvailable
      ? Math.max(rowHeight, viewportHeight)
      : Math.max(
        rowHeight,
        Math.min(Math.max(rowHeight, maxViewportHeight - headerHeight), virtualTotalCount * rowHeight),
      ),
  );
  const range = $derived(getVirtualRange(
    scrollTop,
    viewportHeight,
    virtualTotalCount,
    rowHeight,
    overscan,
    0,
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
      if (!fillAvailable) {
        maxViewportHeight = Math.max(160, Math.min(maxHeight, Math.floor(window.innerHeight * 0.65)));
      }
      viewportHeight = viewport?.clientHeight ?? viewportHeight;
      if (viewport) headerGutter = Math.max(0, viewport.offsetWidth - viewport.clientWidth);
    };
    updateMetrics();
    window.addEventListener('resize', updateMetrics);
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(updateMetrics);
    if (observer && frame) observer.observe(frame);
    if (observer && viewport) observer.observe(viewport);
    return () => {
      window.removeEventListener('resize', updateMetrics);
      observer?.disconnect();
    };
  });

  function updateViewport(): void {
    if (!viewport) return;
    scrollTop = viewport.scrollTop;
    headerShift = viewport.scrollLeft;
    headerGutter = Math.max(0, viewport.offsetWidth - viewport.clientWidth);
    viewportHeight = viewport.clientHeight;
  }

  function setActiveIndex(index: number): void {
    const maxIndex = Math.max(0, virtualTotalCount - 1);
    activeIndex = Math.max(0, Math.min(maxIndex, index));
    if (!viewport) return;
    const rowTop = activeIndex * rowHeight;
    const rowBottom = rowTop + rowHeight;
    let nextScrollTop = viewport.scrollTop;
    if (rowTop < nextScrollTop) nextScrollTop = rowTop;
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
  class="paged-virtual-frame"
  class:fill-available={fillAvailable}
  bind:this={frame}
  style={`--paged-header-height:${headerHeight}px;--paged-row-height:${rowHeight}px;--paged-grid-template:${gridTemplate};--track-list-min-width:${minContentWidth}px`}
>
  <div class="paged-virtual-header" role="presentation" style={`height:${headerHeight}px;padding-right:${headerGutter}px`}>
    <div class="paged-virtual-header-shift" style={`transform:translateX(${-headerShift}px)`}>
      {#if header}{@render header()}{/if}
    </div>
  </div>
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
    style={`${fillAvailable ? '' : `height:${rowViewportHeight}px;`}--paged-header-height:${headerHeight}px;--paged-row-height:${rowHeight}px;--paged-grid-template:${gridTemplate};--track-list-min-width:${minContentWidth}px`}
    onscroll={updateViewport}
    onkeydown={handleKeydown}
  >
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
        tabindex={rowActivates && virtualRow.item !== null ? -1 : undefined}
        aria-label={virtualRow.item === null ? `第 ${virtualRow.index + 1} 項，正在載入` : undefined}
        style={`height:${rowHeight}px;transform:translateY(${virtualRow.index * rowHeight}px)`}
        class:activates={rowActivates && virtualRow.item !== null}
        onclick={() => {
          if (rowActivates) playRow(virtualRow.item, virtualRow.index);
        }}
        onkeydown={(event) => {
          if (!rowActivates || (event.key !== 'Enter' && event.key !== ' ')) return;
          event.preventDefault();
          playRow(virtualRow.item, virtualRow.index);
        }}
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
</div>

<style>
  .paged-virtual-frame {
    display: flex;
    min-width: 0;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--line);
    border-radius: 11px;
    background: rgba(var(--text-rgb), 0.012);
  }

  .paged-virtual-frame.fill-available {
    flex: 1 1 auto;
    min-height: 0;
    height: 100%;
  }

  .paged-virtual-viewport {
    position: relative;
    width: 100%;
    max-height: min(65vh, 680px);
    overflow: auto;
    overscroll-behavior: contain;
    scrollbar-gutter: stable;
    outline: none;
  }

  .paged-virtual-frame.fill-available .paged-virtual-viewport {
    flex: 1 1 auto;
    min-height: 0;
    max-height: none;
  }

  .paged-virtual-viewport:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }

  .paged-virtual-header {
    position: relative;
    z-index: 2;
    flex: 0 0 auto;
    overflow: hidden;
    box-sizing: border-box;
    background: transparent;
  }

  .paged-virtual-header-shift {
    width: max(100%, var(--track-list-min-width, 0px));
    min-width: max(100%, var(--track-list-min-width, 0px));
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
    overflow: hidden;
  }

  .paged-virtual-row.activates {
    cursor: pointer;
  }

  .paged-virtual-row.configurable-track-grid {
    right: auto;
    width: max(100%, var(--track-list-min-width, 0px));
  }
</style>
