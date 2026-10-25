import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import './helpers/untr-fs.mjs';
import { formatBitrateKbps, formatDetailsBitrate, formatDetailsDate, formatDetailsDuration, formatDetailsSampleRate } from '../src/lib/track-details-format.js';

test('details panel dates show seconds and the UTC offset', () => {
  const ms = Date.UTC(2026, 3, 29, 12, 59, 41, 999);
  assert.equal(formatDetailsDate(ms, { offsetMinutes: 480 }), '2026-04-29 20:59:41 (UTC+8)');
  assert.equal(formatDetailsDate(Date.UTC(2026, 0, 2, 3, 4, 5), { offsetMinutes: 0 }), '2026-01-02 03:04:05 (UTC+0)');
  assert.equal(formatDetailsDate(Date.UTC(2026, 0, 1, 0, 0, 7), { offsetMinutes: 330 }), '2026-01-01 05:30:07 (UTC+5:30)');
  assert.equal(formatDetailsDate(Date.UTC(2026, 0, 1, 2, 0, 9), { offsetMinutes: -210 }), '2025-12-31 22:30:09 (UTC-3:30)');
  assert.equal(formatDetailsDate(Date.UTC(2026, 0, 1, 0, 0, 0), { offsetMinutes: 345 }), '2026-01-01 05:45:00 (UTC+5:45)');
  assert.match(formatDetailsDate(ms), /^2026-04-\d\d \d\d:\d\d:41 \(UTC[+-]\d+(:\d\d)?\)$/);
  for (const bad of [null, undefined, Number.NaN]) assert.equal(formatDetailsDate(bad), '—');
});

test('absolute times (NTFS, internal storage, SMB, WebDAV) show local time without the zone suffix', () => {
  const ms = Date.UTC(2026, 9, 22, 18, 16, 58, 400);
  assert.equal(formatDetailsDate(ms, { zone: false, offsetMinutes: 480 }), '2026-10-23 02:16:58');
  assert.equal(formatDetailsDate(ms, { zone: true, offsetMinutes: 480 }), '2026-10-23 02:16:58 (UTC+8)');
  assert.match(formatDetailsDate(ms, { zone: false }), /^2026-10-2\d \d\d:\d\d:58$/);
  assert.equal(formatDetailsDate(null, { zone: false }), '—');
});

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
  // Both times follow the same backend flag.
  assert.match(dialog, /formatDetailsDate\(details\.modifiedUnixMs, \{ zone: !details\.timesAbsolute \}\)/);
  assert.match(dialog, /formatDetailsDate\(details\.createdUnixMs, \{ zone: !details\.timesAbsolute \}\)/);
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
