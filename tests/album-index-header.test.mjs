import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const albumIndex = readFileSync(new URL('../src/lib/AlbumIndex.svelte', import.meta.url), 'utf8');
const columnCells = readFileSync(new URL('../src/lib/TrackColumnCells.svelte', import.meta.url), 'utf8');

test('album index header omits the shared list-number column', () => {
  assert.match(albumIndex, /includeIndexColumn=\{false\}/);
  assert.match(columnCells, /\{#if includeIndexColumn\}[\s\S]*?column-index[\s\S]*?\{\/if\}[\s\S]*?\{#each columns as column/);
});

test('album index header uses the shared list height and four bounded equal columns', () => {
  assert.match(albumIndex, /class="track-table-head track-list-header configurable-track-grid album-index-sort-row"/);
  assert.match(albumIndex, /\.album-index-sort-row\s*\{[^}]*box-sizing:\s*border-box;[^}]*width:\s*100%;[^}]*min-width:\s*0;[^}]*--paged-grid-template:\s*repeat\(4,\s*minmax\(0,\s*1fr\)\)/);
  assert.match(albumIndex, /:global\(\.album-index-sort-row\.track-list-header\)\s*\{[^}]*height:\s*var\(--paged-header-height,\s*37px\);[^}]*min-height:\s*var\(--paged-header-height,\s*37px\)/);
});
