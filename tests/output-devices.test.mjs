import assert from 'node:assert/strict';
import test from 'node:test';

import {
  activePreset,
  buildDeviceRows,
  channelLabels,
  defaultEntry,
  entriesFromRows,
  isLastPlaying,
  mainRowIndex,
  normalizeEntry,
  patchRow,
  playingCount,
  rowEnabled,
  routePresets,
  standardRoutes,
} from '../src/lib/output-devices.js';

const speakers = { id: 'wasapi:{1}', name: 'Speakers', isDefault: true, channels: 2, sampleRate: 48000 };
const earbuds = { id: 'wasapi:{2}', name: 'Earbuds', isDefault: false, channels: 2, sampleRate: 44100 };

test('with nothing saved the system default row plays alone', () => {
  const rows = buildDeviceRows([], [speakers, earbuds]);
  assert.deepEqual(rows.map((row) => row.key), ['default', 'wasapi:{1}', 'wasapi:{2}']);
  assert.equal(rows[0].name, 'Speakers');
  assert.deepEqual(rows.map((_, index) => rowEnabled(rows, index)), [true, false, false]);
  assert.equal(mainRowIndex(rows), 0);
  assert.equal(playingCount(rows), 1);
  assert.equal(isLastPlaying(rows, 0), true);
});

test('enabling a second device keeps the first as the main output and the list order decides', () => {
  let rows = buildDeviceRows([], [speakers, earbuds]);
  rows = patchRow(rows, 2, { enabled: true, delayMs: 180 });
  assert.deepEqual(rows.map((_, index) => rowEnabled(rows, index)), [true, false, true]);
  assert.equal(mainRowIndex(rows), 0);
  assert.equal(isLastPlaying(rows, 0), false);
  const saved = entriesFromRows(rows);
  assert.deepEqual(saved.map((entry) => [entry.id, entry.enabled, entry.delayMs, entry.name]), [[null, true, 0, ''], ['wasapi:{2}', true, 180, 'Earbuds']]);
  // With only the earbuds enabled, the earbuds become the main output.
  rows = patchRow(rows, 0, { enabled: false });
  assert.equal(isLastPlaying(rows, 2), true);
  const reloaded = buildDeviceRows(entriesFromRows(rows), [speakers, earbuds]);
  assert.equal(mainRowIndex(reloaded), 2);
  assert.equal(rowEnabled(reloaded, 0), false);
});

test('a saved device that is unplugged stays in the list under its remembered name', () => {
  const entries = [{ ...defaultEntry('wasapi:{9}', 'Old DAC'), delayMs: 40 }];
  const rows = buildDeviceRows(entries, [speakers]);
  const missing = rows.at(-1);
  assert.equal(missing.connected, false);
  assert.equal(missing.name, 'Old DAC');
  assert.equal(missing.entry.delayMs, 40);
});

test('entries are repaired like the backend does', () => {
  const entry = normalizeEntry({ id: '', volumePercent: 900, delayMs: -4, channelMap: [{ source: 300, gainPercent: 250 }, { source: 1 }] });
  assert.equal(entry.id, null);
  assert.equal(entry.volumePercent, 100);
  assert.equal(entry.delayMs, 0);
  assert.deepEqual(entry.channelMap, [{ source: null, gainPercent: 100 }, { source: 1, gainPercent: 100 }]);
  assert.equal(normalizeEntry(null).enabled, true);
});

test('channel presets are recognised and cover stereo and surround devices', () => {
  assert.deepEqual(channelLabels(2), ['FL', 'FR']);
  assert.equal(channelLabels(6)[3], 'LFE');
  assert.deepEqual(channelLabels(12).length, 12);
  const stereo = routePresets(2);
  assert.deepEqual(stereo.map((preset) => preset.id), ['standard', 'swap', 'left', 'right']);
  assert.equal(activePreset(2, standardRoutes(2)), 'standard');
  assert.equal(activePreset(2, stereo[1].routes), 'swap');
  assert.equal(activePreset(2, null), null);
  assert.equal(activePreset(2, [{ source: 0, gainPercent: 50 }, { source: 1, gainPercent: 100 }]), null);
  assert.deepEqual(routePresets(1).map((preset) => preset.id), ['standard']);
  assert.equal(standardRoutes(10)[9].source, null, 'channels past the 8 nameable ones stay silent');
});
