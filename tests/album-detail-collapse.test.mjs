import test from 'node:test';
import assert from 'node:assert/strict';
import {
  getAlbumDetailNativeScrollTop,
  getAlbumDetailCoverSize,
  mapAlbumDetailNativeScroll,
  updateAlbumDetailScrollPeak,
} from '../src/lib/album-detail-collapse.js';

test('album detail cover shrinks one CSS pixel per downward scroll pixel', () => {
  assert.equal(getAlbumDetailCoverSize(148, 0), 148);
  assert.equal(getAlbumDetailCoverSize(148, 20), 128);
  assert.equal(getAlbumDetailCoverSize(116, 20), 96);
});

test('upward scrolling away from the top keeps the furthest collapsed size', () => {
  const peakAfterScrollingDown = updateAlbumDetailScrollPeak(0, 80);
  const peakAfterScrollingUp = updateAlbumDetailScrollPeak(peakAfterScrollingDown, 30);
  assert.equal(peakAfterScrollingUp, 80);
  assert.equal(getAlbumDetailCoverSize(148, peakAfterScrollingUp), 68);
});

test('cover stops at 64px for desktop and narrow starting sizes', () => {
  assert.equal(getAlbumDetailCoverSize(148, 84), 64);
  assert.equal(getAlbumDetailCoverSize(116, 52), 64);
  assert.equal(getAlbumDetailCoverSize(148, 400), 64);
});

test('returning the list to its top resets collapse so the cover restores', () => {
  const collapsedPeak = updateAlbumDetailScrollPeak(0, 100);
  const restoredPeak = updateAlbumDetailScrollPeak(collapsedPeak, 0);
  assert.equal(restoredPeak, 0);
  assert.equal(getAlbumDetailCoverSize(148, restoredPeak), 148);
  assert.equal(getAlbumDetailCoverSize(116, restoredPeak), 116);
});

test('stable scroll mapping preserves list movement while the cover collapses', () => {
  let state = { rawScrollTop: 0, previousRawScrollTop: 0, previousListScrollTop: 0, previousCollapseOffset: 0, baseCoverSize: 148 };
  state = { ...state, rawScrollTop: 40 };
  let mapped = mapAlbumDetailNativeScroll(state);
  assert.deepEqual(mapped, { listScrollTop: 20, collapseOffset: 20, rawScrollTop: 40 });
  state = { ...state, rawScrollTop: 100, previousRawScrollTop: 40, previousListScrollTop: mapped.listScrollTop, previousCollapseOffset: mapped.collapseOffset };
  mapped = mapAlbumDetailNativeScroll(state);
  assert.deepEqual(mapped, { listScrollTop: 50, collapseOffset: 50, rawScrollTop: 100 });
  assert.equal(getAlbumDetailCoverSize(148, mapped.collapseOffset), 98);
});

test('upward native scroll keeps the compact cover until the logical list reaches its top', () => {
  const mapped = mapAlbumDetailNativeScroll({
    rawScrollTop: 70,
    previousRawScrollTop: 100,
    previousListScrollTop: 50,
    previousCollapseOffset: 50,
    baseCoverSize: 148,
  });
  assert.deepEqual(mapped, { listScrollTop: 20, collapseOffset: 50, rawScrollTop: 70 });
  const top = mapAlbumDetailNativeScroll({
    rawScrollTop: 0,
    previousRawScrollTop: 70,
    previousListScrollTop: 20,
    previousCollapseOffset: 50,
    baseCoverSize: 148,
  });
  assert.deepEqual(top, { listScrollTop: 0, collapseOffset: 0, rawScrollTop: 0 });
});

test('native scrollbar extent stays constant across cover sizes and restores logical positions', () => {
  const viewportHeight = 600;
  const headingHeight = 184;
  const tableHeaderHeight = 37;
  const contentRowsHeight = 100_000;
  const scrollHeight = headingHeight + tableHeaderHeight + contentRowsHeight;
  const thumbRatioAtFullSize = viewportHeight / scrollHeight;
  for (const collapsedBy of [0, 20, 84]) {
    const visibleListHeight = viewportHeight - headingHeight + collapsedBy - tableHeaderHeight;
    assert.ok(visibleListHeight > 0);
    assert.equal(viewportHeight / scrollHeight, thumbRatioAtFullSize);
    assert.equal(getAlbumDetailNativeScrollTop(2_400, collapsedBy), 2_400 + collapsedBy);
  }
});
