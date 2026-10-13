<script lang="ts">
  import { onMount } from 'svelte';
  import { IconSearch } from '@tabler/icons-svelte-runes';
  import { getErrorText, invokeCommand, type ArtistIndexEntry } from './ipc';
  import { buildArtistIdentityMappings, preferredArtistName } from './artist-identities.js';
  import { getTraditionalIdentity } from './search-variants.js';
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
  let query = $state('');
  let normalizedQuery = $state('');
  let sortField = $state<SortField>('name');
  let sortDirection = $state<'asc' | 'desc'>('asc');
  let loading = $state(false);
  let error = $state<string | null>(null);
  let requestRevision = 0;
  let queryRevision = 0;

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

  async function updateQuery(value: string): Promise<void> {
    const revision = ++queryRevision;
    query = value;
    const traditional = await getTraditionalIdentity(value);
    if (revision === queryRevision) normalizedQuery = traditional.normalize('NFKC').toLowerCase();
  }

  function sortValue(row: ArtistRow, field: SortField): string | number {
    return field === 'name' ? row.displayName : row[field];
  }

  let visibleRows = $derived.by(() => {
    const needle = normalizedQuery;
    return rows
      .filter((row) => !needle || row.key.includes(needle) || row.artistNames.some((name) => name.toLowerCase().includes(needle)))
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
  <label class="artist-search">
    <IconSearch size={16} stroke={1.6} aria-hidden="true" />
    <input type="search" aria-label="搜尋演出者" placeholder="搜尋演出者" value={query} oninput={(event) => void updateQuery(event.currentTarget.value)} />
  </label>
  {#if error}
    <div class="artist-message" role="alert">
      <span>無法載入演出者清單：{error}</span>
      <button class="text-button" type="button" onclick={() => void load()}>重試</button>
    </div>
  {:else if loading && rows.length === 0}
    <div class="artist-message" role="status">正在計算演出者資料…</div>
  {:else if visibleRows.length === 0}
    <div class="artist-message" role="status">{query ? '找不到相符演出者。' : '目前啟用的曲庫沒有演出者資料。'}</div>
  {:else}
    <div class="artist-table" role="table" aria-label="演出者統計">
      <div class="artist-row artist-heading" role="row">
        <span class="artist-order-heading" role="columnheader">#</span>
        <button type="button" role="columnheader" aria-sort={sortField === 'name' ? (sortDirection === 'asc' ? 'ascending' : 'descending') : 'none'} onclick={() => changeSort('name')}>
          <span>演出者</span>{#if sortField === 'name'}<span aria-hidden="true">{sortDirection === 'asc' ? '↑' : '↓'}</span>{/if}
        </button>
        <button type="button" role="columnheader" aria-sort={sortField === 'trackCount' ? (sortDirection === 'asc' ? 'ascending' : 'descending') : 'none'} onclick={() => changeSort('trackCount')}>
          <span>歌曲數量</span>{#if sortField === 'trackCount'}<span aria-hidden="true">{sortDirection === 'asc' ? '↑' : '↓'}</span>{/if}
        </button>
        <button type="button" role="columnheader" aria-sort={sortField === 'albumCount' ? (sortDirection === 'asc' ? 'ascending' : 'descending') : 'none'} onclick={() => changeSort('albumCount')}>
          <span>專輯數量</span>{#if sortField === 'albumCount'}<span aria-hidden="true">{sortDirection === 'asc' ? '↑' : '↓'}</span>{/if}
        </button>
        <button type="button" role="columnheader" aria-sort={sortField === 'playedMs' ? (sortDirection === 'asc' ? 'ascending' : 'descending') : 'none'} onclick={() => changeSort('playedMs')}>
          <span>播放時長</span>{#if sortField === 'playedMs'}<span aria-hidden="true">{sortDirection === 'asc' ? '↑' : '↓'}</span>{/if}
        </button>
      </div>
      {#each visibleRows as artist, index (artist.key)}
        <button class="artist-row artist-entry" type="button" role="row" onclick={() => onSelect(artist)}>
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
  .artist-index { display: flex; min-height: 0; height: 100%; flex-direction: column; gap: 12px; }
  .artist-search { display: flex; height: 36px; flex: 0 0 auto; align-items: center; gap: 9px; padding: 0 11px; border: 1px solid var(--line); border-radius: 10px; color: var(--text-muted); }
  .artist-search input { min-width: 0; width: 100%; border: 0; outline: 0; background: transparent; color: var(--text); font: inherit; }
  .artist-table { min-height: 0; overflow: auto; border-top: 1px solid var(--line); }
  .artist-row { display: grid; width: 100%; grid-template-columns: 36px minmax(150px, 1fr) 120px 120px 150px; align-items: center; gap: 12px; text-align: left; }
  .artist-heading { position: sticky; z-index: 2; top: 0; min-height: 37px; border-bottom: 1px solid var(--line); background: var(--surface); }
  .artist-heading button { display: flex; height: 37px; align-items: center; gap: 5px; padding: 0 10px; border: 0; background: transparent; color: var(--text-muted); font: inherit; font-size: var(--font-caption); cursor: pointer; }
  .artist-entry { min-height: var(--artist-row-height); padding: 0 10px; border: 0; border-bottom: 1px solid var(--line); background: transparent; color: var(--text); font: inherit; cursor: pointer; }
  .artist-entry:hover { background: color-mix(in srgb, var(--text) 5%, transparent); }
  .artist-entry > span:not(:nth-child(2)) { color: var(--text-muted); }
  .artist-order-heading, .artist-order { text-align: center; color: var(--text-muted); }
  .artist-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .artist-message { display: flex; align-items: center; gap: 10px; color: var(--text-muted); font-size: var(--font-body); }
  @media (max-width: 620px) { .artist-row { grid-template-columns: 26px minmax(100px, 1fr) 68px 68px 88px; gap: 5px; } }
</style>
