import assert from 'node:assert/strict';
import test from 'node:test';
import { DRAG_SCROLL_IGNORE_SELECTOR, dragScrollOffsets, isOnScrollbar } from '../src/lib/drag-scroll.js';

test('dragging moves the scroll position the opposite way and stays in range', () => {
  const limits = { maxLeft: 100, maxTop: 1000 };
  assert.deepEqual(dragScrollOffsets({ left: 0, top: 500 }, 0, -120, limits), { left: 0, top: 620 });
  assert.deepEqual(dragScrollOffsets({ left: 0, top: 500 }, 30, 80, limits), { left: 0, top: 420 });
  assert.deepEqual(dragScrollOffsets({ left: 10, top: 20 }, -500, 500, limits), { left: 100, top: 0 });
  assert.deepEqual(dragScrollOffsets({ left: 0, top: 0 }, 5, 5, { maxLeft: -4, maxTop: 0 }), { left: 0, top: 0 });
});

test('a press on the scrollbar is left to the browser', () => {
  const node = { clientWidth: 800, clientHeight: 400 };
  assert.equal(isOnScrollbar(node, { x: 810, y: 100 }), true);
  assert.equal(isOnScrollbar(node, { x: 100, y: 405 }), true);
  assert.equal(isOnScrollbar(node, { x: 100, y: 100 }), false);
});

test('fields and sliders keep their own drags; buttons and cards do not', () => {
  assert.match(DRAG_SCROLL_IGNORE_SELECTOR, /input/);
  assert.match(DRAG_SCROLL_IGNORE_SELECTOR, /role="slider"/);
  assert.doesNotMatch(DRAG_SCROLL_IGNORE_SELECTOR, /button/);
});
