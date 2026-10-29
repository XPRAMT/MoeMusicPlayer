import assert from 'node:assert/strict';
import test from 'node:test';
import { parseSwipeFeedback } from '../src/lib/cover-swipe-feedback.js';
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

test('only an enlarged cover is pannable by mouse; shrunk and rest keep the swipe', async () => {
  const { isCoverPannable, COVER_ZOOM_REST } = await import('../src/lib/cover-zoom.js');
  assert.equal(isCoverPannable(COVER_ZOOM_MIN), false);
  assert.equal(isCoverPannable(0.8), false);
  assert.equal(isCoverPannable(COVER_ZOOM_REST), false);
  assert.equal(isCoverPannable(1.01), true);
  assert.equal(isCoverPannable(COVER_ZOOM_MAX), true);
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
    assert.equal('coverPannable' in node.dataset, true);
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
    assert.equal('coverPannable' in node.dataset, false);
    assert.equal('coverResetting' in node.dataset, true);
    assert.equal(animations.length, 0);
    await Promise.resolve(); await Promise.resolve();
    // Same task, before any frame: the glide starts from the painted box (centre 200, size 200 -> layout centre 140).
    assert.equal(frames.length, 0, 'nothing waits for a later frame');
    assert.equal(animations.length, 1);
    assert.equal(animations[0].keyframes[0].transform, 'translate(60.00px, 0.00px) scale(1.0000)');
    assert.equal(animations[0].keyframes[1].transform, 'translate(0px, 0px) scale(1)');
    assert.equal(animations[0].timing.duration, 200);
    assert.equal(animations[0].timing.easing, 'ease');
    assert.equal(animations[0].timing.fill, 'none');
    animations[0].onfinish();
    assert.equal(node.style.transform, '');
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
  assert.match(glide, /restoreGlide\(flipTransform\(from, to\)\)/);
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

test('restoreGlide: keyframes run from the inverse transform to identity with fill none and the 200 ms ease', async () => {
  const { restoreGlide, flipTransform, stateFromVisual } = await import('../src/lib/cover-zoom.js');
  const plan = restoreGlide(flipTransform({ left: 0, top: 0, width: 600, height: 600 }, { left: 40, top: 100, width: 200, height: 200 }));
  assert.deepEqual(plan.keyframes, [
    { transform: 'translate(160.00px, 100.00px) scale(3.0000)' },
    { transform: 'translate(0px, 0px) scale(1)' },
  ]);
  assert.deepEqual(plan.options, { duration: 200, easing: 'ease', fill: 'none' });
  // Shrunk: the inverse enlarges nothing, it starts at the shrunk size.
  assert.equal(restoreGlide({ scale: 0.5, x: 0, y: 0 }).keyframes[0].transform, 'translate(0.00px, 0.00px) scale(0.5000)');
  // A glide interrupted half-way: the state that paints the visual box on the final layout box.
  const state = stateFromVisual({ left: 30, top: 80, width: 300, height: 300 }, { left: 40, top: 100, width: 200, height: 200 });
  assert.deepEqual(state, { scale: 1.5, x: 40, y: 30 });
  // At or below scale 1 no offset can be carried.
  assert.deepEqual(stateFromVisual({ left: 0, top: 0, width: 100, height: 100 }, { left: 40, top: 100, width: 200, height: 200 }), { scale: 0.5, x: 0, y: 0 });
});

function fakeStageDom() {
  const pane = {
    clientLeft: 0, clientTop: 0, scrollLeft: 0, scrollTop: 0,
    children: [],
    appendChild(child) { this.children.push(child); child.parentElement = this; },
    getBoundingClientRect: () => ({ left: 20, top: 50, width: 500, height: 500 }),
  };
  // Computed swipe feedback = the inline value (no transitions run in the fake).
  globalThis.getComputedStyle = (element) => ({ borderTopLeftRadius: '18px', overflowX: 'visible', overflowY: 'visible', translate: element?.style?.translate || 'none', scale: element?.style?.scale || 'none' });
  return { pane };
}

function clearStageDom() {
  delete globalThis.document;
  delete globalThis.getComputedStyle;
  delete globalThis.devicePixelRatio;
}

/** A live cover (painted rect follows its inline transform, layout centre fixed) whose Web Animations are recorded; while
 *  one runs, the painted rect is `visualWhileGliding` (the glide's current frame). */
function glideCover(layout, initial, onZoom) {
  const { node, listeners } = liveCover(layout);
  const animations = [];
  const hooks = { visual: null };
  const painted = node.getBoundingClientRect;
  node.getBoundingClientRect = () => (hooks.visual && animations.some((a) => !a.cancelled && !a.finished) ? { ...hooks.visual } : painted());
  node.animate = (keyframes, timing) => {
    const animation = { keyframes, timing, currentTime: 0, cancelled: false, finished: false, cancel() { this.cancelled = true; }, onfinish: null };
    animations.push(animation);
    return animation;
  };
  const queue = [];
  globalThis.requestAnimationFrame = (fn) => { queue.push(fn); return queue.length; };
  globalThis.cancelAnimationFrame = () => {};
  return { node, listeners, animations, hooks, queue };
}

test('restore from zoomed-in: the stage itself glides (no second element), ends with nothing left inline', async () => {
  const { coverZoom } = await import('../src/lib/cover-zoom.js');
  const rig = glideCover({ x: 300, y: 300 }, undefined);
  try {
    const { node, listeners, animations } = rig;
    const action = coverZoom(node, { initial: { scale: 3, x: 20, y: -10 }, settleLayout: () => Promise.resolve() });
    assert.equal(node.style.transform, 'translate(20.00px, -10.00px) scale(3.0000)');
    listeners.get('dblclick')();
    // The final layout is committed at once: the stage's own transform is the real final state.
    assert.equal(node.style.transform, '');
    await settle();
    assert.equal(animations.length, 1);
    assert.equal(animations[0].keyframes[0].transform, 'translate(20.00px, -10.00px) scale(3.0000)');
    assert.equal(animations[0].timing.fill, 'none');
    assert.equal('coverResetting' in node.dataset, true);
    assert.equal('coverZoomed' in node.dataset, false);
    animations[0].finished = true;
    animations[0].onfinish();
    assert.equal(node.style.transform, '');
    assert.equal('coverResetting' in node.dataset, false);
    assert.equal(node.style.willChange, undefined, 'no layer property is ever written');
    action.destroy();
  } finally {
    clearStageDom();
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

test('restore from shrunk: the glide starts at the shrunk size and ends at rest, with no nudge needed at scale 1', async () => {
  const { coverZoom } = await import('../src/lib/cover-zoom.js');
  const rig = glideCover({ x: 300, y: 300 }, undefined);
  try {
    const { node, listeners, animations } = rig;
    const action = coverZoom(node, { initial: { scale: 0.5, x: 0, y: 0 }, settleLayout: () => Promise.resolve() });
    assert.equal('coverZoomed' in node.dataset, true);
    listeners.get('dblclick')();
    await settle();
    assert.equal(animations.length, 1);
    assert.equal(animations[0].keyframes[0].transform, 'translate(0.00px, 0.00px) scale(0.5000)');
    assert.equal(animations[0].keyframes[1].transform, 'translate(0px, 0px) scale(1)');
    animations[0].finished = true;
    animations[0].onfinish();
    assert.equal(node.style.transform, '');
    await wait(120);
    assert.equal(node.style.translate ?? '', '', 'no re-raster nudge after the restore');
    action.destroy();
  } finally {
    clearStageDom();
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

test('a wheel step during the glide cancels it and carries on from the visual it had reached, without a jump', async () => {
  const { coverZoom } = await import('../src/lib/cover-zoom.js');
  const rig = glideCover({ x: 300, y: 300 }, undefined);
  try {
    const { node, listeners, animations, hooks } = rig;
    const action = coverZoom(node, { initial: { scale: 3, x: 0, y: 0 }, settleLayout: () => Promise.resolve() });
    listeners.get('dblclick')();
    await settle();
    // Half-way: the glide currently paints 1.5x, 40 px / 30 px right of / below the layout centre.
    hooks.visual = { left: 300 + 40 - 150, top: 300 + 30 - 150, width: 300, height: 300 };
    listeners.get('wheel')({ deltaY: 0, deltaMode: 0, clientX: 300, clientY: 300, preventDefault() {} });
    assert.equal(animations[0].cancelled, true);
    assert.equal('coverResetting' in node.dataset, false);
    assert.equal(node.style.transform, 'translate(40.00px, 30.00px) scale(1.5000)', 'the stage paints exactly what the glide showed');
    await settle();
    assert.equal(node.style.transform, 'translate(40.00px, 30.00px) scale(1.5000)');
    action.destroy();
  } finally {
    clearStageDom();
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

test('a finger down during the glide cancels it; a glide nearly at rest hands over the identity', async () => {
  const { coverZoom } = await import('../src/lib/cover-zoom.js');
  const rig = glideCover({ x: 300, y: 300 }, undefined);
  try {
    const { node, listeners, animations, hooks } = rig;
    const zooms = [];
    const action = coverZoom(node, { initial: { scale: 2, x: 0, y: 0 }, settleLayout: () => Promise.resolve(), onZoom: (state) => zooms.push(state.scale) });
    listeners.get('dblclick')();
    await settle();
    hooks.visual = { left: 200, top: 200, width: 200, height: 200 }; // already at rest
    listeners.get('pointerdown')({ pointerId: 1, clientX: 300, clientY: 300, pointerType: 'touch' });
    assert.equal(animations[0].cancelled, true);
    assert.equal(node.style.transform, '');
    assert.equal(zooms.at(-1), 1, 'no zoom state invented for a glide that had reached rest');
    action.destroy();
  } finally {
    clearStageDom();
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

test('destroying the action (track change) during the glide cancels the animation and clears the flags', async () => {
  const { coverZoom } = await import('../src/lib/cover-zoom.js');
  const rig = glideCover({ x: 300, y: 300 }, undefined);
  try {
    const { node, listeners, animations } = rig;
    const action = coverZoom(node, { initial: { scale: 3, x: 0, y: 0 }, settleLayout: () => Promise.resolve() });
    listeners.get('dblclick')();
    await settle();
    action.destroy();
    assert.equal(animations[0].cancelled, true);
    assert.equal('coverResetting' in node.dataset, false);
    assert.equal(node.style.transform, '');
  } finally {
    clearStageDom();
    delete globalThis.requestAnimationFrame;
    delete globalThis.cancelAnimationFrame;
  }
});

test('source shape: no canvas or overlay element, style or file remains for the cover', async () => {
  const { readFileSync, existsSync } = await import('node:fs');
  assert.equal(existsSync(new URL('../src/lib/cover-overlay.js', import.meta.url)), false);
  const forbidden = /createOverlayCanvas|drawOverlay|hideOverlay|cover-stage-overlay|data-cover-overlay|coverOverlay|createElement\(['"]canvas['"]\)|<canvas|overlayMatchesSource|coverDrawPlan|snapRectToDevicePixels/;
  for (const file of ['cover-zoom.js', 'cover-swipe-feedback.js', 'cover-reraster.js']) {
    assert.doesNotMatch(readFileSync(new URL(`../src/lib/${file}`, import.meta.url), 'utf8'), forbidden, file);
  }
  assert.doesNotMatch(readFileSync(new URL('../src/app.css', import.meta.url), 'utf8'), /cover-stage-overlay|data-cover-overlay/);
  assert.doesNotMatch(readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8'), /cover-stage-overlay|<canvas/);
});

function assertRectClose(actual, expected, message, epsilon = 0.02) {
  for (const key of ['left', 'top', 'width', 'height']) {
    assert.ok(Math.abs(actual[key] - expected[key]) < epsilon, `${message}: ${key} ${actual[key]} vs ${expected[key]}`);
  }
}

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

test('swipe feedback geometry: parsed from computed values and removed from a read rect', async () => {
  const { parseSwipeFeedback: parse, withoutSwipeFeedback } = await import('../src/lib/cover-swipe-feedback.js');
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
      // Like a browser: `translate` is serialised without a zero second component ('0.01px 0px' reads back '0.01px').
      if (key === 'translate' && typeof value === 'string') value = value.trim().split(/\s+/).filter((part, i, all) => !(i === 1 && parseFloat(part) === 0 && all.length === 2)).join(' ');
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

test('source shape: the cover layer structure never changes at a gesture or animation end', async () => {
  const { readFileSync } = await import('node:fs');
  const css = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  // The stage is promoted and stacked for its whole life.
  assert.match(css, /\.now-playing-view \.cover-stage \{\s*z-index: 1;\s*will-change: transform;\s*\}/);
  // No rule keyed on a cover attribute touches a compositing or stacking property.
  const attributeRules = [...css.matchAll(/\.cover-stage\[data-cover-[a-z-]+\][^{]*\{[^}]*\}/g)].map((match) => match[0]);
  assert.ok(attributeRules.length >= 1);
  for (const rule of attributeRules) {
    assert.doesNotMatch(rule.slice(rule.indexOf('{')), /will-change|z-index|contain|isolation|position|opacity|filter|transform|mix-blend/, rule);
  }
  assert.doesNotMatch(css, /data-cover-gesture|\[data-cover-resetting\]/);
  const source = readFileSync(new URL('../src/lib/cover-zoom.js', import.meta.url), 'utf8').replace(/\r\n/g, '\n');
  assert.doesNotMatch(source, /coverGesture|style\.willChange|style\.zIndex|style\.contain|style\.isolation|style\.position/);
  // The restore glide animates the stage itself, and only its transform.
  const glide = source.slice(source.indexOf('  function startGlide'), source.indexOf('  function beginGlide'));
  assert.match(glide, /node\.animate\(plan\.keyframes, plan\.options\)/);
  assert.doesNotMatch(source, /style\.visibility|visibility: hidden/);
});

test('source shape: the nudge writes only translate and transition, never a layer property, and every gesture entry cancels it', async () => {
  const { readFileSync } = await import('node:fs');
  const source = readFileSync(new URL('../src/lib/cover-zoom.js', import.meta.url), 'utf8').replace(/\r\n/g, '\n');
  const nudge = source.slice(source.indexOf('  function runNudge'), source.indexOf('  /** A feedback (translate / scale / opacity) CSS transition'));
  assert.doesNotMatch(nudge, /willChange|zIndex|style\.contain|isolation|style\.position|style\.opacity|style\.scale|style\.transform/);
  assert.match(nudge, /style\.translate = COVER_RERASTER_TRANSLATE/);
  assert.match(nudge, /style\.transition = COVER_RERASTER_TRANSITION/);
  // The raf step bails out when something else owns translate / transition now.
  assert.match(nudge, /!isRerasterTranslate\(style\.translate\) \|\| \(style\.transition \|\| ''\) !== mine\.transition/);
  // The restore timer only touches the transition it set.
  assert.match(nudge, /\(style\.transition \|\| ''\) === mine\.transition/);
  for (const entry of ['function wheel', 'function down', 'function apply']) {
    const body = source.slice(source.indexOf(`  ${entry}(`));
    assert.match(body.slice(0, body.indexOf('\n  }\n')), /cancelNudge\(true\)/, entry);
  }
});

/** A stage in a pane with the frame queue, mutation observers and swipe wiring the nudge tests share. */
async function nudgeRig(initial) {
  const { coverZoom } = await import('../src/lib/cover-zoom.js');
  const { swipeDismiss } = await import('../src/lib/swipe-dismiss.js');
  const { pane } = fakeStageDom();
  const env = installSharedStageGlobals();
  const frames = [];
  globalThis.requestAnimationFrame = (fn) => { frames.push(fn); return frames.length; };
  globalThis.cancelAnimationFrame = () => {};
  const stage = sharedStage({ x: 300, y: 300 });
  const { node, fire } = stage;
  pane.appendChild(node);
  const zoom = coverZoom(node, { initial, settleLayout: () => Promise.resolve() });
  stage.observers.push(...env.observers);
  const swipe = swipeDismiss(node, { follow: false, damped: true, actions: { left: () => {}, right: () => {}, down: () => {} } });
  const runFrames = async () => { while (frames.length) frames.shift()(); await settle(); };
  const wheel = async (deltaY) => {
    fire('wheel', { deltaY, deltaMode: 0, clientX: 300, clientY: 300, preventDefault() {} });
    await runFrames();
  };
  const touch = (type, id = 1) => ({ pointerId: id, isPrimary: id === 1, pointerType: 'touch', clientX: 300, clientY: 300, timeStamp: 0, target: null, cancelable: true, preventDefault() {}, ...(type ? { type } : {}) });
  return {
    node, fire, frames, runFrames, wheel, touch,
    destroy() {
      swipe.destroy();
      zoom.destroy();
      env.restore();
      clearStageDom();
      delete globalThis.requestAnimationFrame;
      delete globalThis.cancelAnimationFrame;
    },
  };
}

test('re-raster nudge: once the zoom-in is stable it moves the stage 0.01px and back by transition, then restores the transition', async () => {
  const { COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const rig = await nudgeRig(undefined);
  try {
    const { node } = rig;
    await rig.wheel(-300); // zoom in
    assert.ok(node.dataset.coverZoomed !== undefined);
    assert.equal(node.style.translate ?? '', '', 'not nudged while the zoom was just applied');
    await wait(COVER_LAYER_IDLE_MS + 40);
    assert.equal(node.style.translate, '0.01px');
    assert.equal(node.style.transition, 'none');
    await rig.runFrames();
    assert.equal(node.style.translate, '');
    assert.equal(node.style.transition, 'translate 180ms');
    await wait(300);
    assert.equal(node.style.transition, 'none', 'the transition string from before the nudge is back');
    // Nothing more happens while the scale stays: one nudge per settled scale.
    await wait(COVER_LAYER_IDLE_MS + 40);
    assert.equal(node.style.translate, '');
    assert.equal(rig.frames.length, 0);
    // A later zoom change nudges again once it is stable.
    await rig.wheel(-200);
    assert.equal(node.style.translate ?? '', '');
    await wait(COVER_LAYER_IDLE_MS + 40);
    assert.equal(node.style.translate, '0.01px');
    await rig.runFrames();
    await wait(300);
    assert.equal(node.style.translate, '');
    assert.equal(node.style.transition, 'none');
  } finally {
    rig.destroy();
  }
});

test('re-raster nudge: continuous zooming postpones it; no nudge at scale 1 or below', async () => {
  const { COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const rig = await nudgeRig(undefined);
  try {
    const { node } = rig;
    await wait(COVER_LAYER_IDLE_MS + 40);
    assert.equal(node.style.translate ?? '', '', 'scale 1: nothing');
    await rig.wheel(300); // zoom out below 1
    await wait(COVER_LAYER_IDLE_MS + 40);
    await rig.runFrames();
    assert.equal(node.style.translate ?? '', '', 'shrunk: nothing');
    await rig.wheel(-900); // back above 1
    for (let i = 0; i < 4; i += 1) {
      await wait(COVER_LAYER_IDLE_MS / 2);
      await rig.wheel(-50);
      assert.equal(node.style.translate ?? '', '', 'still zooming');
    }
    await wait(COVER_LAYER_IDLE_MS + 40);
    assert.equal(node.style.translate, '0.01px');
  } finally {
    rig.destroy();
  }
});

test('re-raster nudge: a finger down cancels a pending nudge and it waits until all fingers are up', async () => {
  const { COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const rig = await nudgeRig(undefined);
  try {
    const { node, fire } = rig;
    await rig.wheel(-300);
    fire('pointerdown', rig.touch());
    await wait(COVER_LAYER_IDLE_MS * 3);
    assert.equal(node.style.translate ?? '', '', 'no nudge under a finger');
    fire('pointerup', rig.touch());
    await wait(COVER_LAYER_IDLE_MS + 40);
    assert.equal(node.style.translate, '0.01px', 'nudged once the gesture is over');
  } finally {
    rig.destroy();
  }
});

test('re-raster nudge: swipe feedback wins over a nudge in flight, and its settle is not nudged over', async () => {
  const { COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const rig = await nudgeRig(undefined);
  try {
    const { node } = rig;
    await rig.wheel(-300);
    await wait(COVER_LAYER_IDLE_MS + 40);
    assert.equal(node.style.translate, '0.01px');
    // The swipe feedback lands before the nudge's second step.
    node.style.transition = 'none';
    node.style.translate = '12px';
    node.style.scale = '0.97';
    node.style.opacity = '0.9';
    await settle();
    await rig.runFrames();
    assert.equal(node.style.translate, '12px', 'the nudge did not clear the swipe translate');
    assert.equal(node.style.scale, '0.97');
    assert.equal(node.style.transition, 'none');
    await wait(300);
    assert.equal(node.style.translate, '12px');
    assert.equal(node.style.transition, 'none', 'the restore timer left the swipe transition alone');
    // The swipe settles back by its own transition (which re-rasters the layer by itself): no nudge over it.
    node.style.transition = 'translate 180ms ease, scale 180ms ease, opacity 180ms ease';
    node.style.translate = '';
    node.style.scale = '';
    node.style.opacity = '';
    await settle();
    await wait(COVER_LAYER_IDLE_MS * 2);
    assert.equal(node.style.translate ?? '', '');
    node.style.transition = '';
    await settle();
    await wait(COVER_LAYER_IDLE_MS + 40);
    assert.equal(node.style.translate ?? '', '');
  } finally {
    rig.destroy();
  }
});

test('re-raster nudge: swipe feedback written without any observer batch yet is not cleared by the nudge\'s second step', async () => {
  const { COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const rig = await nudgeRig(undefined);
  try {
    const { node } = rig;
    await rig.wheel(-300);
    await wait(COVER_LAYER_IDLE_MS + 40);
    node.style.translate = '9px';
    await rig.runFrames(); // the frame runs before any mutation observer batch
    assert.equal(node.style.translate, '9px');
    assert.equal(node.style.transition, 'none');
  } finally {
    rig.destroy();
  }
});

test('re-raster nudge: a swipe written in the second step of a nudge is not undone by its restore', async () => {
  const { COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const rig = await nudgeRig(undefined);
  try {
    const { node } = rig;
    await rig.wheel(-300);
    await wait(COVER_LAYER_IDLE_MS + 40);
    await rig.runFrames(); // the nudge's transition is now running
    assert.equal(node.style.transition, 'translate 180ms');
    node.style.transition = 'none';
    node.style.translate = '-20px';
    await settle();
    await wait(300);
    assert.equal(node.style.translate, '-20px');
    assert.equal(node.style.transition, 'none');
  } finally {
    rig.destroy();
  }
});

test('re-raster nudge: a wheel step during the nudge takes it back, and the new scale is nudged once stable', async () => {
  const { COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const rig = await nudgeRig(undefined);
  try {
    const { node } = rig;
    await rig.wheel(-300);
    await wait(COVER_LAYER_IDLE_MS + 40);
    assert.equal(node.style.translate, '0.01px');
    rig.fire('wheel', { deltaY: -100, deltaMode: 0, clientX: 300, clientY: 300, preventDefault() {} });
    assert.equal(node.style.translate, '', 'the wheel took the nudge back');
    await rig.runFrames();
    assert.equal(node.style.translate ?? '', '');
    await wait(COVER_LAYER_IDLE_MS + 40);
    assert.equal(node.style.translate, '0.01px');
  } finally {
    rig.destroy();
  }
});

test('re-raster nudge: a track change at an enlarged scale (new cover element, then its image) nudges when stable', async () => {
  const { COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const rig = await nudgeRig({ scale: 2, x: 0, y: 0 });
  try {
    const { node } = rig;
    await wait(COVER_LAYER_IDLE_MS + 40);
    assert.equal(node.style.translate, '0.01px');
    await rig.runFrames();
    await wait(300);
    assert.equal(node.style.translate, '');
    // The next cover image loads into the element: the layer needs another nudge.
    node.dispatchEvent({ type: 'load' });
    await wait(COVER_LAYER_IDLE_MS + 40);
    assert.equal(node.style.translate, '0.01px');
  } finally {
    rig.destroy();
  }
});

test('re-raster nudge: the full sequence completes with a browser-serialised translate (regression: stuck 0.01px / transition none)', async () => {
  const { COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const rig = await nudgeRig(undefined);
  try {
    const { node } = rig;
    node.style.transition = 'none';
    await rig.wheel(-300);
    await wait(COVER_LAYER_IDLE_MS + 40);
    assert.equal(node.style.translate, '0.01px', 'serialised without the zero component');
    await rig.runFrames();
    assert.equal(node.style.translate, '', 'step 2 ran: translate cleared');
    assert.equal(node.style.transition, 'translate 180ms', 'step 2 ran: the transition is set');
    await wait(300);
    assert.equal(node.style.translate, '');
    assert.equal(node.style.transition, 'none', 'transition restored');
  } finally {
    rig.destroy();
  }
});

test('re-raster nudge: a nudge whose frame never comes is taken back by the safety net', async () => {
  const { COVER_LAYER_IDLE_MS } = await import('../src/lib/cover-zoom.js');
  const rig = await nudgeRig(undefined);
  try {
    const { node } = rig;
    await rig.wheel(-300);
    await wait(COVER_LAYER_IDLE_MS + 40);
    assert.equal(node.style.translate, '0.01px');
    rig.frames.length = 0; // the animation frame is lost
    await wait(520);
    assert.equal(node.style.translate, '');
    assert.equal(node.style.transition, 'none');
  } finally {
    rig.destroy();
  }
});
