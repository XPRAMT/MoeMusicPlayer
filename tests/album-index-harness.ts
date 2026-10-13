import { mount } from 'svelte';
import AlbumIndexHarness from './AlbumIndexHarness.svelte';
import '../src/app.css';

Object.assign(window, {
  isTauri: true,
  __TAURI_INTERNALS__: {
    invoke: async (command: string) => {
      if (command === 'library_get_album_index_facets') {
        await new Promise((resolve) => setTimeout(resolve, 120));
        return Array.from({ length: 12 }, (_, index) => ({ name: `Album ${index + 1}`, trackCount: 1 }));
      }
      if (command === 'library_get_album_page') return { items: [], totalCount: 0 };
      throw new Error(`Unexpected album harness command: ${command}`);
    },
  },
});

mount(AlbumIndexHarness, { target: document.querySelector('#app')! });
