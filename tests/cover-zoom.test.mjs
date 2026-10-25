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
  globalThis.getComputedStyle = () => ({ borderTopLeftRadius: '18px', overflowX: 'visible', overflowY: 'visible' });
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

test('a settled zoom draws only the visible part into the overlay; the next input hides it at once', async () => {
  const { coverZoom, COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const { pane, drawn, canvases } = fakeOverlayDom();
  const frames = [];
  globalThis.requestAnimationFrame = (fn) => { frames.push(fn); return frames.length; };
  globalThis.cancelAnimationFrame = () => {};
  try {
    const { node, listeners } = fakeCover({ left: 0, top: 0, width: 600, height: 600 });
    pane.appendChild(node);
    node.querySelector = () => ({ complete: true, naturalWidth: 1000, naturalHeight: 1000 });
    const action = coverZoom(node, { initial: { scale: 3, x: 0, y: 0 } });
    const overlay = canvases[0];
    assert.equal(overlay.style.opacity, undefined, 'nothing is drawn before the cover has been quiet');
    await new Promise((resolve) => setTimeout(resolve, COVER_LAYER_IDLE_MS + 30));
    assert.equal(drawn.length, 1);
    assert.equal(overlay.style.opacity, '1');
    assert.equal('coverOverlay' in node.dataset, false, 'the stage image stays painted (ready) under the sharp view');
    // A wheel step: the overlay is hidden in the handler, before the transformed stage moves.
    listeners.get('wheel')({ deltaY: -50, deltaMode: 0, clientX: 300, clientY: 300, preventDefault() {} });
    assert.equal(overlay.style.opacity, '0');
    assert.equal('coverOverlay' in node.dataset, false);
    action.destroy();
  } finally {
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
