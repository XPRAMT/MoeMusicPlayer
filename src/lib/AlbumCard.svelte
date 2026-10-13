<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { getAlbumThumbnailBytesForTrack, getTrackArtworkBytes, invokeCommand, markAlbumArtworkMissing, putAlbumThumbnailBase64, putAlbumThumbnailBase64ForTrack } from './ipc';
  import { acquireAlbumCardArtwork, copyToArrayBuffer, createAlbumThumbnailInWorker, rememberAlbumCardArtwork, withAlbumArtworkSlot } from './album-artwork.js';
  import type { AlbumEntry } from './album-identities.js';
  import { detectArtworkMimeType } from './active-track-artwork';
  import AlbumArtworkMosaic from './AlbumArtworkMosaic.svelte';
  import defaultCoverUrl from '../../icon/DefaultCover.jpg?inline';

  interface Props { album: AlbumEntry; active: boolean; refreshKey: number; onSelect: (album: AlbumEntry) => void }
  let { album, active, refreshKey, onSelect }: Props = $props();
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
  const displayedImageUrls = $derived(
    imageUrls.length > 0 ? imageUrls : cacheResolved || album.unmarked ? [defaultCoverUrl] : [],
  );

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
    let selection;
    try { selection = await invokeCommand('album_artwork_selection_get', { key }); } catch { return false; }
    if (!selection || selection.trackCount !== album.trackCount || revision !== requestRevision) return false;
    const urls: string[] = [];
    let bytes = 0;
    for (const trackId of selection.trackIds.slice(0, selection.representative ? 4 : 1)) {
      if (!active || destroyed || revision !== requestRevision) return false;
      try {
        const image = await loadTrackThumbnail(trackId);
        if (image) { urls.push(image.url); bytes += image.bytes; }
      } catch { /* keep other cached artwork if one source track disappeared */ }
    }
    if (urls.length === 0) return false;
    if (!active || destroyed || revision !== requestRevision) {
      urls.forEach((url) => URL.revokeObjectURL(url));
      return false;
    }
    remember(urls, bytes);
    return true;
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
    const selectedIds: string[] = [];
    const urls: string[] = [];
    let bytes = 0;
    try {
      if (await restorePersistentSelection(revision)) return;
      const pageSize = 8;
      // Until the detail page has ranked all artwork, show the first usable cover.
      // This keeps first paint to one async thumbnail lookup per album card.
      for (let offset = 0; offset < album.trackCount && urls.length < 1; offset += pageSize) {
        if (!active || destroyed || revision !== requestRevision) return;
        const candidates = await invokeCommand(
          'library_get_album_artwork_candidates',
          { albumNames: album.aliases, offset, limit: pageSize },
        );
        for (const trackId of candidates) {
          if (!active || destroyed || revision !== requestRevision || urls.length >= 1) break;
          try {
            const image = await loadTrackThumbnail(trackId);
            if (image) {
              selectedIds.push(trackId);
              urls.push(image.url);
              bytes += image.bytes;
            }
          } catch { /* try the next indexed track with artwork */ }
        }
        if (candidates.length < pageSize) break;
      }
      if (!active || destroyed || revision !== requestRevision) {
        urls.forEach((url) => URL.revokeObjectURL(url));
        return;
      }
      if (selectedIds.length > 0) {
        try {
          const key = await persistentSelectionKey();
          await invokeCommand('album_artwork_selection_put', {
            key,
            selection: { trackCount: album.trackCount, trackIds: selectedIds, representative: false },
          });
        } catch (error) { console.warn('無法保存專輯封面索引', error); }
      }
      remember(urls, bytes);
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

<button class="album-card" type="button" bind:this={card} onclick={() => onSelect(album)} aria-label={`開啟專輯「${album.displayName}」${album.unmarked ? '，未標記' : ''}，${album.trackCount} 首曲目`}>
  <span class="album-card-art" aria-hidden="true">
    <AlbumArtworkMosaic sources={displayedImageUrls} />
  </span>
  <span class="album-card-name">{album.displayName}</span>
  {#if album.unmarked}<span class="album-card-unmarked">未標記</span>{/if}
</button>

<style>
  .album-card { display:flex; width:var(--album-card-width); height:calc(var(--album-card-width) + 48px); min-width:var(--album-card-width); flex:0 0 var(--album-card-width); flex-direction:column; gap:8px; padding:0; border:0; color:var(--text); background:transparent; text-align:left; cursor:pointer; }
  .album-card-art { display:block; width:var(--album-card-width); height:var(--album-card-width); flex:0 0 var(--album-card-width); box-sizing:border-box; overflow:hidden; border-radius:10px; background:#000; }
  .album-card-name { overflow:hidden; font-size:var(--font-body); line-height:1.4; text-overflow:ellipsis; white-space:nowrap; }
  .album-card-unmarked { margin-top:-7px; color:var(--muted); font-size:var(--font-caption); }
  .album-card:focus-visible { outline:2px solid var(--accent); outline-offset:3px; border-radius:10px; }
</style>
