import assert from 'node:assert/strict';
import test from 'node:test';
import {
  COVER_ZOOM_MAX,
  COVER_ZOOM_MIN,
  clampCoverZoom,
  coverZoomFromFractions,
  coverZoomToFractions,
  coverZoomTransform,
  getWheelZoomFactor,
  isDoubleTap,
  zoomCoverAt,
} from '../src/lib/cover-zoom.js';

const identity = { scale: 1, x: 0, y: 0 };

test('zoom is clamped to min..max and scale 1 or below always recentres', () => {
  assert.deepEqual(zoomCoverAt(identity, 0.1, 50, 50, 800, 600), { scale: COVER_ZOOM_MIN, x: 0, y: 0 });
  assert.deepEqual(zoomCoverAt(identity, 0.7, 50, 50, 800, 600), { scale: 0.7, x: 0, y: 0 });
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
  assert.match(coverZoomTransform({ scale: 0.5, x: 0, y: 0 }), /scale\(0\.5000\)/);
  assert.match(coverZoomTransform({ scale: 2, x: 10, y: -5 }), /translate\(10\.00px, -5\.00px\) scale\(2\.0000\)/);
});

test('double tap needs two close taps in time and place', () => {
  assert.equal(isDoubleTap(null, { x: 0, y: 0, t: 0 }), false);
  assert.equal(isDoubleTap({ x: 0, y: 0, t: 0 }, { x: 5, y: 5, t: 200 }), true);
  assert.equal(isDoubleTap({ x: 0, y: 0, t: 0 }, { x: 5, y: 5, t: 900 }), false);
  assert.equal(isDoubleTap({ x: 0, y: 0, t: 0 }, { x: 300, y: 5, t: 100 }), false);
});

test('a saved zoom round-trips as fractions of the cover size and survives a different cover size', () => {
  const state = { scale: 2, x: 100, y: -50 };
  const saved = coverZoomToFractions(state, 400, 400);
  assert.deepEqual(saved, { scale: 2, offsetX: 0.25, offsetY: -0.125 });
  assert.deepEqual(coverZoomFromFractions(saved, 400, 400), state);
  // On a smaller cover the pan scales with it.
  assert.deepEqual(coverZoomFromFractions(saved, 200, 200), { scale: 2, x: 50, y: -25 });
  // Missing, malformed or out-of-range values are repaired and the pan is clamped (|offset| <= (scale - 1) * size / 2).
  assert.deepEqual(coverZoomFromFractions(null, 400, 400), { scale: 1, x: 0, y: 0 });
  assert.deepEqual(coverZoomFromFractions({ scale: 'big', offsetX: 'left' }, 400, 400), { scale: 1, x: 0, y: 0 });
  assert.deepEqual(coverZoomFromFractions({ scale: 2, offsetX: 3, offsetY: -3 }, 400, 400), { scale: 2, x: 200, y: -200 });
  assert.deepEqual(coverZoomFromFractions({ scale: 0.6, offsetX: 1, offsetY: 1 }, 400, 400), { scale: 0.6, x: 0, y: 0 });
  assert.deepEqual(coverZoomToFractions({ scale: 1, x: 0, y: 0 }, 0, 0), { scale: 1, offsetX: 0, offsetY: 0 });
});

test('the browser image drag is cancelled so a mouse drag can pan the zoomed cover', async () => {
  const listeners = new Map();
  const node = {
    style: {},
    dataset: {},
    offsetWidth: 100,
    offsetHeight: 100,
    addEventListener: (type, handler) => listeners.set(type, handler),
    removeEventListener: (type) => listeners.delete(type),
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 100, height: 100 }),
  };
  const { coverZoom } = await import('../src/lib/cover-zoom.js');
  const action = coverZoom(node, {});
  assert.ok(listeners.has('dragstart'));
  let prevented = false;
  listeners.get('dragstart')({ preventDefault: () => { prevented = true; } });
  assert.equal(prevented, true);
  const hadCancel = 'cancelAnimationFrame' in globalThis;
  if (!hadCancel) globalThis.cancelAnimationFrame = () => {};
  action.destroy();
  if (!hadCancel) delete globalThis.cancelAnimationFrame;
  assert.equal(listeners.has('dragstart'), false);
});
