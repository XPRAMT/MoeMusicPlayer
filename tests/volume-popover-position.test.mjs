import assert from 'node:assert/strict';
import test from 'node:test';
import { getVolumePopoverLeftShiftPx } from '../src/lib/volume-popover-position.js';

test('volume popover stays centered while its right edge fits the viewport', () => {
  assert.equal(getVolumePopoverLeftShiftPx(500, 192, 1000), 0);
  assert.equal(getVolumePopoverLeftShiftPx(896, 192, 1000), 0);
});

test('volume popover shifts left only by the amount crossing the viewport inset', () => {
  assert.equal(getVolumePopoverLeftShiftPx(900, 192, 1000), 4);
  assert.equal(getVolumePopoverLeftShiftPx(950, 192, 1000), 54);
});
