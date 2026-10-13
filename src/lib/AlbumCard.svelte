<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { getAlbumThumbnailBytes, getTrackArtworkBytes, invokeCommand, putAlbumThumbnailBytes } from './ipc';
  import { copyToArrayBuffer, createAlbumThumbnail, sha256Hex, withAlbumArtworkSlot } from './album-artwork.js';
  import type { AlbumEntry } from './album-identities.js';
  import { detectArtworkMimeType } from './active-track-artwork';

  interface Props { album: AlbumEntry; active: boolean; onSelect: (album: AlbumEntry) => void }
  let { album, active, onSelect }: Props = $props();
  let card = $state<HTMLElement | undefined>(undefined);
  let inViewport = $state(false);
  let imageUrl = $state<string | null>(null);
  let busy = false;
  let destroyed = false;
  let requestRevision = 0;

  async function loadCover(): Promise<void> {
    if (!active || busy || imageUrl || album.unmarked) return;
    const revision = ++requestRevision;
    busy = true;
    try {
      for (let offset = 0; offset < album.trackCount; offset += 100) {
        if (!active || destroyed || revision !== requestRevision) return;
        const page = await invokeCommand('library_get_album_page', { albumNames: album.aliases, offset, limit: 100 });
        for (const track of page.items) {
          if (!active || destroyed || revision !== requestRevision) return;
          let thumbnailUrl: string | null;
          try {
            thumbnailUrl = await withAlbumArtworkSlot(async () => {
              if (!active || destroyed || revision !== requestRevision) return null;
              const original = await getTrackArtworkBytes(track.id);
              if (original.byteLength === 0) return null;
              const key = await sha256Hex(original);
              let thumbnail: Uint8Array;
              try { thumbnail = new Uint8Array(await getAlbumThumbnailBytes(key)); } catch { thumbnail = new Uint8Array(); }
              if (thumbnail.length === 0) {
                const mimeType = detectArtworkMimeType(original);
                if (!mimeType) return null;
                const blob = await createAlbumThumbnail(original, mimeType);
                thumbnail = new Uint8Array(await blob.arrayBuffer());
                await putAlbumThumbnailBytes(key, copyToArrayBuffer(thumbnail)).catch((error) => {
                  console.warn('無法保存專輯卡片縮圖快取', error);
                });
              }
              return URL.createObjectURL(new Blob([copyToArrayBuffer(thumbnail)], { type: 'image/jpeg' }));
            });
          } catch { thumbnailUrl = null; }
          if (thumbnailUrl) {
            if (!active || destroyed || revision !== requestRevision) URL.revokeObjectURL(thumbnailUrl);
            else { imageUrl = thumbnailUrl; return; }
          }
        }
        if (page.items.length === 0 || offset + page.items.length >= page.totalCount) break;
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
    }, { rootMargin: '240px' });
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
  onDestroy(() => { destroyed = true; requestRevision += 1; if (imageUrl) URL.revokeObjectURL(imageUrl); });
</script>

<button class="album-card" type="button" bind:this={card} onclick={() => onSelect(album)} aria-label={`開啟專輯「${album.displayName}」${album.unmarked ? '，未標記' : ''}，${album.trackCount} 首曲目`}>
  <span class="album-card-art" aria-hidden="true">
    {#if imageUrl}<img src={imageUrl} alt="" loading="lazy" />{/if}
  </span>
  <span class="album-card-name">{album.displayName}</span>
  {#if album.unmarked}<span class="album-card-unmarked">未標記</span>{/if}
</button>

<style>
  .album-card { display: flex; height: calc(var(--album-card-width) + 48px); min-width: 0; flex-direction: column; gap: 8px; padding: 0; border: 0; color: var(--text); background: transparent; text-align: left; cursor: pointer; }
  .album-card-art { display: block; width: 100%; aspect-ratio: 1; overflow: hidden; border: 1px solid var(--line); border-radius: 10px; background: #000; }
  .album-card-art img { display: block; width: 100%; height: 100%; object-fit: cover; }
  .album-card-name { overflow: hidden; font-size: var(--font-body); line-height: 1.4; text-overflow: ellipsis; white-space: nowrap; }
  .album-card-unmarked { margin-top: -7px; color: var(--muted); font-size: var(--font-caption); }
  .album-card:hover .album-card-art { border-color: var(--text-soft); }
  .album-card:focus-visible { outline: 2px solid var(--accent); outline-offset: 3px; border-radius: 10px; }
</style>
