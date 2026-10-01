import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DEFAULT_TRACK_COLUMN_PREFERENCES,
  formatAudioFormat,
  formatTrackColumnValue,
  formatYear,
  isHiResTrack,
  moveTrackColumn,
  normalizeTrackColumnPreferences,
  nowPlayingLayoutOrder,
  setTrackColumnVisibility,
  trackListColumnCount,
  trackListGridTemplate,
  visibleTrackColumns,
} from '../src/lib/track-columns.js';

const formatDuration = (value) => `${Math.floor(value / 60_000)}:${String(Math.floor(value / 1000) % 60).padStart(2, '0')}`;

test('default list columns are shared and keep index/play as fixed extra columns', () => {
  assert.deepEqual(DEFAULT_TRACK_COLUMN_PREFERENCES.map(({ id }) => id), [
    'title', 'artist', 'album', 'year', 'audioFormat', 'duration',
  ]);
  assert.equal(trackListColumnCount(DEFAULT_TRACK_COLUMN_PREFERENCES), 8);
  assert.equal(trackListGridTemplate(DEFAULT_TRACK_COLUMN_PREFERENCES).split(' ').length, 8);
});

test('column order can move the first and last information columns without moving fixed actions', () => {
  const movedFirst = moveTrackColumn(DEFAULT_TRACK_COLUMN_PREFERENCES, 'title', 'down');
  assert.deepEqual(movedFirst.map(({ id }) => id), [
    'artist', 'title', 'album', 'year', 'audioFormat', 'duration',
  ]);
  const movedLast = moveTrackColumn(movedFirst, 'duration', 'up');
  assert.deepEqual(movedLast.map(({ id }) => id), [
    'artist', 'title', 'album', 'year', 'duration', 'audioFormat',
  ]);
  assert.deepEqual(moveTrackColumn(movedLast, 'artist', 'up'), movedLast);
  assert.deepEqual(moveTrackColumn(movedLast, 'audioFormat', 'down'), movedLast);
  assert.equal(trackListGridTemplate(movedLast).split(' ').at(0), '42px');
  assert.equal(trackListGridTemplate(movedLast).split(' ').at(-1), '42px');
});

test('visibility omits any information column from rows, skeletons, header count and grid template', () => {
  let preferences = DEFAULT_TRACK_COLUMN_PREFERENCES.map((column) => ({ ...column }));
  for (const id of ['title', 'artist', 'album', 'year', 'audioFormat', 'duration']) {
    preferences = setTrackColumnVisibility(preferences, id, false);
    const visible = visibleTrackColumns(preferences);
    assert.equal(visible.some((column) => column.id === id), false);
    assert.equal(trackListColumnCount(preferences), visible.length + 2);
    assert.equal(trackListGridTemplate(preferences).split(' ').length, visible.length + 2);
  }
  assert.equal(trackListColumnCount(preferences), 2);
});

test('malformed stored column settings safely fall back to defaults', () => {
  assert.deepEqual(normalizeTrackColumnPreferences(null), DEFAULT_TRACK_COLUMN_PREFERENCES);
  assert.deepEqual(normalizeTrackColumnPreferences([{ id: 'title', visible: true }]), DEFAULT_TRACK_COLUMN_PREFERENCES);
  assert.deepEqual(normalizeTrackColumnPreferences([
    ...DEFAULT_TRACK_COLUMN_PREFERENCES,
    { id: 'artist', visible: false },
  ]), DEFAULT_TRACK_COLUMN_PREFERENCES);
});

test('YEAR displays only YYYY or valid YYYY-MM-DD and never accepts other date sources', () => {
  assert.equal(formatYear(2024), '2024');
  assert.equal(formatYear('2024'), '2024');
  assert.equal(formatYear('2024-02-29'), '2024');
  assert.equal(formatYear('2023-02-29'), '—');
  assert.equal(formatYear('2024-13-01'), '—');
  assert.equal(formatYear('2024/01/02'), '—');
  assert.equal(formatYear('not a year'), '—');
  assert.equal(formatYear(null), '—');
});

test('audio format uses only supplied source fields and formats nullable values', () => {
  assert.equal(formatAudioFormat({
    codec: 'flac', sampleRateHz: 48_000, bitDepth: 16, bitrateBps: 1_024_000,
  }), 'FLAC．48 kHz．16 bit．1024 kbps');
  assert.equal(formatAudioFormat({
    codec: 'flac', sampleRateHz: 48_000, bitDepth: 24, bitrateBps: 1_989_000,
  }), 'FLAC．48 kHz．24 bit．1989 kbps');
  assert.equal(formatAudioFormat({ codec: null, sampleRateHz: null, bitDepth: null, bitrateBps: null }), '—');
  assert.equal(formatAudioFormat({ codec: 'mp3', sampleRateHz: 44_100, bitDepth: null, bitrateBps: 192_000 }), 'MP3．44.1 kHz．192 kbps');
});

test('Hi-Res badge requires at least 48 kHz sample rate and 24-bit source depth', () => {
  assert.equal(isHiResTrack({ sampleRateHz: 48_000, bitDepth: 24 }), true);
  assert.equal(isHiResTrack({ sampleRateHz: 96_000, bitDepth: 24 }), true);
  assert.equal(isHiResTrack({ sampleRateHz: 44_100, bitDepth: 24 }), false);
  assert.equal(isHiResTrack({ sampleRateHz: 48_000, bitDepth: 16 }), false);
  assert.equal(isHiResTrack({ sampleRateHz: 48_000, bitDepth: null }), false);
  assert.equal(isHiResTrack({ sampleRateHz: null, bitDepth: 24 }), false);
  assert.equal(isHiResTrack(null), false);
});

test('playlist entries show placeholders for missing metadata without an artist subtitle', () => {
  const missing = {
    title: null,
    artist: null,
    album: null,
    year: null,
    codec: null,
    sampleRateHz: null,
    bitDepth: null,
    bitrateBps: null,
    durationMs: null,
  };
  for (const id of ['artist', 'album', 'year', 'audioFormat', 'duration']) {
    assert.equal(formatTrackColumnValue(id, missing, formatDuration, '未命名項目'), '—');
  }
  assert.equal(formatTrackColumnValue('title', missing, formatDuration, '未命名項目'), '未命名項目');
  assert.equal(formatTrackColumnValue('artist', { ...missing, artist: 'hanser' }, formatDuration), 'hanser');
});

test('Now Playing A and B keep artwork/lyrics order when stacked', () => {
  assert.deepEqual(nowPlayingLayoutOrder('a'), ['artwork', 'lyrics']);
  assert.deepEqual(nowPlayingLayoutOrder('b'), ['lyrics', 'artwork']);
  assert.deepEqual(nowPlayingLayoutOrder(undefined), ['artwork', 'lyrics']);
});
