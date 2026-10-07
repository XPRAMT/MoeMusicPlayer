import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { moeBuildDefine } from './scripts/build-version.mjs';

export default defineConfig({
  plugins: [svelte()],
  define: moeBuildDefine(),
  build: {
    outDir: 'target/playback-progress-dom',
    emptyOutDir: true,
    rollupOptions: {
      input: 'tests/playback-progress-harness.html',
    },
  },
});
