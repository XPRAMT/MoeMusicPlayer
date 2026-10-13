<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { getErrorText, invokeCommand } from './ipc';
  import type { AlbumIndexFacet } from './ipc';
  import { buildAlbumEntries } from './album-identities.js';
  import type { AlbumEntry } from './album-identities.js';
  import AlbumCard from './AlbumCard.svelte';

  interface Props {
    refreshKey: number;
    active: boolean;
    onCountChange: (count: number | null) => void;
    onRowsChange: (rows: AlbumEntry[]) => void;
    onSelect: (album: AlbumEntry) => void;
  }
  let { refreshKey, active, onCountChange, onRowsChange, onSelect }: Props = $props();
  let albums = $state<AlbumEntry[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let viewport = $state<HTMLElement | undefined>(undefined);
  let viewportWidth = $state(0);
  let viewportHeight = $state(0);
  let scrollTop = $state(0);
  let requestRevision = 0;
  let seenRefreshKey: number | null = null;
  let initialized = false;
  const gap = 18;
  const minimumCardWidth = 168;
  const overscanRows = 2;

  const columns = $derived(Math.max(1, Math.floor((viewportWidth + gap) / (minimumCardWidth + gap))));
  const cardWidth = $derived(viewportWidth > 0 ? (viewportWidth - gap * (columns - 1)) / columns : minimumCardWidth);
  const rowHeight = $derived(cardWidth + 48 + gap);
  const rowCount = $derived(Math.ceil(albums.length / columns));
  const firstRow = $derived(Math.max(0, Math.floor(scrollTop / rowHeight) - overscanRows));
  const visibleRowCount = $derived(Math.ceil(viewportHeight / rowHeight) + overscanRows * 2 + 1);
  const visibleStart = $derived(firstRow * columns);
  const visibleEnd = $derived(Math.min(albums.length, (firstRow + visibleRowCount) * columns));
  const visibleAlbums = $derived(albums.slice(visibleStart, visibleEnd));
  const topSpacer = $derived(firstRow * rowHeight);
  const bottomSpacer = $derived(Math.max(0, (rowCount - Math.ceil(visibleEnd / columns)) * rowHeight));

  async function load(): Promise<void> {
    const revision = ++requestRevision;
    loading = true;
    error = null;
    onCountChange(null);
    try {
      const facets = await invokeCommand('library_get_album_index_facets', {}) as AlbumIndexFacet[];
      const rows = await buildAlbumEntries(facets);
      if (revision !== requestRevision) return;
      albums = rows;
      onRowsChange(rows);
      onCountChange(rows.length);
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

  function measure(): void {
    if (!viewport) return;
    viewportWidth = viewport.clientWidth;
    viewportHeight = viewport.clientHeight;
    scrollTop = viewport.scrollTop;
  }

  onMount(() => {
    const observer = viewport ? new ResizeObserver(measure) : null;
    if (viewport) observer?.observe(viewport);
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
  onDestroy(() => { requestRevision += 1; });
</script>

<div class="album-index-shell" aria-label="專輯卡片清單">
  {#if error}
    <div class="album-index-message" role="alert"><span>無法載入專輯：{error}</span><button class="text-button" type="button" onclick={() => void load()}>重試</button></div>
  {:else if albums.length === 0 && loading}
    <div class="album-index-message" role="status">正在整理專輯…</div>
  {:else if albums.length === 0}
    <div class="album-index-message" role="status">目前啟用的曲庫沒有專輯資料。</div>
  {:else}
    <div class="album-grid-viewport" bind:this={viewport} onscroll={measure}>
      <div class="album-grid" style={`grid-template-columns:repeat(${columns},minmax(0,1fr));gap:${gap}px;--album-card-width:${cardWidth}px`}>
        <div class="album-grid-spacer" style={`height:${topSpacer}px`} aria-hidden="true"></div>
        {#each visibleAlbums as album (`${album.key}:${refreshKey}`)}
          <AlbumCard {album} {active} {onSelect} />
        {/each}
        <div class="album-grid-spacer" style={`height:${bottomSpacer}px`} aria-hidden="true"></div>
      </div>
    </div>
  {/if}
</div>

<style>
  .album-index-shell { display:flex; min-width:0; min-height:0; flex:1 1 auto; flex-direction:column; }
  .album-grid-viewport { min-width:0; min-height:0; flex:1 1 auto; overflow:auto; overscroll-behavior:contain; padding:16px clamp(18px,2vw,34px) 28px; scrollbar-color:rgba(var(--text-rgb),0.2) transparent; scrollbar-width:thin; }
  .album-grid { display:grid; align-items:start; }
  .album-grid-spacer { grid-column:1 / -1; pointer-events:none; }
  .album-index-message { display:flex; min-height:160px; flex:1; align-items:center; justify-content:center; gap:12px; color:var(--muted); font-size:var(--font-body); }
  @media (max-width:620px) { .album-grid-viewport { padding-right:14px; padding-left:14px; } }
</style>
