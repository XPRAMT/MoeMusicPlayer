import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import {
  DEFAULT_LIBRARY_SORT,
  DEFAULT_TRACK_COLUMN_PREFERENCES,
  DEFAULT_TRACK_COLUMN_WIDTHS,
  TRACK_COLUMN_DEFINITIONS,
  formatAudioFormat,
  formatPlayCount,
  displayTrackTitle,
  formatReleaseDate,
  formatTrackColumnValue,
  formatYear,
  isHiResTrack,
  isLosslessCodec,
  moveTrackColumn,
  normalizeLibrarySort,
  normalizeTrackColumnPreferences,
  normalizeTrackColumnWidths,
  nowPlayingLayoutOrder,
  setTrackColumnVisibility,
  setTrackColumnWidth,
  toggleLibrarySort,
  trackListColumnCount,
  trackListGridTemplate,
  visibleTrackColumns,
} from '../src/lib/track-columns.js';

const formatDuration = (value) => `${Math.floor(value / 60_000)}:${String(Math.floor(value / 1000) % 60).padStart(2, '0')}`;

test('list headers use only the shared bottom boundary and sort buttons fill their cells', async () => {
  const [appCss, trackList, columnCells, virtualList] = await Promise.all([
    readFile(new URL('../src/app.css', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/TrackList.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/TrackColumnCells.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/PagedVirtualList.svelte', import.meta.url), 'utf8'),
  ]);
  const tableHeader = appCss.match(/\.track-table-head\s*\{([^}]+)\}/)?.[1] ?? '';
  const listHeader = trackList.match(/:global\(\.track-list-header\)\s*\{([^}]+)\}/)?.[1] ?? '';
  const sortButton = columnCells.match(/\.column-sort-button\s*\{([^}]+)\}/)?.[1] ?? '';
  const virtualHeaderLine = virtualList.match(/\.paged-virtual-header::after\s*\{([^}]+)\}/)?.[1] ?? '';

  assert.doesNotMatch(tableHeader, /background\s*:/);
  assert.doesNotMatch(tableHeader, /border-bottom\s*:/);
  assert.doesNotMatch(listHeader, /background\s*:/);
  assert.match(virtualHeaderLine, /bottom:\s*0/);
  assert.match(virtualHeaderLine, /height:\s*1px/);
  assert.match(virtualHeaderLine, /background:\s*var\(--line\)/);
  assert.match(sortButton, /width:\s*100%/);
  assert.match(sortButton, /height:\s*100%/);
});

test('default list columns are shared and keep the index as the only fixed column', () => {
  assert.deepEqual(DEFAULT_TRACK_COLUMN_PREFERENCES.map(({ id }) => id), [
    'title', 'artist', 'album', 'year', 'audioFormat', 'duration', 'playCount',
  ]);
  assert.equal(trackListColumnCount(DEFAULT_TRACK_COLUMN_PREFERENCES), 8);
  assert.equal(trackListGridTemplate(DEFAULT_TRACK_COLUMN_PREFERENCES).split(' ').length, 8);
  assert.equal(trackListGridTemplate(DEFAULT_TRACK_COLUMN_PREFERENCES, DEFAULT_TRACK_COLUMN_WIDTHS).split(' ')[1], '220px');
});

test('flexible column widths clamp and feed the shared grid template', () => {
  const widths = setTrackColumnWidth(DEFAULT_TRACK_COLUMN_WIDTHS, 'artist', 12);
  assert.equal(widths.artist, 80);
  const wide = setTrackColumnWidth(widths, 'title', 1200);
  assert.equal(wide.title, 800);
  assert.deepEqual(normalizeTrackColumnWidths(null), DEFAULT_TRACK_COLUMN_WIDTHS);
  assert.match(trackListGridTemplate(DEFAULT_TRACK_COLUMN_PREFERENCES, wide), /^26px 800px /);
});

test('library sort toggles direction on the same field and resets on a new field', () => {
  assert.deepEqual(normalizeLibrarySort(null), DEFAULT_LIBRARY_SORT);
  const byArtist = toggleLibrarySort(DEFAULT_LIBRARY_SORT, 'artist');
  assert.deepEqual(byArtist, { field: 'artist', direction: 'asc' });
  assert.deepEqual(toggleLibrarySort(byArtist, 'artist'), { field: 'artist', direction: 'desc' });
  assert.deepEqual(toggleLibrarySort({ field: 'artist', direction: 'desc' }, 'album'), {
    field: 'album',
    direction: 'asc',
  });
});

test('column order can move the first and last information columns without moving the index', () => {
  const movedFirst = moveTrackColumn(DEFAULT_TRACK_COLUMN_PREFERENCES, 'title', 'down');
  assert.deepEqual(movedFirst.map(({ id }) => id), [
    'artist', 'title', 'album', 'year', 'audioFormat', 'duration', 'playCount',
  ]);
  const movedLast = moveTrackColumn(movedFirst, 'playCount', 'up');
  assert.deepEqual(movedLast.map(({ id }) => id), [
    'artist', 'title', 'album', 'year', 'audioFormat', 'playCount', 'duration',
  ]);
  assert.deepEqual(moveTrackColumn(movedLast, 'artist', 'up'), movedLast);
  assert.deepEqual(moveTrackColumn(movedLast, 'duration', 'down'), movedLast);
  assert.equal(trackListGridTemplate(movedLast).split(' ').at(0), '26px');
  assert.notEqual(trackListGridTemplate(movedLast).split(' ').at(-1), '42px');
});

test('visibility omits any information column from rows, skeletons, header count and grid template', () => {
  let preferences = DEFAULT_TRACK_COLUMN_PREFERENCES.map((column) => ({ ...column }));
  for (const id of ['title', 'artist', 'album', 'year', 'audioFormat', 'duration', 'playCount']) {
    preferences = setTrackColumnVisibility(preferences, id, false);
    const visible = visibleTrackColumns(preferences);
    assert.equal(visible.some((column) => column.id === id), false);
    assert.equal(trackListColumnCount(preferences), visible.length + 1);
    assert.equal(trackListGridTemplate(preferences).split(' ').length, visible.length + 1);
  }
  assert.equal(trackListColumnCount(preferences), 1);
});

test('malformed stored column settings safely fall back to defaults', () => {
  assert.deepEqual(normalizeTrackColumnPreferences(null), DEFAULT_TRACK_COLUMN_PREFERENCES);
  assert.deepEqual(normalizeTrackColumnPreferences([{ id: 'title', visible: true }]), DEFAULT_TRACK_COLUMN_PREFERENCES);
  assert.deepEqual(normalizeTrackColumnPreferences([
    ...DEFAULT_TRACK_COLUMN_PREFERENCES,
    { id: 'artist', visible: false },
  ]), DEFAULT_TRACK_COLUMN_PREFERENCES);
});

test('release date column keeps available YYYY / YYYY-MM / YYYY-MM-DD precision', () => {
  assert.equal(TRACK_COLUMN_DEFINITIONS.find((column) => column.id === 'year')?.label, '發行時間');
  assert.equal(formatReleaseDate('2024'), '2024');
  assert.equal(formatReleaseDate('2024-03'), '2024-03');
  assert.equal(formatReleaseDate('2024-02-29'), '2024-02-29');
  assert.equal(formatReleaseDate(null, 2024), '2024');
  assert.equal(formatReleaseDate('2023-02-29'), '—');
  assert.equal(formatReleaseDate('2024-13'), '—');
  assert.equal(formatReleaseDate('2024-13-01'), '—');
  assert.equal(formatReleaseDate('2024/01/02'), '—');
  assert.equal(formatYear(2024), '2024');
  assert.equal(formatTrackColumnValue('year', { releaseDate: '2024-06-01' }, formatDuration), '2024-06-01');
  assert.equal(formatTrackColumnValue('year', { year: 1999 }, formatDuration), '1999');
});

test('audio format uses only supplied source fields and formats nullable values', () => {
  assert.equal(isLosslessCodec('flac'), true);
  assert.equal(isLosslessCodec('mp3'), false);
  assert.equal(formatAudioFormat({
    codec: 'flac', sampleRateHz: 48_000, bitDepth: 16, bitrateBps: 1_024_000,
  }), 'FLAC．48 kHz．16 bit．1024 kbps');
  assert.equal(formatAudioFormat({
    codec: 'flac', sampleRateHz: 48_000, bitDepth: 24, bitrateBps: 1_989_000,
  }, { omitLosslessBitrate: true }), 'FLAC．48 kHz．24 bit');
  assert.equal(formatAudioFormat({
    codec: 'wav', sampleRateHz: 44_100, bitDepth: null, bitrateBps: 1_411_000,
  }, { omitLosslessBitrate: true }), 'WAV．44.1 kHz');
  assert.equal(formatAudioFormat({
    codec: 'wav', sampleRateHz: 48_000, bitDepth: 24, bitrateBps: 2_304_000,
  }, { omitLosslessBitrate: true }), 'WAV．48 kHz．24 bit');
  assert.equal(formatAudioFormat({ codec: null, sampleRateHz: null, bitDepth: null, bitrateBps: null }), '—');
  assert.equal(formatAudioFormat({ codec: 'mp3', sampleRateHz: 44_100, bitDepth: null, bitrateBps: 192_000 }), 'MP3．44.1 kHz．192 kbps');
  assert.equal(
    formatTrackColumnValue('audioFormat', {
      codec: 'flac', sampleRateHz: 96_000, bitDepth: 24, bitrateBps: 2_800_000,
    }, formatDuration),
    'FLAC．96 kHz．24 bit',
  );
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
  for (const id of ['artist', 'album', 'year', 'audioFormat', 'duration', 'playCount']) {
    assert.equal(formatTrackColumnValue(id, missing, formatDuration, '未命名項目'), '—');
  }
  assert.equal(formatTrackColumnValue('title', missing, formatDuration, '未命名項目'), '未命名項目');
  assert.equal(formatTrackColumnValue('artist', { ...missing, artist: 'hanser' }, formatDuration), 'hanser');
});

test('a missing or whitespace title shows the file name and not artist or album', () => {
  const untitled = {
    title: '   ',
    artist: 'hanser',
    album: 'album',
    fileName: '  曲 目  ',
  };
  assert.equal(displayTrackTitle(untitled, '未命名曲目'), '曲 目');
  assert.equal(formatTrackColumnValue('title', untitled, formatDuration), '曲 目');
  assert.equal(formatTrackColumnValue('title', untitled, formatDuration, '未命名項目'), '曲 目');
  assert.equal(formatTrackColumnValue('artist', untitled, formatDuration), 'hanser');
  assert.equal(
    formatTrackColumnValue('title', { title: null, artist: 'hanser', album: 'album', fileName: '   ' }, formatDuration, '未命名項目'),
    '未命名項目',
  );
  assert.equal(displayTrackTitle({ title: ' 晴天 ' }, '未命名曲目'), '晴天');
});

test('play count is played_ms / duration_ms with tidy precision', () => {
  assert.equal(formatPlayCount(130_000, 100_000), '1.3');
  assert.equal(formatPlayCount(200_000, 100_000), '2');
  assert.equal(formatPlayCount(0, 100_000), '0');
  assert.equal(formatPlayCount(50_000, 0), '—');
  assert.equal(formatPlayCount(50_000, null), '—');
  assert.equal(formatPlayCount(50_000, undefined), '—');
  assert.equal(formatPlayCount(undefined, 100_000), '0');
  assert.equal(formatTrackColumnValue('playCount', { playedMs: 130_000, durationMs: 100_000 }, formatDuration), '1.3');
  assert.equal(formatTrackColumnValue('playCount', { playedMs: 10, durationMs: null }, formatDuration), '—');
});

test('Now Playing A and B keep artwork/lyrics order when stacked', () => {
  assert.deepEqual(nowPlayingLayoutOrder('a'), ['artwork', 'lyrics']);
  assert.deepEqual(nowPlayingLayoutOrder('b'), ['lyrics', 'artwork']);
  assert.deepEqual(nowPlayingLayoutOrder(undefined), ['artwork', 'lyrics']);
});
