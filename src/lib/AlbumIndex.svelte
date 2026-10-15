<script lang="ts">
  import { onDestroy } from 'svelte';
  import { getAlbumIndexScrollTop, setAlbumIndexScrollTop } from './album-index-scroll.js';
  import { getErrorText, invokeCommand } from './ipc';
  import type { AlbumIndexEntry, AlbumIndexSort, AlbumIndexSortField } from './ipc';
  import TrackColumnCells from './TrackColumnCells.svelte';
  import AlbumCard from './AlbumCard.svelte';
  import { PagedListController } from './track-list-data.js';
  import { buildAlbumIndexRows, findVisibleAlbumIndexRows, shouldGroupAlbumIndex } from './album-index-layout.js';
  import { getQuickIndexColumnCount, getQuickIndexDisplayLabel } from './quick-index.js';

  type AlbumIndexSortDirection = 'asc' | 'desc';

  interface Props {
    refreshKey: number;
    active: boolean;
    groupingEnabled?: boolean;
    onCountChange: (count: number | null) => void;
    onSelect: (album: AlbumIndexEntry, coverSources: string[]) => void;
  }
  let { refreshKey, active, groupingEnabled = true, onCountChange, onSelect }: Props = $props();
  let sortField = $state<AlbumIndexSortField>('title');
  let sortDirection = $state<AlbumIndexSortDirection>('asc');
  let loading = $state(false);
  let error = $state<string | null>(null);
  let viewport = $state<HTMLElement | undefined>(undefined);
  let viewportWidth = $state(0);
  let viewportHeight = $state(0);
  let scrollTop = $state(0);
  let scrollPositionRestored = $state(false);
  let restoreFrame: number | null = null;
  let requestRevision = 0;
  let seenRefreshKey: number | null = null;
  let initialized = false;
  let sortLoaded = false;
  let quickIndexTotal = $state(0);
  let quickIndex = $state<{ labels: string[]; groups: { label: string; beforeIndex: number }[]; totalCount: number } | null>(null);
  let quickIndexOpen = $state(false);
  let quickIndexDialog: HTMLDivElement | undefined = $state();
  let quickIndexCenterX = $state(0);
  let quickIndexCenterY = $state(0);
  let quickIndexMaxWidth = $state(0);
  let quickIndexMaxHeight = $state(0);
  let quickIndexTrigger: HTMLButtonElement | null = null;
  let seenGroupingEnabled: boolean | null = null;
  type ListSnapshot = ReturnType<PagedListController<AlbumIndexEntry, string>['snapshot']>;
  let pageSnapshot = $state<ListSnapshot>({
    scope: null, totalCount: null, cachedPageCount: 0, cachedItemCount: 0,
    pendingPageCount: 0, refreshing: false, errors: [], generation: 0, revision: 0,
  });
  const listData = new PagedListController<AlbumIndexEntry, string>(
    async ({ scope, offset, limit }) => invokeCommand('library_get_album_index_snapshot_page', {
      snapshotId: String(scope ?? ''), offset, limit,
    }),
    { listName: '專輯', onChange: () => { pageSnapshot = listData.snapshot(); } },
  );
  const gap = 18;
  const cardWidth = 168;
  const overscanRows = 2;
  const albumColumns = [
    { id: 'title', label: '專輯' },
    { id: 'trackCount', label: '歌曲數量' },
    { id: 'played', label: '累積播放' },
    { id: 'duration', label: '播放時長' },
  ];

  const columns = $derived(Math.max(1, Math.floor((viewportWidth + gap) / (cardWidth + gap))));
  const rowHeight = $derived(cardWidth + 52 + gap);
  const totalCount = $derived(pageSnapshot.totalCount ?? quickIndexTotal);
  const shouldGroup = $derived(shouldGroupAlbumIndex({ sortField, groupingEnabled }));
  const groups = $derived(shouldGroup ? quickIndex?.groups ?? [] : []);
  const rowLayout = $derived(buildAlbumIndexRows({
    totalCount,
    columns,
    cardRowHeight: rowHeight,
    groups,
  }));
  const visibleRows = $derived.by(() => {
    // Album entries live in a non-reactive paged controller. Rebuild the
    // virtual row collection whenever a page is committed so itemAt() is
    // evaluated again even when sorting leaves the grid geometry unchanged.
    pageSnapshot.revision;
    return findVisibleAlbumIndexRows(
      rowLayout.rows,
      scrollTop,
      viewportHeight,
      rowHeight * overscanRows,
    );
  });
  const visibleDataRange = $derived.by(() => {
    pageSnapshot.revision;
    const cardRows = visibleRows.filter((row) => row.kind === 'cards');
    if (cardRows.length === 0) return null;
    return {
      start: Math.min(...cardRows.map((row) => row.start)),
      end: Math.max(...cardRows.map((row) => row.end)),
    };
  });
  const availableQuickIndexLabels = $derived(new Set(groups.map(({ label }) => label)));
  const cardLoadDiagnostic = $derived.by(() => {
    if (totalCount <= 0 || pageSnapshot.cachedItemCount > 0 || pageSnapshot.errors.length > 0) return null;
    return `總數 ${totalCount}；scope ${String(pageSnapshot.scope ?? 'null').slice(0, 8)}；頁面總數 ${pageSnapshot.totalCount ?? 'null'}；已載入 ${pageSnapshot.cachedItemCount}；載入中 ${pageSnapshot.pendingPageCount}；範圍 ${visibleDataRange ? `${visibleDataRange.start}-${visibleDataRange.end}` : 'null'}`;
  });

  async function load(): Promise<void> {
    const revision = ++requestRevision;
    loading = true;
    error = null;
    onCountChange(null);
    try {
      if (!sortLoaded) {
        try {
          const saved: AlbumIndexSort = await invokeCommand('settings_get_album_index_sort', {});
          if (revision !== requestRevision) return;
          sortField = saved.field;
          sortDirection = saved.direction;
        } catch (reason) {
          console.warn('無法讀取專輯列表排序設定，使用預設排序', reason);
        }
        sortLoaded = true;
      }
      if (revision !== requestRevision) return;
      await loadSnapshot(revision);
      if (viewport) scrollTop = viewport.scrollTop;
    } catch (reason) {
      if (revision === requestRevision) {
        error = getErrorText(reason);
        onCountChange(null);
      }
    } finally {
      if (revision === requestRevision) loading = false;
    }
  }

  async function loadSnapshot(revision = ++requestRevision): Promise<void> {
    if (revision !== requestRevision) return;
    loading = true;
    error = null;
    try {
      const info = await invokeCommand('library_create_album_index_snapshot', {
        sortField,
        direction: sortDirection,
        groupingEnabled: shouldGroupAlbumIndex({ sortField, groupingEnabled }),
      });
      if (revision !== requestRevision) return;
      quickIndexTotal = info.quickIndex.totalCount;
      quickIndex = {
        labels: info.quickIndex.labels,
        groups: info.quickIndex.groups,
        totalCount: info.quickIndex.totalCount,
      };
      listData.reset(info.snapshotId, info.snapshotId);
      pageSnapshot = listData.snapshot();
      onCountChange(info.quickIndex.totalCount);
    } catch (reason) {
      if (revision === requestRevision) {
        error = getErrorText(reason);
        onCountChange(null);
      }
    } finally {
      if (revision === requestRevision) loading = false;
    }
  }

  function changeSort(field: AlbumIndexSortField): void {
    if (field === sortField) {
      sortDirection = sortDirection === 'asc' ? 'desc' : 'asc';
    } else {
      sortField = field;
      sortDirection = field === 'title' ? 'asc' : 'desc';
    }
    const preference: AlbumIndexSort = { field, direction: sortDirection };
    void invokeCommand('settings_set_album_index_sort', { preference }).catch((reason) => {
      console.warn('無法保存專輯列表排序設定', reason);
    });
    if (viewport) {
      viewport.scrollTop = 0;
      scrollTop = 0;
      setAlbumIndexScrollTop(0);
    }
    void loadSnapshot();
  }

  function retryPage(offset: number): void {
    void listData.retry(offset);
  }

  function requestFirstCardPage(): void {
    void listData.ensureRange(0, Math.min(totalCount, listData.pageSize));
  }

  $effect(() => {
    const range = visibleDataRange;
    if (pageSnapshot.scope !== null && range && range.end > range.start) {
      void listData.ensureRange(range.start, range.end);
    }
  });

  function openQuickIndex(event: MouseEvent): void {
    if (!viewport || groups.length === 0) return;
    const rect = viewport.getBoundingClientRect();
    quickIndexCenterX = rect.left + rect.width / 2;
    quickIndexCenterY = rect.top + rect.height / 2;
    quickIndexMaxWidth = Math.max(0, rect.width - 24);
    quickIndexMaxHeight = Math.max(0, rect.height - 24);
    quickIndexTrigger = event.currentTarget as HTMLButtonElement;
    quickIndexOpen = true;
  }

  function chooseQuickIndexLabel(label: string): void {
    quickIndexOpen = false;
    const group = groups.find((candidate) => candidate.label === label);
    const groupRow = group && rowLayout.rows.find((row) => row.kind === 'group' && row.label === group.label);
    if (groupRow && viewport) {
      viewport.scrollTop = groupRow.top;
      scrollTop = groupRow.top;
      setAlbumIndexScrollTop(groupRow.top);
      viewport.focus();
    }
  }

  function closeQuickIndex(restoreFocus = true): void {
    quickIndexOpen = false;
    if (restoreFocus) queueMicrotask(() => quickIndexTrigger?.focus());
  }

  function handleQuickIndexDialogKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.preventDefault();
      closeQuickIndex();
      return;
    }
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

  function measure(): void {
    if (!viewport) return;
    const style = getComputedStyle(viewport);
    viewportWidth = Math.max(0, viewport.clientWidth - Number.parseFloat(style.paddingLeft) - Number.parseFloat(style.paddingRight));
    viewportHeight = Math.max(0, viewport.clientHeight - Number.parseFloat(style.paddingTop) - Number.parseFloat(style.paddingBottom));
    scrollTop = viewport.scrollTop;
  }

  function handleScroll(): void {
    measure();
    if (scrollPositionRestored && viewport) setAlbumIndexScrollTop(viewport.scrollTop);
  }

  $effect(() => {
    const element = viewport;
    if (!element) return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    measure();
    return () => observer?.disconnect();
  });
  $effect(() => {
    const current = refreshKey;
    if (!initialized) {
      seenRefreshKey = current;
      initialized = true;
      void load();
      return;
    }
    if (current === seenRefreshKey) return;
    seenRefreshKey = current;
    void load();
  });
  $effect(() => {
    const current = groupingEnabled;
    if (!initialized) return;
    if (seenGroupingEnabled === null) {
      seenGroupingEnabled = current;
      return;
    }
    if (current === seenGroupingEnabled) return;
    seenGroupingEnabled = current;
    if (shouldGroupAlbumIndex({ sortField, groupingEnabled: true })) void loadSnapshot();
  });
  $effect(() => {
    if (quickIndexOpen) quickIndexDialog?.querySelector<HTMLButtonElement>('.album-quick-index-label:not(:disabled)')?.focus();
  });
  $effect(() => {
    const element = viewport;
    if (!active || loading || !element || viewportWidth <= 0 || scrollPositionRestored) return;
    if (restoreFrame !== null) cancelAnimationFrame(restoreFrame);
    restoreFrame = requestAnimationFrame(() => {
      restoreFrame = null;
      if (!active || !viewport || scrollPositionRestored) return;
      const maxScrollTop = Math.max(0, viewport.scrollHeight - viewport.clientHeight);
      const restored = Math.min(getAlbumIndexScrollTop(), maxScrollTop);
      viewport.scrollTop = restored;
      scrollTop = restored;
      setAlbumIndexScrollTop(restored);
      scrollPositionRestored = true;
    });
    return () => {
      if (restoreFrame !== null) cancelAnimationFrame(restoreFrame);
      restoreFrame = null;
    };
  });
  onDestroy(() => {
    requestRevision += 1;
    if (restoreFrame !== null) cancelAnimationFrame(restoreFrame);
  });
</script>

<div class="album-index-shell" aria-label="專輯卡片清單">
  {#if !error}
    <div class="paged-virtual-header album-index-sort-header">
      <div class="paged-virtual-header-shift">
        <div class="track-table-head track-list-header configurable-track-grid album-index-sort-row" role="row" aria-rowindex="1">
          <TrackColumnCells
            variant="header"
            columns={albumColumns}
            includeIndexColumn={false}
            sortable={true}
            sort={{ field: sortField, direction: sortDirection }}
            onSort={(field) => changeSort(field as AlbumIndexSortField)}
          />
        </div>
      </div>
    </div>
  {/if}
  {#if pageSnapshot.errors.length > 0}
    {@const pageError = pageSnapshot.errors[0]}
    <div class="album-index-page-error" role="alert">
      <span>專輯卡片載入失敗：{getErrorText(pageError?.message)}</span>
      {#if pageError}
        <button class="text-button" type="button" onclick={() => retryPage(pageError.offset)}>重試</button>
      {/if}
    </div>
  {/if}
  {#if cardLoadDiagnostic}
    <div class="album-index-page-error" role="status" data-album-page-diagnostic>
      <span>專輯卡片尚未載入：{cardLoadDiagnostic}</span>
      <button class="text-button" type="button" onclick={requestFirstCardPage}>重新請求</button>
    </div>
  {/if}
  {#if error}
    <div class="album-index-message" role="alert"><span>無法載入專輯：{error}</span><button class="text-button" type="button" onclick={() => void load()}>重試</button></div>
  {:else if totalCount === 0 && loading}
    <div class="album-index-message" aria-hidden="true"></div>
  {:else if totalCount === 0}
    <div class="album-index-message" role="status">目前啟用的曲庫沒有專輯資料。</div>
  {:else}
    <div class="album-grid-viewport" bind:this={viewport} data-scroll-restored={scrollPositionRestored} onscroll={handleScroll}>
      <div class="album-grid-canvas" style={`height:${rowLayout.totalHeight}px;--album-card-width:${cardWidth}px`}>
        {#each visibleRows as row (row.kind === 'group' ? `group:${row.beforeIndex}:${row.label}` : `cards:${row.start}`)}
          {#if row.kind === 'group'}
            <div class="album-index-group-row" style={`height:${row.height}px;transform:translateY(${row.top}px)`}>
              <button type="button" class="album-index-group-button" aria-label={`開啟「${row.label}」字首索引`} onclick={openQuickIndex}>{row.label}</button>
            </div>
          {:else}
            <div class="album-index-card-row" style={`height:${row.height}px;transform:translateY(${row.top}px);grid-template-columns:repeat(${columns},${cardWidth}px);gap:${gap}px`}>
              {#each Array.from({ length: row.end - row.start }, (_, offset) => row.start + offset) as index (listData.itemAt(index)?.key ?? `loading:${index}:${refreshKey}`)}
                {@const album = listData.itemAt(index)}
                {#if album}
                  <AlbumCard {album} {active} {refreshKey} {sortField} {onSelect} />
                {:else}
                  <div class="album-card-placeholder" aria-hidden="true"></div>
                {/if}
              {/each}
            </div>
          {/if}
        {/each}
      </div>
    </div>
  {/if}
</div>
{#if quickIndexOpen}
  <button type="button" class="album-quick-index-backdrop" aria-label="關閉首字索引" onclick={() => closeQuickIndex()}></button>
  <div bind:this={quickIndexDialog} class="album-quick-index-popover floating-surface" style={`left:${quickIndexCenterX}px;top:${quickIndexCenterY}px;--quick-index-columns:${getQuickIndexColumnCount(quickIndex?.labels.length ?? 0)};--quick-index-max-width:${quickIndexMaxWidth}px;--quick-index-max-height:${quickIndexMaxHeight}px`} role="dialog" aria-modal="true" aria-label="快速首字查找" tabindex="-1" onkeydown={handleQuickIndexDialogKeydown}>
    <div class="album-quick-index-label-grid">
      {#each quickIndex?.labels ?? [] as label (label)}
        {@const available = availableQuickIndexLabels.has(label)}
        <button type="button" class="album-quick-index-label" class:wide-label={label === '漢字'} class:unavailable={!available} disabled={!available} onclick={() => chooseQuickIndexLabel(label)}>{getQuickIndexDisplayLabel(label)}</button>
      {/each}
    </div>
  </div>
{/if}

<style>
  .album-index-shell { display:flex; min-width:0; min-height:0; flex:1 1 auto; flex-direction:column; }
  .album-index-sort-header { position:relative; flex:0 0 auto; padding:0 clamp(18px,2vw,34px); }
  .album-index-sort-row { box-sizing:border-box; width:100%; min-width:0; --track-list-min-width:0px; --paged-grid-template:repeat(4,minmax(0,1fr)); }
  :global(.album-index-sort-row.track-list-header) { box-sizing:border-box; height:var(--paged-header-height,37px); min-height:var(--paged-header-height,37px); }
  :global(.album-index-sort-row .column-sort-button) { justify-content:center; text-align:center; }
  .album-grid-viewport { min-width:0; min-height:0; flex:1 1 auto; overflow:auto; overscroll-behavior:contain; padding:16px clamp(18px,2vw,34px) 28px; scrollbar-color:rgba(var(--text-rgb),0.2) transparent; scrollbar-width:thin; }
  .album-grid-canvas { position:relative; min-width:0; }
  .album-index-group-row, .album-index-card-row { position:absolute; top:0; right:0; left:0; box-sizing:border-box; }
  .album-index-group-row { display:flex; align-items:center; border-bottom:1px solid rgba(var(--text-rgb),0.055); background:transparent; }
  .album-index-group-button { display:flex; width:100%; height:100%; min-width:0; align-items:center; justify-content:center; box-sizing:border-box; padding:0; border:0; border-radius:0; color:var(--text); background:transparent; cursor:pointer; font:inherit; }
  .album-index-group-button:hover { background:rgba(var(--text-rgb),0.05); }
  .album-index-group-button:focus-visible { outline:2px solid var(--line); outline-offset:-2px; }
  .album-index-card-row { display:grid; align-content:start; }
  .album-card-placeholder { width:var(--album-card-width); aspect-ratio:1; }
  .album-quick-index-backdrop { position:fixed; z-index:220; inset:0; padding:0; border:0; background:transparent; }
  .album-quick-index-popover { position:fixed; z-index:221; display:flex; width:max-content; max-width:min(calc(100vw - 24px),var(--quick-index-max-width)); max-height:var(--quick-index-max-height); flex-direction:column; padding:12px; overflow:auto; box-sizing:border-box; transform:translate(-50%,-50%); }
  .album-quick-index-label-grid { display:grid; grid-template-columns:repeat(var(--quick-index-columns),2em); gap:4px; font-size:var(--font-title); }
  .album-quick-index-label { display:grid; width:2em; height:2em; box-sizing:border-box; min-height:0; place-items:center; border:1px solid transparent; border-radius:7px; color:var(--text); background:transparent; cursor:pointer; font:inherit; }
  .album-quick-index-label:hover:not(:disabled) { border-color:var(--line); background:rgba(var(--text-rgb),0.09); }
  .album-quick-index-label:focus-visible { outline:2px solid var(--line); outline-offset:0; }
  .album-quick-index-label.unavailable { color:var(--muted); opacity:0.45; cursor:default; }
  .album-index-message { display:flex; min-height:160px; flex:1; align-items:center; justify-content:center; gap:12px; color:var(--muted); font-size:var(--font-body); }
  .album-index-page-error { display:flex; flex:0 0 auto; align-items:center; justify-content:center; gap:12px; padding:8px 16px; color:var(--danger,var(--text)); font-size:var(--font-caption); }
  @media (max-width:620px) { .album-index-sort-header { padding-right:14px; padding-left:14px; } .album-grid-viewport { padding-right:14px; padding-left:14px; } }
</style>
