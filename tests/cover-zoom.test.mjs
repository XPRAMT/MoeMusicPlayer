import assert from 'node:assert/strict';
import test from 'node:test';
import {
  COVER_ZOOM_MAX,
  clampCoverZoom,
  coverZoomTransform,
  getWheelZoomFactor,
  isDoubleTap,
  zoomCoverAt,
} from '../src/lib/cover-zoom.js';

const identity = { scale: 1, x: 0, y: 0 };

test('zoom is clamped to 1..max and scale 1 always recentres', () => {
  assert.deepEqual(zoomCoverAt(identity, 0.2, 50, 50, 800, 600), identity);
  assert.equal(zoomCoverAt(identity, 99, 0, 0, 800, 600).scale, COVER_ZOOM_MAX);
  assert.deepEqual(clampCoverZoom({ scale: 1, x: 40, y: -9 }, 800, 600), identity);
});

test('zoom keeps the point under the cursor fixed', () => {
  const next = zoomCoverAt(identity, 2, 100, 50, 800, 600);
  assert.equal(next.x, -100);
  assert.equal(next.y, -50);
});

test('panning is bounded by the scaled overflow', () => {
  const clamped = clampCoverZoom({ scale: 2, x: 9999, y: -9999 }, 800, 600);
  assert.equal(clamped.x, 400);
  assert.equal(clamped.y, -300);
});

test('wheel up zooms in, wheel down zooms out, line mode is stronger', () => {
  assert.ok(getWheelZoomFactor(-100) > 1);
  assert.ok(getWheelZoomFactor(100) < 1);
  assert.ok(getWheelZoomFactor(-3, 1) > getWheelZoomFactor(-3, 0));
});

test('identity renders no transform; zoomed renders translate and scale', () => {
  assert.equal(coverZoomTransform(identity), '');
  assert.match(coverZoomTransform({ scale: 2, x: 10, y: -5 }), /translate\(10\.00px, -5\.00px\) scale\(2\.0000\)/);
});

test('double tap needs two close taps in time and place', () => {
  assert.equal(isDoubleTap(null, { x: 0, y: 0, t: 0 }), false);
  assert.equal(isDoubleTap({ x: 0, y: 0, t: 0 }, { x: 5, y: 5, t: 200 }), true);
  assert.equal(isDoubleTap({ x: 0, y: 0, t: 0 }, { x: 5, y: 5, t: 900 }), false);
  assert.equal(isDoubleTap({ x: 0, y: 0, t: 0 }, { x: 300, y: 5, t: 100 }), false);
});
