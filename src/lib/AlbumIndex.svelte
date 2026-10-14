<script lang="ts">
  import { onDestroy } from 'svelte';
  import { getAlbumIndexScrollTop, setAlbumIndexScrollTop } from './album-index-scroll.js';
  import { getErrorText, invokeCommand } from './ipc';
  import type { AlbumIndexEntry, AlbumIndexSort, AlbumIndexSortField } from './ipc';
  import TrackColumnCells from './TrackColumnCells.svelte';
  import AlbumCard from './AlbumCard.svelte';
  import { PagedListController } from './track-list-data.js';

  type AlbumIndexSortDirection = 'asc' | 'desc';

  interface Props {
    refreshKey: number;
    active: boolean;
    onCountChange: (count: number | null) => void;
    onSelect: (album: AlbumIndexEntry) => void;
  }
  let { refreshKey, active, onCountChange, onSelect }: Props = $props();
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
  const rowCount = $derived(Math.ceil(totalCount / columns));
  const firstRow = $derived(Math.max(0, Math.floor(scrollTop / rowHeight) - overscanRows));
  const visibleRowCount = $derived(Math.ceil(viewportHeight / rowHeight) + overscanRows * 2 + 1);
  const visibleStart = $derived(firstRow * columns);
  const visibleEnd = $derived(Math.min(totalCount, (firstRow + visibleRowCount) * columns));
  const visibleAlbums = $derived.by(() => {
    pageSnapshot.revision;
    return Array.from({ length: Math.max(0, visibleEnd - visibleStart) }, (_, offset) => {
      const index = visibleStart + offset;
      return { index, album: listData.itemAt(index) };
    });
  });
  const topSpacer = $derived(firstRow * rowHeight);
  const bottomSpacer = $derived(Math.max(0, (rowCount - Math.ceil(visibleEnd / columns)) * rowHeight));

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
        groupingEnabled: false,
      });
      if (revision !== requestRevision) return;
      quickIndexTotal = info.quickIndex.totalCount;
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

  $effect(() => {
    const start = visibleStart;
    const end = visibleEnd;
    if (pageSnapshot.scope !== null && end > start) void listData.ensureRange(start, end);
  });

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
  {#if error}
    <div class="album-index-message" role="alert"><span>無法載入專輯：{error}</span><button class="text-button" type="button" onclick={() => void load()}>重試</button></div>
  {:else if totalCount === 0 && loading}
    <div class="album-index-message" aria-hidden="true"></div>
  {:else if totalCount === 0}
    <div class="album-index-message" role="status">目前啟用的曲庫沒有專輯資料。</div>
  {:else}
    <div class="album-grid-viewport" bind:this={viewport} data-scroll-restored={scrollPositionRestored} onscroll={handleScroll}>
      <div class="album-grid" style={`grid-template-columns:repeat(${columns},${cardWidth}px);gap:${gap}px;--album-card-width:${cardWidth}px`}>
        <div class="album-grid-spacer" style={`height:${topSpacer}px`} aria-hidden="true"></div>
        {#each visibleAlbums as slot (`${slot.album?.key ?? slot.index}:${refreshKey}`)}
          {#if slot.album}
            <AlbumCard album={slot.album} {active} {refreshKey} {sortField} {onSelect} />
          {:else}
            <div class="album-card-placeholder" aria-hidden="true"></div>
          {/if}
        {/each}
        <div class="album-grid-spacer" style={`height:${bottomSpacer}px`} aria-hidden="true"></div>
      </div>
    </div>
  {/if}
</div>

<style>
  .album-index-shell { display:flex; min-width:0; min-height:0; flex:1 1 auto; flex-direction:column; }
  .album-index-sort-header { position:relative; flex:0 0 auto; padding:0 clamp(18px,2vw,34px); }
  .album-index-sort-row { box-sizing:border-box; width:100%; min-width:0; --track-list-min-width:0px; --paged-grid-template:repeat(4,minmax(0,1fr)); }
  :global(.album-index-sort-row.track-list-header) { box-sizing:border-box; height:var(--paged-header-height,37px); min-height:var(--paged-header-height,37px); }
  :global(.album-index-sort-row .column-sort-button) { justify-content:center; text-align:center; }
  .album-grid-viewport { min-width:0; min-height:0; flex:1 1 auto; overflow:auto; overscroll-behavior:contain; padding:16px clamp(18px,2vw,34px) 28px; scrollbar-color:rgba(var(--text-rgb),0.2) transparent; scrollbar-width:thin; }
  .album-grid { display:grid; align-items:start; }
  .album-card-placeholder { width:var(--album-card-width); aspect-ratio:1; }
  .album-grid-spacer { grid-column:1 / -1; pointer-events:none; }
  .album-index-message { display:flex; min-height:160px; flex:1; align-items:center; justify-content:center; gap:12px; color:var(--muted); font-size:var(--font-body); }
  @media (max-width:620px) { .album-index-sort-header { padding-right:14px; padding-left:14px; } .album-grid-viewport { padding-right:14px; padding-left:14px; } }
</style>
