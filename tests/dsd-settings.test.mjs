import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DSD_GAIN_MAX_DB,
  DSD_GAIN_MIN_DB,
  DSD_GAIN_STEP_DB,
  DSD_DEFAULT_NATIVE_BYTE_ORDER,
  DSD_DEFAULT_NATIVE_PASSTHROUGH,
  DSD_OUTPUT_RATES_HZ,
  formatDsdGain,
  formatDsdOutputRate,
  formatDsdName,
  normalizeDsdSettings,
} from '../src/lib/dsd-settings.js';

test('DSD settings are repaired to the supported ranges', () => {
  const rest = { dop: false, nativePassthrough: false, nativeByteOrder: 'a' };
  assert.deepEqual(normalizeDsdSettings({ gainDb: 4, outputRateHz: 352_800 }), { gainDb: 4, outputRateHz: 352_800, ...rest });
  assert.deepEqual(normalizeDsdSettings({ gainDb: 40, outputRateHz: 192_000 }), { gainDb: 6, outputRateHz: 176_400, ...rest });
  assert.deepEqual(normalizeDsdSettings({ gainDb: -3, outputRateHz: 44_100 }), { gainDb: 0, outputRateHz: 176_400, ...rest });
  assert.deepEqual(normalizeDsdSettings({ gainDb: 'loud' }), { gainDb: 3, outputRateHz: 176_400, ...rest });
  assert.equal(normalizeDsdSettings({ dop: true }).dop, true);
  assert.equal(normalizeDsdSettings({ dop: 'yes' }).dop, false, 'only a real boolean turns DoP on');
  assert.deepEqual(normalizeDsdSettings(null), { gainDb: 3, outputRateHz: 176_400, ...rest });
});

test('native passthrough is off and byte order a by default; only a real true and b change them (ADR 0049)', () => {
  assert.equal(DSD_DEFAULT_NATIVE_PASSTHROUGH, false);
  assert.equal(DSD_DEFAULT_NATIVE_BYTE_ORDER, 'a');
  assert.equal(normalizeDsdSettings({}).nativePassthrough, false, 'an old file has no key');
  assert.equal(normalizeDsdSettings({ nativePassthrough: true }).nativePassthrough, true);
  assert.equal(normalizeDsdSettings({ nativePassthrough: 'yes' }).nativePassthrough, false);
  assert.equal(normalizeDsdSettings({ nativePassthrough: 1 }).nativePassthrough, false);
  assert.equal(normalizeDsdSettings({ nativeByteOrder: 'b' }).nativeByteOrder, 'b');
  assert.equal(normalizeDsdSettings({ nativeByteOrder: ' B ' }).nativeByteOrder, 'b');
  assert.equal(normalizeDsdSettings({ nativeByteOrder: 'x' }).nativeByteOrder, 'a');
  assert.equal(normalizeDsdSettings({ nativeByteOrder: 3 }).nativeByteOrder, 'a');
  // The three settings are independent.
  assert.deepEqual(normalizeDsdSettings({ dop: true, nativePassthrough: true, nativeByteOrder: 'b' }), {
    gainDb: 3,
    outputRateHz: 176_400,
    dop: true,
    nativePassthrough: true,
    nativeByteOrder: 'b',
  });
});

test('DSD options and labels', () => {
  assert.deepEqual([DSD_GAIN_MIN_DB, DSD_GAIN_MAX_DB, DSD_GAIN_STEP_DB], [0, 6, 0.5]);
  assert.equal(formatDsdGain(0), '0 dB');
  assert.deepEqual([...DSD_OUTPUT_RATES_HZ], [176_400, 352_800]);
  assert.equal(formatDsdGain(3), '+3 dB');
  assert.equal(formatDsdGain(2.5), '+2.5 dB');
  assert.equal(formatDsdOutputRate(176_400), '176.4 kHz');
  assert.equal(formatDsdOutputRate(352_800), '352.8 kHz');
  assert.equal(formatDsdName(2_822_400), 'DSD64');
  assert.equal(formatDsdName(11_289_600), 'DSD256');
});
