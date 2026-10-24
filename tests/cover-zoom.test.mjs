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

test('geometry from one rect: layout centre, un-scaled size, and an absolute anchor that corrects layout shifts', async () => {
  const { coverGeometry, contentOffsetAt, anchoredZoom } = await import('../src/lib/cover-zoom.js');
  // A 200x200 cover at layout centre (300, 300), zoomed 2x and panned by (-40, 10): the rect is the visual box.
  const state = { scale: 2, x: -40, y: 10 };
  const rect = { left: 300 - 40 - 200, top: 300 + 10 - 200, width: 400, height: 400 };
  const geo = coverGeometry(rect, state);
  assert.deepEqual([geo.centreX, geo.centreY, geo.layoutX, geo.layoutY, geo.width, geo.height], [260, 310, 300, 300, 200, 200]);
  // The content point under the cursor stays under the cursor after zooming, from the same geometry.
  const cursor = { x: 350, y: 330 };
  const u = contentOffsetAt(geo, state, cursor.x, cursor.y);
  const next = anchoredZoom(geo, u, cursor, 3);
  assert.equal(next.scale, 3);
  assert.equal(300 + next.x + u.x * 3, cursor.x);
  assert.equal(300 + next.y + u.y * 3, cursor.y);
  // Same result as the relative formula when the layout did not move.
  const relative = zoomCoverAt(state, 3, cursor.x - geo.centreX, cursor.y - geo.centreY, 200, 200);
  assert.ok(Math.abs(relative.x - next.x) < 1e-9 && Math.abs(relative.y - next.y) < 1e-9);
  // The layout moved by +30px between steps: the next write absorbs it (the cover point stays under the cursor).
  const moved = coverGeometry({ ...rect, left: rect.left + 30 }, state);
  const fixed = anchoredZoom(moved, u, cursor, 2);
  assert.equal(330 + fixed.x + u.x * 2, cursor.x);
});

test('the idle gate turns on at the first poke, off after the idle time, and hold keeps it on', async () => {
  const { createIdleGate } = await import('../src/lib/cover-zoom.js');
  const timers = [];
  const fake = { set: (fn, ms) => { const t = { fn, ms, live: true }; timers.push(t); return t; }, clear: (t) => { t.live = false; } };
  const changes = [];
  const gate = createIdleGate(150, (on) => changes.push(on), fake);
  gate.poke(); gate.poke();
  assert.deepEqual(changes, [true]);
  assert.equal(timers.filter((t) => t.live).length, 1);
  assert.equal(timers.at(-1).ms, 150);
  gate.hold();
  assert.equal(timers.filter((t) => t.live).length, 0);
  gate.release();
  const t = timers.filter((x) => x.live)[0];
  t.fn();
  assert.deepEqual(changes, [true, false]);
  gate.poke(); gate.end();
  assert.deepEqual(changes, [true, false, true, false]);
});

function fakeCover(rect) {
  const listeners = new Map();
  let rectReads = 0;
  const node = {
    style: {}, dataset: {}, offsetWidth: rect.width, offsetHeight: rect.height,
    addEventListener: (type, handler) => listeners.set(type, handler),
    removeEventListener: (type) => listeners.delete(type),
    getBoundingClientRect: () => { rectReads += 1; return { ...rect }; },
    setPointerCapture: () => {},
    dispatchEvent: () => true,
  };
  return { node, listeners, reads: () => rectReads };
}

test('wheel steps and pinch moves are coalesced into one apply and one rect read per frame', async () => {
  const { coverZoom } = await import('../src/lib/cover-zoom.js');
  const frames = [];
  globalThis.requestAnimationFrame = (fn) => { frames.push(fn); return frames.length; };
  globalThis.cancelAnimationFrame = () => {};
  try {
    const { node, listeners, reads } = fakeCover({ left: 100, top: 100, width: 200, height: 200 });
    const zooms = [];
    const zooming = [];
    const action = coverZoom(node, { onZoom: (s) => zooms.push(s), onZoomingChange: (z) => zooming.push(z) });
    const event = (deltaY) => ({ deltaY, deltaMode: 0, clientX: 200, clientY: 200, preventDefault() {} });
    listeners.get('wheel')(event(-100));
    listeners.get('wheel')(event(-100));
    listeners.get('wheel')(event(-100));
    assert.equal(frames.length, 1, 'one scheduled frame');
    assert.equal(reads(), 0, 'no layout read in the handlers');
    assert.deepEqual(zooming, [true]);
    frames.shift()();
    assert.equal(reads(), 1);
    assert.equal(zooms.length, 1);
    assert.ok(Math.abs(zooms[0].scale - Math.exp(0.45)) < 1e-9, 'the three factors multiplied');
    assert.equal('coverGesture' in node.dataset, true);
    assert.equal('coverZoomed' in node.dataset, true);
    // The follow-up settle frame re-pins the anchor with at most one more read and one write.
    assert.equal(frames.length, 1);
    frames.shift()();
    assert.equal(reads(), 2);
    assert.ok(zooms.length <= 2);
    action.destroy();
    assert.deepEqual(zooming, [true, false]);
    assert.equal('coverGesture' in node.dataset, false);
  } finally {
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

test('a pinch applies the latest positions of both fingers once per frame', async () => {
  const { coverZoom } = await import('../src/lib/cover-zoom.js');
  const frames = [];
  globalThis.requestAnimationFrame = (fn) => { frames.push(fn); return frames.length; };
  globalThis.cancelAnimationFrame = () => {};
  try {
    const { node, listeners, reads } = fakeCover({ left: 100, top: 100, width: 200, height: 200 });
    const zooms = [];
    coverZoom(node, { onZoom: (s) => zooms.push(s) });
    const ptr = (id, x, y) => ({ pointerId: id, clientX: x, clientY: y, pointerType: 'touch', cancelable: true, preventDefault() {} });
    listeners.get('pointerdown')(ptr(1, 150, 200));
    listeners.get('pointerdown')(ptr(2, 250, 200));
    for (let i = 1; i <= 5; i += 1) {
      listeners.get('pointermove')(ptr(1, 150 - i * 4, 200));
      listeners.get('pointermove')(ptr(2, 250 + i * 4, 200));
    }
    assert.equal(frames.length, 1);
    frames.shift()();
    assert.equal(reads(), 1);
    assert.equal(zooms.length, 1);
    assert.ok(Math.abs(zooms[0].scale - 1.4) < 1e-9);
  } finally {
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});
