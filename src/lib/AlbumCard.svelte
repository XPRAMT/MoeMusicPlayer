<script lang="ts">
  import { tr } from './i18n.js';
  import { displayUnknown } from './unknown-label.js';
  import { onDestroy, onMount, untrack } from 'svelte';
  import { acquireAlbumCardArtwork, albumCardArtworkMemoryKey, getAlbumArtworkSelectionRevision, rememberAlbumCardArtwork, resolveAlbumArtworkDisplaySources } from './album-artwork.js';
  import { loadAlbumCardCover } from './album-card-cover';
  import type { AlbumIndexEntry as AlbumEntry } from './ipc';
  import type { AlbumIndexSortField } from './ipc';
  import { formatDurationHours } from './format';
  import AlbumArtworkMosaic from './AlbumArtworkMosaic.svelte';
  import MiddleEllipsis from './MiddleEllipsis.svelte';
  import defaultCoverUrl from '../../icon/DefaultCover.jpg?inline';

  interface Props { album: AlbumEntry; active: boolean; refreshKey: number; sortField: AlbumIndexSortField; onSelect: (album: AlbumEntry, coverSources: string[]) => void }
  let { album, active, refreshKey, sortField, onSelect }: Props = $props();
  let card = $state<HTMLElement | undefined>(undefined);
  let inViewport = $state(false);
  let imageUrls = $state<string[]>([]);
  // Read the album-wide revision before the first cache lookup so a remounted
  // virtual card cannot briefly acquire a stale selection under revision 0.
  let artworkRevision = $state(untrack(() => getAlbumArtworkSelectionRevision(album.key)));
  let busy = false;
  let destroyed = false;
  let requestRevision = 0;
  let cacheResolved = false;
  let releaseArtworkCacheLease: (() => void) | null = null;

  const memoryCacheKey = $derived(albumCardArtworkMemoryKey(refreshKey, album.key, artworkRevision));
  const displayedImageUrls = $derived(resolveAlbumArtworkDisplaySources(imageUrls, {
    fallbackUrl: defaultCoverUrl,
    resolved: cacheResolved,
  }));
  const summary = $derived.by(() => {
    const trackCount = tr("{0}首曲目", [album.trackCount]);
    if (sortField === 'duration') return `${trackCount}．${formatDurationHours(album.durationMs)}`;
    if (sortField === 'played') return `${trackCount}．${formatDurationHours(album.playedMs)}`;
    return trackCount;
  });

  function remember(urls: string[], bytes: number): void {
    const previousLease = releaseArtworkCacheLease;
    const cachedArtwork = rememberAlbumCardArtwork(memoryCacheKey, urls.length ? urls : null, bytes);
    releaseArtworkCacheLease = cachedArtwork?.release ?? null;
    const value = cachedArtwork?.value;
    imageUrls = Array.isArray(value) ? value : value ? [value] : [];
    cacheResolved = true;
    previousLease?.();
  }

  async function loadCover(): Promise<void> {
    // The unmarked (未知) group loads artwork like any album: its aliases ("" / blank) match its tracks.
    const cached = acquireAlbumCardArtwork(memoryCacheKey);
    if (cached) {
      const previousLease = releaseArtworkCacheLease;
      releaseArtworkCacheLease = cached.release;
      const value = cached.value;
      imageUrls = Array.isArray(value) ? value : value ? [value] : [];
      cacheResolved = true;
      previousLease?.();
      return;
    }
    const revision = ++requestRevision;
    busy = true;
    const wanted = () => active && !destroyed && revision === requestRevision;
    try {
      const result = await loadAlbumCardCover(album, wanted);
      if (!wanted()) {
        result?.urls.forEach((url) => URL.revokeObjectURL(url));
        return;
      }
      // Nothing loaded now (a failed lookup) is remembered like no cover for this list, as before.
      remember(result?.urls ?? [], result?.bytes ?? 0);
    } catch (error) {
      console.warn(tr("無法載入專輯「{0}」封面", [album.displayName]), error);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    let observer: IntersectionObserver | null = null;
    if (!card || typeof IntersectionObserver === 'undefined') inViewport = true;
    else {
      observer = new IntersectionObserver((entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          observer?.disconnect();
          inViewport = true;
        }
      }, { rootMargin: '80px' });
      observer.observe(card);
    }
    const refreshSelection = (event: Event) => {
      const detail = (event as CustomEvent<{ albumKey?: string }>).detail;
      if (detail?.albumKey !== album.key) return;
      // Keep the old, visible mosaic leased until the refreshed selection is fully loaded.
      requestRevision += 1;
      busy = false;
      cacheResolved = false;
      artworkRevision = getAlbumArtworkSelectionRevision(album.key);
      if (active && inViewport) void loadCover();
    };
    window.addEventListener('album-artwork-selection-updated', refreshSelection);
    return () => {
      observer?.disconnect();
      window.removeEventListener('album-artwork-selection-updated', refreshSelection);
    };
  });
  $effect(() => {
    const enabled = active;
    const visible = inViewport;
    if (!enabled) {
      requestRevision += 1;
      busy = false;
    } else if (visible) void loadCover();
  });
  onDestroy(() => { destroyed = true; requestRevision += 1; releaseArtworkCacheLease?.(); });
</script>

<button class="album-card" type="button" bind:this={card} onclick={() => onSelect(album, displayedImageUrls)} aria-label={tr("開啟專輯「{0}」，{1} 首曲目", [album.displayName, album.trackCount])}>
  <span class="album-card-art" aria-hidden="true">
    <AlbumArtworkMosaic sources={displayedImageUrls} />
  </span>
  <span class="album-card-name"><MiddleEllipsis text={displayUnknown(album.displayName)} mode="end" /></span>
  <span class="album-card-summary"><MiddleEllipsis text={summary} mode="end" /></span>
</button>

<style>
  .album-card { display:flex; width:var(--album-card-width); height:calc(var(--album-card-width) + var(--album-card-text-height)); min-width:var(--album-card-width); flex:0 0 var(--album-card-width); flex-direction:column; gap:4px; padding:0; border:0; color:var(--text); background:transparent; text-align:left; cursor:pointer; }
  .album-card-art { display:block; width:var(--album-card-width); height:var(--album-card-width); flex:0 0 var(--album-card-width); box-sizing:border-box; overflow:hidden; border-radius:10px; background:#000; }
  .album-card-name { overflow:hidden; font-size:var(--font-body); line-height:1.4; white-space:nowrap; }
  .album-card-summary { overflow:hidden; color:var(--text-muted); font-size:var(--font-caption); line-height:1.4; white-space:nowrap; }
  .album-card:focus-visible { outline:2px solid var(--accent); outline-offset:3px; border-radius:10px; }
</style>
