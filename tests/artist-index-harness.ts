import { mount } from 'svelte';
import ArtistIndexHarness from './ArtistIndexHarness.svelte';
import '../src/app.css';

const artists = Array.from({ length: 80 }, (_, index) => {
  const displayName = `Artist ${String(index + 1).padStart(3, '0')}`;
  return {
    key: displayName.toLowerCase(),
    names: [{ name: displayName, count: 2 }],
    artistNames: [displayName],
    trackCount: 80 - index,
    albumCount: index % 8,
    playedMs: (index + 1) * 60_000,
  };
});

Object.assign(window, {
  isTauri: true,
  __TAURI_INTERNALS__: {
    invoke: async (command: string) => {
      if (command === 'library_get_artist_index_facets') return { artists: [], albums: [] };
      if (command === 'library_get_artist_index') return artists;
      throw new Error(`Unexpected harness command: ${command}`);
    },
  },
});

mount(ArtistIndexHarness, { target: document.querySelector('#app')! });
