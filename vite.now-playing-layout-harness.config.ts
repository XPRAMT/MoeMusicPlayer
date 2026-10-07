import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { moeBuildDefine } from './scripts/build-version.mjs';

export default defineConfig({
  plugins: [svelte()],
  define: moeBuildDefine(),
  build: {
    outDir: 'target/now-playing-layout-dom',
    emptyOutDir: true,
    rollupOptions: {
      input: 'tests/now-playing-layout-harness.html',
    },
  },
});
