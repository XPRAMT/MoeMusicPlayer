import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: 'target/playback-progress-dom',
    emptyOutDir: true,
    rollupOptions: {
      input: 'tests/playback-progress-harness.html',
    },
  },
});
