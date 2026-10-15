import test from 'node:test';
import assert from 'node:assert/strict';
import {
  getAlbumDetailCollapseRange,
  getAlbumDetailCoverSize,
  getAlbumDetailNativeScrollTop,
  getAlbumDetailScrollerTop,
  getAlbumDetailScrollState,
} from '../src/lib/album-detail-collapse.js';

test('album detail cover shrinks one CSS pixel per downward scroll pixel', () => {
  assert.equal(getAlbumDetailCoverSize(200, 0), 200);
  assert.equal(getAlbumDetailCoverSize(200, 20), 180);
  assert.equal(getAlbumDetailCoverSize(160, 20), 140);
});

test('cover stops at 80px for desktop and narrow starting sizes', () => {
  assert.equal(getAlbumDetailCoverSize(200, 120), 80);
  assert.equal(getAlbumDetailCoverSize(160, 80), 80);
  assert.equal(getAlbumDetailCoverSize(200, 400), 80);
  assert.equal(getAlbumDetailCollapseRange(200), 120);
  assert.equal(getAlbumDetailCollapseRange(160), 80);
  assert.equal(getAlbumDetailCollapseRange(64), 0);
});

test('track scroller starts at the fully collapsed heading plus table header', () => {
  assert.equal(getAlbumDetailScrollerTop(236, 120, 37), 153);
  assert.equal(getAlbumDetailScrollerTop(188, 80, 32), 140);
  assert.equal(getAlbumDetailScrollerTop(Number.NaN, -1, Number.NaN), 0);
});

test('native scroll collapses the cover first, then scrolls tracks, identically in both directions', () => {
  assert.deepEqual(getAlbumDetailScrollState(0, 120), { collapseOffset: 0, listScrollTop: 0 });
  assert.deepEqual(getAlbumDetailScrollState(40, 120), { collapseOffset: 40, listScrollTop: 0 });
  assert.deepEqual(getAlbumDetailScrollState(120, 120), { collapseOffset: 120, listScrollTop: 0 });
  assert.deepEqual(getAlbumDetailScrollState(1_120, 120), { collapseOffset: 120, listScrollTop: 1_000 });
  // Moving back up restores tracks to their top before the cover expands.
  assert.deepEqual(getAlbumDetailScrollState(126, 120), { collapseOffset: 120, listScrollTop: 6 });
  assert.deepEqual(getAlbumDetailScrollState(30, 120), { collapseOffset: 30, listScrollTop: 0 });
  assert.deepEqual(getAlbumDetailScrollState(Number.NaN, 120), { collapseOffset: 0, listScrollTop: 0 });
});

test('logical list positions round-trip through the native scroll offset', () => {
  for (const list of [0, 1, 50, 2_400]) {
    const native = getAlbumDetailNativeScrollTop(list, 120, 120);
    assert.equal(getAlbumDetailScrollState(native, 120).listScrollTop, list);
  }
  assert.equal(getAlbumDetailNativeScrollTop(0, 120, 20), 20);
  assert.equal(getAlbumDetailNativeScrollTop(0, 120), 0);
  assert.equal(getAlbumDetailNativeScrollTop(10, 120, 0), 130);
});

test('scroller geometry and thumb ratio do not depend on cover collapse', () => {
  for (const [heading, range, header] of [[236, 120, 37], [188, 80, 32]]) {
    const shellHeight = 760;
    const top = getAlbumDetailScrollerTop(heading, range, header);
    const clientHeight = shellHeight - top;
    const scrollHeight = range + 103 * 50;
    const ratios = new Set([0, 20, range, 3_000].map(() => clientHeight / scrollHeight));
    assert.equal(ratios.size, 1);
    // Rows start exactly at the list boundary while collapsing and never above the scroller top.
    for (const native of [0, 30, range, 500]) {
      const { collapseOffset } = getAlbumDetailScrollState(native, range);
      const firstRowViewportTop = top + range - native;
      const listBoundary = heading - collapseOffset + header;
      if (native <= range) assert.equal(firstRowViewportTop, listBoundary);
      assert.ok(listBoundary >= top);
    }
  }
});
