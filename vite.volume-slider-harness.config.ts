import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: 'target/volume-slider-dom',
    emptyOutDir: true,
    rollupOptions: {
      input: 'tests/volume-slider-harness.html',
    },
  },
});
