import assert from 'node:assert/strict';
import test from 'node:test';
import { getVolumeIconLevel } from '../src/lib/volume-icon.js';

test('the speaker icon gains sound waves with the volume', () => {
  assert.equal(getVolumeIconLevel(0), 'off');
  assert.equal(getVolumeIconLevel(0.004), 'off', 'rounds to 0 %, like the number shown');
  assert.equal(getVolumeIconLevel(0.005), 'low');
  assert.equal(getVolumeIconLevel(0.33), 'low');
  assert.equal(getVolumeIconLevel(0.34), 'medium');
  assert.equal(getVolumeIconLevel(0.66), 'medium');
  assert.equal(getVolumeIconLevel(0.67), 'high');
  assert.equal(getVolumeIconLevel(1), 'high');
});

test('a missing or out-of-range volume is handled', () => {
  assert.equal(getVolumeIconLevel(null), 'off');
  assert.equal(getVolumeIconLevel(undefined), 'off');
  assert.equal(getVolumeIconLevel(Number.NaN), 'off');
  assert.equal(getVolumeIconLevel(-1), 'off');
  assert.equal(getVolumeIconLevel(7), 'high');
});
