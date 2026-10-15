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
  import { buildPagedVirtualRows, dataIndexToVisualIndex, getGroupedVirtualRange, getQuickIndexColumnCount, getQuickIndexDisplayLabel, getRestoredListScrollTop, getVirtualRowTop, getVirtualRowsHeight, getVisualIndexAtOffset, reportPagedRange, shouldAdjustGroupAnchor, visualIndexToAnchorDataIndex, visualIndexToDataIndex, visualRangeToDataRange } from './quick-index.js';
  import {
    QUEUE_REORDER_LONG_PRESS_MS,
    QUEUE_REORDER_MOVE_CANCEL_PX,
  } from './queue-reorder.js';
  import { advanceAlbumDetailWheelScroll, getAlbumDetailRowClipTop, mapAlbumDetailNativeScroll, mapAlbumDetailWheelDelta } from './album-detail-collapse.js';

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
    leadingCollapseOffset?: number;
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
    leadingCollapseOffset = 0,
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
  let viewport: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let headerShift = $state(0);
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
  let quickIndexDialog: HTMLDivElement | undefined = $state();
  let quickIndexTrigger: HTMLButtonElement | null = null;
  let previousGroupRows: QuickIndexGroup[] = [];
  let hasPreviousGroupSnapshot = false;
  let lastNativeScrollTop = 0;
  let handlingMappedScroll = false;
  let wheelAnimationFrame: number | null = null;
  let wheelTargetNativeScrollTop: number | null = null;
  let lastWheelAnimationTime = 0;

  const virtualDataCount = $derived(totalCount ?? 40);
  const renderGroupRows = $derived(groupingReady ? groupRows : []);
  const virtualTotalCount = $derived(virtualDataCount + renderGroupRows.length);
  const groupRowHeight = $derived(Math.max(30, rowHeight * 0.7));
  const virtualContentHeight = $derived(
    getVirtualRowsHeight(virtualDataCount, renderGroupRows, rowHeight, groupRowHeight),
  );
  const hasLeadingOverlay = $derived(leadingOverlayHeight > 0);
  const contentLeadingHeight = $derived(hasLeadingOverlay ? leadingOverlayHeight + headerHeight : 0);
  const effectiveViewportHeight = $derived(hasLeadingOverlay
    ? Math.max(rowHeight, viewportHeight - leadingOverlayHeight + leadingCollapseOffset - headerHeight)
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

  function nativeScrollTopFor(logicalTop: number, collapseOffset = leadingCollapseOffset): number {
    return hasLeadingOverlay ? Math.max(0, logicalTop) + Math.max(0, collapseOffset) : Math.max(0, logicalTop);
  }

  function logicalScrollTopFromNative(nativeTop: number): number {
    return hasLeadingOverlay ? Math.max(0, nativeTop - leadingCollapseOffset) : Math.max(0, nativeTop);
  }

  function setLogicalScrollTop(logicalTop: number, collapseOffset = leadingCollapseOffset): void {
    if (!viewport) return;
    cancelWheelScrollAnimation();
    const nativeMaximum = Math.max(0, viewport.scrollHeight - viewport.clientHeight);
    const next = Math.max(0, Math.min(nativeMaximum - (hasLeadingOverlay ? collapseOffset : 0), logicalTop));
    const nativeTop = nativeScrollTopFor(next, collapseOffset);
    handlingMappedScroll = true;
    viewport.scrollTop = nativeTop;
    handlingMappedScroll = false;
    scrollTop = next;
    lastVisibleScrollTop = next;
    lastNativeScrollTop = nativeTop;
    syncLeadingOverlayRowClips(collapseOffset);
    rememberVisibleScrollPosition(next);
    onViewportScrollTopChange?.(next, collapseOffset);
  }

  function syncLeadingOverlayRowClips(collapseOffset = leadingCollapseOffset): void {
    if (!hasLeadingOverlay || !viewport || !frame) return;
    const viewportTop = viewport.getBoundingClientRect().top;
    const listBoundary = viewportTop + Math.max(0, leadingOverlayHeight - collapseOffset) + headerHeight;
    const rowClips = [...frame.querySelectorAll<HTMLElement>('.paged-virtual-row[role="row"]')].map((row) => {
      const rect = row.getBoundingClientRect();
      return { row, clippedTop: Math.min(rect.height, Math.max(0, listBoundary - rect.top)) };
    });
    for (const { row, clippedTop } of rowClips) {
      row.style.clipPath = clippedTop > 0 ? `inset(${clippedTop}px 0px 0px)` : '';
    }
  }

  function getVisibleRowsHeight(): number {
    if (!viewport) return viewportHeight;
    return hasLeadingOverlay
      ? Math.max(rowHeight, viewport.clientHeight - leadingOverlayHeight + leadingCollapseOffset - headerHeight)
      : viewport.clientHeight;
  }

  function getMaximumLogicalScrollTop(): { listScrollTop: number; collapseOffset: number } {
    if (!viewport) return { listScrollTop: 0, collapseOffset: 0 };
    const maximumNativeTop = Math.max(0, viewport.scrollHeight - viewport.clientHeight);
    if (!hasLeadingOverlay) return { listScrollTop: maximumNativeTop, collapseOffset: 0 };
    const mapped = mapAlbumDetailNativeScroll({
      rawScrollTop: maximumNativeTop,
      previousRawScrollTop: 0,
      previousListScrollTop: 0,
      previousCollapseOffset: 0,
      baseCoverSize: 64 + leadingCollapseRange,
    });
    return { listScrollTop: mapped.listScrollTop, collapseOffset: mapped.collapseOffset };
  }

  function moveLogicalBy(delta: number): void {
    if (!viewport) return;
    const mappedWheel = hasLeadingOverlay
      ? mapAlbumDetailWheelDelta({
        delta,
        listScrollTop: scrollTop,
        collapseOffset: leadingCollapseOffset,
        baseCoverSize: 64 + leadingCollapseRange,
      })
      : { listScrollTop: Math.max(0, scrollTop + delta), collapseOffset: 0 };
    let nextLogical = mappedWheel.listScrollTop;
    let nextCollapse = mappedWheel.collapseOffset;
    let nextNative = nativeScrollTopFor(nextLogical, nextCollapse);
    const maximumNative = Math.max(0, viewport.scrollHeight - viewport.clientHeight);
    if (nextNative > maximumNative && hasLeadingOverlay) {
      const mapped = mapAlbumDetailNativeScroll({
        rawScrollTop: maximumNative,
        previousRawScrollTop: lastNativeScrollTop,
        previousListScrollTop: scrollTop,
        previousCollapseOffset: leadingCollapseOffset,
        baseCoverSize: 64 + leadingCollapseRange,
      });
      nextLogical = mapped.listScrollTop;
      nextCollapse = mapped.collapseOffset;
      nextNative = mapped.rawScrollTop;
    }
    setLogicalScrollTop(nextLogical, nextCollapse);
  }

  function cancelWheelScrollAnimation(): void {
    if (wheelAnimationFrame !== null) cancelAnimationFrame(wheelAnimationFrame);
    wheelAnimationFrame = null;
    wheelTargetNativeScrollTop = null;
    lastWheelAnimationTime = 0;
  }

  function animateWheelScroll(now: number): void {
    wheelAnimationFrame = null;
    if (!viewport || wheelTargetNativeScrollTop === null) return;
    const maximum = Math.max(0, viewport.scrollHeight - viewport.clientHeight);
    const target = Math.max(0, Math.min(maximum, wheelTargetNativeScrollTop));
    wheelTargetNativeScrollTop = target;
    const current = viewport.scrollTop;
    const elapsed = lastWheelAnimationTime === 0 ? 16 : now - lastWheelAnimationTime;
    lastWheelAnimationTime = now;
    const next = advanceAlbumDetailWheelScroll(current, target, elapsed);
    if (next === target || Math.abs(target - next) <= 0.25) {
      viewport.scrollTop = target;
      updateViewport();
      cancelWheelScrollAnimation();
      return;
    }
    viewport.scrollTop = next;
    updateViewport();
    wheelAnimationFrame = requestAnimationFrame(animateWheelScroll);
  }

  function getVisibleRowsRect(): DOMRect | null {
    if (!viewport) return null;
    const rect = viewport.getBoundingClientRect();
    const top = rect.top + (hasLeadingOverlay ? leadingOverlayHeight - leadingCollapseOffset + headerHeight : 0);
    return new DOMRect(rect.left, top, rect.width, Math.max(0, rect.bottom - top));
  }

  function handleViewportWheel(event: WheelEvent): void {
    if (!hasLeadingOverlay || !viewport || event.ctrlKey || event.deltaY === 0) return;
    event.preventDefault();
    const delta = event.deltaMode === WheelEvent.DOM_DELTA_LINE
      ? event.deltaY * rowHeight
      : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
        ? event.deltaY * getVisibleRowsHeight()
        : event.deltaY;
    const maximum = Math.max(0, viewport.scrollHeight - viewport.clientHeight);
    const currentTarget = wheelTargetNativeScrollTop ?? viewport.scrollTop;
    wheelTargetNativeScrollTop = Math.max(0, Math.min(maximum, currentTarget + delta));
    if (wheelAnimationFrame === null) {
      lastWheelAnimationTime = 0;
      wheelAnimationFrame = requestAnimationFrame(animateWheelScroll);
    }
  }

  $effect(() => {
    const active = hasLeadingOverlay;
    const collapse = leadingCollapseOffset;
    leadingOverlayHeight;
    if (!active || !metricsReady || !viewport) return;
    const expectedNativeTop = nativeScrollTopFor(scrollTop, collapse);
    if (Math.abs(viewport.scrollTop - expectedNativeTop) > 0.5) {
      viewport.scrollTop = expectedNativeTop;
      lastNativeScrollTop = expectedNativeTop;
    }
    viewportHeight = viewport.clientHeight;
  });

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
      lastVisibleScrollTop = 0;
      viewport?.scrollTo({ top: 0 });
      lastNativeScrollTop = 0;
    } else if (resetChanged) {
      activeIndex = 0;
      scrollTop = 0;
      lastVisibleScrollTop = 0;
      writeListScrollPosition(currentListId, 0);
      pendingListRestore = null;
      viewport?.scrollTo({ top: 0 });
      lastNativeScrollTop = 0;
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
      const safeRestoredScrollTop = Math.min(restoredScrollTop, maximum.listScrollTop);
      const restoredCollapseOffset = hasLeadingOverlay
        ? Math.min(leadingCollapseRange, safeRestoredScrollTop)
        : 0;
      scrollTop = safeRestoredScrollTop;
      lastVisibleScrollTop = safeRestoredScrollTop;
      writeListScrollPosition(currentListId, safeRestoredScrollTop);
      if (viewport && viewport.clientHeight > 0) {
        viewport.scrollTop = nativeScrollTopFor(safeRestoredScrollTop, restoredCollapseOffset);
        lastNativeScrollTop = viewport.scrollTop;
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
      onViewportScrollTopChange?.(safeRestoredScrollTop, leadingCollapseOffset);
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
    const stopWheelAnimation = () => cancelWheelScrollAnimation();
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
          const restoredNativeTop = nativeScrollTopFor(restoredScrollTop, Math.min(leadingCollapseRange, restoredScrollTop));
          if (viewport.scrollTop !== restoredNativeTop) viewport.scrollTop = restoredNativeTop;
          scrollTop = restoredScrollTop;
          lastNativeScrollTop = restoredNativeTop;
          rememberVisibleScrollPosition(restoredScrollTop);
          hadVisibleViewport = true;
        } else {
          scrollTop = logicalScrollTopFromNative(viewport.scrollTop);
          lastNativeScrollTop = viewport.scrollTop;
          lastVisibleScrollTop = scrollTop;
          rememberVisibleScrollPosition(scrollTop);
        }
      } else if (nextViewportHeight === 0) {
        hadVisibleViewport = false;
      }
      viewportHeight = nextViewportHeight;
      if (effectiveViewportHeight > 0) metricsReady = true;
      if (viewport) headerGutter = Math.max(0, viewport.offsetWidth - viewport.clientWidth);
      if (viewport) onViewportScrollTopChange?.(scrollTop, leadingCollapseOffset);
    };
    recalculateMetrics = updateMetrics;
    updateMetrics();
    window.addEventListener('resize', updateMetrics);
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(updateMetrics);
    if (observer && frame) observer.observe(frame);
    if (observer && viewport) observer.observe(viewport);
    viewport?.addEventListener('pointerdown', stopWheelAnimation);
    viewport?.addEventListener('keydown', stopWheelAnimation);
    return () => {
      window.removeEventListener('resize', updateMetrics);
      observer?.disconnect();
      viewport?.removeEventListener('pointerdown', stopWheelAnimation);
      viewport?.removeEventListener('keydown', stopWheelAnimation);
      cancelWheelScrollAnimation();
      endDragState();
      recalculateMetrics = () => {};
    };
  });

  onDestroy(() => {
    cancelWheelScrollAnimation();
    const finalScrollTop = viewport?.clientHeight ? logicalScrollTopFromNative(viewport.scrollTop) : lastVisibleScrollTop;
    rememberVisibleScrollPosition(finalScrollTop);
  });

  function updateViewport(): void {
    if (!viewport) return;
    if (handlingMappedScroll) return;
    let collapseOffset = leadingCollapseOffset;
    if (viewport.clientHeight > 0) {
      const nativeTop = viewport.scrollTop;
      if (hasLeadingOverlay) {
        const mapped = mapAlbumDetailNativeScroll({
          rawScrollTop: nativeTop,
          previousRawScrollTop: lastNativeScrollTop,
          previousListScrollTop: scrollTop,
          previousCollapseOffset: leadingCollapseOffset,
          baseCoverSize: 64 + leadingCollapseRange,
        });
        if (mapped.rawScrollTop !== nativeTop) viewport.scrollTop = mapped.rawScrollTop;
        lastNativeScrollTop = mapped.rawScrollTop;
        scrollTop = mapped.listScrollTop;
        collapseOffset = mapped.collapseOffset;
      } else {
        lastNativeScrollTop = nativeTop;
        scrollTop = nativeTop;
        collapseOffset = 0;
      }
      lastVisibleScrollTop = scrollTop;
      rememberVisibleScrollPosition(scrollTop);
      hadVisibleViewport = true;
    }
    headerShift = viewport.scrollLeft;
    headerGutter = Math.max(0, viewport.offsetWidth - viewport.clientWidth);
    // viewportHeight is the measured viewport's clientHeight. The visible row
    // area for a collapsing album heading is derived from it separately; do
    // not overwrite the measured value during scrolling, or the heading and
    // header deductions are applied again on every scroll event.
    metricsReady = true;
    syncLeadingOverlayRowClips(collapseOffset);
    onViewportScrollTopChange?.(scrollTop, collapseOffset);
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
    const maxScrollTop = Math.max(0, viewport.scrollHeight - viewport.clientHeight - leadingCollapseOffset);
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
    quickIndexMaxWidth = Math.max(0, rect.width - 24);
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
        moveLogicalBy(delta);
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
  class:leading-overlay={hasLeadingOverlay}
  bind:this={frame}
  style={`--paged-header-height:${headerHeight}px;--paged-row-height:${rowHeight}px;--paged-grid-template:${gridTemplate};--track-list-min-width:${minContentWidth}px;--paged-leading-height:${leadingOverlayHeight}px;--paged-collapse-offset:${leadingCollapseOffset}px`}
>
  <div class="paged-virtual-header" role="presentation" style={`height:${headerHeight}px;${hasLeadingOverlay ? `top:${Math.max(0, leadingOverlayHeight - leadingCollapseOffset)}px;right:${headerGutter}px;` : `padding-right:${headerGutter}px;`}`}>
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
    onscroll={handleViewportScroll}
    onwheel={hasLeadingOverlay ? handleViewportWheel : undefined}
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
      {#if virtualRow.kind === 'group'}
        <div
          {id}
          class="paged-virtual-row paged-virtual-group-row list-group-row-surface"
          role="row"
          aria-rowindex={virtualRow.visualIndex + 2}
          style={`height:${groupRowHeight}px;transform:translateY(${contentLeadingHeight + getVirtualRowTop(virtualRow.visualIndex, renderGroupRows, rowHeight, groupRowHeight)}px);${hasLeadingOverlay ? `clip-path:inset(${getAlbumDetailRowClipTop(scrollTop, getVirtualRowTop(virtualRow.visualIndex, renderGroupRows, rowHeight, groupRowHeight), groupRowHeight)}px 0 0)` : ''}`}
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
        style={`height:${rowHeight}px;transform:translateY(${contentLeadingHeight + getVirtualRowTop(virtualRow.visualIndex, renderGroupRows, rowHeight, groupRowHeight)}px);${hasLeadingOverlay ? `clip-path:inset(${getAlbumDetailRowClipTop(scrollTop, getVirtualRowTop(virtualRow.visualIndex, renderGroupRows, rowHeight, groupRowHeight), rowHeight)}px 0 0)` : ''}`}
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
    <div bind:this={quickIndexDialog} class="quick-index-popover floating-surface" style={`left:${quickIndexCenterX}px;top:${quickIndexCenterY}px;--quick-index-columns:${getQuickIndexColumnCount(quickIndexLabels.length)};--quick-index-max-width:${quickIndexMaxWidth}px;--quick-index-max-height:${quickIndexMaxHeight}px`} role="dialog" aria-modal="true" aria-label="快速首字查找" tabindex="-1" onkeydown={handleQuickIndexDialogKeydown}>
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
    inset: 0;
    width: 100%;
    height: 100%;
    max-height: none;
    overflow: auto;
    flex: none;
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
