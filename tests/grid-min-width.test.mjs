import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { getGridTemplateMinWidthPx, getGridTrackMinPx, splitGridTemplate } from '../src/lib/grid-min-width.js';

test('grid template splits at top-level spaces only', () => {
  assert.deepEqual(splitGridTemplate('36px minmax(150px, 1fr) 6em'), ['36px', 'minmax(150px, 1fr)', '6em']);
});

test('track minimums: px, em, minmax min; fr/auto are flexible', () => {
  assert.equal(getGridTrackMinPx('36px', 16), 36);
  assert.equal(getGridTrackMinPx('6em', 14), 84);
  assert.equal(getGridTrackMinPx('minmax(150px,1fr)', 16), 150);
  assert.equal(getGridTrackMinPx('minmax(0,1fr)', 16), 0);
  assert.equal(getGridTrackMinPx('1fr', 16), 0);
  assert.equal(getGridTrackMinPx('auto', 16), 0);
});

test('regression: Artists grid has a real minimum width so narrow windows overflow and scroll', () => {
  // ArtistIndex passes no minContentWidth; before the fix rows were 100% wide and clipped columns.
  const width = getGridTemplateMinWidthPx('36px minmax(150px,1fr) 6em 6em 6em 6em', 14);
  assert.equal(width, 36 + 150 + 4 * 84 + 5 * 10 + 28);
  assert.ok(width > 390, 'wider than a phone viewport');
});

test('all track tables share one min-width path and touch reorder rows allow horizontal pans', () => {
  const list = readFileSync(new URL('../src/lib/PagedVirtualList.svelte', import.meta.url), 'utf8');
  assert.match(list, /effectiveMinContentWidth = \$derived\(Math\.max\(minContentWidth, getGridTemplateMinWidthPx\(gridTemplate, gridEmPx\)\)\)/);
  assert.equal((list.match(/--track-list-min-width:\$\{effectiveMinContentWidth\}px/g) ?? []).length, 2);
  assert.doesNotMatch(list, /--track-list-min-width:\$\{minContentWidth\}px/);
  assert.match(list, /\.paged-virtual-row\.reorderable \{\s*touch-action: pan-x pan-y;/);
});
