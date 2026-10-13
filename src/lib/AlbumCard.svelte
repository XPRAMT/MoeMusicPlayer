<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { getAlbumThumbnailBytes, getAlbumThumbnailBytesForTrack, getTrackArtworkBytes, invokeCommand, markAlbumArtworkMissing, putAlbumThumbnailBase64, putAlbumThumbnailBase64ForTrack } from './ipc';
  import { acquireAlbumCardArtwork, copyToArrayBuffer, createAlbumThumbnailInWorker, rememberAlbumCardArtwork, withAlbumArtworkSlot } from './album-artwork.js';
  import type { AlbumEntry } from './album-identities.js';
  import { detectArtworkMimeType } from './active-track-artwork';

  interface Props { album: AlbumEntry; active: boolean; refreshKey: number; onSelect: (album: AlbumEntry) => void }
  let { album, active, refreshKey, onSelect }: Props = $props();
  let card = $state<HTMLElement | undefined>(undefined);
  let inViewport = $state(false);
  let imageUrl = $state<string | null>(null);
  let busy = false;
  let destroyed = false;
  let requestRevision = 0;
  let cacheResolved = false;
  let releaseArtworkCacheLease: (() => void) | null = null;

  const memoryCacheKey = $derived(`${refreshKey}:${album.key}`);

  async function loadCover(): Promise<void> {
    if (!active || busy || cacheResolved || album.unmarked) return;
    const cached = acquireAlbumCardArtwork(memoryCacheKey);
    if (cached) {
      releaseArtworkCacheLease = cached.release;
      imageUrl = cached.value;
      cacheResolved = true;
      return;
    }
    const revision = ++requestRevision;
    busy = true;
    try {
      const pageSize = 8;
      for (let offset = 0; offset < album.trackCount; offset += pageSize) {
        if (!active || destroyed || revision !== requestRevision) return;
        const page = await invokeCommand('library_get_album_page', { albumNames: album.aliases, offset, limit: pageSize });
        for (const track of page.items) {
          if (!active || destroyed || revision !== requestRevision) return;
          let thumbnailUrl: { url: string; bytes: number } | null;
          try {
            thumbnailUrl = await withAlbumArtworkSlot(async () => {
              if (!active || destroyed || revision !== requestRevision) return null;
              // A track fingerprint points directly to its persistent thumbnail, avoiding an
              // IPC transfer and SHA-256 of the full-resolution cover on every album visit.
              let indexed = new Uint8Array();
              try { indexed = new Uint8Array(await getAlbumThumbnailBytesForTrack(track.id)); } catch { /* rebuild a missing or damaged cache entry */ }
              if (indexed.length === 1 && indexed[0] === 0) return null;
              if (indexed.length > 3) {
              return { url: URL.createObjectURL(new Blob([copyToArrayBuffer(indexed)], { type: 'image/jpeg' })), bytes: indexed.byteLength };
              }

              // Legacy caches are content-addressed only. Resolve once, then persist the track
              // association so future visits take the fast path above.
              const original = await getTrackArtworkBytes(track.id);
              if (original.byteLength === 0) {
                void markAlbumArtworkMissing(track.id).catch(() => undefined);
                return null;
              }
              const mimeType = detectArtworkMimeType(original);
              if (!mimeType) {
                void markAlbumArtworkMissing(track.id).catch(() => undefined);
                return null;
              }
              const processed = await createAlbumThumbnailInWorker(original, mimeType);
              const key = processed.sha256;
              let thumbnail: Uint8Array;
              try { thumbnail = new Uint8Array(await getAlbumThumbnailBytes(key)); } catch { thumbnail = new Uint8Array(); }
              if (thumbnail.length === 0) {
                thumbnail = processed.bytes;
              }
              await putAlbumThumbnailBase64ForTrack(track.id, key, processed.dataBase64).catch((error) => {
                console.warn('無法保存專輯卡片縮圖快取', error);
                return putAlbumThumbnailBase64(key, processed.dataBase64).catch(() => undefined);
              });
              return { url: URL.createObjectURL(new Blob([copyToArrayBuffer(thumbnail)], { type: 'image/jpeg' })), bytes: thumbnail.byteLength };
            });
          } catch { thumbnailUrl = null; }
          if (thumbnailUrl) {
            if (!active || destroyed || revision !== requestRevision) URL.revokeObjectURL(thumbnailUrl.url);
            else {
              const cachedArtwork = rememberAlbumCardArtwork(memoryCacheKey, thumbnailUrl.url, thumbnailUrl.bytes);
              releaseArtworkCacheLease = cachedArtwork?.release ?? null;
              imageUrl = cachedArtwork?.value ?? thumbnailUrl.url;
              cacheResolved = true;
              return;
            }
          }
        }
        if (page.items.length === 0 || offset + page.items.length >= page.totalCount) break;
      }
      if (active && !destroyed && revision === requestRevision) {
        const cachedArtwork = rememberAlbumCardArtwork(memoryCacheKey, null);
        releaseArtworkCacheLease = cachedArtwork?.release ?? null;
        cacheResolved = true;
      }
    } catch (error) {
      console.warn(`無法載入專輯「${album.displayName}」封面`, error);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    if (!card || typeof IntersectionObserver === 'undefined') { inViewport = true; return; }
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) {
        observer.disconnect();
        inViewport = true;
      }
    }, { rootMargin: '80px' });
    observer.observe(card);
    return () => observer.disconnect();
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
    {#if imageUrl}<img src={imageUrl} alt="" loading="lazy" />{/if}
  </span>
  <span class="album-card-name">{album.displayName}</span>
  {#if album.unmarked}<span class="album-card-unmarked">未標記</span>{/if}
</button>

<style>
  .album-card { display: flex; width: var(--album-card-width); height: calc(var(--album-card-width) + 48px); min-width: var(--album-card-width); flex: 0 0 var(--album-card-width); flex-direction: column; gap: 8px; padding: 0; border: 0; color: var(--text); background: transparent; text-align: left; cursor: pointer; }
  .album-card-art { display: block; width: var(--album-card-width); height: var(--album-card-width); flex: 0 0 var(--album-card-width); box-sizing: border-box; overflow: hidden; border: 1px solid var(--line); border-radius: 10px; background: #000; }
  .album-card-art img { display: block; width: 100%; height: 100%; object-fit: cover; }
  .album-card-name { overflow: hidden; font-size: var(--font-body); line-height: 1.4; text-overflow: ellipsis; white-space: nowrap; }
  .album-card-unmarked { margin-top: -7px; color: var(--muted); font-size: var(--font-caption); }
  .album-card:hover .album-card-art { border-color: var(--text-soft); }
  .album-card:focus-visible { outline: 2px solid var(--accent); outline-offset: 3px; border-radius: 10px; }
</style>
