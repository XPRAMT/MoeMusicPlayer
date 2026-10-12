import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { moeBuildDefine } from './scripts/build-version.mjs';

export default defineConfig({
  plugins: [svelte()],
  define: moeBuildDefine(),
  build: {
    outDir: 'target/favorite-dock-dom',
    emptyOutDir: true,
    rollupOptions: {
      input: 'tests/favorite-dock-harness.html',
    },
  },
});
