import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const albumIndex = readFileSync(new URL('../src/lib/AlbumIndex.svelte', import.meta.url), 'utf8');
const columnCells = readFileSync(new URL('../src/lib/TrackColumnCells.svelte', import.meta.url), 'utf8');

test('album index header omits the shared list-number column', () => {
  assert.match(albumIndex, /includeIndexColumn=\{false\}/);
  assert.match(columnCells, /\{#if includeIndexColumn\}[\s\S]*?column-index[\s\S]*?\{\/if\}[\s\S]*?\{#each columns as column/);
});

test('album cumulative playback column uses the requested label', () => {
  assert.match(albumIndex, /id: 'played', label: '累積播放'/);
  assert.doesNotMatch(albumIndex, /id: 'played', label: '累積時長'/);
});

test('album index sort columns are ordered and centered', () => {
  const columnBlock = albumIndex.match(/const albumColumns = \[([\s\S]*?)\];/)?.[1] ?? '';
  const orderedLabels = [...columnBlock.matchAll(/label: '([^']+)'/g)].map((match) => match[1]);
  assert.deepEqual(orderedLabels, ['專輯', '歌曲數量', '累積播放', '播放時長']);
  assert.match(albumIndex, /album-index-sort-row \.column-sort-button\) \{ justify-content:center; text-align:center; \}/);
});

test('album index sort header uses the shared visible bottom boundary', () => {
  const appStyles = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.match(albumIndex, /\.album-index-sort-header \{ position:relative;/);
  assert.match(appStyles, /\.paged-virtual-header::after\s*\{[^}]*height:\s*1px;[^}]*background:\s*var\(--line\)/);
  const pagedList = readFileSync(new URL('../src/lib/PagedVirtualList.svelte', import.meta.url), 'utf8');
  assert.doesNotMatch(pagedList, /\.paged-virtual-header::after/);
});

test('album index header uses the shared list height and four bounded equal columns', () => {
  assert.match(albumIndex, /class="track-table-head track-list-header configurable-track-grid album-index-sort-row"/);
  assert.match(albumIndex, /\.album-index-sort-row\s*\{[^}]*box-sizing:\s*border-box;[^}]*width:\s*100%;[^}]*min-width:\s*0;[^}]*--paged-grid-template:\s*repeat\(4,\s*minmax\(0,\s*1fr\)\)/);
  assert.match(albumIndex, /:global\(\.album-index-sort-row\.track-list-header\)\s*\{[^}]*height:\s*var\(--paged-header-height,\s*37px\);[^}]*min-height:\s*var\(--paged-header-height,\s*37px\)/);
});
