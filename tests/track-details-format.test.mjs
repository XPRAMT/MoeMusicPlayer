import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import './helpers/untr-fs.mjs';
import { formatDetailsBitrate, formatDetailsSampleRate } from '../src/lib/track-details-format.js';

test('bitrate shows kbps up to 1024 and Mbps above it', () => {
  assert.equal(formatDetailsBitrate(320), '320 kbps');
  assert.equal(formatDetailsBitrate(1024), '1024 kbps');
  assert.equal(formatDetailsBitrate(1030), '1.03 Mbps');
  assert.equal(formatDetailsBitrate(1411), '1.41 Mbps');
  assert.equal(formatDetailsBitrate(4608), '4.61 Mbps');
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
