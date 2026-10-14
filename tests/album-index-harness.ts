import { mount } from 'svelte';
import AlbumIndexHarness from './AlbumIndexHarness.svelte';
import '../src/app.css';

const cacheJpeg = Uint8Array.from(atob('/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////2wBDAf//////////////////////////////////////////////////////////////////////////////////////wAARCAABAAEDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAX/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oADAMBAAIQAxAAAAF//8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABBQJ//8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAwEBPwF//8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAgEBPwF//8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQAGPwJ//8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABPyF//9k='), (character) => character.charCodeAt(0));
const fixtureCanvas = document.createElement('canvas');
fixtureCanvas.width = 4;
fixtureCanvas.height = 4;
fixtureCanvas.getContext('2d')!.fillRect(0, 0, 4, 4);
const fixtureBlob = await new Promise<Blob>((resolve, reject) => fixtureCanvas.toBlob((blob) => blob ? resolve(blob) : reject(new Error('fixture JPEG generation failed')), 'image/jpeg'));
const validJpeg = new Uint8Array(await fixtureBlob.arrayBuffer());
const artworkReads = validJpeg.slice().buffer;
const coldArtworkMode = new URLSearchParams(location.search).has('cold');
const stats = { rawArtworkReads: 0, analysisArtworkReads: 0, legacyCacheReads: 0, trackCacheReads: 0, trackCacheReadsByTrack: {} as Record<string, number>, albumCandidates: 0, thumbnailWrites: 0, albumIndexSortRequests: [] as (string | null)[] };
const selections = new Map<string, { trackCount: number; trackIds: string[]; representative: boolean }>();
Object.assign(window, { __albumIndexStats: stats, __albumArtworkSelections: selections });
const albums = Array.from({ length: 80 }, (_, index) => ({
  key: `album:${index + 1}`,
  displayName: index === 1 ? '未知' : `Album ${index + 1}`,
  aliases: index === 1 ? [] : [index === 79 ? 'Multi Disc' : `Album ${index + 1}`],
  trackCount: index === 0 ? 4 : index === 79 ? 6 : 1,
  durationMs: 180_000,
  playedMs: 0,
  unmarked: index === 1,
}));

Object.assign(window, {
  isTauri: true,
  __TAURI_INTERNALS__: {
    invoke: async (command: string, args?: unknown) => {
      if (command === 'library_create_album_index_snapshot') {
        return { snapshotId: 'album-snapshot', quickIndex: { labels: [], groups: [], totalCount: albums.length } };
      }
      if (command === 'library_get_album_index_snapshot_page') {
        const request = args as { offset?: number; limit?: number } | undefined;
        const offset = request?.offset ?? 0;
        const limit = request?.limit ?? 1000;
        return { offset, limit, totalCount: albums.length, items: albums.slice(offset, offset + limit) };
      }
      if (command === 'settings_get_album_index_sort') return { field: 'title', direction: 'asc' };
      if (command === 'settings_set_album_index_sort') return (args as { preference: unknown }).preference;
      if (command === 'settings_get_album_detail_sort') return { field: 'position', direction: 'asc' };
      if (command === 'settings_set_album_detail_sort') return (args as { preference: unknown }).preference;
      if (command === 'album_artwork_selection_ensure') {
        const payload = args as { key: string; albumNames: string[]; trackCount: number; representative: boolean; forceRebuild: boolean };
        const cached = selections.get(payload.key) ?? null;
        if (!payload.forceRebuild && cached?.trackCount === payload.trackCount && (!payload.representative || cached.representative)) return cached;
        stats.albumCandidates += 1;
        const name = payload.albumNames[0] ?? 'Album 2';
        const key = name.replaceAll(' ', '-');
        const candidateIds = name === 'Album 2'
          ? []
          : Array.from({ length: name === 'Multi Disc' ? 6 : 4 }, (_, index) => `track-${key}-${index + 1}`);
        stats.analysisArtworkReads += payload.representative ? candidateIds.length : Math.min(1, candidateIds.length);
        const selection = {
          trackCount: payload.trackCount,
          trackIds: payload.representative ? candidateIds.slice(0, 1) : candidateIds.slice(0, 1),
          representative: payload.representative,
        };
        selections.set(payload.key, selection);
        return selection;
      }
      if (command === 'library_get_album_page') {
        stats.albumIndexSortRequests.push((args as { indexSort?: string | null } | undefined)?.indexSort ?? null);
        const albumNames = (args as { albumNames?: string[] } | undefined)?.albumNames ?? [];
        const unmarkedAlbum = albumNames.includes('Album 2');
        const offset = (args as { offset?: number } | undefined)?.offset ?? 0;
        const limit = (args as { limit?: number } | undefined)?.limit ?? 100;
        if (albumNames.includes('Multi Disc')) {
          const items = [
            { id: 'track-Multi-Disc-1', title: 'Disc 2 Track 1', discNumber: 2, trackNumber: 1 },
            { id: 'track-Multi-Disc-2', title: 'Disc 2 Track 2', discNumber: 2, trackNumber: 2 },
            { id: 'track-Multi-Disc-3', title: 'Disc 4 Track 1', discNumber: 4, trackNumber: 1 },
            { id: 'track-Multi-Disc-4', title: 'Unknown Disc 1 Track 9', discNumber: 1, trackNumber: 9 },
            { id: 'track-Multi-Disc-5', title: 'Unknown Disc 0 Track 2', discNumber: 0, trackNumber: 2 },
            { id: 'track-Multi-Disc-6', title: 'Unknown Disc Track 1', discNumber: null, trackNumber: 1 },
          ].map((track) => ({
            ...track, artist: null, album: 'Multi Disc', albumArtist: null, durationMs: 180000,
            codec: 'flac', bitrateBps: null, sampleRateHz: 48000, releaseDate: null, year: null,
            bitDepth: 24, playedMs: 0, fileName: null,
          }));
          if ((args as { indexSort?: string | null } | undefined)?.indexSort === 'desc') {
            items.sort((left, right) => {
              const leftClass = (left.discNumber ?? 0) >= 2;
              const rightClass = (right.discNumber ?? 0) >= 2;
              if (leftClass !== rightClass) return leftClass ? -1 : 1;
              if (leftClass && left.discNumber !== right.discNumber) return (right.discNumber ?? 0) - (left.discNumber ?? 0);
              return (right.trackNumber ?? 0) - (left.trackNumber ?? 0);
            });
          } else {
            items.sort((left, right) => {
              const leftClass = (left.discNumber ?? 0) >= 2;
              const rightClass = (right.discNumber ?? 0) >= 2;
              if (leftClass !== rightClass) return leftClass ? -1 : 1;
              if (leftClass && left.discNumber !== right.discNumber) return (left.discNumber ?? 0) - (right.discNumber ?? 0);
              return (left.trackNumber ?? 0) - (right.trackNumber ?? 0);
            });
          }
          return { offset, limit, totalCount: items.length, items: items.slice(offset, offset + limit) };
        }
        return {
          offset,
          limit,
          totalCount: unmarkedAlbum ? 1 : 4,
          items: unmarkedAlbum
            ? [{ id: 'track-Album-2-1', title: 'Song 1', artist: null, album: null, albumArtist: null, trackNumber: 1, discNumber: 1, durationMs: 180000, codec: 'flac', bitrateBps: null, sampleRateHz: 48000, releaseDate: null, year: null, bitDepth: 24, playedMs: 0, fileName: null }]
            : [
              { id: 'track-Album-1-1', title: 'Song 1', artist: 'hanser/yousa', album: 'Album 1', albumArtist: null, trackNumber: 1, discNumber: 1, durationMs: 180000, codec: 'flac', bitrateBps: null, sampleRateHz: 48000, releaseDate: null, year: null, bitDepth: 24, playedMs: 0, fileName: null },
              { id: 'track-Album-1-2', title: 'Song 2', artist: 'yousa', album: 'Album 1', albumArtist: null, trackNumber: 2, discNumber: 1, durationMs: 180000, codec: 'flac', bitrateBps: null, sampleRateHz: 48000, releaseDate: null, year: null, bitDepth: 24, playedMs: 0, fileName: null },
              { id: 'track-Album-1-3', title: 'Song 3', artist: 'hanser', album: 'Album 1', albumArtist: null, trackNumber: 3, discNumber: 1, durationMs: 180000, codec: 'flac', bitrateBps: null, sampleRateHz: 48000, releaseDate: null, year: null, bitDepth: 24, playedMs: 0, fileName: null },
              { id: 'track-Album-1-4', title: 'Song 4', artist: 'other artist', album: 'Album 1', albumArtist: null, trackNumber: 4, discNumber: 1, durationMs: 180000, codec: 'flac', bitrateBps: null, sampleRateHz: 48000, releaseDate: null, year: null, bitDepth: 24, playedMs: 0, fileName: null },
            ],
        };
      }
      if (command === 'library_get_track_artwork') {
        stats.rawArtworkReads += 1;
        const trackId = String((args as { trackId?: string } | undefined)?.trackId ?? '');
        if (trackId === 'track-Album-2-1') return new ArrayBuffer(0);
        return artworkReads.slice(0);
      }
      if (command === 'album_artwork_cache_get') {
        stats.legacyCacheReads += 1;
        if (coldArtworkMode) return new ArrayBuffer(0);
        const until = performance.now() + 40;
        while (performance.now() < until) { /* emulate synchronous cache file I/O */ }
        return cacheJpeg.slice().buffer;
      }
      if (command === 'album_artwork_cache_get_for_track') {
        stats.trackCacheReads += 1;
        const trackId = String((args as { trackId?: string } | undefined)?.trackId ?? 'unknown');
        stats.trackCacheReadsByTrack[trackId] = (stats.trackCacheReadsByTrack[trackId] ?? 0) + 1;
        await new Promise((resolve) => setTimeout(resolve, 120));
        return coldArtworkMode ? new ArrayBuffer(0) : cacheJpeg.slice().buffer;
      }
      if (command === 'album_artwork_cache_put_for_track') {
        stats.thumbnailWrites += 1;
        return undefined;
      }
      if (command === 'album_artwork_cache_mark_missing_for_track') {
        return undefined;
      }
      throw new Error(`Unexpected album harness command: ${command}`);
    },
  },
});

mount(AlbumIndexHarness, { target: document.querySelector('#app')! });
