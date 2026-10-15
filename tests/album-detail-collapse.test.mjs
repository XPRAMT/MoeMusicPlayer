import test from 'node:test';
import assert from 'node:assert/strict';
import {
  getAlbumDetailCoverSize,
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
