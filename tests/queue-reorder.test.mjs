import assert from 'node:assert/strict';
import test from 'node:test';
import {
  moveListItem,
  queueDropIndexFromPointer,
  QUEUE_REORDER_LONG_PRESS_MS,
  QUEUE_REORDER_MOVE_CANCEL_PX,
} from '../src/lib/queue-reorder.js';

test('moveListItem places the item at the final index', () => {
  assert.deepEqual(moveListItem(['a', 'b', 'c', 'd'], 1, 3), ['a', 'c', 'd', 'b']);
  assert.deepEqual(moveListItem(['a', 'b', 'c', 'd'], 3, 1), ['a', 'd', 'b', 'c']);
  assert.deepEqual(moveListItem(['a', 'b', 'c'], 1, 1), ['a', 'b', 'c']);
  assert.deepEqual(moveListItem(['a', 'b'], 0, 5), ['a', 'b']);
});

test('queueDropIndexFromPointer maps viewport coordinates into a clamped row index', () => {
  assert.equal(queueDropIndexFromPointer({
    scrollTop: 100,
    pointerClientY: 140,
    viewportTop: 40,
    rowHeight: 50,
    totalCount: 10,
  }), 4);
  assert.equal(queueDropIndexFromPointer({
    scrollTop: 0,
    pointerClientY: -20,
    viewportTop: 0,
    rowHeight: 50,
    totalCount: 3,
  }), 0);
  assert.equal(queueDropIndexFromPointer({
    scrollTop: 0,
    pointerClientY: 999,
    viewportTop: 0,
    rowHeight: 50,
    totalCount: 3,
  }), 2);
});

test('long-press reorder constants stay intentional', () => {
  assert.equal(QUEUE_REORDER_LONG_PRESS_MS, 500);
  assert.ok(QUEUE_REORDER_MOVE_CANCEL_PX >= 4);
});
