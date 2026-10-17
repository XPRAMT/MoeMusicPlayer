import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DSD_GAIN_OPTIONS_DB,
  DSD_OUTPUT_RATES_HZ,
  formatDsdGain,
  formatDsdOutputRate,
  normalizeDsdSettings,
} from '../src/lib/dsd-settings.js';

test('DSD settings are repaired to the supported ranges', () => {
  assert.deepEqual(normalizeDsdSettings({ gainDb: 4, outputRateHz: 352_800 }), { gainDb: 4, outputRateHz: 352_800 });
  assert.deepEqual(normalizeDsdSettings({ gainDb: 40, outputRateHz: 192_000 }), { gainDb: 6, outputRateHz: 176_400 });
  assert.deepEqual(normalizeDsdSettings({ gainDb: -3, outputRateHz: 44_100 }), { gainDb: 1, outputRateHz: 176_400 });
  assert.deepEqual(normalizeDsdSettings({ gainDb: 'loud' }), { gainDb: 3, outputRateHz: 176_400 });
  assert.deepEqual(normalizeDsdSettings(null), { gainDb: 3, outputRateHz: 176_400 });
});

test('DSD options and labels', () => {
  assert.deepEqual([...DSD_GAIN_OPTIONS_DB], [1, 2, 3, 4, 5, 6]);
  assert.deepEqual([...DSD_OUTPUT_RATES_HZ], [176_400, 352_800]);
  assert.equal(formatDsdGain(3), '+3 dB');
  assert.equal(formatDsdGain(2.5), '+2.5 dB');
  assert.equal(formatDsdOutputRate(176_400), '176.4 kHz');
  assert.equal(formatDsdOutputRate(352_800), '352.8 kHz');
});
