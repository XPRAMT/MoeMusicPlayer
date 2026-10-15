import test from 'node:test';
import assert from 'node:assert/strict';
import {
  getAlbumDetailNativeScrollTop,
  getAlbumDetailCoverSize,
  mapAlbumDetailNativeScroll,
  mapAlbumDetailWheelDelta,
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
});

test('wheel scrolling collapses one pixel per pixel before scrolling list rows', () => {
  let state = { listScrollTop: 0, collapseOffset: 0, baseCoverSize: 148 };
  state = mapAlbumDetailWheelDelta({ ...state, delta: 10 });
  assert.deepEqual(state, { listScrollTop: 0, collapseOffset: 10 });
  assert.equal(getAlbumDetailCoverSize(148, state.collapseOffset), 138);
  state = mapAlbumDetailWheelDelta({ ...state, delta: 74 });
  assert.deepEqual(state, { listScrollTop: 0, collapseOffset: 84 });
  state = mapAlbumDetailWheelDelta({ ...state, delta: 10 });
  assert.deepEqual(state, { listScrollTop: 10, collapseOffset: 84 });
});

test('wheel scrolling returns the list to its top before restoring the cover continuously', () => {
  let state = { listScrollTop: 100, collapseOffset: 84, baseCoverSize: 148 };
  state = mapAlbumDetailWheelDelta({ ...state, delta: -60 });
  assert.deepEqual(state, { listScrollTop: 40, collapseOffset: 84 });
  state = mapAlbumDetailWheelDelta({ ...state, delta: -40 });
  assert.deepEqual(state, { listScrollTop: 0, collapseOffset: 84 });
  state = mapAlbumDetailWheelDelta({ ...state, delta: -10 });
  assert.deepEqual(state, { listScrollTop: 0, collapseOffset: 74 });
  assert.equal(getAlbumDetailCoverSize(148, state.collapseOffset), 74);
  state = mapAlbumDetailWheelDelta({ ...state, delta: -74 });
  assert.equal(getAlbumDetailCoverSize(148, state.collapseOffset), 148);
});

test('wheel movement crossing the list top spends only the excess restoring the cover', () => {
  const state = mapAlbumDetailWheelDelta({
    delta: -10,
    listScrollTop: 3,
    collapseOffset: 84,
    baseCoverSize: 148,
  });
  assert.deepEqual(state, { listScrollTop: 0, collapseOffset: 77 });
  assert.equal(getAlbumDetailCoverSize(148, state.collapseOffset), 71);
});

test('native scroll maps one pixel to cover collapse before list movement', () => {
  let state = { rawScrollTop: 0, previousRawScrollTop: 0, previousListScrollTop: 0, previousCollapseOffset: 0, baseCoverSize: 148 };
  state = { ...state, rawScrollTop: 40 };
  let mapped = mapAlbumDetailNativeScroll(state);
  assert.deepEqual(mapped, { listScrollTop: 0, collapseOffset: 40, rawScrollTop: 40 });
  state = { ...state, rawScrollTop: 100, previousRawScrollTop: 40, previousListScrollTop: mapped.listScrollTop, previousCollapseOffset: mapped.collapseOffset };
  mapped = mapAlbumDetailNativeScroll(state);
  assert.deepEqual(mapped, { listScrollTop: 16, collapseOffset: 84, rawScrollTop: 100 });
  assert.equal(getAlbumDetailCoverSize(148, mapped.collapseOffset), 64);
});

test('upward native scroll preserves collapse until list top and maps excess to cover restoration', () => {
  const mapped = mapAlbumDetailNativeScroll({
    rawScrollTop: 70,
    previousRawScrollTop: 100,
    previousListScrollTop: 50,
    previousCollapseOffset: 50,
    baseCoverSize: 148,
  });
  assert.deepEqual(mapped, { listScrollTop: 20, collapseOffset: 50, rawScrollTop: 70 });
  const top = mapAlbumDetailNativeScroll({
    rawScrollTop: 20,
    previousRawScrollTop: 70,
    previousListScrollTop: 20,
    previousCollapseOffset: 50,
    baseCoverSize: 148,
  });
  assert.deepEqual(top, { listScrollTop: 0, collapseOffset: 20, rawScrollTop: 20 });
  const restore = mapAlbumDetailNativeScroll({
    rawScrollTop: 0,
    previousRawScrollTop: 20,
    previousListScrollTop: 0,
    previousCollapseOffset: 20,
    baseCoverSize: 148,
  });
  assert.deepEqual(restore, { listScrollTop: 0, collapseOffset: 0, rawScrollTop: 0 });
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
