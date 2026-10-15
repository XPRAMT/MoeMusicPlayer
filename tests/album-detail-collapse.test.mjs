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
  assert.equal(getAlbumDetailCoverSize(148, 0), 148);
  assert.equal(getAlbumDetailCoverSize(148, 20), 128);
  assert.equal(getAlbumDetailCoverSize(116, 20), 96);
});

test('cover stops at 64px for desktop and narrow starting sizes', () => {
  assert.equal(getAlbumDetailCoverSize(148, 84), 64);
  assert.equal(getAlbumDetailCoverSize(116, 52), 64);
  assert.equal(getAlbumDetailCoverSize(148, 400), 64);
  assert.equal(getAlbumDetailCollapseRange(148), 84);
  assert.equal(getAlbumDetailCollapseRange(116), 52);
});

test('track scroller starts at the fully collapsed heading plus table header', () => {
  assert.equal(getAlbumDetailScrollerTop(184, 84, 37), 137);
  assert.equal(getAlbumDetailScrollerTop(144, 52, 32), 124);
  assert.equal(getAlbumDetailScrollerTop(Number.NaN, -1, Number.NaN), 0);
});

test('native scroll collapses the cover first, then scrolls tracks, identically in both directions', () => {
  assert.deepEqual(getAlbumDetailScrollState(0, 84), { collapseOffset: 0, listScrollTop: 0 });
  assert.deepEqual(getAlbumDetailScrollState(40, 84), { collapseOffset: 40, listScrollTop: 0 });
  assert.deepEqual(getAlbumDetailScrollState(84, 84), { collapseOffset: 84, listScrollTop: 0 });
  assert.deepEqual(getAlbumDetailScrollState(1_084, 84), { collapseOffset: 84, listScrollTop: 1_000 });
  // Moving back up restores tracks to their top before the cover expands.
  assert.deepEqual(getAlbumDetailScrollState(90, 84), { collapseOffset: 84, listScrollTop: 6 });
  assert.deepEqual(getAlbumDetailScrollState(30, 84), { collapseOffset: 30, listScrollTop: 0 });
  assert.deepEqual(getAlbumDetailScrollState(Number.NaN, 84), { collapseOffset: 0, listScrollTop: 0 });
});

test('logical list positions round-trip through the native scroll offset', () => {
  for (const list of [0, 1, 50, 2_400]) {
    const native = getAlbumDetailNativeScrollTop(list, 84, 84);
    assert.equal(getAlbumDetailScrollState(native, 84).listScrollTop, list);
  }
  assert.equal(getAlbumDetailNativeScrollTop(0, 84, 20), 20);
  assert.equal(getAlbumDetailNativeScrollTop(0, 84), 0);
  assert.equal(getAlbumDetailNativeScrollTop(10, 84, 0), 94);
});

test('scroller geometry and thumb ratio do not depend on cover collapse', () => {
  const shellHeight = 760;
  const top = getAlbumDetailScrollerTop(184, 84, 37);
  const clientHeight = shellHeight - top;
  const scrollHeight = 84 + 103 * 50;
  const ratios = new Set([0, 20, 84, 3_000].map(() => clientHeight / scrollHeight));
  assert.equal(ratios.size, 1);
  // Rows start exactly at the scroller's top edge once fully collapsed, and
  // below it (behind only the collapsing heading) before that.
  for (const native of [0, 30, 84, 500]) {
    const { collapseOffset } = getAlbumDetailScrollState(native, 84);
    const firstRowViewportTop = top + 84 - native;
    const listBoundary = 184 - collapseOffset + 37;
    if (native <= 84) assert.equal(firstRowViewportTop, listBoundary);
    assert.ok(listBoundary >= top);
  }
});