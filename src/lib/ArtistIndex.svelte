<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { getErrorText, invokeCommand, type ArtistIndexEntry, type ArtistListSort } from './ipc';
  import { DEFAULT_TRACK_ROW_HEIGHT_PX } from './track-columns.js';
  import { formatDuration } from './format';
  import { formatListOrder } from './list-order.js';
  import PagedVirtualList from './PagedVirtualList.svelte';
  import type { VirtualListRow } from './PagedVirtualList.svelte';
  import TrackColumnCells from './TrackColumnCells.svelte';
  import { PagedListController } from './track-list-data.js';

  type ArtistRow = ArtistIndexEntry;
  type SortField = 'title' | 'trackCount' | 'albumCount' | 'duration' | 'trackDuration';

  const ARTIST_COLUMNS = [
    { id: 'title', label: '演出者' },
    { id: 'trackCount', label: '歌曲數量' },
    { id: 'albumCount', label: '專輯數量' },
    { id: 'duration', label: '累積播放' },
    { id: 'trackDuration', label: '播放時長' },
  ];
  // All columns share the same font-size as their cells so em widths match between header and rows.
  const ARTIST_GRID = '36px minmax(150px,1fr) 6em 6em 6em 6em';

  interface Props {
    refreshKey: number;
    rowHeightPx?: number;
    groupingEnabled?: boolean;
    onSelect: (artist: ArtistRow) => void;
    onCountChange: (count: number | null) => void;
  }

  let { refreshKey, rowHeightPx = DEFAULT_TRACK_ROW_HEIGHT_PX, groupingEnabled = true, onSelect, onCountChange }: Props = $props();
  let sortField = $state<SortField>('title');
  let sortDirection = $state<'asc' | 'desc'>('asc');
  let loading = $state(false);
  let error = $state<string | null>(null);
  let requestRevision = 0;
  let listRevision = $state(0);
  let sortSaveQueue: Promise<void> = Promise.resolve();
  let initialized = $state(false);
  let seenRefreshKey = $state<number | null>(null);
  let quickIndex = $state<{ labels: string[]; groups: { label: string; beforeIndex: number }[]; totalCount: number } | null>(null);
  let quickIndexReady = $state(true);
  type ListSnapshot = ReturnType<PagedListController<ArtistRow, string>['snapshot']>;
  let pageSnapshot = $state<ListSnapshot>({
    scope: null, totalCount: null, cachedPageCount: 0, cachedItemCount: 0,
    pendingPageCount: 0, refreshing: false, errors: [], generation: 0, revision: 0,
  });
  const listData = new PagedListController<ArtistRow, string>(
    async ({ scope, offset, limit }) => {
      const snapshotId = String(scope ?? '');
      return invokeCommand('library_get_artist_index_snapshot_page', { snapshotId, offset, limit });
    },
    { listName: '演出者', onChange: () => { pageSnapshot = listData.snapshot(); } },
  );

  async function load(): Promise<void> {
    const revision = ++requestRevision;
    loading = true;
    error = null;
    onCountChange(null);
    try {
      await loadSnapshot(revision);
      listRevision += 1;
    } catch (reason) {
      if (revision === requestRevision) {
        onCountChange(null);
        error = getErrorText(reason);
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
      const info = await invokeCommand('library_create_artist_index_snapshot', {
        sortField,
        direction: sortDirection,
        groupingEnabled: groupingEnabled && sortField === 'title',
      });
      if (revision !== requestRevision) return;
      quickIndex = { labels: info.quickIndex.labels, groups: info.quickIndex.groups, totalCount: info.quickIndex.totalCount };
      quickIndexReady = true;
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

  function changeSort(field: SortField): void {
    if (field === sortField) sortDirection = sortDirection === 'asc' ? 'desc' : 'asc';
    else {
      sortField = field;
      sortDirection = field === 'title' ? 'asc' : 'desc';
    }
    listRevision += 1;
    persistSort({ field: sortField, direction: sortDirection });
    void loadSnapshot();
  }

  function persistSort(preference: ArtistListSort): void {
    sortSaveQueue = sortSaveQueue
      .catch(() => undefined)
      .then(() => invokeCommand('settings_set_artist_list_sort', { preference }))
      .then(() => undefined)
      .catch((reason: unknown) => {
        console.error('無法保存演出者清單排序方式', reason);
      });
  }

  async function initialize(): Promise<void> {
    try {
      const preference = await invokeCommand('settings_get_artist_list_sort', {});
      sortField = preference.field;
      sortDirection = preference.direction;
    } catch (reason) {
      console.error('無法讀取演出者清單排序方式，使用預設排序', reason);
    }
    await load();
    initialized = true;
  }

  let visibleCount = $derived(pageSnapshot.totalCount ?? quickIndex?.totalCount ?? 0);

  onMount(() => { void initialize(); });
  onDestroy(() => { requestRevision += 1; });
  $effect(() => {
    const current = refreshKey;
    if (!initialized) {
      if (seenRefreshKey === null) seenRefreshKey = current;
      return;
    }
    if (seenRefreshKey === null) {
      seenRefreshKey = current;
      return;
    }
    if (current === seenRefreshKey) return;
    seenRefreshKey = current;
    void load();
  });
</script>

<div class="track-list-shell" aria-label="演出者清單">
  {#if error}
    <div class="track-list-error" role="alert">
      <span>無法載入演出者清單：{error}</span>
      <button class="text-button" type="button" onclick={() => void load()}>重試</button>
    </div>
  {:else if !initialized || (loading && pageSnapshot.cachedItemCount === 0)}
    <div class="track-list-empty" aria-hidden="true"></div>
  {:else if visibleCount === 0}
    <div class="track-list-empty" role="status">目前啟用的曲庫沒有演出者資料。</div>
  {:else}
    <PagedVirtualList
      totalCount={visibleCount}
      pending={loading || pageSnapshot.pendingPageCount > 0}
      revision={pageSnapshot.revision + listRevision}
      generation={pageSnapshot.generation}
      itemAt={(index) => listData.itemAt(index)}
      getKey={(artist) => artist.key}
      rowHeight={rowHeightPx}
      compactRowHeight={rowHeightPx}
      shortDesktopRowHeight={rowHeightPx}
      headerHeight={37}
      compactHeaderHeight={37}
      fillAvailable={true}
      listId="artist-index"
      scrollResetKey={JSON.stringify({ field: sortField, direction: sortDirection })}
      groupRows={groupingEnabled && sortField === 'title' ? quickIndex?.groups : []}
      groupingReady={!groupingEnabled || sortField !== 'title' || quickIndexReady}
      quickIndexLabels={groupingEnabled && sortField === 'title' ? quickIndex?.labels : []}
      ariaLabel="演出者統計；使用方向鍵瀏覽，按 Enter 開啟演出者歌曲"
      columnCount={ARTIST_COLUMNS.length + 1}
      gridTemplate={ARTIST_GRID}
      className="track-list-viewport"
      rowClassName="track-row virtual-track-row configurable-track-grid artist-index-row"
      listName="演出者"
      onRange={(range) => { void listData.ensureRange(range.start, range.end); }}
      onPlay={(artist) => onSelect(artist)}
    >
      {#snippet header()}
        <div class="track-table-head track-list-header configurable-track-grid" role="row" aria-rowindex="1">
          <TrackColumnCells
            variant="header"
            columns={ARTIST_COLUMNS}
            sortable={true}
            sort={{ field: sortField, direction: sortDirection }}
            onSort={(field) => changeSort(field as SortField)}
          />
        </div>
      {/snippet}
      {#snippet row(row: VirtualListRow<ArtistRow>)}
        {#if row.item}
          <TrackColumnCells
            variant="item"
            columns={ARTIST_COLUMNS}
            indexLabel={formatListOrder(row.index)}
            cells={[
              { id: 'title', text: row.item.displayName },
              { id: 'trackCount', text: row.item.trackCount.toLocaleString() },
              { id: 'albumCount', text: row.item.albumCount.toLocaleString() },
              { id: 'duration', text: formatDuration(row.item.playedMs) },
              { id: 'trackDuration', text: formatDuration(row.item.durationMs) },
            ]}
          />
        {:else}
          <TrackColumnCells variant="placeholder" columns={ARTIST_COLUMNS} />
        {/if}
      {/snippet}
    </PagedVirtualList>
  {/if}
</div>
