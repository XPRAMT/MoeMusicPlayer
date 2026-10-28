import test from 'node:test';
import assert from 'node:assert/strict';
import { albumReflowProbeEnabled, createReflowStats } from '../src/lib/album-reflow-debug.js';

test('the reflow probe is off unless the flag is exactly 1, and never throws on blocked storage', () => {
  assert.equal(albumReflowProbeEnabled({ getItem: () => '1' }), true);
  assert.equal(albumReflowProbeEnabled({ getItem: () => '0' }), false);
  assert.equal(albumReflowProbeEnabled({ getItem: () => null }), false);
  assert.equal(albumReflowProbeEnabled({ getItem: () => { throw new Error('blocked'); } }), false);
  assert.equal(albumReflowProbeEnabled(undefined), false);
});

test('reflow stats report the average and maximum cost since the last report', () => {
  const stats = createReflowStats();
  assert.equal(stats.take(), null);
  stats.add(1, 150, 100);
  stats.add(3, 170, 90);
  assert.equal(stats.take(), '[album-reflow] frames 2, forced layout avg 2.00 ms max 3.00 ms, summary 170 x 90 px');
  assert.equal(stats.take(), null);
});
