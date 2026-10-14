import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const artistIndexSource = readFileSync(path.join(root, 'src/lib/ArtistIndex.svelte'), 'utf8').replace(/\r\n/g, '\n');

test('artist count and duration columns use six fullwidth character widths', () => {
  assert.match(artistIndexSource, /const ARTIST_GRID = '36px minmax\(150px,1fr\) 6em 6em 6em 6em';/);
  assert.match(artistIndexSource, /rowClassName="track-row virtual-track-row configurable-track-grid artist-index-row"/);
});

test('fixed em-based columns resolve from the same font size in headers and rows', () => {
  const appStyles = readFileSync(path.join(root, 'src/app.css'), 'utf8').replace(/\r\n/g, '\n');
  assert.match(appStyles, /\.track-table-head\s*\{[^}]*font-size:\s*var\(--font-body\)/);
  assert.match(appStyles, /\.track-row\s*\{[^}]*font-size:\s*var\(--font-body\)/);
});

test('artist-name sorting uses the shared ICU quick-index groups only for that sort', () => {
  assert.match(artistIndexSource, /buildQuickIndexGroups, cacheQuickIndex, getCachedQuickIndex/);
  assert.match(artistIndexSource, /const shouldGroup = activeSortField === 'title'/);
  assert.match(artistIndexSource, /groupRows=\{sortField === 'title' \? quickIndex\?\.groups : \[\]\}/);
  assert.match(artistIndexSource, /groupingReady=\{sortField !== 'title' \|\| quickIndexReady\}/);
});
