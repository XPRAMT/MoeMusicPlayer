import assert from 'node:assert/strict';
import test from 'node:test';

import {
  activePreset,
  buildDeviceRows,
  channelLabels,
  defaultEntry,
  disconnectedPlayingRows,
  sameDeviceList,
  takeoverRow,
  entriesFromRows,
  isLastPlaying,
  isMainStream,
  nudgeCell,
  sameDevices,
  sourceRowCount,
  statusFor,
  shortDeviceName,
  toggleCell,
  volumeDeviceId,
  mainRowIndex,
  normalizeEntry,
  patchRow,
  playingCount,
  rowEnabled,
  routePresets,
  selectOnly,
  setRowEnabled,
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
  assert.equal('volumePercent' in entry, false, 'the software volume is gone; the device volume is the real one');
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

test('choosing a device turns the system default off and choosing the default turns the devices off', () => {
  let rows = buildDeviceRows([], [speakers, earbuds]);
  rows = setRowEnabled(rows, 1, true);
  assert.deepEqual(rows.map((_, index) => rowEnabled(rows, index)), [false, true, false]);
  rows = setRowEnabled(rows, 2, true);
  assert.deepEqual(rows.map((_, index) => rowEnabled(rows, index)), [false, true, true]);
  rows = setRowEnabled(rows, 0, true);
  assert.deepEqual(rows.map((_, index) => rowEnabled(rows, index)), [true, false, false]);
});

test('matrix cells toggle one source per output channel and the gain only moves active cells', () => {
  let map = toggleCell(null, 2, 1, 0);
  assert.deepEqual(map, [{ source: 1, gainPercent: 100 }, { source: 1, gainPercent: 100 }]);
  map = toggleCell(map, 2, 1, 0);
  assert.deepEqual(map[0], { source: null, gainPercent: 100 });
  map = toggleCell(map, 2, 0, 0);
  assert.deepEqual(map[0], { source: 0, gainPercent: 100 });
  assert.equal(nudgeCell(map, 2, 1, 0, -5), null, 'a cell that is off does not change');
  assert.equal(nudgeCell(map, 2, 0, 0, -30)[0].gainPercent, 70);
  assert.equal(nudgeCell(map, 2, 0, 0, 500)[0].gainPercent, 100);
  assert.equal(nudgeCell(map, 2, 0, 0, -500)[0].gainPercent, 0);
});

test('the matrix shows the stereo pair, more when the map needs them, all eight when expanded', () => {
  assert.equal(sourceRowCount(null, false), 2);
  assert.equal(sourceRowCount([{ source: 0, gainPercent: 100 }], false), 2);
  assert.equal(sourceRowCount([{ source: 4, gainPercent: 100 }], false), 5);
  assert.equal(sourceRowCount(null, true), 8);
});

test('presets match the current choice ignoring remembered names and order', () => {
  const a = { ...defaultEntry('x', 'Old name'), delayMs: 100 };
  const b = defaultEntry(null, '');
  assert.equal(sameDevices([a, b], [b, { ...a, name: 'New name' }]), true);
  assert.equal(sameDevices([a], [{ ...a, delayMs: 110 }]), false);
  assert.equal(sameDevices(null, null), true);
  assert.equal(sameDevices(null, [a]), false);
});

test('the main stream is the first reported device', () => {
  const statuses = [
    { id: null, state: 'playing', message: null, deviceRateHz: 96000, channels: 2, converted: false },
    { id: 'x', state: 'lost', message: 'gone', deviceRateHz: null, channels: null, converted: false },
  ];
  assert.equal(isMainStream(statuses, null), true);
  assert.equal(isMainStream(statuses, 'x'), false);
  assert.equal(statusFor(statuses, 'x').message, 'gone');
  assert.equal(statusFor(statuses, 'y'), null);
  assert.equal(isMainStream([], null), false);
});

test('short device names use the text in the last parentheses', () => {
  assert.equal(shortDeviceName('Headphones (WF-1000XM6)'), 'WF-1000XM6');
  assert.equal(shortDeviceName('Realtek Digital Output (Realtek(R) Audio)'), 'Realtek Digital Output (Realtek(R) Audio)', 'nested parentheses keep the whole name');
  assert.equal(shortDeviceName('Plain name'), 'Plain name');
});

test('the system default row reads the volume of the device that is the default now', () => {
  const rows = buildDeviceRows([], [speakers, earbuds]);
  assert.equal(volumeDeviceId(rows[0], [speakers, earbuds]), speakers.id);
  assert.equal(volumeDeviceId(rows[2], [speakers, earbuds]), earbuds.id);
  assert.equal(volumeDeviceId(rows[0], [earbuds]), null);
});

test('exclusive mode keeps exactly one device playing', () => {
  let rows = buildDeviceRows([], [speakers, earbuds]);
  rows = setRowEnabled(rows, 1, true);
  rows = setRowEnabled(rows, 2, true);
  assert.equal(playingCount(rows), 2);
  const single = selectOnly(rows, 2);
  assert.deepEqual(single.map((_, index) => rowEnabled(single, index)), [false, false, true]);
  const first = selectOnly(single, 0);
  assert.deepEqual(first.map((_, index) => rowEnabled(first, index)), [true, false, false]);
});

test('a playing device that disappeared is found, and the new default takes over', () => {
  const saved = [{ ...defaultEntry(earbuds.id, 'Earbuds'), enabled: true }, { ...defaultEntry(null, ''), enabled: false }];
  // The earbuds are connected and playing.
  let rows = buildDeviceRows(saved, [speakers, earbuds]);
  assert.deepEqual(disconnectedPlayingRows(rows), []);
  assert.equal(takeoverRow(rows), -1);
  // They disconnect; Windows makes the speakers the default.
  rows = buildDeviceRows(saved, [speakers]);
  const missing = disconnectedPlayingRows(rows);
  assert.equal(missing.length, 1);
  assert.equal(rows[missing[0]].id, earbuds.id);
  assert.equal(rows[takeoverRow(rows)].id, speakers.id);
  // A saved device that is not meant to play does not count.
  const idle = [{ ...defaultEntry(earbuds.id, 'Earbuds'), enabled: false }, { ...defaultEntry(null, ''), enabled: true }];
  assert.deepEqual(disconnectedPlayingRows(buildDeviceRows(idle, [speakers])), []);
});

test('a device poll that found nothing new is recognised as the same list', () => {
  assert.equal(sameDeviceList([speakers, earbuds], [{ ...speakers }, { ...earbuds }]), true);
  assert.equal(sameDeviceList([speakers, earbuds], [speakers]), false);
  assert.equal(sameDeviceList([speakers], [{ ...speakers, isDefault: false }]), false);
});
