import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { moeBuildDefine } from './scripts/build-version.mjs';

export default defineConfig({
  plugins: [svelte()],
  define: moeBuildDefine(),
  clearScreen: false,
  server: {
    host: '127.0.0.1',
    port: 1450,
    strictPort: true,
  },
});
