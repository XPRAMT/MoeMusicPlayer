import assert from 'node:assert/strict';
import test from 'node:test';
import { parseSwipeFeedback } from '../src/lib/cover-overlay.js';
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
    assert.equal('coverZoomed' in node.dataset, true);
    // The layout the scale change caused is committed in a microtask of the same frame (before paint): the anchor is
    // re-pinned there with one more read and at most one more write, not in a later frame.
    assert.equal(frames.length, 0, 'no follow-up frame');
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(reads(), 2);
    assert.ok(zooms.length <= 2);
    action.destroy();
    assert.deepEqual(zooming, [true, false]);
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

test('FLIP: the inverse transform paints the moved cover exactly where it was', async () => {
  const { flipTransform, coverZoomTransform } = await import('../src/lib/cover-zoom.js');
  // Zoomed 3.2x: the painted box is 640 wide, centred at x 523.7; the restored layout box is 200 wide, centred at 412.1.
  const from = { left: 523.7 - 320, top: 100, width: 640, height: 640 };
  const to = { left: 412.1 - 100, top: 420, width: 200, height: 200 };
  const flip = flipTransform(from, to);
  assert.ok(Math.abs(flip.scale - 3.2) < 1e-9);
  assert.ok(Math.abs(flip.x - 111.6) < 1e-9);
  assert.ok(Math.abs(flip.y - (100 + 320 - (420 + 100))) < 1e-9);
  // Applied around the layout centre it reproduces the old centre and size.
  assert.ok(Math.abs(412.1 + flip.x - 523.7) < 1e-9);
  assert.ok(Math.abs(to.width * flip.scale - from.width) < 1e-9);
  assert.match(coverZoomTransform(flip), /^translate\(111\.60px, -100\.00px\) scale\(3\.2000\)$/);
  assert.deepEqual(flipTransform(to, to), { scale: 1, x: 0, y: 0 });
});

test('reset commits the final layout and the compensating glide in the same task, with no uncompensated frame', async () => {
  const { coverZoom } = await import('../src/lib/cover-zoom.js');
  const frames = [];
  globalThis.requestAnimationFrame = (fn) => { frames.push(fn); return frames.length; };
  globalThis.cancelAnimationFrame = () => {};
  try {
    const { node, listeners } = fakeCover({ left: 100, top: 100, width: 200, height: 200 });
    // The layout box moves when the owner has committed the identity state (the lyrics column snaps back).
    let layoutCommitted = false;
    node.getBoundingClientRect = () => (layoutCommitted
      ? { left: 40, top: 100, width: 200, height: 200 }
      : { left: 100, top: 100, width: 200, height: 200 });
    const animations = [];
    node.animate = (keyframes, timing) => {
      const animation = { keyframes, timing, currentTime: 0, cancel() { this.cancelled = true; }, onfinish: null };
      animations.push(animation);
      return animation;
    };
    const events = [];
    const action = coverZoom(node, {
      initial: { scale: 2, x: 0, y: 0 },
      onZoom: (state) => { events.push(['zoom', state.scale]); layoutCommitted = true; },
      settleLayout: () => { events.push(['settle']); return Promise.resolve(); },
    });
    assert.equal(node.style.transform, 'translate(0.00px, 0.00px) scale(2.0000)');
    listeners.get('dblclick')();
    // Synchronously: layout committed (owner told, transform and attribute cleared), glide not yet started.
    assert.deepEqual(events, [['zoom', 1], ['settle']]);
    assert.equal(node.style.transform, '');
    assert.equal('coverZoomed' in node.dataset, false);
    assert.equal('coverResetting' in node.dataset, true);
    assert.equal(animations.length, 0);
    await Promise.resolve(); await Promise.resolve();
    // Same task, before any frame: the glide starts from the painted box (centre 200, size 200 -> layout centre 140).
    assert.equal(frames.length, 0, 'nothing waits for a later frame');
    assert.equal(animations.length, 1);
    assert.equal(animations[0].keyframes[0].transform, 'translate(60.00px, 0.00px) scale(1.0000)');
    assert.equal(animations[0].keyframes[1].transform, 'translate(0px, 0px) scale(1)');
    assert.equal(animations[0].timing.duration, 200);
    animations[0].onfinish();
    assert.equal('coverResetting' in node.dataset, false);
    action.destroy();
  } finally {
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

test('source shape: reset has no timer or later frame between the layout commit and its compensation', async () => {
  const { readFileSync } = await import('node:fs');
  const source = readFileSync(new URL('../src/lib/cover-zoom.js', import.meta.url), 'utf8');
  const reset = source.slice(source.indexOf('  function reset()'), source.indexOf('  /** @param {{ left: number, top: number, width: number, height: number }} from @param {number} [elapsedMs] */'));
  assert.doesNotMatch(reset, /setTimeout|raf\(|requestAnimationFrame/);
  assert.ok(reset.lastIndexOf('apply({ ...COVER_ZOOM_IDENTITY }, false)') < reset.indexOf('settleLayout'));
  assert.ok(reset.indexOf('getBoundingClientRect') < reset.lastIndexOf('apply({ ...COVER_ZOOM_IDENTITY }, false)'));
  const glide = source.slice(source.indexOf('  function startGlide'), source.indexOf('  function endGlide'));
  assert.doesNotMatch(glide, /setTimeout|raf\(|requestAnimationFrame/);
  assert.match(glide, /const to = node\.getBoundingClientRect\(\);/);
  assert.match(glide, /const start = flipTransform\(from, to\);/);
  // The per-step squeeze is re-pinned in a microtask of the same frame, never by scheduling another frame.
  const repin = source.slice(source.indexOf('  function repin'), source.indexOf('  /** Back to the original size'));
  assert.doesNotMatch(repin, /schedule\(|raf\(|setTimeout/);
  assert.match(repin, /settleLayout/);
});

test('timings: the reset glide lasts 200 ms and the lyrics re-wrap 50 ms after the last zoom input', async () => {
  const { COVER_RESET_MS, COVER_ZOOM_SETTLE_MS } = await import('../src/lib/cover-zoom.js');
  assert.equal(COVER_RESET_MS, 200);
  assert.equal(COVER_ZOOM_SETTLE_MS, 50);
});

test('overlay geometry: object-fit cover placement with the 1px bleed', async () => {
  const { objectFitCoverPlacement } = await import('../src/lib/cover-overlay.js');
  // A 2:1 image in a 100x100 box that bleeds 1px: scaled to the 102 px height, centred horizontally.
  const wide = objectFitCoverPlacement(100, 100, 400, 200, 1);
  assert.equal(wide.k, 102 / 200);
  assert.equal(wide.top, -1);
  assert.ok(Math.abs(wide.left - (-1 + (102 - 204) / 2)) < 1e-9);
  // Square image, no bleed: exact fit.
  assert.deepEqual(objectFitCoverPlacement(300, 300, 1500, 1500, 0), { k: 0.2, left: 0, top: 0 });
});

test('overlay geometry: the visible part of a zoomed, panned cover maps to the right source rect at device resolution', async () => {
  const { coverDrawPlan, intersectRects, snapRectToDevicePixels } = await import('../src/lib/cover-overlay.js');
  // A 200x200 stage (square 1000x1000 image, no bleed) zoomed 3x around its centre (300, 300) and panned by (+60, -30):
  // painted box 600x600 at (60, -30) .. (660, 570). The card clips to (100, 0) .. (500, 400).
  const visual = { left: 300 + 60 - 300, top: 300 - 30 - 300, width: 600, height: 600 };
  const visible = intersectRects(visual, { left: 100, top: 0, width: 400, height: 400 });
  assert.deepEqual(visible, { left: 100, top: 0, width: 400, height: 400 });
  const plan = coverDrawPlan({ visual, scale: 3, naturalWidth: 1000, naturalHeight: 1000, bleed: 0, visible, dpr: 3.25 });
  assert.deepEqual(plan.canvas, { width: 1300, height: 1300 });
  // Client x 100 is (100 - 60) / 3 stage px from the left; 5 source px per stage px.
  assert.ok(Math.abs(plan.source.x - (40 / 3) * 5) < 1e-9);
  assert.ok(Math.abs(plan.source.y - (30 / 3) * 5) < 1e-9);
  assert.ok(Math.abs(plan.source.width - (400 / 3) * 5) < 1e-9);
  assert.deepEqual(plan.dest, { x: 0, y: 0, width: 1300, height: 1300 });
  // The rounded clip is the painted stage box in canvas px.
  assert.deepEqual(plan.clip, { x: -40 * 3.25, y: -30 * 3.25, width: 600 * 3.25, height: 600 * 3.25 });
  // Snapping rounds each edge to a device pixel.
  assert.deepEqual(snapRectToDevicePixels({ left: 10.1, top: 0.2, width: 20, height: 10 }, 2), { left: 10, top: 0, width: 20, height: 10 });
  assert.equal(snapRectToDevicePixels({ left: 0, top: 0, width: 0.1, height: 5 }, 2), null);
});

test('overlay geometry: the reset copy is the whole cover at the screen-resolution thumbnail size', async () => {
  const { coverDrawPlan } = await import('../src/lib/cover-overlay.js');
  const box = { left: 40, top: 100, width: 340, height: 340 };
  const plan = coverDrawPlan({ visual: box, scale: 1, naturalWidth: 3000, naturalHeight: 2000, bleed: 1, visible: box, dpr: 3.25 });
  assert.deepEqual(plan.canvas, { width: 1105, height: 1105 });
  // object-fit cover over the 342 px bled box: the full height, the centre of the width.
  const k = 342 / 2000;
  assert.ok(Math.abs(plan.source.y - 1 / k) < 1e-6);
  assert.ok(Math.abs(plan.source.height - 340 / k) < 1e-6);
  assert.ok(Math.abs(plan.source.x - (1500 - 170 / k)) < 1e-6);
});

test('overlay is wanted at any settled scale but the promote-time one at 1', async () => {
  const { coverOverlayWanted } = await import('../src/lib/cover-overlay.js');
  assert.equal(coverOverlayWanted(1, 1), false);
  assert.equal(coverOverlayWanted(3, 1), true);
  assert.equal(coverOverlayWanted(0.5, 1), true);
  assert.equal(coverOverlayWanted(1, 0.5), true);
});

function fakeOverlayDom() {
  const drawn = [];
  const canvases = [];
  const pane = {
    clientLeft: 0, clientTop: 0, scrollLeft: 0, scrollTop: 0,
    children: [],
    appendChild(child) { this.children.push(child); child.parentElement = this; },
    getBoundingClientRect: () => ({ left: 20, top: 50, width: 500, height: 500 }),
  };
  globalThis.document = {
    createElement: () => {
      const canvas = {
        style: {}, width: 300, height: 150, className: '', animations: [],
        setAttribute() {},
        remove() { canvas.removed = true; },
        getContext: () => ({
          save() {}, restore() {}, beginPath() {}, clip() {}, roundRect() {}, rect() {},
          drawImage: (...args) => drawn.push(args),
        }),
        animate: (keyframes, timing) => {
          const animation = { keyframes, timing, currentTime: 0, cancel() { this.cancelled = true; }, onfinish: null };
          canvas.animations.push(animation);
          return animation;
        },
      };
      canvases.push(canvas);
      return canvas;
    },
  };
  // Computed swipe feedback = the inline value (no transitions run in the fake).
  globalThis.getComputedStyle = (element) => ({ borderTopLeftRadius: '18px', overflowX: 'visible', overflowY: 'visible', translate: element?.style?.translate || 'none', scale: element?.style?.scale || 'none' });
  globalThis.devicePixelRatio = 2;
  return { pane, drawn, canvases };
}

function clearOverlayDom() {
  delete globalThis.document;
  delete globalThis.getComputedStyle;
  delete globalThis.devicePixelRatio;
}

test('reset with artwork: the copy canvas glides (drawn at the thumbnail size before paint), the stage never animates', async () => {
  const { coverZoom } = await import('../src/lib/cover-zoom.js');
  const { pane, drawn, canvases } = fakeOverlayDom();
  globalThis.requestAnimationFrame = () => 1;
  globalThis.cancelAnimationFrame = () => {};
  try {
    const { node, listeners } = fakeCover({ left: 100, top: 100, width: 200, height: 200 });
    pane.appendChild(node);
    const image = { complete: true, naturalWidth: 1000, naturalHeight: 1000 };
    node.querySelector = () => image;
    let layoutCommitted = false;
    node.getBoundingClientRect = () => (layoutCommitted
      ? { left: 40, top: 100, width: 200, height: 200 }
      : { left: 0, top: 0, width: 600, height: 600 });
    let stageAnimated = false;
    node.animate = () => { stageAnimated = true; return {}; };
    const action = coverZoom(node, {
      initial: { scale: 3, x: 0, y: 0 },
      onZoom: () => { layoutCommitted = true; },
      settleLayout: () => Promise.resolve(),
    });
    assert.equal(canvases.length, 1, 'one persistent overlay canvas');
    const overlay = canvases[0];
    assert.equal(pane.children.includes(overlay), true);
    listeners.get('dblclick')();
    await Promise.resolve(); await Promise.resolve();
    assert.equal(stageAnimated, false, 'the stage has no compositor transform animation');
    assert.equal(overlay.animations.length, 1);
    // Drawn before it is shown: backing = the final 200x200 box at DPR 2, placed at the box in pane coordinates.
    assert.deepEqual([overlay.width, overlay.height], [400, 400]);
    assert.deepEqual([overlay.style.left, overlay.style.top, overlay.style.width, overlay.style.height], ['20px', '50px', '200px', '200px']);
    assert.equal(drawn.length, 1);
    assert.equal(overlay.style.opacity, '1');
    assert.equal('coverOverlay' in node.dataset, true, 'the stage image is hidden under the copy');
    // The glide starts where the zoomed cover was painted (centre 300 vs 140/200, 3x).
    assert.equal(overlay.animations[0].keyframes[0].transform, 'translate(160.00px, 100.00px) scale(3.0000)');
    assert.equal(overlay.animations[0].timing.duration, 200);
    overlay.animations[0].onfinish();
    // Same task: the copy goes and the stage image comes back.
    assert.equal(overlay.style.opacity, '0');
    assert.equal('coverOverlay' in node.dataset, false);
    assert.deepEqual([overlay.width, overlay.height], [1, 1], 'backing freed while hidden');
    action.destroy();
    assert.equal(overlay.removed, true);
  } finally {
    clearOverlayDom();
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

/**
 * Where the transformed canvas is painted, from its pane position and its inline transform-origin, translate, scale and
 * transform, composed as CSS does: p = O + translate + scale * (transform(q - O)), O = transform-origin.
 */
function paintedCanvasRect(canvas, paneBox) {
  const left = paneBox.left + parseFloat(canvas.style.left);
  const top = paneBox.top + parseFloat(canvas.style.top);
  const width = parseFloat(canvas.style.width);
  const height = parseFloat(canvas.style.height);
  const origin = /(-?[\d.]+)px (-?[\d.]+)px/.exec(canvas.style.transformOrigin || '');
  const [ox, oy] = origin ? origin.slice(1).map(Number) : [width / 2, height / 2];
  const match = /translate\((-?[\d.]+)px, (-?[\d.]+)px\) scale\((-?[\d.]+), (-?[\d.]+)\)/.exec(canvas.style.transform || '');
  const [x, y, kx, ky] = match ? match.slice(1).map(Number) : [0, 0, 1, 1];
  const feedback = parseSwipeFeedback(canvas.style.translate, canvas.style.scale);
  const map = (qx, qy) => ({
    x: left + ox + feedback.x + feedback.scale * (x + kx * (qx - ox)),
    y: top + oy + feedback.y + feedback.scale * (y + ky * (qy - oy)),
  });
  const a = map(0, 0);
  const b = map(width, height);
  return { left: a.x, top: a.y, width: b.x - a.x, height: b.y - a.y };
}

/** The client rect a content region drawn for the stage at `drawnVisual` must occupy when the stage is at `visual`. */
function mappedRect(rect, drawnVisual, visual) {
  const s = visual.width / drawnVisual.width;
  return { left: visual.left + (rect.left - drawnVisual.left) * s, top: visual.top + (rect.top - drawnVisual.top) * s, width: rect.width * s, height: rect.height * s };
}

function assertRectClose(actual, expected, message, epsilon = 0.02) {
  for (const key of ['left', 'top', 'width', 'height']) {
    assert.ok(Math.abs(actual[key] - expected[key]) < epsilon, `${message}: ${key} ${actual[key]} vs ${expected[key]}`);
  }
}

test('overlay follow: a pan moves the drawn canvas by the same delta, no scale', async () => {
  const { overlayFollowTransform, overlayTransformCss } = await import('../src/lib/cover-overlay.js');
  const drawnVisual = { left: 0, top: 0, width: 600, height: 600 };
  const drawnCanvas = { left: 100, top: 0, width: 400, height: 400 };
  const panned = { ...drawnVisual, left: 30, top: -20 };
  assert.deepEqual(overlayFollowTransform(drawnVisual, drawnCanvas, panned, { x: 0, y: 0 }), { x: 30, y: -20, scaleX: 1, scaleY: 1 });
  // The pane (and the canvas with it) moved by -12 px in the same frame: the transform absorbs it.
  assert.deepEqual(overlayFollowTransform(drawnVisual, drawnCanvas, panned, { x: -12, y: 0 }), { x: 42, y: -20, scaleX: 1, scaleY: 1 });
  assert.equal(overlayTransformCss(overlayFollowTransform(drawnVisual, drawnCanvas, drawnVisual, { x: 0, y: 0 })), '');
  assert.equal(overlayTransformCss({ x: 30, y: -20, scaleX: 1, scaleY: 1 }), 'translate(30.00px, -20.00px) scale(1.00000, 1.00000)');
});

test('overlay follow: zooming out around an anchor keeps the anchor content point under it, also when the pane moves', async () => {
  const { overlayFollowTransform } = await import('../src/lib/cover-overlay.js');
  // Drawn at 3x: stage painted at (0, 0) 600x600, canvas covering (100, 50) 400x400. Zoom out to 2x at (250, 250).
  const drawnVisual = { left: 0, top: 0, width: 600, height: 600 };
  const drawnCanvas = { left: 100, top: 50, width: 400, height: 400 };
  const anchor = { x: 250, y: 250 };
  const k = 2 / 3;
  const visual = { left: anchor.x - (anchor.x - drawnVisual.left) * k, top: anchor.y - (anchor.y - drawnVisual.top) * k, width: 400, height: 400 };
  for (const shift of [{ x: 0, y: 0 }, { x: -40, y: 7 }]) {
    const t = overlayFollowTransform(drawnVisual, drawnCanvas, visual, shift);
    assert.ok(Math.abs(t.scaleX - k) < 1e-12 && Math.abs(t.scaleY - k) < 1e-12);
    // The canvas point that was at the anchor (client, at draw time) is at the anchor again.
    const cx = drawnCanvas.left + drawnCanvas.width / 2 + shift.x;
    const cy = drawnCanvas.top + drawnCanvas.height / 2 + shift.y;
    const px = cx + t.x + t.scaleX * (anchor.x + shift.x - cx);
    const py = cy + t.y + t.scaleY * (anchor.y + shift.y - cy);
    assert.ok(Math.abs(px - anchor.x) < 1e-9 && Math.abs(py - anchor.y) < 1e-9, JSON.stringify({ shift, px, py }));
  }
});

test('overlay draw region: visible part plus an even margin, capped at 2x the pixels and clamped to the cover', async () => {
  const { overlayDrawRegion, COVER_OVERLAY_MARGIN_PIXEL_RATIO } = await import('../src/lib/cover-overlay.js');
  assert.equal(COVER_OVERLAY_MARGIN_PIXEL_RATIO, 2);
  const cover = { left: -400, top: -400, width: 1200, height: 1200 };
  // Centred: an even margin on every side until the area is twice the visible area.
  const centred = overlayDrawRegion({ visible: { left: 0, top: 0, width: 400, height: 400 }, cover, dpr: 3 });
  const side = 400 * Math.SQRT2;
  assertRectClose(centred, { left: -(side - 400) / 2, top: -(side - 400) / 2, width: side, height: side }, 'centred', 1e-6);
  // At the cover's left edge: that side stops, the others keep growing to the same pixel budget.
  const edge = overlayDrawRegion({ visible: { left: -400, top: 0, width: 400, height: 400 }, cover, dpr: 3 });
  assert.equal(edge.left, -400);
  assert.ok(Math.abs(edge.width * edge.height - 2 * 400 * 400) < 1e-3);
  assert.ok(Math.abs((edge.width - 400) - (edge.height - 400) / 2) < 1e-6, 'right margin = top margin = bottom margin');
  // Small cover: the whole cover is under the budget.
  const small = { left: -50, top: -50, width: 500, height: 500 };
  assert.deepEqual(overlayDrawRegion({ visible: { left: 0, top: 0, width: 400, height: 400 }, cover: small, dpr: 2 }), small);
  // Nothing outside the visible part: the visible part itself.
  const whole = { left: 10, top: 10, width: 300, height: 300 };
  assert.deepEqual(overlayDrawRegion({ visible: whole, cover: whole, dpr: 2 }), whole);
  // A side never exceeds the max canvas dimension (4 x 125 = 500 device px), unless the visible part already does.
  const capped = overlayDrawRegion({ visible: { left: 0, top: 0, width: 100, height: 100 }, cover, dpr: 4, maxDimension: 500 });
  assert.ok(capped.width <= 125 + 1e-6 && capped.height <= 125 + 1e-6 && capped.width > 100);
  const huge = overlayDrawRegion({ visible: { left: 0, top: 0, width: 400, height: 400 }, cover, dpr: 4, maxDimension: 500 });
  assertRectClose(huge, { left: 0, top: 0, width: 400, height: 400 }, 'visible part over the cap', 1e-6);
});

/** The painted rect of a 200x200 stage at layout centre `layout` with inline `style` (transform, then the swipe feedback
 *  translate / scale around the layout centre, as CSS composes them). */
function stagePaintedRect(layout, style) {
  const match = /translate\((-?[\d.]+)px, (-?[\d.]+)px\) scale\(([\d.]+)\)/.exec(style.transform || '');
  const [x, y, z] = match ? match.slice(1).map(Number) : [0, 0, 1];
  const feedback = parseSwipeFeedback(style.translate, style.scale);
  const s = feedback.scale;
  const cx = layout.x + feedback.x + s * x;
  const cy = layout.y + feedback.y + s * y;
  return { left: cx - 100 * z * s, top: cy - 100 * z * s, width: 200 * z * s, height: 200 * z * s };
}

/** A 200x200 cover whose layout centre is (layout.x, layout.y) and whose painted rect follows its inline transform. */
function liveCover(layout) {
  const { node, listeners } = fakeCover({ left: 0, top: 0, width: 200, height: 200 });
  node.getBoundingClientRect = () => stagePaintedRect(layout, node.style);
  node.getAttribute = (name) => (name === 'style' ? JSON.stringify(node.style) : null);
  node.querySelector = () => ({ complete: true, naturalWidth: 1000, naturalHeight: 1000 });
  return { node, listeners };
}

test('input never hides a shown overlay: pointer down, zoom out and zoom in follow the stage in the same task; redrawn 50 ms after', async () => {
  const { coverZoom, COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  assert.equal(COVER_LAYER_IDLE_MS, 50);
  const { pane, drawn, canvases } = fakeOverlayDom();
  const frames = [];
  globalThis.requestAnimationFrame = (fn) => { frames.push(fn); return frames.length; };
  globalThis.cancelAnimationFrame = () => {};
  const layout = { x: 300, y: 300 };
  let paneBox = { left: 20, top: 50, width: 500, height: 500 };
  pane.getBoundingClientRect = () => ({ ...paneBox });
  let action;
  try {
    const { node, listeners } = liveCover(layout);
    pane.appendChild(node);
    let squeeze = false;
    action = coverZoom(node, {
      initial: { scale: 3, x: 0, y: 0 },
      onZoom: () => { squeeze = true; },
      // The owner commits the squeeze the scale change caused: the cover's layout and its pane move left by 30 px.
      settleLayout: () => { if (squeeze) { squeeze = false; layout.x -= 30; paneBox = { ...paneBox, left: paneBox.left - 30 }; } return Promise.resolve(); },
    });
    const overlay = canvases[0];
    await new Promise((resolve) => setTimeout(resolve, COVER_LAYER_IDLE_MS + 30));
    assert.equal(drawn.length, 1);
    assert.equal(overlay.style.opacity, '1');
    assert.equal(overlay.style.transform, '');
    const drawnVisual = node.getBoundingClientRect();
    const drawnCanvas = paintedCanvasRect(overlay, paneBox);
    const touch = (id, x, y) => ({ pointerId: id, clientX: x, clientY: y, pointerType: 'touch', type: 'pointerup', timeStamp: 0, cancelable: true, preventDefault() {} });
    listeners.get('pointerdown')(touch(1, 350, 320));
    assert.equal(overlay.style.opacity, '1', 'pointer down keeps the overlay');
    listeners.get('pointerup')(touch(1, 350, 320));
    const wheel = (deltaY) => listeners.get('wheel')({ deltaY, deltaMode: 0, clientX: 350, clientY: 320, preventDefault() {} });
    for (const [deltaY, label] of [[120, 'zoom out'], [-200, 'zoom in']]) {
      wheel(deltaY);
      assert.equal(overlay.style.opacity, '1', `${label}: not hidden in the handler`);
      frames.shift()();
      await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
      assert.equal(frames.length, 0, `${label}: no later frame`);
      assert.equal(overlay.style.opacity, '1', `${label}: still shown`);
      assert.notEqual(overlay.style.transform, '');
      // After the squeeze and the stage's re-pin (same task): the canvas content sits exactly on the stage.
      assertRectClose(paintedCanvasRect(overlay, paneBox), mappedRect(drawnCanvas, drawnVisual, node.getBoundingClientRect()), label);
    }
    assert.equal(drawn.length, 1, 'no redraw while the input runs');
    await new Promise((resolve) => setTimeout(resolve, COVER_LAYER_IDLE_MS + 30));
    assert.equal(drawn.length, 2, 'redrawn sharp at the new scale once quiet');
    assert.equal(overlay.style.transform, '');
    assert.equal(overlay.style.opacity, '1');
  } finally {
    action?.destroy();
    clearOverlayDom();
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

test('without a drawn overlay (first zoom from 1x) the stage shows until the cover settles; at the promote-time scale none is drawn', async () => {
  const { coverZoom, COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const { pane, drawn, canvases } = fakeOverlayDom();
  const frames = [];
  globalThis.requestAnimationFrame = (fn) => { frames.push(fn); return frames.length; };
  globalThis.cancelAnimationFrame = () => {};
  let action;
  try {
    const { node, listeners } = liveCover({ x: 300, y: 300 });
    pane.appendChild(node);
    action = coverZoom(node, { settleLayout: () => Promise.resolve() });
    const overlay = canvases[0];
    await new Promise((resolve) => setTimeout(resolve, COVER_LAYER_IDLE_MS + 30));
    assert.equal(drawn.length, 0, 'scale 1 at the promote-time scale: no overlay');
    listeners.get('wheel')({ deltaY: -200, deltaMode: 0, clientX: 300, clientY: 300, preventDefault() {} });
    frames.shift()();
    await Promise.resolve(); await Promise.resolve();
    assert.notEqual(overlay.style.opacity, '1', 'nothing drawn yet: the stage shows');
    await new Promise((resolve) => setTimeout(resolve, COVER_LAYER_IDLE_MS + 30));
    assert.equal(drawn.length, 1);
    assert.equal(overlay.style.opacity, '1');
    // Back at the promote-time scale (the owner drops the zoom): no overlay.
    action.update({ settleLayout: () => Promise.resolve(), syncScale: 1 });
    await new Promise((resolve) => setTimeout(resolve, COVER_LAYER_IDLE_MS + 30));
    assert.equal(overlay.style.opacity, '0');
  } finally {
    action?.destroy();
    clearOverlayDom();
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

test('the change observer ignores the action\'s own transform writes, mirrors swipe feedback, hides the overlay on a foreign change', async () => {
  const { coverZoom, COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const { pane, canvases } = fakeOverlayDom();
  const observers = [];
  globalThis.MutationObserver = class { constructor(callback) { observers.push(callback); } observe() {} disconnect() {} };
  globalThis.requestAnimationFrame = (fn) => { fn(); return 0; };
  globalThis.cancelAnimationFrame = () => {};
  let action;
  try {
    const { node, listeners } = liveCover({ x: 300, y: 300 });
    pane.appendChild(node);
    action = coverZoom(node, { initial: { scale: 3, x: 0, y: 0 }, settleLayout: () => Promise.resolve() });
    const overlay = canvases[0];
    await new Promise((resolve) => setTimeout(resolve, COVER_LAYER_IDLE_MS + 30));
    assert.equal(overlay.style.opacity, '1');
    listeners.get('wheel')({ deltaY: 100, deltaMode: 0, clientX: 300, clientY: 300, preventDefault() {} });
    observers[0]([{ target: node, attributeName: 'style' }]);
    assert.equal(overlay.style.opacity, '1', 'own transform write');
    node.style.translate = '12px 0px'; // swipe feedback
    observers[0]([{ target: node, attributeName: 'style' }]);
    assert.equal(overlay.style.opacity, '1', 'swipe feedback is mirrored, not a foreign change');
    assert.equal(overlay.style.translate, '12px 0px');
    node.style.width = '320px'; // the owner's cover frame
    observers[0]([{ target: node, attributeName: 'style' }]);
    assert.equal(overlay.style.opacity, '0', 'foreign style write');
  } finally {
    action?.destroy();
    clearOverlayDom();
    delete globalThis.MutationObserver;
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

test('swipe feedback geometry: parsed from computed values, removed from a read rect, and the follow transform is origin-independent', async () => {
  const { parseSwipeFeedback: parse, withoutSwipeFeedback, overlayFollowTransform, overlayOrigin } = await import('../src/lib/cover-overlay.js');
  assert.deepEqual(parse('none', 'none'), { x: 0, y: 0, scale: 1 });
  assert.deepEqual(parse('', undefined), { x: 0, y: 0, scale: 1 });
  assert.deepEqual(parse('12px', '0.97'), { x: 12, y: 0, scale: 0.97 });
  assert.deepEqual(parse('0px -8.5px', '0.98 0.98'), { x: 0, y: -8.5, scale: 0.98 });
  // A 3x zoomed stage panned by (40, -20) at layout centre (300, 300), with feedback translate (15, 0) scale 0.97.
  const layout = { x: 300, y: 300 };
  const style = { transform: 'translate(40.00px, -20.00px) scale(3.0000)', translate: '15px 0px', scale: '0.97' };
  const fed = stagePaintedRect(layout, style);
  const plain = stagePaintedRect(layout, { transform: style.transform });
  assertRectClose(withoutSwipeFeedback(fed, parse(style.translate, style.scale), { x: 40, y: -20 }), plain, 'feedback removed', 1e-9);
  assert.equal(withoutSwipeFeedback(plain, parse('none', 'none'), { x: 40, y: -20 }), plain);
  // The same follow map with any transform-origin: content point p lands on the same client point.
  const drawnVisual = { left: 0, top: 0, width: 600, height: 600 };
  const drawnCanvas = { left: 100, top: 50, width: 400, height: 400 };
  const visual = { left: 30, top: 10, width: 450, height: 450 };
  const shift = { x: -10, y: 4 };
  const centred = overlayFollowTransform(drawnVisual, drawnCanvas, visual, shift);
  const origin = overlayOrigin({ x: 250, y: 260 }, { ...drawnCanvas, left: drawnCanvas.left + shift.x, top: drawnCanvas.top + shift.y });
  assert.deepEqual(origin, { x: 160, y: 206 });
  const pivoted = overlayFollowTransform(drawnVisual, drawnCanvas, visual, shift, origin);
  for (const q of [{ x: 0, y: 0 }, { x: 400, y: 400 }, { x: 37, y: 311 }]) {
    const k = { x: 200, y: 200 };
    const a = { x: k.x + centred.x + centred.scaleX * (q.x - k.x), y: k.y + centred.y + centred.scaleY * (q.y - k.y) };
    const b = { x: origin.x + pivoted.x + pivoted.scaleX * (q.x - origin.x), y: origin.y + pivoted.y + pivoted.scaleY * (q.y - origin.y) };
    assert.ok(Math.abs(a.x - b.x) < 1e-9 && Math.abs(a.y - b.y) < 1e-9, JSON.stringify({ q, a, b }));
  }
});

/**
 * A 200x200 stage at layout centre `layout` for both actions (coverZoom and swipeDismiss): several listeners per type,
 * dispatchEvent, a style and a class list whose writes queue one MutationObserver record each (with its old value, also
 * for a write that changes nothing) and one batch per microtask like the browser, and a painted rect that includes the
 * swipe feedback.
 */
function sharedStage(layout) {
  const listeners = new Map();
  const observers = [];
  let pending = [];
  let node;
  const flush = () => { const records = pending; pending = []; for (const callback of observers) callback(records); };
  const record = (attributeName) => {
    if (pending.length === 0) queueMicrotask(flush);
    pending.push({ target: node, attributeName, oldValue: node.getAttribute(attributeName) });
  };
  const styleValues = {};
  const style = new Proxy(styleValues, {
    set(target, key, value) {
      record('style');
      target[key] = value;
      return true;
    },
  });
  const classes = new Set(['cover-stage', 'has-artwork']);
  node = {
    style, dataset: {}, offsetWidth: 200, offsetHeight: 200,
    classList: {
      add(name) { record('class'); classes.add(name); },
      remove(name) { record('class'); classes.delete(name); },
      contains: (name) => classes.has(name),
    },
    getAttribute: (name) => {
      if (name === 'class') return [...classes].join(' ');
      if (name === 'style') return Object.entries(styleValues).filter(([, value]) => value != null && value !== '').map(([key, value]) => `${key}: ${value};`).join(' ') || null;
      return null;
    },
    addEventListener: (type, handler) => { if (!listeners.has(type)) listeners.set(type, []); listeners.get(type).push(handler); },
    removeEventListener: (type, handler) => { const list = listeners.get(type) ?? []; const i = list.indexOf(handler); if (i >= 0) list.splice(i, 1); },
    dispatchEvent(event) { for (const handler of [...(listeners.get(event.type) ?? [])]) handler(event); return true; },
    setPointerCapture() {}, releasePointerCapture() {}, hasPointerCapture: () => true,
    querySelector: () => ({ complete: true, naturalWidth: 1000, naturalHeight: 1000 }),
    getBoundingClientRect: () => stagePaintedRect(layout, style),
  };
  const fire = (type, event) => { for (const handler of [...(listeners.get(type) ?? [])]) handler({ type, ...event }); };
  return { node, observers, fire };
}

function installSharedStageGlobals() {
  const had = { Element: 'Element' in globalThis, PointerEvent: 'PointerEvent' in globalThis };
  if (!had.Element) globalThis.Element = class {};
  if (!had.PointerEvent) globalThis.PointerEvent = class { constructor(type, init) { this.type = type; Object.assign(this, init); } };
  const observers = [];
  globalThis.MutationObserver = class { constructor(callback) { observers.push(callback); } observe() {} disconnect() {} };
  return {
    observers,
    restore() {
      if (!had.Element) delete globalThis.Element;
      if (!had.PointerEvent) delete globalThis.PointerEvent;
      delete globalThis.MutationObserver;
    },
  };
}

const settle = async () => { for (let i = 0; i < 4; i += 1) await Promise.resolve(); };
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

test('swipe feedback on a zoomed stage is mirrored onto the overlay in the same frame, pivoting on the stage centre, never hidden', async () => {
  const { coverZoom, COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const { swipeDismiss, SWIPE_SETTLE_MS } = await import('../src/lib/swipe-dismiss.js');
  const { pane, drawn, canvases } = fakeOverlayDom();
  const env = installSharedStageGlobals();
  globalThis.requestAnimationFrame = () => 1;
  globalThis.cancelAnimationFrame = () => {};
  const layout = { x: 300, y: 300 };
  let zoom;
  let swipe;
  try {
    const stage = sharedStage(layout);
    // The MutationObserver the action created gets the stage's batches.
    const { node, fire } = stage;
    pane.appendChild(node);
    zoom = coverZoom(node, { initial: { scale: 3, x: 40, y: -20 }, settleLayout: () => Promise.resolve() });
    stage.observers.push(...env.observers);
    let skipped = 0;
    swipe = swipeDismiss(node, { follow: false, damped: true, actions: { left: () => { skipped += 1; }, right: () => {}, down: () => {} } });
    const overlay = canvases[0];
    await wait(COVER_LAYER_IDLE_MS + 30);
    assert.equal(drawn.length, 1);
    assert.equal(overlay.style.opacity, '1');
    const paneBox = pane.getBoundingClientRect();
    const drawnVisual = node.getBoundingClientRect();
    const drawnCanvas = paintedCanvasRect(overlay, paneBox);
    // transform-origin = the stage's layout centre (300, 300) in the canvas's own box.
    const origin = /(-?[\d.]+)px (-?[\d.]+)px/.exec(overlay.style.transformOrigin).slice(1).map(Number);
    const canvasLeft = paneBox.left + parseFloat(overlay.style.left);
    const canvasTop = paneBox.top + parseFloat(overlay.style.top);
    assert.ok(Math.abs(canvasLeft + origin[0] - layout.x) < 0.01 && Math.abs(canvasTop + origin[1] - layout.y) < 0.01, 'pivot on the stage centre');
    const touch = (x, y, extra = {}) => ({ pointerId: 1, isPrimary: true, pointerType: 'touch', clientX: x, clientY: y, timeStamp: 0, target: null, cancelable: true, preventDefault() {}, ...extra });
    // One finger drags left: the damped feedback is written on the stage...
    fire('pointerdown', touch(300, 300));
    fire('pointermove', touch(270, 302, { timeStamp: 16 }));
    assert.notEqual(node.style.translate, '');
    assert.notEqual(node.style.scale, '');
    await settle(); // ...and mirrored in the observer batch (before paint)
    assert.equal(overlay.style.opacity, node.style.opacity, 'shown, at the stage feedback opacity');
    assert.equal(overlay.style.translate, node.style.translate);
    assert.equal(overlay.style.scale, node.style.scale);
    assert.equal(overlay.style.transition, node.style.transition);
    assertRectClose(paintedCanvasRect(overlay, paneBox), mappedRect(drawnCanvas, drawnVisual, node.getBoundingClientRect()), 'mirrored feedback lines up');
    // Released short of the skip distance: the feedback settles back with a transition.
    fire('pointerup', touch(285, 302, { timeStamp: 40 }));
    await settle();
    assert.equal(skipped, 0);
    assert.equal(overlay.style.opacity, '1', 'the settle transition is mirrored, not hidden');
    assert.equal(overlay.style.transition, node.style.transition);
    assert.match(overlay.style.transition, /translate/);
    assert.equal(overlay.style.translate, '');
    // The settled redraw 50 ms after the fingers are up, then the swipe's delayed `transition = ''` write.
    await wait(COVER_LAYER_IDLE_MS + 20);
    assert.equal(drawn.length, 2, 'redrawn once quiet');
    assert.equal(overlay.style.opacity, '1');
    await wait(SWIPE_SETTLE_MS);
    await settle();
    assert.equal(node.style.transition, '');
    assert.equal(overlay.style.opacity, '1', "the delayed transition = '' write does not hide it");
    assert.equal(overlay.style.transition, '');
    assert.equal(drawn.length, 2);
    // A single-finger swipe far enough left still skips (far from the last release: not a double tap); the overlay
    // moved with the feedback and snaps back with it.
    fire('pointerdown', touch(300, 300, { timeStamp: 2000 }));
    fire('pointermove', touch(260, 300, { timeStamp: 2016 }));
    await settle();
    assert.equal(overlay.style.translate, node.style.translate);
    fire('pointerup', touch(150, 300, { timeStamp: 2040 }));
    await settle();
    assert.equal(skipped, 1, 'single-finger swipe still skips');
    assert.equal(overlay.style.opacity, '1');
    assert.equal(overlay.style.translate, '');
    // A foreign style write still hides the overlay.
    node.style.width = '320px';
    await settle();
    assert.equal(overlay.style.opacity, '0');
  } finally {
    swipe?.destroy();
    zoom?.destroy();
    env.restore();
    clearOverlayDom();
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

test('a settled redraw while the stage still carries swipe feedback is drawn without it and keeps the mirrored feedback on top', async () => {
  const { coverZoom, COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const { pane, drawn, canvases } = fakeOverlayDom();
  const env = installSharedStageGlobals();
  globalThis.requestAnimationFrame = () => 1;
  globalThis.cancelAnimationFrame = () => {};
  const layout = { x: 300, y: 300 };
  let zoom;
  try {
    const stage = sharedStage(layout);
    const { node, fire } = stage;
    pane.appendChild(node);
    zoom = coverZoom(node, { initial: { scale: 2.5, x: -30, y: 10 }, settleLayout: () => Promise.resolve() });
    stage.observers.push(...env.observers);
    const overlay = canvases[0];
    await wait(COVER_LAYER_IDLE_MS + 30);
    assert.equal(drawn.length, 1);
    const plainVisual = node.getBoundingClientRect();
    // A finger is down (holds the redraw) and feedback is on the stage when it lifts.
    fire('pointerdown', { pointerId: 1, pointerType: 'touch', clientX: 300, clientY: 300 });
    node.style.transition = 'none';
    node.style.translate = '-18px 0px';
    node.style.scale = '0.98';
    node.style.opacity = '0.9';
    await settle();
    fire('pointerup', { pointerId: 1, pointerType: 'touch', clientX: 300, clientY: 300, timeStamp: 0 });
    await wait(COVER_LAYER_IDLE_MS + 20);
    assert.equal(drawn.length, 2, 'redrawn although the feedback is not identity');
    assert.equal(overlay.style.opacity, '0.9');
    assert.equal(overlay.style.translate, '-18px 0px');
    // Drawn for the stage without its feedback: the canvas content is where the stage's (fed) content is.
    const drawnCanvas = paintedCanvasRect({ style: { ...overlay.style, translate: '', scale: '' } }, pane.getBoundingClientRect());
    assertRectClose(paintedCanvasRect(overlay, pane.getBoundingClientRect()), mappedRect(drawnCanvas, plainVisual, node.getBoundingClientRect()), 'redraw lines up under the feedback');
  } finally {
    zoom?.destroy();
    env.restore();
    clearOverlayDom();
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

test('pinch on a zoomed cover after the first finger moved (swipe feedback, synthetic pointercancel) keeps the overlay shown and aligned', async () => {
  const { coverZoom, COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const { swipeDismiss, SWIPE_SETTLE_MS } = await import('../src/lib/swipe-dismiss.js');
  const { pane, drawn, canvases } = fakeOverlayDom();
  const env = installSharedStageGlobals();
  const frames = [];
  globalThis.requestAnimationFrame = (fn) => { frames.push(fn); return frames.length; };
  globalThis.cancelAnimationFrame = () => {};
  const layout = { x: 300, y: 300 };
  let zoom;
  let swipe;
  try {
    const stage = sharedStage(layout);
    const { node, fire } = stage;
    pane.appendChild(node);
    zoom = coverZoom(node, { initial: { scale: 3, x: 0, y: 0 }, settleLayout: () => Promise.resolve() });
    stage.observers.push(...env.observers);
    const actions = [];
    swipe = swipeDismiss(node, { follow: false, damped: true, actions: { left: () => actions.push('left'), right: () => actions.push('right'), down: () => actions.push('down') } });
    const overlay = canvases[0];
    await wait(COVER_LAYER_IDLE_MS + 30);
    assert.equal(drawn.length, 1);
    const paneBox = pane.getBoundingClientRect();
    const drawnVisual = node.getBoundingClientRect();
    const drawnCanvas = paintedCanvasRect(overlay, paneBox);
    const ptr = (id, x, y, extra = {}) => ({ pointerId: id, isPrimary: id === 1, pointerType: 'touch', clientX: x, clientY: y, timeStamp: 0, target: null, cancelable: true, preventDefault() {}, ...extra });
    fire('pointerdown', ptr(1, 260, 300));
    fire('pointermove', ptr(1, 275, 301, { timeStamp: 10 })); // the first finger moves before the second lands
    await settle();
    assert.notEqual(node.style.translate, '', 'swipe feedback on the stage');
    assert.equal(overlay.style.opacity, node.style.opacity);
    fire('pointerdown', ptr(2, 360, 300)); // second finger: coverZoom cancels the swipe (synthetic pointercancel)
    await settle();
    assert.equal(node.style.translate, '', 'the swipe settled back');
    assert.equal(overlay.style.opacity, '1', 'the settle write is mirrored, not hidden');
    assert.equal(overlay.style.transition, node.style.transition);
    for (let i = 1; i <= 4; i += 1) {
      fire('pointermove', ptr(1, 275 - i * 5, 301));
      fire('pointermove', ptr(2, 360 + i * 5, 300));
    }
    assert.equal(frames.length, 1);
    frames.shift()();
    await settle();
    assert.equal(overlay.style.opacity, '1', 'shown during the pinch');
    assert.equal(overlay.style.transition, node.style.transition, 'the zoom frame\'s transition write is mirrored too');
    assertRectClose(paintedCanvasRect(overlay, paneBox), mappedRect(drawnCanvas, drawnVisual, node.getBoundingClientRect()), 'aligned with the pinched stage');
    fire('pointerup', ptr(1, 255, 301));
    fire('pointerup', ptr(2, 380, 300));
    await settle();
    assert.deepEqual(actions, [], 'the pinch never skips or closes');
    await wait(COVER_LAYER_IDLE_MS + 20);
    assert.equal(drawn.length, 2, 'redrawn sharp once quiet');
    await wait(SWIPE_SETTLE_MS);
    await settle();
    assert.equal(overlay.style.opacity, '1', 'still shown after the swipe\'s delayed transition write');
    assert.equal(drawn.length, 2);
  } finally {
    swipe?.destroy();
    zoom?.destroy();
    env.restore();
    clearOverlayDom();
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

test('mutation records that leave the attribute at its old value (no-op class or style writes) do not hide the overlay', async () => {
  const { coverZoom, COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const { pane, canvases } = fakeOverlayDom();
  const env = installSharedStageGlobals();
  globalThis.requestAnimationFrame = () => 1;
  globalThis.cancelAnimationFrame = () => {};
  let zoom;
  try {
    const stage = sharedStage({ x: 300, y: 300 });
    const { node } = stage;
    pane.appendChild(node);
    zoom = coverZoom(node, { initial: { scale: 3, x: 0, y: 0 }, settleLayout: () => Promise.resolve() });
    stage.observers.push(...env.observers);
    const overlay = canvases[0];
    await wait(COVER_LAYER_IDLE_MS + 30);
    assert.equal(overlay.style.opacity, '1');
    // swipe-dismiss's up() on a stage that never had the class: a class record with no change.
    node.classList.remove('is-swiping');
    await settle();
    assert.equal(overlay.style.opacity, '1', 'no-op class record');
    // Added and removed in one batch: no net change.
    node.classList.add('is-swiping');
    node.classList.remove('is-swiping');
    await settle();
    assert.equal(overlay.style.opacity, '1', 'no net change in the batch');
    // A real class change still hides it.
    node.classList.add('is-swiping');
    await settle();
    assert.equal(overlay.style.opacity, '0', 'real class change');
    // ...and so does a real style change after it is drawn again.
    await wait(COVER_LAYER_IDLE_MS + 30);
    assert.equal(overlay.style.opacity, '1');
    node.style.width = '320px';
    await settle();
    assert.equal(overlay.style.opacity, '0', 'real style change');
    // The owner writes the same (non-own) value again: no change.
    await wait(COVER_LAYER_IDLE_MS + 30);
    assert.equal(overlay.style.opacity, '1');
    node.style.width = '320px';
    await settle();
    assert.equal(overlay.style.opacity, '1', 'no-op style record');
  } finally {
    zoom?.destroy();
    env.restore();
    clearOverlayDom();
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

test('source shape: the cover layer structure never changes at a gesture or animation end', async () => {
  const { readFileSync } = await import('node:fs');
  const css = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  // The stage is promoted and stacked for its whole life.
  assert.match(css, /\.now-playing-view \.cover-stage \{\s*z-index: 1;\s*will-change: transform;\s*\}/);
  // No rule keyed on a cover attribute touches a compositing or stacking property.
  const attributeRules = [...css.matchAll(/\.cover-stage\[data-cover-[a-z-]+\][^{]*\{[^}]*\}/g)].map((match) => match[0]);
  assert.ok(attributeRules.length >= 2);
  for (const rule of attributeRules) {
    assert.doesNotMatch(rule.slice(rule.indexOf('{')), /will-change|z-index|contain|isolation|position|opacity|filter|transform|mix-blend/, rule);
  }
  assert.doesNotMatch(css, /data-cover-gesture|\[data-cover-resetting\]/);
  // The overlay canvas is persistent and promoted; shown and hidden by opacity only.
  assert.match(css, /\.cover-stage-overlay \{[^}]*will-change: transform;[^}]*\}/);
  const source = readFileSync(new URL('../src/lib/cover-zoom.js', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /coverGesture|style\.willChange|style\.zIndex|style\.contain|style\.isolation|style\.position/);
  const overlay = source.slice(source.indexOf('function createOverlayCanvas'));
  assert.doesNotMatch(overlay, /style\.display|style\.visibility|willChange/);
  // The stage itself is only animated without a drawable image (fallback art).
  const glide = source.slice(source.indexOf('  function startGlide'), source.indexOf('  function beginGlide'));
  assert.match(glide, /drawOverlay\(to, 1, to, true\) \? overlay\.element : node/);
});
