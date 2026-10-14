<script lang="ts">
  import { onMount } from 'svelte';
  import { getErrorText, invokeCommand, type ArtistIndexEntry, type ArtistListSort } from './ipc';
  import { buildArtistIdentityMappings, preferredArtistName } from './artist-identities.js';
  import { DEFAULT_TRACK_ROW_HEIGHT_PX } from './track-columns.js';
  import { formatDuration } from './format';
  import { formatListOrder } from './list-order.js';
  import PagedVirtualList from './PagedVirtualList.svelte';
  import type { VirtualListRow } from './PagedVirtualList.svelte';
  import TrackColumnCells from './TrackColumnCells.svelte';

  type ArtistRow = ArtistIndexEntry & { displayName: string };
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
    onSelect: (artist: ArtistRow) => void;
    onCountChange: (count: number | null) => void;
    onRowsChange: (rows: ArtistRow[]) => void;
  }

  let { refreshKey, rowHeightPx = DEFAULT_TRACK_ROW_HEIGHT_PX, onSelect, onCountChange, onRowsChange }: Props = $props();
  let rows = $state<ArtistRow[]>([]);
  let titleSortRank = $state<Record<string, number>>({});
  let sortField = $state<SortField>('title');
  let sortDirection = $state<'asc' | 'desc'>('asc');
  let loading = $state(false);
  let error = $state<string | null>(null);
  let requestRevision = 0;
  let listRevision = $state(0);
  let sortSaveQueue: Promise<void> = Promise.resolve();
  let initialized = $state(false);
  let seenRefreshKey = $state<number | null>(null);

  const traditionalByName = new Map<string, string>();

  async function load(): Promise<void> {
    const revision = ++requestRevision;
    loading = true;
    error = null;
    onCountChange(null);
    try {
      const facets = await invokeCommand('library_get_artist_index_facets', {});
      const [artistIdentities, albumIdentities] = await Promise.all([
        buildArtistIdentityMappings(facets.artists),
        buildArtistIdentityMappings(facets.albums),
      ]);
      if (revision !== requestRevision) return;
      traditionalByName.clear();
      for (const identity of artistIdentities) traditionalByName.set(identity.name, identity.traditional);
      const result = await invokeCommand('library_get_artist_index', {
        artistIdentities,
        albumIdentities,
      }) as ArtistIndexEntry[];
      if (revision !== requestRevision) return;
      const nextRows = result.map((entry) => ({
        ...entry,
        displayName: preferredArtistName(entry.names, traditionalByName) || entry.artistNames[0] || '',
      }));
      const sortedIndices = await invokeCommand('library_sort_text_indices', {
        values: nextRows.map((entry) => entry.displayName),
      });
      if (revision !== requestRevision) return;
      titleSortRank = Object.fromEntries(sortedIndices.map((rowIndex, rank) => [nextRows[rowIndex]?.key, rank]));
      rows = nextRows;
      onRowsChange(rows);
      onCountChange(rows.length);
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

  function changeSort(field: SortField): void {
    if (field === sortField) sortDirection = sortDirection === 'asc' ? 'desc' : 'asc';
    else {
      sortField = field;
      sortDirection = field === 'title' ? 'asc' : 'desc';
    }
    listRevision += 1;
    persistSort({ field: sortField, direction: sortDirection });
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

  function sortValue(row: ArtistRow, field: SortField): string | number {
    if (field === 'title') return row.displayName;
    if (field === 'duration') return row.playedMs;
    if (field === 'trackDuration') return row.durationMs;
    return row[field];
  }

  let visibleRows = $derived.by(() => rows.toSorted((left, right) => {
    const leftValue = sortValue(left, sortField);
    const rightValue = sortValue(right, sortField);
    const compared = typeof leftValue === 'string' && typeof rightValue === 'string'
      ? (titleSortRank[left.key] ?? 0) - (titleSortRank[right.key] ?? 0)
      : Number(leftValue) - Number(rightValue);
    return sortDirection === 'asc' ? compared : -compared;
  }));

  onMount(() => { void initialize(); });
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
  {:else if loading && rows.length === 0}
    <div class="track-list-empty" aria-hidden="true"></div>
  {:else if visibleRows.length === 0}
    <div class="track-list-empty" role="status">目前啟用的曲庫沒有演出者資料。</div>
  {:else}
    <PagedVirtualList
      totalCount={visibleRows.length}
      pending={loading}
      revision={listRevision}
      generation={listRevision}
      itemAt={(index) => visibleRows[index] ?? null}
      getKey={(artist) => artist.key}
      rowHeight={rowHeightPx}
      compactRowHeight={rowHeightPx}
      shortDesktopRowHeight={rowHeightPx}
      headerHeight={37}
      compactHeaderHeight={37}
      fillAvailable={true}
      listId="artist-index"
      scrollResetKey={JSON.stringify({ field: sortField, direction: sortDirection })}
      ariaLabel="演出者統計；使用方向鍵瀏覽，按 Enter 開啟演出者歌曲"
      columnCount={ARTIST_COLUMNS.length + 1}
      gridTemplate={ARTIST_GRID}
      className="track-list-viewport"
      rowClassName="track-row virtual-track-row configurable-track-grid artist-index-row"
      listName="演出者"
      onRange={() => {}}
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
