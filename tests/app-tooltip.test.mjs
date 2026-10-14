import test from 'node:test';
import assert from 'node:assert/strict';
import { getAppTooltipPosition } from '../src/lib/app-tooltip.js';

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
