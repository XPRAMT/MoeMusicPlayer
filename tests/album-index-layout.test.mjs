import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { buildAlbumIndexRenderRows, buildAlbumIndexRows, findVisibleAlbumIndexRows, physicalPixelsToCssPixels, shouldGroupAlbumIndex } from '../src/lib/album-index-layout.js';

const albumIndexSource = readFileSync(new URL('../src/lib/AlbumIndex.svelte', import.meta.url), 'utf8').replace(/\r\n/g, '\n');

test('only title ordering uses album first-character grouping when the setting is enabled', () => {
  assert.equal(shouldGroupAlbumIndex({ sortField: 'title', groupingEnabled: true }), true);
  assert.equal(shouldGroupAlbumIndex({ sortField: 'title', groupingEnabled: false }), false);
  assert.equal(shouldGroupAlbumIndex({ sortField: 'duration', groupingEnabled: true }), false);
  assert.equal(shouldGroupAlbumIndex({ sortField: 'played', groupingEnabled: true }), false);
  assert.equal(shouldGroupAlbumIndex({ sortField: 'trackCount', groupingEnabled: true }), false);
});

test('physical album spacing converts to CSS pixels at the active display scale', () => {
  assert.equal(physicalPixelsToCssPixels(40, 1), 40);
  assert.equal(physicalPixelsToCssPixels(40, 1.25), 32);
  assert.equal(physicalPixelsToCssPixels(40, 2.5), 16);
});

test('album title groups become full-width virtual rows and restart the card grid', () => {
  const layout = buildAlbumIndexRows({
    totalCount: 10,
    columns: 3,
    cardRowHeight: 220,
    groupHeadingHeight: 37,
    groupCardGap: 12,
    groups: [
      { label: 'A', beforeIndex: 0 },
      { label: 'B', beforeIndex: 4 },
      { label: '未知', beforeIndex: 8 },
    ],
  });

  assert.deepEqual(layout.rows.map((row) => row.kind === 'group'
    ? ['group', row.label, row.beforeIndex, row.top]
    : ['cards', row.start, row.end, row.top]), [
    ['group', 'A', 0, 0],
    ['cards', 0, 3, 49],
    ['cards', 3, 4, 269],
    ['group', 'B', 4, 489],
    ['cards', 4, 7, 538],
    ['cards', 7, 8, 758],
    ['group', '未知', 8, 978],
    ['cards', 8, 10, 1027],
  ]);
  assert.equal(layout.totalHeight, 1247);
});

test('album index rows remain a normal responsive card grid when grouping is disabled', () => {
  const layout = buildAlbumIndexRows({ totalCount: 8, columns: 3, cardRowHeight: 220 });
  assert.deepEqual(layout.rows.map((row) => [row.start, row.end, row.top]), [
    [0, 3, 0], [3, 6, 220], [6, 8, 440],
  ]);
  assert.equal(layout.totalHeight, 660);
});

test('album virtual rows select by pixel viewport and overscan', () => {
  const layout = buildAlbumIndexRows({
    totalCount: 6,
    columns: 2,
    cardRowHeight: 200,
    groupHeadingHeight: 40,
    groupCardGap: 12,
    groups: [{ label: 'A', beforeIndex: 0 }, { label: 'B', beforeIndex: 4 }],
  });
  assert.deepEqual(findVisibleAlbumIndexRows(layout.rows, 240, 250, 40).map((row) => row.kind === 'group'
    ? `group:${row.label}`
    : `cards:${row.start}-${row.end}`), ['cards:0-2', 'cards:2-4', 'group:B', 'cards:4-6']);
});

test('album card rows rematerialize cached page items when revision changes without geometry changes', () => {
  const layout = buildAlbumIndexRows({ totalCount: 4, columns: 2, cardRowHeight: 220 });
  const visible = findVisibleAlbumIndexRows(layout.rows, 0, 300, 0);
  let cached = new Map();
  const itemAt = (index) => cached.get(index) ?? null;

  const before = buildAlbumIndexRenderRows(visible, itemAt, 1, 0);
  assert.deepEqual(before.filter((row) => row.kind === 'cards').flatMap((row) => row.items), [
    { index: 0, album: null, key: 'loading:0:0' },
    { index: 1, album: null, key: 'loading:1:0' },
    { index: 2, album: null, key: 'loading:2:0' },
    { index: 3, album: null, key: 'loading:3:0' },
  ]);

  cached = new Map([[0, { key: 'album:a', displayName: 'A' }], [1, { key: 'album:b', displayName: 'B' }]]);
  const after = buildAlbumIndexRenderRows(visible, itemAt, 2, 0);
  assert.deepEqual(after.filter((row) => row.kind === 'cards').flatMap((row) => row.items), [
    { index: 0, album: { key: 'album:a', displayName: 'A' }, key: 'album:a' },
    { index: 1, album: { key: 'album:b', displayName: 'B' }, key: 'album:b' },
    { index: 2, album: null, key: 'loading:2:0' },
    { index: 3, album: null, key: 'loading:3:0' },
  ]);
  assert.notEqual(after[0], before[0]);
  assert.notEqual(after[0].items, before[0].items);
  assert.match(albumIndexSource, /const visibleRenderRows = \$derived\.by\(\(\) => \{[\s\S]*?pageSnapshot\.revision[\s\S]*?buildAlbumIndexRenderRows\(/);
  assert.match(albumIndexSource, /\{#each row\.items \?\? \[\] as item \(item\.key\)\}/);
});

test('album group and card rows share compact vertical spacing and global group colors', () => {
  assert.match(albumIndexSource, /physicalPixelsToCssPixels\(40, devicePixelRatio\)/);
  assert.match(albumIndexSource, /groupHeadingHeight = \$derived\(Math\.max\(30, rowHeightPx \* 0\.7\)\)/);
  assert.match(albumIndexSource, /background:linear-gradient\(to bottom,transparent 0,transparent var\(--divider-width\),var\(--list-group-row-background\) var\(--divider-width\)\)/);
  assert.match(albumIndexSource, /groupCardGap: verticalGap/);
  assert.match(albumIndexSource, /cardWidth \+ 52 \+ verticalGap/);
  assert.match(albumIndexSource, /--album-vertical-gap:\$\{verticalGap\}px/);
  assert.match(albumIndexSource, /padding:16px clamp\(18px,2vw,34px\) var\(--album-vertical-gap\)/);
  assert.match(albumIndexSource, /devicePixelRatio = window\.devicePixelRatio > 0 \? window\.devicePixelRatio : 1/);
  assert.match(albumIndexSource, /var\(--list-group-row-background\) var\(--divider-width\)\)/);
  assert.match(albumIndexSource, /\.album-index-group-row:hover \{ background:linear-gradient\(to bottom,transparent 0,transparent var\(--divider-width\),var\(--list-group-row-hover-background\) var\(--divider-width\)\); \}/);
  assert.match(albumIndexSource, /column-gap:\$\{gap\}px/);
});
