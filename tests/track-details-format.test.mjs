import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import './helpers/untr-fs.mjs';
import { formatBitrateKbps, formatDetailsBitrate, formatDetailsDuration, formatDetailsSampleRate } from '../src/lib/track-details-format.js';

test('shared bitrate rule: whole kbps below 1000, Mbps (/1000, trailing zeros trimmed) from 1000 up', () => {
  assert.equal(formatBitrateKbps(320, 1), '320 kbps');
  assert.equal(formatBitrateKbps(999.4, 1), '999 kbps');
  assert.equal(formatBitrateKbps(1000, 1), '1 Mbps');
  assert.equal(formatBitrateKbps(1000, 3), '1 Mbps');
  assert.equal(formatBitrateKbps(1411.2, 1), '1.4 Mbps');
  assert.equal(formatBitrateKbps(1411.2, 3), '1.411 Mbps');
  assert.equal(formatBitrateKbps(5644.8, 1), '5.6 Mbps');
  assert.equal(formatBitrateKbps(2000, 1), '2 Mbps');
  for (const bad of [null, undefined, 0, -5, Number.NaN, Number.POSITIVE_INFINITY]) assert.equal(formatBitrateKbps(bad, 1), '—');
});

test('details panel bitrate uses the shared rule with up to three Mbps decimals', () => {
  assert.equal(formatDetailsBitrate(320), '320 kbps');
  assert.equal(formatDetailsBitrate(1000), '1 Mbps');
  assert.equal(formatDetailsBitrate(1024), '1.024 Mbps');
  assert.equal(formatDetailsBitrate(1030), '1.03 Mbps');
  assert.equal(formatDetailsBitrate(1411.2), '1.411 Mbps');
  assert.equal(formatDetailsBitrate(4608), '4.608 Mbps');
  assert.equal(formatDetailsBitrate(null), '—');
  assert.equal(formatDetailsBitrate(0), '—');
});

test('sample rate shows kHz, or MHz from 1 MHz up', () => {
  assert.equal(formatDetailsSampleRate(44100), '44.1 kHz');
  assert.equal(formatDetailsSampleRate(48000), '48 kHz');
  assert.equal(formatDetailsSampleRate(88200), '88.2 kHz');
  assert.equal(formatDetailsSampleRate(22050), '22.05 kHz');
  assert.equal(formatDetailsSampleRate(192000), '192 kHz');
  assert.equal(formatDetailsSampleRate(2_822_400), '2.8224 MHz');
  assert.equal(formatDetailsSampleRate(5_644_800), '5.6448 MHz');
  assert.equal(formatDetailsSampleRate(undefined), '—');
});

test('the details panel uses both helpers', () => {
  const dialog = readFileSync(new URL('../src/lib/TrackDetailsDialog.svelte', import.meta.url), 'utf8');
  assert.match(dialog, /formatDetailsBitrate\(/);
  assert.match(dialog, /formatDetailsSampleRate\(/);
});

test('duration shows a tenth of a second', () => {
  assert.equal(formatDetailsDuration(200), '0:00.2');
  assert.equal(formatDetailsDuration(211_400), '3:31.4');
  assert.equal(formatDetailsDuration(211_960), '3:32.0');
  assert.equal(formatDetailsDuration(59_950), '1:00.0');
  assert.equal(formatDetailsDuration(0), '0:00.0');
  assert.equal(formatDetailsDuration(3_723_456), '1:02:03.5');
  assert.equal(formatDetailsDuration(undefined), '—');
});
