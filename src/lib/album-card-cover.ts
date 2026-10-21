// The one pipeline that turns an album card into its cover thumbnails, shared by the card itself (high priority) and by the
// prefetcher that warms the memory cache around the visible cards (low priority).
import { tr } from './i18n.js';
import { getAlbumThumbnailBytesForTrack, getTrackArtworkBytes, invokeCommand, markAlbumArtworkMissing, putAlbumThumbnailBase64, putAlbumThumbnailBase64ForTrack } from './ipc';
import type { AlbumIndexEntry } from './ipc';
import { copyToArrayBuffer, createAlbumThumbnailInWorker, getAlbumCardSelectionTrackIds, hasAlbumCardArtwork, loadAlbumCardThumbnails, rememberAlbumCardArtwork, withAlbumArtworkSlot, withAlbumCoverLookupSlot } from './album-artwork.js';
import { detectArtworkMimeType } from './active-track-artwork';
import { ALBUM_PREFETCH_QUIET_MS } from './album-cover-prefetch.js';

export type AlbumCoverPriority = 'high' | 'low';
/** `urls` empty = the album has no cover (confirmed). */
export interface AlbumCoverResult { urls: string[]; bytes: number }

async function persistentSelectionKey(album: AlbumIndexEntry): Promise<string> {
  const bytes = new TextEncoder().encode(`${album.key}\0${album.trackCount}`);
  return [...new Uint8Array(await crypto.subtle.digest('SHA-256', copyToArrayBuffer(bytes)))]
    .map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

async function loadTrackThumbnail(trackId: string, wanted: () => boolean, priority: AlbumCoverPriority): Promise<{ url: string; bytes: number } | null> {
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
      void markAlbumArtworkMissing(trackId, true).catch(() => undefined);
      return null;
    }
    let processed;
    try {
      processed = await createAlbumThumbnailInWorker(original, mimeType);
    } catch {
      // The cover was read but cannot be turned into a thumbnail: it will not work the next time either.
      void markAlbumArtworkMissing(trackId, true).catch(() => undefined);
      return null;
    }
    let thumbnail: Uint8Array;
    try { thumbnail = new Uint8Array(await getAlbumThumbnailBytesForTrack(trackId)); } catch { thumbnail = new Uint8Array(); }
    if (thumbnail.length <= 3) thumbnail = processed.bytes;
    await putAlbumThumbnailBase64ForTrack(trackId, processed.sha256, processed.dataBase64).catch((error) => {
      console.warn(tr('無法保存專輯卡片縮圖快取'), error);
      return putAlbumThumbnailBase64(processed.sha256, processed.dataBase64).catch(() => undefined);
    });
    return {
      url: URL.createObjectURL(new Blob([copyToArrayBuffer(thumbnail)], { type: 'image/jpeg' })),
      bytes: thumbnail.byteLength,
    };
  }, wanted, priority);
}

/**
 * Find and load the cover thumbnail of an album card: persisted selection (backend, limited lookups) then the per-track
 * thumbnail from the disk cache or built from the original cover. Returns `null` when nothing could be loaded now (a failed
 * or unwanted lookup: callers must not treat that as "no cover"); the caller owns the returned object URLs.
 * `wanted` is checked before each queued step so work for a card that has scrolled away is skipped.
 */
export async function loadAlbumCardCover(album: AlbumIndexEntry, wanted: () => boolean, priority: AlbumCoverPriority = 'high'): Promise<AlbumCoverResult | null> {
  const key = await persistentSelectionKey(album);
  for (let attempt = 0; attempt < 2; attempt += 1) {
    if (!wanted()) return null;
    let selection;
    try {
      // Only a few card lookups at the backend at once; one that waited and whose card is gone is not run.
      selection = await withAlbumCoverLookupSlot(() => invokeCommand('album_artwork_selection_ensure', {
        key,
        albumNames: album.aliases,
        trackCount: album.trackCount,
        representative: false,
        forceRebuild: attempt > 0,
      }), wanted, priority);
    } catch { return null; }
    if (selection === null) return null;
    if (!selection || selection.trackCount !== album.trackCount || !wanted()) return null;
    const selectedTrackIds = getAlbumCardSelectionTrackIds(selection);
    // Empty selections are persisted negative-cache entries: this album has no readable cover.
    if (selectedTrackIds.length === 0) return { urls: [], bytes: 0 };
    const loaded = await loadAlbumCardThumbnails(selectedTrackIds, (trackId) => loadTrackThumbnail(trackId, wanted, priority));
    if (loaded.urls.length > 0) return loaded;
    // Rebuild the selection once if every representative thumbnail has gone stale.
  }
  return null;
}

export interface AlbumCoverPrefetchItem { album: AlbumIndexEntry; memoryKey: string }

/**
 * Warms the card memory cache for albums near the visible cards. `schedule` replaces the previous plan (anything not yet
 * started is dropped), waits for the quiet time, then loads the items in order with the low-priority pipeline. Results are
 * cached without a lease (evictable at once, so they can never push out a rendered card); failures are not cached.
 */
export function createAlbumCoverPrefetcher({ quietMs = ALBUM_PREFETCH_QUIET_MS, concurrency = 2 } = {}) {
  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;

  function cancel(): void {
    generation += 1;
    if (timer !== null) { clearTimeout(timer); timer = null; }
  }

  async function run(items: AlbumCoverPrefetchItem[], current: number): Promise<void> {
    const wanted = () => current === generation;
    let next = 0;
    const worker = async () => {
      while (wanted()) {
        const item = items[next++];
        if (!item) return;
        if (hasAlbumCardArtwork(item.memoryKey)) continue;
        try {
          const result = await loadAlbumCardCover(item.album, wanted, 'low');
          if (!result) continue;
          // Even if the window moved on meanwhile the finished work is kept (evictable); a card that wants it takes a lease.
          rememberAlbumCardArtwork(item.memoryKey, result.urls.length > 0 ? result.urls : null, result.bytes)?.release();
        } catch { /* a card that really needs this cover loads it itself */ }
      }
    };
    await Promise.all(Array.from({ length: Math.max(1, concurrency) }, worker));
  }

  function schedule(items: AlbumCoverPrefetchItem[]): void {
    cancel();
    if (items.length === 0) return;
    const current = generation;
    timer = setTimeout(() => { timer = null; void run(items, current); }, quietMs);
  }

  return { schedule, cancel };
}
