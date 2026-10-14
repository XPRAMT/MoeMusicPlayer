<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { getAlbumThumbnailBytesForTrack, getTrackArtworkBytes, invokeCommand, markAlbumArtworkMissing, putAlbumThumbnailBase64, putAlbumThumbnailBase64ForTrack } from './ipc';
  import { acquireAlbumCardArtwork, copyToArrayBuffer, createAlbumThumbnailInWorker, rememberAlbumCardArtwork, resolveAlbumArtworkDisplaySources, withAlbumArtworkSlot } from './album-artwork.js';
  import type { AlbumIndexEntry as AlbumEntry } from './ipc';
  import type { AlbumIndexSortField } from './ipc';
  import { formatDurationHours } from './format';
  import { detectArtworkMimeType } from './active-track-artwork';
  import AlbumArtworkMosaic from './AlbumArtworkMosaic.svelte';
  import defaultCoverUrl from '../../icon/DefaultCover.jpg?inline';

  interface Props { album: AlbumEntry; active: boolean; refreshKey: number; sortField: AlbumIndexSortField; onSelect: (album: AlbumEntry, coverSources: string[]) => void }
  let { album, active, refreshKey, sortField, onSelect }: Props = $props();
  let card = $state<HTMLElement | undefined>(undefined);
  let inViewport = $state(false);
  let imageUrls = $state<string[]>([]);
  let artworkRevision = $state(0);
  let busy = false;
  let destroyed = false;
  let requestRevision = 0;
  let cacheResolved = false;
  let releaseArtworkCacheLease: (() => void) | null = null;

  const memoryCacheKey = $derived(`${refreshKey}:${album.key}:${artworkRevision}`);
  const displayedImageUrls = $derived(resolveAlbumArtworkDisplaySources(imageUrls, {
    fallbackUrl: defaultCoverUrl,
    resolved: cacheResolved || album.unmarked,
  }));
  const summary = $derived.by(() => {
    const trackCount = `${album.trackCount}首曲目`;
    if (sortField === 'duration') return `${trackCount}．${formatDurationHours(album.durationMs)}`;
    if (sortField === 'played') return `${trackCount}．${formatDurationHours(album.playedMs)}`;
    return trackCount;
  });

  async function persistentSelectionKey(): Promise<string> {
    const bytes = new TextEncoder().encode(`${album.key}\0${album.trackCount}`);
    return [...new Uint8Array(await crypto.subtle.digest('SHA-256', copyToArrayBuffer(bytes)))]
      .map((byte) => byte.toString(16).padStart(2, '0')).join('');
  }

  async function loadTrackThumbnail(trackId: string): Promise<{ url: string; bytes: number } | null> {
    return withAlbumArtworkSlot(async () => {
      let indexed = new Uint8Array();
      try { indexed = new Uint8Array(await getAlbumThumbnailBytesForTrack(trackId)); } catch { /* rebuild a missing or damaged cache entry */ }
      if (indexed.length === 1 && indexed[0] === 0) return null;
      if (indexed.length > 3) {
        return {
          url: URL.createObjectURL(new Blob([copyToArrayBuffer(indexed)], { type: 'image/jpeg' })),
          bytes: indexed.byteLength,
        };
      }

      const original = await getTrackArtworkBytes(trackId);
      if (original.byteLength === 0) {
        void markAlbumArtworkMissing(trackId).catch(() => undefined);
        return null;
      }
      const mimeType = detectArtworkMimeType(original);
      if (!mimeType) {
        void markAlbumArtworkMissing(trackId).catch(() => undefined);
        return null;
      }
      const processed = await createAlbumThumbnailInWorker(original, mimeType);
      let thumbnail: Uint8Array;
      try { thumbnail = new Uint8Array(await getAlbumThumbnailBytesForTrack(trackId)); } catch { thumbnail = new Uint8Array(); }
      if (thumbnail.length <= 3) thumbnail = processed.bytes;
      await putAlbumThumbnailBase64ForTrack(trackId, processed.sha256, processed.dataBase64).catch((error) => {
        console.warn('無法保存專輯卡片縮圖快取', error);
        return putAlbumThumbnailBase64(processed.sha256, processed.dataBase64).catch(() => undefined);
      });
      return {
        url: URL.createObjectURL(new Blob([copyToArrayBuffer(thumbnail)], { type: 'image/jpeg' })),
        bytes: thumbnail.byteLength,
      };
    });
  }

  function remember(urls: string[], bytes: number): void {
    const cachedArtwork = rememberAlbumCardArtwork(memoryCacheKey, urls.length ? urls : null, bytes);
    releaseArtworkCacheLease = cachedArtwork?.release ?? null;
    const value = cachedArtwork?.value;
    imageUrls = Array.isArray(value) ? value : value ? [value] : [];
    cacheResolved = true;
  }

  async function restorePersistentSelection(revision: number): Promise<boolean> {
    const key = await persistentSelectionKey();
    for (let attempt = 0; attempt < 2; attempt += 1) {
      if (!active || destroyed || revision !== requestRevision) return false;
      let selection;
      try {
        selection = await invokeCommand('album_artwork_selection_ensure', {
          key,
          albumNames: album.aliases,
          trackCount: album.trackCount,
          representative: false,
          forceRebuild: attempt > 0,
        });
      } catch { return false; }
      if (!selection || selection.trackCount !== album.trackCount || revision !== requestRevision) return false;
      const trackId = selection.trackIds[0];
      if (!trackId) {
        // Empty selections are persisted negative-cache entries: this album has no readable cover.
        remember([], 0);
        return true;
      }
      try {
        const image = await loadTrackThumbnail(trackId);
        if (!image) continue;
        if (!active || destroyed || revision !== requestRevision) {
          URL.revokeObjectURL(image.url);
          return false;
        }
        remember([image.url], image.bytes);
        return true;
      } catch { /* rebuild the first-artwork selection once if its source disappeared */ }
    }
    return false;
  }

  async function loadCover(): Promise<void> {
    if (!active || busy || cacheResolved || album.unmarked) return;
    const cached = acquireAlbumCardArtwork(memoryCacheKey);
    if (cached) {
      releaseArtworkCacheLease = cached.release;
      const value = cached.value;
      imageUrls = Array.isArray(value) ? value : value ? [value] : [];
      cacheResolved = true;
      return;
    }
    const revision = ++requestRevision;
    busy = true;
    const urls: string[] = [];
    let bytes = 0;
    try {
      await restorePersistentSelection(revision);
      if (cacheResolved) return;
      if (!active || destroyed || revision !== requestRevision) {
        urls.forEach((url) => URL.revokeObjectURL(url));
        return;
      }
      if (!cacheResolved) remember(urls, bytes);
    } catch (error) {
      urls.forEach((url) => URL.revokeObjectURL(url));
      console.warn(`無法載入專輯「${album.displayName}」封面`, error);
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
      releaseArtworkCacheLease?.();
      releaseArtworkCacheLease = null;
      imageUrls = [];
      cacheResolved = false;
      artworkRevision += 1;
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

<button class="album-card" type="button" bind:this={card} onclick={() => onSelect(album, displayedImageUrls)} aria-label={`開啟專輯「${album.displayName}」，${album.trackCount} 首曲目`}>
  <span class="album-card-art" aria-hidden="true">
    <AlbumArtworkMosaic sources={displayedImageUrls} />
  </span>
  <span class="album-card-name">{album.displayName}</span>
  <span class="album-card-summary">{summary}</span>
</button>

<style>
  .album-card { display:flex; width:var(--album-card-width); height:calc(var(--album-card-width) + 52px); min-width:var(--album-card-width); flex:0 0 var(--album-card-width); flex-direction:column; gap:4px; padding:0; border:0; color:var(--text); background:transparent; text-align:left; cursor:pointer; }
  .album-card-art { display:block; width:var(--album-card-width); height:var(--album-card-width); flex:0 0 var(--album-card-width); box-sizing:border-box; overflow:hidden; border-radius:10px; background:#000; }
  .album-card-name { overflow:hidden; font-size:var(--font-body); line-height:1.4; text-overflow:ellipsis; white-space:nowrap; }
  .album-card-summary { overflow:hidden; color:var(--text-muted); font-size:var(--font-caption); line-height:1.4; text-overflow:ellipsis; white-space:nowrap; }
  .album-card:focus-visible { outline:2px solid var(--accent); outline-offset:3px; border-radius:10px; }
</style>
