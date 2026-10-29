import assert from 'node:assert/strict';
import test from 'node:test';
import { isMouseEntry, releasesOnMove, releasesOnWindowEvent } from '../src/lib/cover-pointers.js';
import { coverZoom } from '../src/lib/cover-zoom.js';

test('pure rules: only mouse entries are released by a window event or a buttons-0 move', () => {
  const mouse = { x: 0, y: 0, type: 'mouse' };
  const touch = { x: 0, y: 0, type: 'touch' };
  const pen = { x: 0, y: 0, type: 'pen' };
  assert.equal(isMouseEntry(mouse), true);
  assert.equal(releasesOnWindowEvent(mouse), true);
  assert.equal(releasesOnWindowEvent(touch), false);
  assert.equal(releasesOnWindowEvent(undefined), false);
  assert.equal(releasesOnMove(mouse, 0), true);
  assert.equal(releasesOnMove(mouse, 1), false);
  assert.equal(releasesOnMove(touch, 0), false);
  assert.equal(releasesOnMove(pen, 0), false);
});

function harness() {
  const nodeListeners = new Map();
  const winListeners = new Map();
  const timers = [];
  const saved = {};
  const set = (k, v) => { saved[k] = Object.getOwnPropertyDescriptor(globalThis, k); globalThis[k] = v; };
  set('addEventListener', (type, fn, capture) => winListeners.set(`${type}:${capture === true}`, fn));
  set('removeEventListener', (type, fn, capture) => winListeners.delete(`${type}:${capture === true}`));
  set('setTimeout', (fn, ms) => { const t = { fn, ms, live: true }; timers.push(t); return t; });
  set('clearTimeout', (t) => { if (t) t.live = false; });
  set('requestAnimationFrame', () => 1);
  set('cancelAnimationFrame', () => {});
  const node = {
    style: {}, dataset: {}, offsetWidth: 200, offsetHeight: 200,
    addEventListener: (type, fn) => nodeListeners.set(type, fn),
    removeEventListener: (type) => nodeListeners.delete(type),
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 200, height: 200 }),
    setPointerCapture: () => {},
    dispatchEvent: () => true,
  };
  const action = coverZoom(node, { initial: { scale: 3, x: 0, y: 0 }, settleLayout: () => Promise.resolve() });
  const restore = () => {
    action.destroy();
    for (const [k, d] of Object.entries(saved)) { if (d) Object.defineProperty(globalThis, k, d); else delete globalThis[k]; }
  };
  const armed = () => timers.filter((t) => t.live).length;
  return { node, nodeListeners, winListeners, action, restore, armed, timers };
}

const ev = (type, pointerId, pointerType, extra = {}) => ({ type, pointerId, pointerType, clientX: 5, clientY: 5, buttons: 1, timeStamp: 0, cancelable: false, ...extra });

test('a mouse press whose release the node never sees is cleared by the window-level pointerup and re-arms the nudge', () => {
  const h = harness();
  try {
    h.nodeListeners.get('pointerdown')(ev('pointerdown', 1, 'mouse'));
    for (const t of h.timers) t.live = false;
    assert.equal(h.armed(), 0);
    // While the entry is stale nothing is armed by a release; the window capture listener releases it.
    h.winListeners.get('pointerup:true')(ev('pointerup', 1, 'mouse', { buttons: 0 }));
    assert.equal(h.armed(), 1, 'pointers.size === 0 path re-armed the re-raster schedule');
    // Idempotent: a second delivery (the node also sees it) changes nothing.
    const armedAfter = h.armed();
    h.nodeListeners.get('pointerup')(ev('pointerup', 1, 'mouse', { buttons: 0 }));
    h.winListeners.get('pointerup:true')(ev('pointerup', 1, 'mouse', { buttons: 0 }));
    assert.equal(h.armed(), armedAfter);
  } finally { h.restore(); }
});

test('a pointermove with buttons 0 clears a stale mouse entry; a held button does not', () => {
  const h = harness();
  try {
    h.nodeListeners.get('pointerdown')(ev('pointerdown', 1, 'mouse'));
    for (const t of h.timers) t.live = false;
    h.nodeListeners.get('pointermove')(ev('pointermove', 1, 'mouse', { buttons: 1 }));
    assert.equal(h.armed(), 0, 'button held: still a gesture, nothing re-armed');
    h.nodeListeners.get('pointermove')(ev('pointermove', 1, 'mouse', { buttons: 0 }));
    assert.equal(h.armed(), 1, 'released: schedule re-armed');
  } finally { h.restore(); }
});

test('a touch entry is not cleared by buttons 0 or by a window-level event', () => {
  const h = harness();
  try {
    h.nodeListeners.get('pointerdown')(ev('pointerdown', 7, 'touch'));
    for (const t of h.timers) t.live = false;
    h.nodeListeners.get('pointermove')(ev('pointermove', 7, 'touch', { buttons: 0 }));
    h.winListeners.get('pointerup:true')(ev('pointerup', 7, 'touch', { buttons: 0 }));
    assert.equal(h.armed(), 0, 'touch contact still active');
    h.nodeListeners.get('pointerup')(ev('pointerup', 7, 'touch', { buttons: 0 }));
    assert.equal(h.armed(), 1, 'the node release clears it as before');
  } finally { h.restore(); }
});

test('destroy removes the window listeners', () => {
  const h = harness();
  assert.equal(h.winListeners.has('pointerup:true'), true);
  assert.equal(h.winListeners.has('pointercancel:true'), true);
  h.restore();
  assert.equal(h.winListeners.size, 0);
});
