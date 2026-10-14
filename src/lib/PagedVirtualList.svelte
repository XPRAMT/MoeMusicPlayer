<script module lang="ts">
  const SCROLL_POSITION_CACHE_LIMIT = 64;
  const scrollPositionsByList = new Map<string, number>();

  export interface VirtualListRow<Item> {
    item: Item | null;
    index: number;
    key: string | number;
    id: string;
    active: boolean;
    selected: boolean;
    play: () => void;
  }

  export interface QuickIndexGroup {
    label: string;
    beforeIndex: number;
  }

  function readListScrollPosition(listId: string): number {
    const position = scrollPositionsByList.get(listId);
    if (position === undefined) return 0;
    scrollPositionsByList.delete(listId);
    scrollPositionsByList.set(listId, position);
    return position;
  }

  function writeListScrollPosition(listId: string, position: number): void {
    scrollPositionsByList.delete(listId);
    scrollPositionsByList.set(listId, Math.max(0, position));
    if (scrollPositionsByList.size > SCROLL_POSITION_CACHE_LIMIT) {
      const oldestListId = scrollPositionsByList.keys().next().value;
      if (oldestListId !== undefined) scrollPositionsByList.delete(oldestListId);
    }
  }
</script>

<script lang="ts" generics="Item">
  import { onDestroy, onMount } from 'svelte';
  import type { Snippet } from 'svelte';
  import {
    getNextActiveIndex,
    getVirtualRange,
    type VirtualRange,
  } from './track-list-data.js';
  import { dataIndexToVisualIndex, visualIndexToAnchorDataIndex, visualIndexToDataIndex, visualRangeToDataRange } from './quick-index.js';
  import {
    QUEUE_REORDER_LONG_PRESS_MS,
    QUEUE_REORDER_MOVE_CANCEL_PX,
    queueDropIndexFromPointer,
  } from './queue-reorder.js';

  interface Props<Item> {
    totalCount: number | null;
    pending?: boolean;
    revision: number;
    generation: number;
    /** Override which generation changes reset the viewport, e.g. stable across data-only refreshes. */
    scrollResetKey?: unknown;
    /** Wait until the data belongs to the current list before restoring its saved position. */
    scrollRestoreReady?: boolean;
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
    longPressReorder?: boolean;
    onReorder?: (from: number, to: number) => void | Promise<void>;
    /** Bump `token` to scroll `index` into view (centered when possible). */
    scrollToIndexRequest?: { index: number; token: number } | null;
    /** Locale-aware group headings, inserted as virtual rows before their first data item. */
    groupRows?: QuickIndexGroup[];
    /** Only locale index labels that have rows in this list. */
    quickIndexLabels?: string[];
    header?: Snippet;
    row: Snippet<[VirtualListRow<Item>]>;
  }

  let {
    totalCount,
    pending = false,
    revision,
    generation,
    scrollResetKey,
    scrollRestoreReady = true,
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
    longPressReorder = false,
    onReorder = () => {},
    scrollToIndexRequest = null,
    groupRows = [],
    quickIndexLabels = [],
    header,
    row,
  }: Props<Item> = $props();

  let frame: HTMLDivElement | undefined = $state();
  let viewport: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let headerShift = $state(0);
  let headerGutter = $state(0);
  let viewportHeight = $state(480);
  let lastVisibleScrollTop = 0;
  let hadVisibleViewport = false;
  let activeIndex = $state(0);
  let rowHeight = $state(50);
  let headerHeight = $state(37);
  let maxViewportHeight = $state(680);
  let seenScrollResetKey: unknown;
  let hasSeenScrollResetKey = false;
  let seenListId: string | undefined;
  let pendingListRestore: { listId: string; position: number } | null = null;
  let dragFromIndex = $state<number | null>(null);
  let dragToIndex = $state<number | null>(null);
  let suppressNextClick = false;
  let longPressTimer: ReturnType<typeof setTimeout> | null = null;
  let pendingPointer: {
    pointerId: number;
    fromIndex: number;
    startX: number;
    startY: number;
  } | null = null;
  let activePointerId: number | null = null;
  let autoScrollFrame = 0;
  let latestPointerClientY = 0;
  let quickIndexOpen = $state(false);
  let quickIndexCenterX = $state(0);
  let quickIndexCenterY = $state(0);
  let quickIndexColumns = $state(1);
  let quickIndexMaxWidth = $state(0);
  let quickIndexMaxHeight = $state(0);
  let quickIndexDialog: HTMLDivElement | undefined = $state();
  let quickIndexTrigger: HTMLButtonElement | null = null;
  let previousGroupRows: QuickIndexGroup[] = [];

  const virtualDataCount = $derived(totalCount ?? 40);
  const virtualTotalCount = $derived(virtualDataCount + groupRows.length);
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
  const virtualRows = $derived.by(() => {
    const rows: Array<
      | { kind: 'group'; visualIndex: number; label: string; key: string }
      | { kind: 'item'; visualIndex: number; index: number; item: Item | null; key: string | number }
    > = [];
    for (let visualIndex = range.start; visualIndex < range.end; visualIndex += 1) {
      let groupRow: QuickIndexGroup | null = null;
      for (let groupIndex = 0; groupIndex < groupRows.length; groupIndex += 1) {
        if (groupRows[groupIndex].beforeIndex + groupIndex === visualIndex) {
          groupRow = groupRows[groupIndex];
          break;
        }
      }
      if (groupRow) {
        rows.push({ kind: 'group', visualIndex, label: groupRow.label, key: `quick-index:${visualIndex}:${groupRow.label}` });
      } else {
        const index = visualIndexToDataIndex(visualIndex, groupRows);
        if (index === null || index < 0 || index >= virtualDataCount) continue;
        const item = itemAt(index);
        rows.push({ kind: 'item', visualIndex, index, item, key: item === null ? `loading:${index}` : getKey(item, index) });
      }
    }
    return rows;
  });
  const activeDescendantId = $derived(`${listId}-row-${dataIndexToVisualIndex(activeIndex, groupRows)}`);
  const availableIndexLabels = $derived(new Set(groupRows.map(({ label }) => label)));

  function rememberVisibleScrollPosition(position: number): void {
    if (seenListId === listId) writeListScrollPosition(listId, position);
  }

  $effect(() => {
    const currentListId = listId;
    const currentScrollResetKey = scrollResetKey === undefined ? generation : scrollResetKey;
    const listChanged = seenListId !== undefined && currentListId !== seenListId;
    const resetChanged = hasSeenScrollResetKey && !Object.is(currentScrollResetKey, seenScrollResetKey);
    if (seenListId === undefined || listChanged) {
      activeIndex = 0;
      const position = readListScrollPosition(currentListId);
      pendingListRestore = { listId: currentListId, position };
      scrollTop = 0;
      lastVisibleScrollTop = 0;
      viewport?.scrollTo({ top: 0 });
    } else if (resetChanged) {
      activeIndex = 0;
      scrollTop = 0;
      lastVisibleScrollTop = 0;
      writeListScrollPosition(currentListId, 0);
      pendingListRestore = null;
      viewport?.scrollTo({ top: 0 });
    }
    if (pendingListRestore?.listId === currentListId && scrollRestoreReady && totalCount !== null && !pending) {
      const maxScrollTop = Math.max(0, virtualTotalCount * rowHeight - viewportHeight);
      const restoredScrollTop = Math.min(pendingListRestore.position, maxScrollTop);
      scrollTop = restoredScrollTop;
      lastVisibleScrollTop = restoredScrollTop;
      writeListScrollPosition(currentListId, restoredScrollTop);
      if (viewport && viewport.clientHeight > 0) {
        viewport.scrollTop = restoredScrollTop;
        hadVisibleViewport = true;
      }
      const restoredRange = getVirtualRange(
        restoredScrollTop,
        viewportHeight,
        virtualTotalCount,
        rowHeight,
        overscan,
        0,
      );
      onRange(visualRangeToDataRange(restoredRange.start, restoredRange.end, groupRows, virtualDataCount));
      pendingListRestore = null;
    }
    seenListId = currentListId;
    seenScrollResetKey = currentScrollResetKey;
    hasSeenScrollResetKey = true;
    const currentRange = visualRangeToDataRange(range.start, range.end, groupRows, virtualDataCount);
    onRange(currentRange);
  });

  $effect(() => {
    const nextGroupRows = groupRows;
    const previous = previousGroupRows;
    const groupsChanged = previous.length !== nextGroupRows.length
      || previous.some((group, index) => group.label !== nextGroupRows[index]?.label
        || group.beforeIndex !== nextGroupRows[index]?.beforeIndex);
    if (groupsChanged && viewport && viewport.clientHeight > 0) {
      const oldVisualIndex = Math.floor(viewport.scrollTop / rowHeight);
      const anchorDataIndex = visualIndexToAnchorDataIndex(oldVisualIndex, previous, virtualDataCount);
      if (anchorDataIndex !== null && anchorDataIndex >= 0 && anchorDataIndex < virtualDataCount) {
        const withinRowOffset = viewport.scrollTop - oldVisualIndex * rowHeight;
        const nextScrollTop = dataIndexToVisualIndex(anchorDataIndex, nextGroupRows) * rowHeight + withinRowOffset;
        viewport.scrollTop = nextScrollTop;
        scrollTop = nextScrollTop;
        lastVisibleScrollTop = nextScrollTop;
        rememberVisibleScrollPosition(nextScrollTop);
      }
    }
    previousGroupRows = nextGroupRows;
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
      const nextViewportHeight = viewport?.clientHeight ?? viewportHeight;
      if (viewport && nextViewportHeight > 0) {
        if (!hadVisibleViewport) {
          const restoredScrollTop = Math.min(
            lastVisibleScrollTop,
            Math.max(0, viewport.scrollHeight - nextViewportHeight),
          );
          if (viewport.scrollTop !== restoredScrollTop) viewport.scrollTop = restoredScrollTop;
          scrollTop = restoredScrollTop;
          lastVisibleScrollTop = restoredScrollTop;
          rememberVisibleScrollPosition(restoredScrollTop);
          hadVisibleViewport = true;
        } else {
          scrollTop = viewport.scrollTop;
          lastVisibleScrollTop = viewport.scrollTop;
          rememberVisibleScrollPosition(viewport.scrollTop);
        }
      } else if (nextViewportHeight === 0) {
        hadVisibleViewport = false;
      }
      viewportHeight = nextViewportHeight;
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
      endDragState();
    };
  });

  onDestroy(() => {
    const finalScrollTop = viewport?.clientHeight ? viewport.scrollTop : lastVisibleScrollTop;
    rememberVisibleScrollPosition(finalScrollTop);
  });

  function updateViewport(): void {
    if (!viewport) return;
    if (viewport.clientHeight > 0) {
      scrollTop = viewport.scrollTop;
      lastVisibleScrollTop = viewport.scrollTop;
      rememberVisibleScrollPosition(viewport.scrollTop);
      hadVisibleViewport = true;
    }
    headerShift = viewport.scrollLeft;
    headerGutter = Math.max(0, viewport.offsetWidth - viewport.clientWidth);
    viewportHeight = viewport.clientHeight;
  }

  function setActiveIndex(index: number): void {
    const maxIndex = Math.max(0, virtualDataCount - 1);
    activeIndex = Math.max(0, Math.min(maxIndex, index));
    if (!viewport) return;
    const rowTop = dataIndexToVisualIndex(activeIndex, groupRows) * rowHeight;
    const rowBottom = rowTop + rowHeight;
    let nextScrollTop = viewport.scrollTop;
    if (rowTop < nextScrollTop) nextScrollTop = rowTop;
    else if (rowBottom > nextScrollTop + viewport.clientHeight) {
      nextScrollTop = rowBottom - viewport.clientHeight;
    }
    if (nextScrollTop !== viewport.scrollTop) viewport.scrollTo({ top: nextScrollTop });
    lastVisibleScrollTop = nextScrollTop;
    rememberVisibleScrollPosition(nextScrollTop);
  }

  function scrollIndexIntoView(index: number, align: 'nearest' | 'center' = 'center'): void {
    const maxIndex = Math.max(0, virtualDataCount - 1);
    const target = Math.max(0, Math.min(maxIndex, index));
    activeIndex = target;
    if (!viewport) return;
    const rowTop = dataIndexToVisualIndex(target, groupRows) * rowHeight;
    let nextScrollTop = viewport.scrollTop;
    if (align === 'center') {
      nextScrollTop = rowTop - Math.max(0, (viewport.clientHeight - rowHeight) / 2);
    } else {
      const rowBottom = rowTop + rowHeight;
      if (rowTop < nextScrollTop) nextScrollTop = rowTop;
      else if (rowBottom > nextScrollTop + viewport.clientHeight) {
        nextScrollTop = rowBottom - viewport.clientHeight;
      }
    }
    const maxScrollTop = Math.max(0, viewport.scrollHeight - viewport.clientHeight);
    nextScrollTop = Math.max(0, Math.min(maxScrollTop, nextScrollTop));
    viewport.scrollTo({ top: nextScrollTop });
    scrollTop = nextScrollTop;
    lastVisibleScrollTop = nextScrollTop;
    rememberVisibleScrollPosition(nextScrollTop);
  }

  let seenScrollRequestToken = $state<number | null>(null);
  $effect(() => {
    const request = scrollToIndexRequest;
    const readyViewport = viewport;
    if (!request || !readyViewport) return;
    if (seenScrollRequestToken === request.token) return;
    seenScrollRequestToken = request.token;
    scrollIndexIntoView(request.index, 'center');
  });

  function playRow(item: Item | null, index: number): void {
    if (item === null) return;
    activeIndex = index;
    onPlay(item, index);
  }

  function clearLongPressTimer(): void {
    if (longPressTimer !== null) {
      clearTimeout(longPressTimer);
      longPressTimer = null;
    }
  }

  function stopAutoScroll(): void {
    if (autoScrollFrame !== 0) {
      cancelAnimationFrame(autoScrollFrame);
      autoScrollFrame = 0;
    }
  }

  function endDragState(): void {
    clearLongPressTimer();
    stopAutoScroll();
    pendingPointer = null;
    activePointerId = null;
    dragFromIndex = null;
    dragToIndex = null;
  }

  function dropIndexAtPointer(clientY: number): number {
    if (!viewport) return 0;
    const visualIndex = queueDropIndexFromPointer({
      scrollTop: viewport.scrollTop,
      pointerClientY: clientY,
      viewportTop: viewport.getBoundingClientRect().top,
      rowHeight,
      totalCount: virtualTotalCount,
    });
    const dataIndex = visualIndexToDataIndex(visualIndex, groupRows);
    if (dataIndex !== null) return dataIndex;
    const groupIndex = groupRows.findIndex((group, index) => group.beforeIndex + index === visualIndex);
    return groupIndex < 0 ? 0 : groupRows[groupIndex].beforeIndex;
  }

  function openQuickIndex(event: MouseEvent): void {
    if (!viewport) return;
    const rect = viewport.getBoundingClientRect();
    const probe = document.createElement('span');
    probe.style.position = 'fixed';
    probe.style.display = 'block';
    probe.style.visibility = 'hidden';
    probe.style.font = getComputedStyle(viewport).font;
    probe.style.width = '10ch';
    probe.style.boxSizing = 'border-box';
    document.body.append(probe);
    const columnWidth = probe.getBoundingClientRect().width;
    probe.remove();
    quickIndexCenterX = rect.left + rect.width / 2;
    quickIndexCenterY = rect.top + rect.height / 2;
    quickIndexMaxWidth = Math.max(0, rect.width - 24);
    quickIndexMaxHeight = Math.max(0, rect.height - 24);
    quickIndexColumns = Math.max(1, Math.floor((quickIndexMaxWidth - 24 + 4) / (columnWidth + 4)));
    quickIndexTrigger = event.currentTarget as HTMLButtonElement;
    quickIndexOpen = true;
  }

  function chooseQuickIndexLabel(label: string): void {
    quickIndexOpen = false;
    const group = groupRows.find((candidate) => candidate.label === label);
    if (group) {
      scrollIndexIntoView(group.beforeIndex, 'center');
      viewport?.focus();
    }
  }

  function closeQuickIndex(restoreFocus = true): void {
    quickIndexOpen = false;
    if (restoreFocus) queueMicrotask(() => quickIndexTrigger?.focus());
  }

  function handleQuickIndexDialogKeydown(event: KeyboardEvent): void {
    if (event.key !== 'Tab' || !quickIndexDialog) return;
    const focusable = [...quickIndexDialog.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  function handleWindowKeydown(event: KeyboardEvent): void {
    if (quickIndexOpen && event.key === 'Escape') {
      event.preventDefault();
      closeQuickIndex();
    }
  }

  $effect(() => {
    if (!quickIndexOpen) return;
    quickIndexDialog?.querySelector<HTMLButtonElement>('.quick-index-label:not(:disabled)')?.focus();
  });

  function updateDragTarget(clientY: number): void {
    latestPointerClientY = clientY;
    if (dragFromIndex === null || !viewport) return;
    dragToIndex = dropIndexAtPointer(clientY);
    const edge = 36;
    const rect = viewport.getBoundingClientRect();
    const nearTop = clientY - rect.top < edge;
    const nearBottom = rect.bottom - clientY < edge;
    if (!nearTop && !nearBottom) {
      stopAutoScroll();
      return;
    }
    if (autoScrollFrame !== 0) return;
    const step = () => {
      if (!viewport || dragFromIndex === null) {
        stopAutoScroll();
        return;
      }
      const bounds = viewport.getBoundingClientRect();
      let delta = 0;
      if (latestPointerClientY - bounds.top < edge) delta = -14;
      else if (bounds.bottom - latestPointerClientY < edge) delta = 14;
      if (delta !== 0) {
        viewport.scrollTop = Math.max(0, viewport.scrollTop + delta);
        scrollTop = viewport.scrollTop;
        lastVisibleScrollTop = viewport.scrollTop;
        rememberVisibleScrollPosition(viewport.scrollTop);
        dragToIndex = dropIndexAtPointer(latestPointerClientY);
        autoScrollFrame = requestAnimationFrame(step);
      } else {
        autoScrollFrame = 0;
      }
    };
    autoScrollFrame = requestAnimationFrame(step);
  }

  function beginDrag(fromIndex: number, pointerId: number, target: Element): void {
    if (!viewport) return;
    dragFromIndex = fromIndex;
    dragToIndex = fromIndex;
    activePointerId = pointerId;
    pendingPointer = null;
    suppressNextClick = true;
    try {
      target.setPointerCapture(pointerId);
    } catch {
      // Capture can fail if the pointer already ended; drag still tracks window events.
    }
  }

  function handleRowPointerDown(event: PointerEvent, index: number, item: Item | null): void {
    if (!longPressReorder || item === null || event.button !== 0) return;
    clearLongPressTimer();
    pendingPointer = {
      pointerId: event.pointerId,
      fromIndex: index,
      startX: event.clientX,
      startY: event.clientY,
    };
    const target = event.currentTarget as Element;
    longPressTimer = setTimeout(() => {
      if (!pendingPointer || pendingPointer.pointerId !== event.pointerId) return;
      beginDrag(pendingPointer.fromIndex, pendingPointer.pointerId, target);
    }, QUEUE_REORDER_LONG_PRESS_MS);
  }

  function handleRowPointerMove(event: PointerEvent): void {
    if (pendingPointer && pendingPointer.pointerId === event.pointerId && dragFromIndex === null) {
      const dx = event.clientX - pendingPointer.startX;
      const dy = event.clientY - pendingPointer.startY;
      if ((dx * dx) + (dy * dy) > QUEUE_REORDER_MOVE_CANCEL_PX * QUEUE_REORDER_MOVE_CANCEL_PX) {
        clearLongPressTimer();
        pendingPointer = null;
      }
      return;
    }
    if (dragFromIndex === null || activePointerId !== event.pointerId) return;
    event.preventDefault();
    updateDragTarget(event.clientY);
  }

  async function finishRowPointer(event: PointerEvent): Promise<void> {
    if (pendingPointer?.pointerId === event.pointerId) {
      clearLongPressTimer();
      pendingPointer = null;
    }
    if (dragFromIndex === null || activePointerId !== event.pointerId) return;
    event.preventDefault();
    const from = dragFromIndex;
    const to = dragToIndex ?? dragFromIndex;
    endDragState();
    if (from !== to) {
      await onReorder(from, to);
    }
  }

  function handleRowPointerCancel(event: PointerEvent): void {
    if (pendingPointer?.pointerId === event.pointerId || activePointerId === event.pointerId) {
      endDragState();
      suppressNextClick = true;
    }
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
      virtualDataCount,
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
    aria-rowcount={totalCount === null ? -1 : virtualTotalCount + 1}
    aria-colcount={columnCount}
    aria-activedescendant={activeDescendantId}
    aria-busy={pending || totalCount === null}
    tabindex="0"
    style={`${fillAvailable ? '' : `height:${rowViewportHeight}px;`}--paged-header-height:${headerHeight}px;--paged-row-height:${rowHeight}px;--paged-grid-template:${gridTemplate};--track-list-min-width:${minContentWidth}px`}
    class:is-reordering={dragFromIndex !== null}
    onscroll={updateViewport}
    onkeydown={handleKeydown}
  >
    <div
      class="paged-virtual-spacer"
      role="presentation"
      aria-hidden="true"
      style={`height:${virtualTotalCount * rowHeight}px`}
    ></div>
    {#if dragFromIndex !== null && dragToIndex !== null && dragToIndex !== dragFromIndex}
      <div
        class="paged-virtual-drop-line"
        role="presentation"
        aria-hidden="true"
        style={`transform:translateY(${dataIndexToVisualIndex(dragToIndex, groupRows) * rowHeight + (dragToIndex > dragFromIndex ? rowHeight - 2 : 0)}px)`}
      ></div>
    {/if}
    {#each virtualRows as virtualRow (virtualRow.key)}
      {@const id = `${listId}-row-${virtualRow.visualIndex}`}
      {#if virtualRow.kind === 'group'}
        <div
          {id}
          class="paged-virtual-row paged-virtual-group-row"
          role="row"
          aria-rowindex={virtualRow.visualIndex + 2}
          style={`height:${rowHeight}px;transform:translateY(${virtualRow.visualIndex * rowHeight}px)`}
        >
          <div class="paged-virtual-group-cell" role="gridcell" aria-colspan={columnCount}>
            <button type="button" class="paged-virtual-group-button" aria-label={`開啟「${virtualRow.label}」字首索引`} onclick={openQuickIndex}>{virtualRow.label}</button>
          </div>
        </div>
      {:else}
      {@const selected = virtualRow.item !== null && isSelected(virtualRow.item, virtualRow.index)}
      <div
        id={id}
        class={`paged-virtual-row ${rowClassName}`}
        class:selected
        class:active={activeIndex === virtualRow.index}
        class:is-dragging={dragFromIndex === virtualRow.index}
        role="row"
        aria-rowindex={virtualRow.visualIndex + 2}
        aria-selected={selected}
        tabindex={rowActivates && virtualRow.item !== null ? -1 : undefined}
        aria-label={virtualRow.item === null ? `第 ${virtualRow.index + 1} 項，正在載入` : undefined}
        style={`height:${rowHeight}px;transform:translateY(${virtualRow.visualIndex * rowHeight}px)`}
        class:activates={rowActivates && virtualRow.item !== null}
        class:reorderable={longPressReorder && virtualRow.item !== null}
        onpointerdown={longPressReorder ? (event) => handleRowPointerDown(event, virtualRow.index, virtualRow.item) : undefined}
        onpointermove={longPressReorder ? handleRowPointerMove : undefined}
        onpointerup={longPressReorder ? finishRowPointer : undefined}
        onpointercancel={longPressReorder ? handleRowPointerCancel : undefined}
        onclick={() => {
          if (suppressNextClick) {
            suppressNextClick = false;
            return;
          }
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
      {/if}
    {/each}
  </div>
  {#if quickIndexOpen}
    <button type="button" class="quick-index-backdrop" aria-label="關閉首字索引" onclick={() => closeQuickIndex()}></button>
    <div bind:this={quickIndexDialog} class="quick-index-popover floating-surface" style={`left:${quickIndexCenterX}px;top:${quickIndexCenterY}px;--quick-index-columns:${quickIndexColumns};--quick-index-max-width:${quickIndexMaxWidth}px;--quick-index-max-height:${quickIndexMaxHeight}px`} role="dialog" aria-modal="true" aria-label="快速首字查找" tabindex="-1" onkeydown={handleQuickIndexDialogKeydown}>
      <header class="quick-index-popover-header">
        <strong>快速首字查找</strong>
      </header>
      <div class="quick-index-label-grid">
        {#each quickIndexLabels as label (label)}
          {@const available = availableIndexLabels.has(label)}
          <button type="button" class="quick-index-label" class:unavailable={!available} disabled={!available} onclick={() => chooseQuickIndexLabel(label)}>{label}</button>
        {/each}
      </div>
    </div>
  {/if}
</div>

<svelte:window onkeydown={handleWindowKeydown} />

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
    border: 0;
    border-radius: 0;
    background: transparent;
  }

  .paged-virtual-frame.fill-available .paged-virtual-viewport:focus-visible {
    outline-offset: -2px;
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

  .paged-virtual-viewport.is-reordering {
    cursor: grabbing;
    user-select: none;
    touch-action: none;
  }

  .paged-virtual-viewport:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }

  .paged-virtual-row.reorderable {
    touch-action: pan-y;
  }

  .paged-virtual-row.is-dragging {
    opacity: 0.38;
    z-index: 2;
  }

  .paged-virtual-drop-line {
    position: absolute;
    top: 0;
    right: 0;
    left: 0;
    z-index: 3;
    height: 2px;
    pointer-events: none;
    background: var(--accent-text);
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-text) 35%, transparent);
  }

  .paged-virtual-header {
    position: relative;
    z-index: 2;
    flex: 0 0 auto;
    overflow: hidden;
    box-sizing: border-box;
    background: transparent;
  }

  .paged-virtual-header::after {
    content: '';
    position: absolute;
    z-index: 3;
    right: 0;
    bottom: 0;
    left: 0;
    height: 1px;
    pointer-events: none;
    background: var(--line);
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

  .paged-virtual-group-row {
    display: flex;
    align-items: center;
    background: transparent;
  }

  .paged-virtual-group-row::after {
    position: absolute;
    right: 0;
    bottom: 0;
    left: 0;
    height: 1px;
    content: '';
    pointer-events: none;
    background: var(--line);
  }

  .paged-virtual-group-cell {
    display: flex;
    width: 100%;
    height: 100%;
    min-width: 0;
    align-items: center;
  }

  .paged-virtual-group-button {
    display: flex;
    width: 100%;
    height: 100%;
    min-width: 0;
    align-items: center;
    justify-content: center;
    box-sizing: border-box;
    padding: 0;
    border: 0;
    border-radius: 0;
    color: var(--text);
    background: transparent;
    cursor: pointer;
    font: inherit;
  }

  .paged-virtual-group-button:hover {
    background: transparent;
  }

  .paged-virtual-group-button:focus-visible {
    outline: 2px solid var(--line);
    outline-offset: -2px;
  }

  .quick-index-backdrop {
    position: fixed;
    z-index: 220;
    inset: 0;
    padding: 0;
    border: 0;
    background: transparent;
  }

  .quick-index-popover {
    position: fixed;
    z-index: 221;
    display: flex;
    width: max-content;
    max-width: min(calc(100vw - 24px), var(--quick-index-max-width));
    max-height: var(--quick-index-max-height);
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    overflow-x: hidden;
    overflow-y: auto;
    box-sizing: border-box;
    transform: translate(-50%, -50%);
  }

  .quick-index-popover-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }

  .quick-index-popover-header strong {
    font: var(--font-body);
  }

  .quick-index-label-grid {
    display: grid;
    grid-template-columns: repeat(var(--quick-index-columns), 10ch);
    gap: 4px;
  }

  .quick-index-label {
    display: grid;
    width: 10ch;
    box-sizing: border-box;
    min-height: 34px;
    place-items: center;
    border: 1px solid transparent;
    border-radius: 7px;
    color: var(--text);
    background: transparent;
    cursor: pointer;
    font: var(--font-body);
  }

  .quick-index-label:hover:not(:disabled) {
    border-color: var(--line);
    background: rgba(var(--text-rgb), 0.09);
  }

  .quick-index-label:disabled,
  .quick-index-label.unavailable {
    color: var(--muted-text);
    opacity: 0.42;
    cursor: default;
  }
</style>
