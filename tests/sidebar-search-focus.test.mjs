import test from 'node:test';
import assert from 'node:assert/strict';
import { scrollDeltaToReveal } from '../src/lib/sidebar-search-focus.js';

const scroller = { top: 100, bottom: 500 };

test('a fully visible search field does not scroll', () => {
  assert.equal(scrollDeltaToReveal(scroller, { top: 120, bottom: 156 }, 8), 0);
});

test('a field scrolled above the scroller comes back with its scroll margin', () => {
  // Field top is 140 px above the scroller top; margin 8 → scroll up 148 px.
  assert.equal(scrollDeltaToReveal(scroller, { top: -40, bottom: -4 }, 8), -148);
  // Partially hidden under the top edge (the fixed brand sits above it).
  assert.equal(scrollDeltaToReveal(scroller, { top: 90, bottom: 126 }, 8), -18);
});

test('a field below the scroller scrolls down just enough', () => {
  assert.equal(scrollDeltaToReveal(scroller, { top: 480, bottom: 516 }, 8), 24);
});

test('a target taller than the scroller keeps its top visible', () => {
  assert.equal(scrollDeltaToReveal(scroller, { top: 200, bottom: 800 }, 0), 100);
});

test('invalid margins count as zero', () => {
  assert.equal(scrollDeltaToReveal(scroller, { top: 90, bottom: 126 }, Number.NaN), -10);
  assert.equal(scrollDeltaToReveal(scroller, { top: 90, bottom: 126 }, -5), -10);
});
