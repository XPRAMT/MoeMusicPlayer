import assert from 'node:assert/strict';
import test from 'node:test';
import { getPopoverHorizontalShiftPx } from '../src/lib/volume-popover-position.js';

test('popover remains centered while both horizontal edges fit the viewport', () => {
  assert.equal(getPopoverHorizontalShiftPx(500, 192, 1000), 0);
});

test('popover shifts inward from either edge by only the required amount', () => {
  assert.equal(getPopoverHorizontalShiftPx(80, 192, 1000), 24);
  assert.equal(getPopoverHorizontalShiftPx(920, 192, 1000), -24);
});

test('popover keeps the configured inset at each edge', () => {
  assert.equal(getPopoverHorizontalShiftPx(30, 200, 1000, 12), 82);
  assert.equal(getPopoverHorizontalShiftPx(970, 200, 1000, 12), -82);
});
