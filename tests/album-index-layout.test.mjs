import test from 'node:test';
import assert from 'node:assert/strict';
import { buildAlbumIndexRows, findVisibleAlbumIndexRows, shouldGroupAlbumIndex } from '../src/lib/album-index-layout.js';

test('only title ordering uses album first-character grouping when the setting is enabled', () => {
  assert.equal(shouldGroupAlbumIndex({ sortField: 'title', groupingEnabled: true }), true);
  assert.equal(shouldGroupAlbumIndex({ sortField: 'title', groupingEnabled: false }), false);
  assert.equal(shouldGroupAlbumIndex({ sortField: 'duration', groupingEnabled: true }), false);
  assert.equal(shouldGroupAlbumIndex({ sortField: 'played', groupingEnabled: true }), false);
  assert.equal(shouldGroupAlbumIndex({ sortField: 'trackCount', groupingEnabled: true }), false);
});

test('album title groups become full-width virtual rows and restart the card grid', () => {
  const layout = buildAlbumIndexRows({
    totalCount: 10,
    columns: 3,
    cardRowHeight: 220,
    groupHeadingHeight: 37,
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
    ['cards', 0, 3, 37],
    ['cards', 3, 4, 257],
    ['group', 'B', 4, 477],
    ['cards', 4, 7, 514],
    ['cards', 7, 8, 734],
    ['group', '未知', 8, 954],
    ['cards', 8, 10, 991],
  ]);
  assert.equal(layout.totalHeight, 1211);
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
    groups: [{ label: 'A', beforeIndex: 0 }, { label: 'B', beforeIndex: 4 }],
  });
  assert.deepEqual(findVisibleAlbumIndexRows(layout.rows, 240, 220, 40).map((row) => row.kind === 'group'
    ? `group:${row.label}`
    : `cards:${row.start}-${row.end}`), ['cards:0-2', 'cards:2-4', 'group:B', 'cards:4-6']);
});
