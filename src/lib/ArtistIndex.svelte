<script lang="ts">
  import { onMount } from 'svelte';
  import { getErrorText, invokeCommand, type ArtistIndexEntry } from './ipc';
  import { buildArtistIdentityMappings, preferredArtistName } from './artist-identities.js';
  import { DEFAULT_TRACK_ROW_HEIGHT_PX } from './track-columns.js';
  import { formatDuration } from './format';

  type ArtistRow = ArtistIndexEntry & { displayName: string };
  type SortField = 'name' | 'trackCount' | 'albumCount' | 'playedMs';

  interface Props {
    refreshKey: number;
    rowHeightPx?: number;
    onSelect: (artist: ArtistRow) => void;
  }

  let { refreshKey, rowHeightPx = DEFAULT_TRACK_ROW_HEIGHT_PX, onSelect }: Props = $props();
  let rows = $state<ArtistRow[]>([]);
  let sortField = $state<SortField>('name');
  let sortDirection = $state<'asc' | 'desc'>('asc');
  let loading = $state(false);
  let error = $state<string | null>(null);
  let requestRevision = 0;

  const traditionalByName = new Map<string, string>();

  async function load(): Promise<void> {
    const revision = ++requestRevision;
    loading = true;
    error = null;
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
      rows = result.map((entry) => ({
        ...entry,
        displayName: preferredArtistName(entry.names, traditionalByName) || entry.artistNames[0] || '',
      }));
    } catch (reason) {
      if (revision === requestRevision) error = getErrorText(reason);
    } finally {
      if (revision === requestRevision) loading = false;
    }
  }

  function changeSort(field: SortField): void {
    if (field === sortField) sortDirection = sortDirection === 'asc' ? 'desc' : 'asc';
    else {
      sortField = field;
      sortDirection = field === 'name' ? 'asc' : 'desc';
    }
  }

  function sortValue(row: ArtistRow, field: SortField): string | number {
    return field === 'name' ? row.displayName : row[field];
  }

  let visibleRows = $derived.by(() => {
    return rows
      .toSorted((left, right) => {
        const leftValue = sortValue(left, sortField);
        const rightValue = sortValue(right, sortField);
        const compared = typeof leftValue === 'string' && typeof rightValue === 'string'
          ? leftValue.localeCompare(rightValue, 'zh-Hant', { numeric: true, sensitivity: 'base' })
          : Number(leftValue) - Number(rightValue);
        return sortDirection === 'asc' ? compared : -compared;
      });
  });

  onMount(() => { void load(); });
  $effect(() => {
    const current = refreshKey;
    if (current > 0) void load();
  });
</script>

<section class="artist-index" aria-label="演出者清單" style={`--artist-row-height: ${rowHeightPx}px`}>
  {#if error}
    <div class="artist-message" role="alert">
      <span>無法載入演出者清單：{error}</span>
      <button class="text-button" type="button" onclick={() => void load()}>重試</button>
    </div>
  {:else if loading && rows.length === 0}
    <div class="artist-message" role="status">正在計算演出者資料…</div>
  {:else if visibleRows.length === 0}
    <div class="artist-message" role="status">目前啟用的曲庫沒有演出者資料。</div>
  {:else}
    <div class="artist-table track-table" role="table" aria-label="演出者統計">
      <div class="artist-row artist-heading track-table-head" role="row">
        <span class="artist-order-heading column-index" role="columnheader">#</span>
        <button class="column-sort-button" type="button" role="columnheader" aria-sort={sortField === 'name' ? (sortDirection === 'asc' ? 'ascending' : 'descending') : 'none'} onclick={() => changeSort('name')}>
          <span class="column-sort-label">演出者</span>{#if sortField === 'name'}<span class="column-sort-indicator" aria-hidden="true">{sortDirection === 'asc' ? '↑' : '↓'}</span>{/if}
        </button>
        <button class="column-sort-button" type="button" role="columnheader" aria-sort={sortField === 'trackCount' ? (sortDirection === 'asc' ? 'ascending' : 'descending') : 'none'} onclick={() => changeSort('trackCount')}>
          <span class="column-sort-label">歌曲數量</span>{#if sortField === 'trackCount'}<span class="column-sort-indicator" aria-hidden="true">{sortDirection === 'asc' ? '↑' : '↓'}</span>{/if}
        </button>
        <button class="column-sort-button" type="button" role="columnheader" aria-sort={sortField === 'albumCount' ? (sortDirection === 'asc' ? 'ascending' : 'descending') : 'none'} onclick={() => changeSort('albumCount')}>
          <span class="column-sort-label">專輯數量</span>{#if sortField === 'albumCount'}<span class="column-sort-indicator" aria-hidden="true">{sortDirection === 'asc' ? '↑' : '↓'}</span>{/if}
        </button>
        <button class="column-sort-button" type="button" role="columnheader" aria-sort={sortField === 'playedMs' ? (sortDirection === 'asc' ? 'ascending' : 'descending') : 'none'} onclick={() => changeSort('playedMs')}>
          <span class="column-sort-label">播放時長</span>{#if sortField === 'playedMs'}<span class="column-sort-indicator" aria-hidden="true">{sortDirection === 'asc' ? '↑' : '↓'}</span>{/if}
        </button>
      </div>
      {#each visibleRows as artist, index (artist.key)}
        <button class="artist-row artist-entry track-row" type="button" role="row" onclick={() => onSelect(artist)}>
          <span class="artist-order" role="cell">{index + 1}</span>
          <span class="artist-name" role="cell" title={artist.displayName}>{artist.displayName}</span>
          <span role="cell">{artist.trackCount.toLocaleString()}</span>
          <span role="cell">{artist.albumCount.toLocaleString()}</span>
          <span role="cell">{formatDuration(artist.playedMs)}</span>
        </button>
      {/each}
    </div>
  {/if}
</section>

<style>
  .artist-index { display: flex; min-height: 0; height: 100%; flex-direction: column; }
  .artist-table { min-height: 0; overflow: auto; }
  .artist-row { display: grid; width: 100%; grid-template-columns: 36px minmax(150px, 1fr) 120px 120px 150px; align-items: center; text-align: left; }
  .artist-heading { position: sticky; z-index: 2; top: 0; background: var(--surface); }
  .artist-heading .column-sort-button { display: flex; width: 100%; height: 37px; box-sizing: border-box; min-width: 0; align-items: center; gap: 4px; margin: 0; padding: 0; border: 0; color: var(--text-soft); background: transparent; font-family: Consolas, monospace; font-size: var(--font-body); letter-spacing: 0.07em; text-align: left; cursor: pointer; }
  .artist-heading .column-sort-button:hover .column-sort-label,
  .artist-heading .column-sort-button:focus-visible .column-sort-label { color: var(--accent-text); }
  .artist-heading .column-sort-button:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; border-radius: 4px; }
  .artist-heading .column-sort-label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .artist-heading .column-sort-indicator { flex: 0 0 auto; font-size: 9px; line-height: 1; opacity: 0.85; }
  .artist-entry { position: relative; min-height: var(--artist-row-height); background: transparent; font: inherit; cursor: pointer; }
  .artist-entry > span:not(:nth-child(2)) { color: var(--text-soft); }
  .artist-order-heading, .artist-order { text-align: center; color: var(--text-soft); }
  .artist-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .artist-message { display: flex; align-items: center; gap: 10px; color: var(--text-muted); font-size: var(--font-body); }
  @media (max-width: 620px) { .artist-row { grid-template-columns: 26px minmax(100px, 1fr) 68px 68px 88px; } }
</style>
