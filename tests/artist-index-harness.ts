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
    playedMs: (80 - index) * 60_000,
    durationMs: (index + 1) * 120_000,
  };
});

Object.assign(window, {
  isTauri: true,
  __TAURI_INTERNALS__: {
    invoke: async (command: string, args?: Record<string, unknown>) => {
      const storedSort = sessionStorage.getItem('artist-list-sort');
      if (command === 'library_create_artist_index_snapshot') {
        return { snapshotId: 'artist-snapshot', quickIndex: { labels: [], groups: [], totalCount: artists.length } };
      }
      if (command === 'library_get_artist_index_snapshot_page') {
        await new Promise((resolve) => setTimeout(resolve, 50));
        const offset = Number(args?.offset ?? 0);
        const limit = Number(args?.limit ?? 1000);
        return { offset, limit, totalCount: artists.length, items: artists.slice(offset, offset + limit) };
      }
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
