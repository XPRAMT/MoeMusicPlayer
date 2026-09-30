import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: 'target/now-playing-layout-dom',
    emptyOutDir: true,
    rollupOptions: {
      input: 'tests/now-playing-layout-harness.html',
    },
  },
});
