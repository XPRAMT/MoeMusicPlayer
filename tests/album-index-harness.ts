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
const stats = { rawArtworkReads: 0, legacyCacheReads: 0, trackCacheReads: 0, trackCacheReadsByTrack: {} as Record<string, number>, albumCandidates: 0, thumbnailWrites: 0, albumIndexSortRequests: [] as (string | null)[] };
const selections = new Map<string, { trackCount: number; trackIds: string[]; representative: boolean }>();
Object.assign(window, { __albumIndexStats: stats, __albumArtworkSelections: selections });

Object.assign(window, {
  isTauri: true,
  __TAURI_INTERNALS__: {
    invoke: async (command: string, args?: unknown) => {
      if (command === 'library_get_album_index_facets') {
        await new Promise((resolve) => setTimeout(resolve, 120));
        return Array.from({ length: 80 }, (_, index) => ({ name: `Album ${index + 1}`, trackCount: index === 0 ? 4 : 1 }));
      }
      if (command === 'library_sort_text_indices') {
        const values = (args as { values?: string[] } | undefined)?.values ?? [];
        return values.map((_, index) => index).sort((left, right) =>
          new Intl.Collator('zh-TW', { numeric: true }).compare(values[left] ?? '', values[right] ?? ''),
        );
      }
      if (command === 'library_get_album_artwork_candidates') {
        stats.albumCandidates += 1;
        const name = String((args?.albumNames as string[] | undefined)?.[0] ?? 'Album 1');
        if (name === 'Album 2') return [];
        const key = name.replaceAll(' ', '-');
        return Array.from({ length: 4 }, (_, index) => `track-${key}-${index + 1}`);
      }
      if (command === 'album_artwork_selection_get') {
        return selections.get(String((args as { key?: string } | undefined)?.key ?? '')) ?? null;
      }
      if (command === 'album_artwork_selection_put') {
        const payload = args as { key: string; selection: { trackCount: number; trackIds: string[]; representative: boolean } };
        selections.set(payload.key, payload.selection);
        return undefined;
      }
      if (command === 'library_get_album_page') {
        stats.albumIndexSortRequests.push((args as { indexSort?: string | null } | undefined)?.indexSort ?? null);
        const albumNames = (args as { albumNames?: string[] } | undefined)?.albumNames ?? [];
        const unmarkedAlbum = albumNames.includes('Album 2');
        return {
          offset: (args as { offset?: number } | undefined)?.offset ?? 0,
          limit: (args as { limit?: number } | undefined)?.limit ?? 100,
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
