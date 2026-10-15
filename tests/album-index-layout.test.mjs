import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { buildAlbumIndexRenderRows, buildAlbumIndexRows, findVisibleAlbumIndexRows, getAlbumCardGrid, getAlbumCardTextHeight, normalizeAlbumCardMaxWidthPx, physicalPixelsToCssPixels, shouldGroupAlbumIndex } from '../src/lib/album-index-layout.js';

const albumIndexSource = readFileSync(new URL('../src/lib/AlbumIndex.svelte', import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const pagedVirtualListSource = readFileSync(new URL('../src/lib/PagedVirtualList.svelte', import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const appCssSource = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8').replace(/\r\n/g, '\n');

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

test('album card content ends at its measured height so following card gap is exactly 40 physical pixels', () => {
  const textHeight = getAlbumCardTextHeight(13, 12);
  assert.equal(textHeight, 43);
  const cardHeight = 168 + textHeight;
  const rowHeight = cardHeight + physicalPixelsToCssPixels(40, 2.5);
  const textContentHeight = 8 + 13 * 1.4 + 12 * 1.4;
  assert.equal(cardHeight, 168 + textContentHeight);
  assert.equal((rowHeight - cardHeight) * 2.5, 40);
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
  assert.match(albumIndexSource, /class="album-index-group-row list-group-row-surface"/);
  assert.match(pagedVirtualListSource, /class="paged-virtual-row paged-virtual-group-row list-group-row-surface"/);
  assert.match(appCssSource, /\.list-group-row-surface \{[\s\S]*?border-bottom: var\(--divider-width\) solid var\(--divider-color\)[\s\S]*?var\(--list-group-row-background\) 100%/);
  assert.match(appCssSource, /\.list-group-row-surface:hover \{[\s\S]*?var\(--list-group-row-hover-background\) 100%/);
  assert.match(albumIndexSource, /groupCardGap: verticalGap/);
  assert.match(albumIndexSource, /cardHeight = \$derived\(cardWidth \+ cardTextHeight\)/);
  assert.match(albumIndexSource, /rowHeight = \$derived\(cardHeight \+ verticalGap\)/);
  assert.match(albumIndexSource, /getAlbumCardTextHeight\([\s\S]*?--font-body[\s\S]*?--font-caption/);
  assert.match(albumIndexSource, /--album-vertical-gap:\$\{verticalGap\}px/);
  assert.match(albumIndexSource, /padding:16px clamp\(18px,2vw,34px\) var\(--album-vertical-gap\)/);
  assert.match(albumIndexSource, /devicePixelRatio = window\.devicePixelRatio > 0 \? window\.devicePixelRatio : 1/);
  assert.match(albumIndexSource, /column-gap:\$\{gap\}px/);
});

test('album card grid fills the row with cards no wider than the maximum', () => {
  const gap = 18;
  for (const available of [1, 150, 179, 180, 197, 198, 377, 1000, 1269, 1500, 2560]) {
    for (const max of [100, 168, 180, 320, 500]) {
      const { columns, cardWidth } = getAlbumCardGrid(available, max, gap);
      assert.ok(columns >= 1);
      assert.ok(cardWidth <= max + 1e-9, `card ${cardWidth} > ${max} at ${available}`);
      if (cardWidth < max) {
        const used = columns * cardWidth + (columns - 1) * gap;
        assert.ok(Math.abs(used - available) < 1e-6, `row leaves a gap at ${available}/${max}`);
      }
      assert.equal(columns, Math.max(1, Math.ceil((available + gap) / (max + gap))));
    }
  }
});

test('album card grid adds a narrower column instead of leaving a trailing gap', () => {
  // Five 180px cards plus gaps need 972px; a 1000px row gets six narrower cards.
  const grid = getAlbumCardGrid(1000, 180, 18);
  assert.equal(grid.columns, 6);
  assert.ok(Math.abs(grid.cardWidth - (1000 - 18 * 5) / 6) < 1e-9);
  // Exactly five full-width cards fit without extra columns.
  assert.deepEqual(getAlbumCardGrid(5 * 180 + 4 * 18, 180, 18), { columns: 5, cardWidth: 180 });
  assert.deepEqual(getAlbumCardGrid(0, 180, 18), { columns: 1, cardWidth: 180 });
  assert.deepEqual(getAlbumCardGrid(Number.NaN, Number.NaN, Number.NaN), { columns: 1, cardWidth: 180 });
});

test('album card maximum width normalizes persisted values', () => {
  assert.equal(normalizeAlbumCardMaxWidthPx(undefined), 180);
  assert.equal(normalizeAlbumCardMaxWidthPx(10), 100);
  assert.equal(normalizeAlbumCardMaxWidthPx(999), 500);
  assert.equal(normalizeAlbumCardMaxWidthPx(200.4), 200);
});