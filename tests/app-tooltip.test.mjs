import test from 'node:test';
import assert from 'node:assert/strict';
import { appTooltip, getAppTooltipPosition } from '../src/lib/app-tooltip.js';

class FakeElement extends EventTarget {
  attributes = new Map();
  classes = new Set();
  style = { setProperty() {} };
  isConnected = true;
  offsetWidth = 140;
  offsetHeight = 32;
  textContent = '';
  classList = {
    add: (name) => this.classes.add(name),
    remove: (name) => this.classes.delete(name),
    contains: (name) => this.classes.has(name),
  };

  setAttribute(name, value) { this.attributes.set(name, value); }
  getAttribute(name) { return this.attributes.get(name) ?? null; }
  removeAttribute(name) { this.attributes.delete(name); }
  contains(node) { return node === this; }
  getBoundingClientRect() { return { left: 20, right: 60, top: 40, bottom: 60 }; }
  emit(type, properties = {}) {
    const event = new Event(type);
    for (const [name, value] of Object.entries(properties)) {
      Object.defineProperty(event, name, { configurable: true, value });
    }
    this.dispatchEvent(event);
  }
}

function installTooltipDom() {
  const timers = new Map();
  let nextTimerId = 0;
  let tooltip = null;
  const body = { append(element) { element.isConnected = true; tooltip = element; } };
  const original = {
    window: globalThis.window,
    document: globalThis.document,
    Node: globalThis.Node,
  };
  globalThis.Node = FakeElement;
  globalThis.document = {
    body,
    createElement: () => new FakeElement(),
    querySelector: (selector) => selector === '#moe-app-tooltip' ? tooltip : null,
  };
  globalThis.window = {
    innerWidth: 800,
    innerHeight: 600,
    setTimeout(callback) { const id = ++nextTimerId; timers.set(id, callback); return id; },
    clearTimeout(id) { timers.delete(id); },
    addEventListener() {},
    removeEventListener() {},
    getComputedStyle() { return { getPropertyValue: () => '' }; },
  };
  return {
    restore() {
      if (tooltip) tooltip.isConnected = false;
      globalThis.window = original.window;
      globalThis.document = original.document;
      globalThis.Node = original.Node;
    },
    runTimers() {
      const pending = [...timers.values()];
      timers.clear();
      for (const callback of pending) callback();
    },
  };
}

test('tooltip prefers above the anchor when there is room', () => {
  assert.deepEqual(
    getAppTooltipPosition({ left: 100, right: 140, top: 100, bottom: 120 }, 120, 32, 800, 600),
    { left: 60, top: 60 },
  );
});

test('tooltip uses the space below near the window titlebar', () => {
  assert.deepEqual(
    getAppTooltipPosition({ left: 100, right: 140, top: 4, bottom: 24 }, 120, 32, 800, 600),
    { left: 60, top: 32 },
  );
});

test('tooltip stays inside horizontal and vertical viewport edges', () => {
  assert.deepEqual(
    getAppTooltipPosition({ left: 780, right: 800, top: 580, bottom: 600 }, 180, 48, 800, 600),
    { left: 612, top: 524 },
  );
});

test('pointer-clicked tooltip hides on pointer leave even while its button keeps focus', () => {
  const dom = installTooltipDom();
  let action;
  try {
    const target = new FakeElement();
    action = appTooltip(target, 'Click action');
    target.emit('pointerenter', { pointerType: 'mouse' });
    dom.runTimers();
    assert.equal(document.querySelector?.('#moe-app-tooltip')?.getAttribute('aria-hidden'), 'false');

    target.emit('focusin');
    target.emit('click', { detail: 1 });
    target.emit('pointerleave');
    dom.runTimers();

    assert.equal(document.querySelector?.('#moe-app-tooltip')?.getAttribute('aria-hidden'), 'true');
  } finally {
    action?.destroy();
    dom.restore();
  }
});

test('keyboard focus keeps its tooltip visible until focus leaves', () => {
  const dom = installTooltipDom();
  let action;
  try {
    const target = new FakeElement();
    action = appTooltip(target, 'Keyboard action');
    target.emit('focusin');
    dom.runTimers();
    assert.equal(document.querySelector('#moe-app-tooltip').getAttribute('aria-hidden'), 'false');

    target.emit('focusout', { relatedTarget: null });
    assert.equal(document.querySelector('#moe-app-tooltip').getAttribute('aria-hidden'), 'true');
  } finally {
    action?.destroy();
    dom.restore();
  }
});
