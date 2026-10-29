import './helpers/untr-fs.mjs';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import {
  computeNumericColumnWidthPx, DEFAULT_TRACK_COLUMN_PREFERENCES, DEFAULT_TRACK_COLUMN_WIDTHS, formatCreatedAt, formatPlayCount,
  formatReleaseDate, NUMERIC_COLUMN_SAMPLE_PATTERNS, numericColumnSample, SOURCE_CELL_PADDING_PX, SOURCE_HEADER_INDICATOR_PX,
  trackListGridTemplate, trackListMinWidthPx, visibleTrackColumns, widestDigit,
} from '../src/lib/track-columns.js';
import { formatDuration, formatDurationHours } from '../src/lib/format.ts';
import { computeNumericColumnWidths } from '../src/lib/numeric-column-widths.js';

// A proportional-digit font: '1' is narrow, '8' widest.
const digitWidth = { 0: 7, 1: 4, 2: 7, 3: 7, 4: 7, 5: 7, 6: 7, 7: 6, 8: 9, 9: 7 };
const measure = (text) => [...text].reduce((sum, ch) => sum + (digitWidth[ch] ?? 6), 0);

test('the sample uses the widest digit of the font, measured once per digit', () => {
  let calls = 0;
  assert.equal(widestDigit((text) => { calls += 1; return measure(text); }), '8');
  assert.equal(calls, 10);
  assert.equal(numericColumnSample('0000-00-00', '8'), '8888-88-88');
});

test('samples match the real display formats', () => {
  const shape = (value) => value.replace(/[0-9]/g, '0');
  assert.equal(shape(formatReleaseDate('2024-03-05')), NUMERIC_COLUMN_SAMPLE_PATTERNS.year);
  assert.equal(shape(formatCreatedAt(Date.UTC(2024, 2, 5, 12))), NUMERIC_COLUMN_SAMPLE_PATTERNS.createdAt);
  assert.equal(shape(formatDuration(123 * 60_000 + 4_000)), NUMERIC_COLUMN_SAMPLE_PATTERNS.duration);
  assert.equal(shape(formatPlayCount(1234.5 * 1000, 1000)), NUMERIC_COLUMN_SAMPLE_PATTERNS.playCount);
  assert.equal(shape(formatDurationHours(123 * 3_600_000)), NUMERIC_COLUMN_SAMPLE_PATTERNS.hours);
  assert.equal(shape((12_345).toLocaleString()), shape(NUMERIC_COLUMN_SAMPLE_PATTERNS.count));
});

test('width is the wider of the widest sample and the header with its sort arrow', () => {
  assert.equal(computeNumericColumnWidthPx('000:00', 'x', '8', measure, 52), 9 * 5 + 6 + SOURCE_CELL_PADDING_PX);
  assert.equal(computeNumericColumnWidthPx('0', 'Release date', '8', measure, 52), 12 * 6 + SOURCE_HEADER_INDICATOR_PX + SOURCE_CELL_PADDING_PX);
  assert.equal(computeNumericColumnWidthPx('0', 'x', '8', () => Number.NaN, 52), 52);
});

test('every numeric column of the track lists and the artist index gets a width; a font change measures again', () => {
  const widths = computeNumericColumnWidths(measure);
  for (const key of ['year', 'createdAt', 'modifiedAt', 'duration', 'playCount', 'artist:trackCount', 'artist:albumCount', 'artist:played', 'artist:trackDuration']) {
    assert.ok(widths[key] > 0, key);
  }
  const wider = computeNumericColumnWidths((text) => measure(text) * 1.5);
  assert.ok(wider.year > widths.year);
});

test('measured numeric widths replace the fixed em widths in the grid and the minimum width', () => {
  const columns = DEFAULT_TRACK_COLUMN_PREFERENCES.map((column) => ({ ...column, visible: true }));
  const index = visibleTrackColumns(columns).findIndex(({ id }) => id === 'duration') + 1;
  assert.equal(trackListGridTemplate(columns, DEFAULT_TRACK_COLUMN_WIDTHS).split(' ')[index], '4em');
  assert.equal(trackListGridTemplate(columns, DEFAULT_TRACK_COLUMN_WIDTHS, 84, 170, { duration: 47.2 }).split(' ')[index], '48px');
  const base = trackListMinWidthPx(columns, DEFAULT_TRACK_COLUMN_WIDTHS, 84, 170, { duration: 52 });
  assert.equal(trackListMinWidthPx(columns, DEFAULT_TRACK_COLUMN_WIDTHS, 84, 170, { duration: 62 }) - base, 10);
});

test('the artist index uses the measured widths', async () => {
  const source = await readFile(new URL('../src/lib/ArtistIndex.svelte', import.meta.url), 'utf8');
  assert.ok(source.includes('$numericColumnWidths[key]'));
  assert.ok(!source.includes('6em 6em 6em 6em'));
});
