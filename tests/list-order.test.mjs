import test from 'node:test';
import assert from 'node:assert/strict';
import { formatListOrder } from '../src/lib/list-order.js';

test('list order labels are one-based positions in the visible sequence', () => {
  assert.equal(formatListOrder(0), '1');
  assert.equal(formatListOrder(1), '2');
  assert.equal(formatListOrder(9), '10');
});

test('invalid list positions have no numeric label', () => {
  for (const position of [-1, 1.5, Number.MAX_SAFE_INTEGER, Number.NaN, null]) {
    assert.equal(formatListOrder(position), '—');
  }
});
