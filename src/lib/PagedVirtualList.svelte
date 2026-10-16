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
    type VirtualRange,
  } from './track-list-data.js';
  import { buildPagedVirtualRows, dataIndexToVisualIndex, getGroupedVirtualRange, getQuickIndexAvailableWidth, getQuickIndexCellSizePx, getQuickIndexColumnCount, getQuickIndexDisplayLabel, getRestoredListScrollTop, getVirtualRowTop, getVirtualRowsHeight, getVisualIndexAtOffset, reportPagedRange, shouldAdjustGroupAnchor, visualIndexToAnchorDataIndex, visualIndexToDataIndex, visualRangeToDataRange } from './quick-index.js';
  import {
    QUEUE_REORDER_LONG_PRESS_MS,
    QUEUE_REORDER_MOVE_CANCEL_PX,
  } from './queue-reorder.js';
  import { getGridTemplateMinWidthPx } from './grid-min-width.js';
  import { exceedsLongPressSlop, longPressDragGuard } from './long-press-reorder.js';
  import { getAlbumDetailNativeScrollTop, getAlbumDetailScrollerTop, getAlbumDetailScrollState } from './album-detail-collapse.js';

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
    /** Keep the scroll viewport fixed while a leading album heading overlays its content. */
    leadingOverlayHeight?: number;
    leadingCollapseRange?: number;
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
    onViewportScrollTopChange?: (scrollTop: number, collapseOffset: number) => void;
    rowActivates?: boolean;
    longPressReorder?: boolean;
    onReorder?: (from: number, to: number) => void | Promise<void>;
    /** Bump `token` to scroll `index` into view (centered when possible). */
    scrollToIndexRequest?: { index: number; token: number } | null;
    /** Locale-aware group headings, inserted as virtual rows before their first data item. */
    groupRows?: QuickIndexGroup[];
    /** Keep rows blank until all group boundaries for this list are ready. */
    groupingReady?: boolean;
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
    leadingOverlayHeight = 0,
    leadingCollapseRange = 0,
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
    onViewportScrollTopChange,
    rowActivates = true,
    longPressReorder = false,
    onReorder = () => {},
    scrollToIndexRequest = null,
    groupRows = [],
    groupingReady = true,
    quickIndexLabels = [],
    header,
    row,
  }: Props<Item> = $props();

  let frame: HTMLDivElement | undefined = $state();
  // Rows never shrink below the grid template's own minimums (px/em/minmax), so every track table
  // overflows its viewport and scrolls horizontally instead of clipping columns in narrow windows.
  let gridEmPx = $state(16);
  const effectiveMinContentWidth = $derived(Math.max(minContentWidth, getGridTemplateMinWidthPx(gridTemplate, gridEmPx)));
  let viewport: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let headerShift = $state(0);
  // Visible width for pinned group headers; updated on resize only (never per scroll frame).
  let viewportClientWidth = $state(0);
  let headerGutter = $state(0);
  let viewportHeight = $state(480);
  let metricsReady = $state(false);
  let lastVisibleScrollTop = 0;
  let hadVisibleViewport = false;
  let activeIndex = $state(0);
  let rowHeight = $state(50);
  let headerHeight = $state(37);
  let maxViewportHeight = $state(680);
  let recalculateMetrics = () => {};
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
  let quickIndexMaxWidth = $state(0);
  let quickIndexMaxHeight = $state(0);
  let quickIndexCellSizePx = $state(32);
  let quickIndexDialog: HTMLDivElement | undefined = $state();
  let quickIndexTrigger: HTMLButtonElement | null = null;
  let previousGroupRows: QuickIndexGroup[] = [];
  let hasPreviousGroupSnapshot = false;
  let collapseOffset = $state(0);

  const virtualDataCount = $derived(totalCount ?? 40);
  const renderGroupRows = $derived(groupingReady ? groupRows : []);
  const virtualTotalCount = $derived(virtualDataCount + renderGroupRows.length);
  const groupRowHeight = $derived(Math.max(30, rowHeight * 0.7));
  const virtualContentHeight = $derived(
    getVirtualRowsHeight(virtualDataCount, renderGroupRows, rowHeight, groupRowHeight),
  );
  const hasLeadingOverlay = $derived(leadingOverlayHeight > 0);
  const supportsScrollDrivenAlbumOverlay = typeof CSS !== 'undefined'
    && CSS.supports('animation-timeline', 'scroll()')
    && CSS.supports('animation-range', '0px 1px')
    && CSS.supports('timeline-scope', '--album-detail-scroll')
    && CSS.supports('scroll-timeline-name', '--album-detail-scroll');
  /** Album detail: the scroller starts below the fully collapsed heading and table header; it never moves or resizes. */
  const viewportTopOffset = $derived(hasLeadingOverlay
    ? getAlbumDetailScrollerTop(leadingOverlayHeight, leadingCollapseRange, headerHeight)
    : 0);
  /** Album detail: a top spacer equal to the collapse travel; rows begin below it. */
  const contentLeadingHeight = $derived(hasLeadingOverlay ? leadingCollapseRange : 0);
  const effectiveViewportHeight = $derived(hasLeadingOverlay
    ? Math.max(rowHeight, viewportHeight - Math.max(0, leadingCollapseRange - collapseOffset))
    : viewportHeight);
  const rowViewportHeight = $derived(
    fillAvailable
      ? Math.max(rowHeight, effectiveViewportHeight)
      : Math.max(
        rowHeight,
        Math.min(Math.max(rowHeight, maxViewportHeight - headerHeight), virtualContentHeight),
      ),
  );
  const range = $derived(getGroupedVirtualRange(
    scrollTop,
    effectiveViewportHeight,
    virtualDataCount,
    renderGroupRows,
    rowHeight,
    groupRowHeight,
    overscan,
  ));
  const virtualRows = $derived(buildPagedVirtualRows({
    ready: groupingReady,
    groups: renderGroupRows,
    start: range.start,
    end: range.end,
    totalCount: virtualDataCount,
    revision,
    itemAt,
    getKey,
  }));
  const activeDescendantId = $derived(`${listId}-row-${dataIndexToVisualIndex(activeIndex, renderGroupRows)}`);
  const availableIndexLabels = $derived(new Set(renderGroupRows.map(({ label }) => label)));

  function nativeScrollTopFor(logicalTop: number, collapse = collapseOffset): number {
    return hasLeadingOverlay
      ? getAlbumDetailNativeScrollTop(logicalTop, leadingCollapseRange, collapse)
      : Math.max(0, logicalTop);
  }

  function readNativeScrollState(nativeTop: number): { listScrollTop: number; collapseOffset: number } {
    return hasLeadingOverlay
      ? getAlbumDetailScrollState(nativeTop, leadingCollapseRange)
      : { listScrollTop: Math.max(0, nativeTop), collapseOffset: 0 };
  }

  function logicalScrollTopFromNative(nativeTop: number): number {
    return readNativeScrollState(nativeTop).listScrollTop;
  }

  function setLogicalScrollTop(logicalTop: number): void {
    if (!viewport) return;
    const next = Math.max(0, Math.min(getMaximumLogicalScrollTop(), logicalTop));
    viewport.scrollTop = nativeScrollTopFor(next);
    updateViewport();
  }

  function getVisibleRowsHeight(): number {
    if (!viewport) return viewportHeight;
    return hasLeadingOverlay
      ? Math.max(rowHeight, viewport.clientHeight - Math.max(0, leadingCollapseRange - collapseOffset))
      : viewport.clientHeight;
  }

  function getMaximumLogicalScrollTop(): number {
    if (!viewport) return 0;
    return readNativeScrollState(Math.max(0, viewport.scrollHeight - viewport.clientHeight)).listScrollTop;
  }

  function moveNativeBy(delta: number): void {
    if (!viewport) return;
    viewport.scrollTop = Math.max(0, viewport.scrollTop + delta);
    updateViewport();
  }

  function getVisibleRowsRect(): DOMRect | null {
    if (!viewport) return null;
    const rect = viewport.getBoundingClientRect();
    const top = rect.top + (hasLeadingOverlay ? Math.max(0, leadingCollapseRange - collapseOffset) : 0);
    return new DOMRect(rect.left, top, rect.width, Math.max(0, rect.bottom - top));
  }

  $effect(() => {
    baseRowHeight;
    compactRowHeight;
    shortDesktopRowHeight;
    baseHeaderHeight;
    compactHeaderHeight;
    recalculateMetrics();
  });

  function rememberVisibleScrollPosition(position: number): void {
    if (seenListId === listId) writeListScrollPosition(listId, position);
  }

  $effect(() => {
    if (!metricsReady) return;
    const currentListId = listId;
    const currentScrollResetKey = scrollResetKey === undefined ? generation : scrollResetKey;
    const listChanged = seenListId !== undefined && currentListId !== seenListId;
    const resetChanged = hasSeenScrollResetKey && !Object.is(currentScrollResetKey, seenScrollResetKey);
    if (seenListId === undefined || listChanged) {
      activeIndex = 0;
      if (listChanged) {
        previousGroupRows = [];
        hasPreviousGroupSnapshot = false;
      }
      const position = readListScrollPosition(currentListId);
      pendingListRestore = { listId: currentListId, position };
      scrollTop = 0;
      collapseOffset = 0;
      lastVisibleScrollTop = 0;
      viewport?.scrollTo({ top: 0 });
    } else if (resetChanged) {
      activeIndex = 0;
      scrollTop = 0;
      lastVisibleScrollTop = 0;
      writeListScrollPosition(currentListId, 0);
      pendingListRestore = null;
      collapseOffset = 0;
      viewport?.scrollTo({ top: 0 });
    }
    const restoredScrollTop = pendingListRestore?.listId === currentListId
      ? getRestoredListScrollTop({
        position: pendingListRestore.position,
        groupingReady,
        totalCount,
        pending,
        virtualTotalCount,
        rowHeight,
        groupRowHeight,
        groups: renderGroupRows,
        viewportHeight: effectiveViewportHeight,
      })
      : null;
    if (restoredScrollTop !== null && scrollRestoreReady) {
      const maximum = getMaximumLogicalScrollTop();
      const safeRestoredScrollTop = Math.min(restoredScrollTop, maximum);
      const restoredCollapseOffset = hasLeadingOverlay && safeRestoredScrollTop > 0 ? leadingCollapseRange : 0;
      scrollTop = safeRestoredScrollTop;
      lastVisibleScrollTop = safeRestoredScrollTop;
      writeListScrollPosition(currentListId, safeRestoredScrollTop);
      if (viewport && viewport.clientHeight > 0) {
        viewport.scrollTop = nativeScrollTopFor(safeRestoredScrollTop, restoredCollapseOffset);
        collapseOffset = restoredCollapseOffset;
        hadVisibleViewport = true;
      }
      const restoredRange = getGroupedVirtualRange(
        safeRestoredScrollTop,
        effectiveViewportHeight,
        virtualDataCount,
        renderGroupRows,
        rowHeight,
        groupRowHeight,
        overscan,
      );
      onRange(visualRangeToDataRange(restoredRange.start, restoredRange.end, renderGroupRows, virtualDataCount));
      onViewportScrollTopChange?.(safeRestoredScrollTop, restoredCollapseOffset);
      pendingListRestore = null;
    }
    seenListId = currentListId;
    seenScrollResetKey = currentScrollResetKey;
    hasSeenScrollResetKey = true;
  });

  $effect(() => {
    reportPagedRange({
      metricsReady,
      start: range.start,
      end: range.end,
      groups: renderGroupRows,
      totalCount: virtualDataCount,
      onRange,
    });
  });

  function commitGroupRows(nextGroupRows: QuickIndexGroup[]): void {
    const previous = previousGroupRows;
    const groupsChanged = previous.length !== nextGroupRows.length
      || previous.some((group, index) => group.label !== nextGroupRows[index]?.label
        || group.beforeIndex !== nextGroupRows[index]?.beforeIndex);
    if (shouldAdjustGroupAnchor({
      groupsChanged,
      hasPreviousSnapshot: hasPreviousGroupSnapshot,
      scrollTop,
    }) && viewport && viewport.clientHeight > 0) {
      const oldVisualIndex = getVisualIndexAtOffset(
        scrollTop,
        virtualDataCount,
        previous,
        rowHeight,
        groupRowHeight,
      );
      const anchorDataIndex = visualIndexToAnchorDataIndex(oldVisualIndex, previous, virtualDataCount);
      if (anchorDataIndex !== null && anchorDataIndex >= 0 && anchorDataIndex < virtualDataCount) {
        const oldTop = getVirtualRowTop(oldVisualIndex, previous, rowHeight, groupRowHeight);
        const withinRowOffset = Math.max(0, scrollTop - oldTop);
        const nextVisualIndex = dataIndexToVisualIndex(anchorDataIndex, nextGroupRows);
        const nextScrollTop = getVirtualRowTop(nextVisualIndex, nextGroupRows, rowHeight, groupRowHeight)
          + Math.min(withinRowOffset, rowHeight - 1);
        setLogicalScrollTop(nextScrollTop);
      }
    }
    previousGroupRows = nextGroupRows;
    hasPreviousGroupSnapshot = true;
  }

  $effect(() => {
    const nextGroupRows = groupRows;
    if (groupingReady) commitGroupRows(nextGroupRows);
  });

  onMount(() => {
    if (viewport) {
      const fontSize = Number.parseFloat(getComputedStyle(viewport).fontSize);
      if (Number.isFinite(fontSize) && fontSize > 0) gridEmPx = fontSize;
    }
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
          const restoredScrollTop = lastVisibleScrollTop;
          const restoredNativeTop = nativeScrollTopFor(restoredScrollTop, restoredScrollTop > 0 ? leadingCollapseRange : collapseOffset);
          if (viewport.scrollTop !== restoredNativeTop) viewport.scrollTop = restoredNativeTop;
          scrollTop = restoredScrollTop;
          collapseOffset = readNativeScrollState(restoredNativeTop).collapseOffset;
          rememberVisibleScrollPosition(restoredScrollTop);
          hadVisibleViewport = true;
        } else {
          const state = readNativeScrollState(viewport.scrollTop);
          scrollTop = state.listScrollTop;
          collapseOffset = state.collapseOffset;
          lastVisibleScrollTop = scrollTop;
          rememberVisibleScrollPosition(scrollTop);
        }
      } else if (nextViewportHeight === 0) {
        hadVisibleViewport = false;
      }
      viewportHeight = nextViewportHeight;
      if (effectiveViewportHeight > 0) metricsReady = true;
      if (viewport) headerGutter = Math.max(0, viewport.offsetWidth - viewport.clientWidth);
      if (viewport) viewportClientWidth = viewport.clientWidth;
      if (viewport) onViewportScrollTopChange?.(scrollTop, collapseOffset);
    };
    recalculateMetrics = updateMetrics;
    updateMetrics();
    window.addEventListener('resize', updateMetrics);
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(updateMetrics);
    if (observer && frame) observer.observe(frame);
    if (observer && viewport) observer.observe(viewport);
    return () => {
      window.removeEventListener('resize', updateMetrics);
      observer?.disconnect();
      endDragState();
      recalculateMetrics = () => {};
    };
  });

  onDestroy(() => {
    const finalScrollTop = viewport?.clientHeight ? logicalScrollTopFromNative(viewport.scrollTop) : lastVisibleScrollTop;
    rememberVisibleScrollPosition(finalScrollTop);
  });

  function updateViewport(): void {
    if (!viewport) return;
    let nextCollapseOffset = collapseOffset;
    if (viewport.clientHeight > 0) {
      const state = readNativeScrollState(viewport.scrollTop);
      scrollTop = state.listScrollTop;
      nextCollapseOffset = state.collapseOffset;
      collapseOffset = nextCollapseOffset;
      lastVisibleScrollTop = scrollTop;
      rememberVisibleScrollPosition(scrollTop);
      hadVisibleViewport = true;
    }
    headerShift = viewport.scrollLeft;
    headerGutter = Math.max(0, viewport.offsetWidth - viewport.clientWidth);
    // viewportHeight is the measured viewport's clientHeight; the album-detail
    // scroller has a fixed size, so scrolling never changes it.
    metricsReady = true;
    onViewportScrollTopChange?.(scrollTop, nextCollapseOffset);
  }

  function handleViewportScroll(): void {
    updateViewport();
  }

  function setActiveIndex(index: number): void {
    const maxIndex = Math.max(0, virtualDataCount - 1);
    activeIndex = Math.max(0, Math.min(maxIndex, index));
    if (!viewport) return;
    const rowTop = getVirtualRowTop(dataIndexToVisualIndex(activeIndex, renderGroupRows), renderGroupRows, rowHeight, groupRowHeight);
    const rowBottom = rowTop + rowHeight;
    let nextScrollTop = scrollTop;
    const rowsHeight = getVisibleRowsHeight();
    if (rowTop < nextScrollTop) nextScrollTop = rowTop;
    else if (rowBottom > nextScrollTop + rowsHeight) {
      nextScrollTop = rowBottom - rowsHeight;
    }
    if (nextScrollTop !== scrollTop) setLogicalScrollTop(nextScrollTop);
  }

  function scrollIndexIntoView(index: number, align: 'nearest' | 'center' = 'center'): void {
    const maxIndex = Math.max(0, virtualDataCount - 1);
    const target = Math.max(0, Math.min(maxIndex, index));
    activeIndex = target;
    if (!viewport) return;
    const rowTop = getVirtualRowTop(dataIndexToVisualIndex(target, renderGroupRows), renderGroupRows, rowHeight, groupRowHeight);
    let nextScrollTop = scrollTop;
    const rowsHeight = getVisibleRowsHeight();
    if (align === 'center') {
      nextScrollTop = rowTop - Math.max(0, (rowsHeight - rowHeight) / 2);
    } else {
      const rowBottom = rowTop + rowHeight;
      if (rowTop < nextScrollTop) nextScrollTop = rowTop;
      else if (rowBottom > nextScrollTop + rowsHeight) {
        nextScrollTop = rowBottom - rowsHeight;
      }
    }
    const maxScrollTop = getMaximumLogicalScrollTop();
    nextScrollTop = Math.max(0, Math.min(maxScrollTop, nextScrollTop));
    setLogicalScrollTop(nextScrollTop);
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
    const visibleRowsRect = getVisibleRowsRect();
    if (!visibleRowsRect) return 0;
    const visualIndex = getVisualIndexAtOffset(
      scrollTop + clientY - visibleRowsRect.top,
      virtualDataCount,
      renderGroupRows,
      rowHeight,
      groupRowHeight,
    );
    const dataIndex = visualIndexToDataIndex(visualIndex, renderGroupRows);
    if (dataIndex !== null) return dataIndex;
    const groupIndex = renderGroupRows.findIndex((group, index) => group.beforeIndex + index === visualIndex);
    return groupIndex < 0 ? 0 : renderGroupRows[groupIndex].beforeIndex;
  }

  function openQuickIndex(event: MouseEvent): void {
    if (!viewport) return;
    const rect = getVisibleRowsRect() ?? viewport.getBoundingClientRect();
    quickIndexCenterX = rect.left + rect.width / 2;
    quickIndexCenterY = rect.top + rect.height / 2;
    quickIndexMaxWidth = getQuickIndexAvailableWidth(rect.width, window.innerWidth);
    quickIndexCellSizePx = getQuickIndexCellSizePx(Number.parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--font-title')));
    quickIndexMaxHeight = Math.max(0, rect.height - 24);
    quickIndexTrigger = event.currentTarget as HTMLButtonElement;
    quickIndexOpen = true;
  }

  function chooseQuickIndexLabel(label: string): void {
    quickIndexOpen = false;
    const group = renderGroupRows.find((candidate) => candidate.label === label);
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

  $effect(() => {
    if (quickIndexOpen && quickIndexLabels.length === 0) closeQuickIndex(false);
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
        moveNativeBy(delta);
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
      if (exceedsLongPressSlop(event.clientX - pendingPointer.startX, event.clientY - pendingPointer.startY, QUEUE_REORDER_MOVE_CANCEL_PX)) {
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
  class:leading-overlay={hasLeadingOverlay}
  bind:this={frame}
  style={`--paged-header-height:${headerHeight}px;--paged-row-height:${rowHeight}px;--paged-grid-template:${gridTemplate};--track-list-min-width:${effectiveMinContentWidth}px;--paged-leading-height:${leadingOverlayHeight}px;--paged-collapse-range:${leadingCollapseRange}px;--paged-viewport-top:${viewportTopOffset}px`}
>
  <div class="paged-virtual-header" role="presentation" style={`height:${headerHeight}px;${hasLeadingOverlay ? `top:${Math.max(0, leadingOverlayHeight - collapseOffset)}px;right:${headerGutter}px;` : `padding-right:${headerGutter}px;`}`}>
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
    style={`${fillAvailable ? '' : `height:${rowViewportHeight}px;`}--paged-header-height:${headerHeight}px;--paged-row-height:${rowHeight}px;--paged-grid-template:${gridTemplate};--track-list-min-width:${effectiveMinContentWidth}px;--paged-viewport-width:${viewportClientWidth > 0 ? `${viewportClientWidth}px` : '100%'}`}
    class:album-detail-scroll-timeline={hasLeadingOverlay && supportsScrollDrivenAlbumOverlay}
    class:is-reordering={dragFromIndex !== null}
    use:longPressDragGuard={() => dragFromIndex !== null}
    onscroll={handleViewportScroll}
    onkeydown={handleKeydown}
  >
    <div
      class="paged-virtual-spacer"
      role="presentation"
      aria-hidden="true"
      style={`height:${contentLeadingHeight + virtualContentHeight}px`}
    ></div>
    {#if dragFromIndex !== null && dragToIndex !== null && dragToIndex !== dragFromIndex}
      <div
        class="paged-virtual-drop-line"
        role="presentation"
        aria-hidden="true"
        style={`transform:translateY(${contentLeadingHeight + getVirtualRowTop(dataIndexToVisualIndex(dragToIndex, renderGroupRows), renderGroupRows, rowHeight, groupRowHeight) + (dragToIndex > dragFromIndex ? rowHeight - 2 : 0)}px)`}
      ></div>
    {/if}
    {#each virtualRows.rows as virtualRow (virtualRow.key)}
      {@const id = `${listId}-row-${virtualRow.visualIndex}`}
      {@const rowTop = getVirtualRowTop(virtualRow.visualIndex, renderGroupRows, rowHeight, groupRowHeight)}
      {#if virtualRow.kind === 'group'}
        <div
          {id}
          class="paged-virtual-row paged-virtual-group-row"
          role="row"
          aria-rowindex={virtualRow.visualIndex + 2}
          style={`height:${groupRowHeight}px;transform:translateY(${contentLeadingHeight + rowTop}px)`}
        >
          <div class="paged-virtual-group-cell list-group-row-surface" role="gridcell" aria-colspan={columnCount}>
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
        style={`height:${rowHeight}px;transform:translateY(${contentLeadingHeight + rowTop}px)`}
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
    <div bind:this={quickIndexDialog} class="quick-index-popover floating-surface" style={`left:${quickIndexCenterX}px;top:${quickIndexCenterY}px;--quick-index-columns:${getQuickIndexColumnCount(quickIndexLabels.length, { availableWidthPx: quickIndexMaxWidth, cellSizePx: quickIndexCellSizePx })};--quick-index-max-width:${quickIndexMaxWidth}px;--quick-index-max-height:${quickIndexMaxHeight}px`} role="dialog" aria-modal="true" aria-label="快速首字查找" tabindex="-1" onkeydown={handleQuickIndexDialogKeydown}>
      <div class="quick-index-label-grid">
        {#each quickIndexLabels as label (label)}
          {@const available = availableIndexLabels.has(label)}
          <button type="button" class="quick-index-label" class:wide-label={label === '漢字'} class:unavailable={!available} disabled={!available} onclick={() => chooseQuickIndexLabel(label)}>{getQuickIndexDisplayLabel(label)}</button>
        {/each}
      </div>
    </div>
  {/if}
</div>

<svelte:window onkeydown={handleWindowKeydown} />

<style>
  .paged-virtual-frame {
    position: relative;
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

  .paged-virtual-frame.leading-overlay {
    position: absolute;
    z-index: 1;
    inset: 0;
    display: block;
    height: 100%;
    overflow: visible;
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

  .paged-virtual-frame.leading-overlay .paged-virtual-viewport {
    position: absolute;
    z-index: 1;
    top: var(--paged-viewport-top, 0px);
    right: 0;
    bottom: 0;
    left: 0;
    width: 100%;
    height: auto;
    max-height: none;
    overflow: auto;
    flex: none;
  }

  @supports (animation-timeline: scroll()) and (timeline-scope: --album-detail-scroll) {
    .paged-virtual-viewport.album-detail-scroll-timeline {
      scroll-timeline: --album-detail-scroll block;
    }

    .paged-virtual-frame.leading-overlay .paged-virtual-header {
      animation: album-detail-grid-header-collapse auto linear both;
      animation-timeline: --album-detail-scroll;
      animation-range: 0px var(--paged-collapse-range);
    }

    @keyframes album-detail-grid-header-collapse {
      from { top: var(--paged-leading-height); }
      to { top: calc(var(--paged-leading-height) - var(--paged-collapse-range)); }
    }
  }

  .paged-virtual-frame.leading-overlay .paged-virtual-header {
    position: absolute;
    z-index: 3;
    right: 0;
    left: 0;
    width: auto;
    min-width: 0;
    pointer-events: auto;
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

  /* Long-press reorder still allows native pans on both axes (horizontal overflow on touch). */
  .paged-virtual-row.reorderable {
    touch-action: pan-x pan-y;
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

  /* Group headers stay pinned to the visible viewport width (translate follows scrollLeft) while
     the rows scroll horizontally; vertical behavior is unchanged. */
  .paged-virtual-group-row {
    display: flex;
    align-items: center;
    right: auto;
    width: max(100%, var(--track-list-min-width, 0px));
    overflow: visible;
  }

  /* Pure-CSS pin: the row spans the full scroll width; the cell sticks to the left edge and is
     exactly the visible width, so it never moves while rows scroll horizontally (no JS per frame). */
  .paged-virtual-group-cell {
    position: sticky;
    left: 0;
    box-sizing: border-box;
    display: flex;
    width: var(--paged-viewport-width, 100%);
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

  .paged-virtual-group-button:focus-visible {
    outline: 2px solid var(--accent);
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
    padding: 12px;
    overflow: auto;
    box-sizing: border-box;
    transform: translate(-50%, -50%);
  }

  .quick-index-label-grid {
    display: grid;
    grid-template-columns: repeat(var(--quick-index-columns), 2em);
    gap: 4px;
    font-size: var(--font-title);
  }

  .quick-index-label {
    display: grid;
    width: 2em;
    height: 2em;
    box-sizing: border-box;
    min-height: 0;
    place-items: center;
    border: 1px solid transparent;
    border-radius: 7px;
    color: var(--text);
    background: transparent;
    cursor: pointer;
    font: inherit;
  }

  .quick-index-label:hover:not(:disabled) {
    border-color: var(--line);
    background: rgba(var(--text-rgb), 0.09);
  }

  .quick-index-label.wide-label {
    grid-column: span 2;
    width: auto;
    white-space: nowrap;
  }

  .quick-index-label:disabled,
  .quick-index-label.unavailable {
    color: var(--muted-text);
    opacity: 0.42;
    cursor: default;
  }
</style>
