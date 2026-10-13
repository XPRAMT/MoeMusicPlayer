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
    invoke: async (command: string, args?: Record<string, unknown>) => {
      const storedSort = sessionStorage.getItem('artist-list-sort');
      if (command === 'library_get_artist_index_facets') return { artists: [], albums: [] };
      if (command === 'library_get_artist_index') return artists;
      if (command === 'settings_get_artist_list_sort') {
        return storedSort ? JSON.parse(storedSort) : { field: 'title', direction: 'asc' };
      }
      if (command === 'settings_set_artist_list_sort') {
        sessionStorage.setItem('artist-list-sort', JSON.stringify(args?.preference));
        return args?.preference;
      }
      throw new Error(`Unexpected harness command: ${command}`);
    },
  },
});

mount(ArtistIndexHarness, { target: document.querySelector('#app')! });
