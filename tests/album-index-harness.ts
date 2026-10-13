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
const stats = { rawArtworkReads: 0, legacyCacheReads: 0, trackCacheReads: 0, trackCacheReadsByTrack: {} as Record<string, number>, albumPages: 0, thumbnailWrites: 0 };
Object.assign(window, { __albumIndexStats: stats });

Object.assign(window, {
  isTauri: true,
  __TAURI_INTERNALS__: {
    invoke: async (command: string, args?: unknown) => {
      if (command === 'library_get_album_index_facets') {
        await new Promise((resolve) => setTimeout(resolve, 120));
        return Array.from({ length: 80 }, (_, index) => ({ name: `Album ${index + 1}`, trackCount: 1 }));
      }
      if (command === 'library_get_album_page') {
        stats.albumPages += 1;
        const name = String((args?.albumNames as string[] | undefined)?.[0] ?? 'Album 1');
        return { items: [{ id: `track-${name.replaceAll(' ', '-')}` }], totalCount: 1 };
      }
      if (command === 'library_get_track_artwork') {
        stats.rawArtworkReads += 1;
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
