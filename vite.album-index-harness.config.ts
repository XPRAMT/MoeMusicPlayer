import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { moeBuildDefine } from './scripts/build-version.mjs';

export default defineConfig({
  plugins: [svelte()],
  define: moeBuildDefine(),
  build: {
    outDir: 'target/album-index-harness',
    emptyOutDir: true,
    rollupOptions: {
      input: 'tests/album-index-harness.html',
    },
  },
});
