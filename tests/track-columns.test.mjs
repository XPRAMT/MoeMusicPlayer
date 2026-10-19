import './helpers/untr-fs.mjs';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import {
  DEFAULT_LIBRARY_SORT,
  DEFAULT_PLAYLIST_SORT,
  DEFAULT_TRACK_COLUMN_PREFERENCES,
  DEFAULT_TRACK_ROW_HEIGHT_PX,
  DEFAULT_TRACK_COLUMN_WIDTHS,
  TRACK_COLUMN_DEFINITIONS,
  formatAudioFormat,
  formatPlayCount,
  displayTrackTitle,
  formatReleaseDate,
  formatCreatedAt,
  formatTrackColumnValue,
  isNumericListColumn,
  formatYear,
  isDsdCodec,
  isHiResTrack,
  isLosslessCodec,
  moveTrackColumn,
  moveTrackColumnTo,
  normalizeLibrarySort,
  normalizePlaylistSort,
  normalizeTrackColumnPreferences,
  normalizeTrackColumnWidths,
  normalizeTrackRowHeightPx,
  nowPlayingLayoutOrder,
  setTrackColumnVisibility,
  setTrackColumnWidth,
  toggleLibrarySort,
  togglePlaylistSort,
  trackColumnDropIndexFromPointer,
  trackListColumnCount,
  trackListGridTemplate,
  visibleTrackColumns,
} from '../src/lib/track-columns.js';

const formatDuration = (value) => `${Math.floor(value / 60_000)}:${String(Math.floor(value / 1000) % 60).padStart(2, '0')}`;

test('DSD tracks show the DSD rate name, MHz and 1 bit, hide the bitrate and always get the Hi-Res badge', () => {
  const dsd64 = { codec: 'DSD64', sampleRateHz: 2_822_400, bitDepth: 1, bitrateBps: 5_644_800 };
  assert.equal(formatAudioFormat(dsd64), 'DSD64．2.8 MHz．1 bit．5645 kbps');
  assert.equal(formatAudioFormat(dsd64, { omitDsdBitDepth: true }), 'DSD64．2.8 MHz．5645 kbps');
  assert.equal(formatAudioFormat(dsd64, { omitLosslessBitrate: true }), 'DSD64．2.8 MHz．1 bit');
  assert.equal(formatAudioFormat({ codec: 'DSD256', sampleRateHz: 11_289_600, bitDepth: 1, bitrateBps: null }), 'DSD256．11.3 MHz．1 bit');
  assert.equal(isDsdCodec('dsd128'), true);
  assert.equal(isDsdCodec('DSD'), false);
  assert.equal(isDsdCodec('FLAC'), false);
  assert.equal(isLosslessCodec('DSD64'), true);
  assert.equal(isHiResTrack(dsd64), true);
  assert.equal(isHiResTrack({ codec: 'FLAC', sampleRateHz: 44_100, bitDepth: 16 }), false);
});

test('numeric list columns are centered while text columns keep their left alignment', async () => {
  assert.deepEqual(
    ['year', 'createdAt', 'duration', 'playCount', 'trackCount', 'albumCount', 'trackDuration', 'played']
      .filter(isNumericListColumn),
    ['year', 'createdAt', 'duration', 'playCount', 'trackCount', 'albumCount', 'trackDuration', 'played'],
  );
  assert.deepEqual(['title', 'artist', 'album', 'audioFormat'].filter(isNumericListColumn), []);

  const cells = await readFile(new URL('../src/lib/TrackColumnCells.svelte', import.meta.url), 'utf8');
  const appCss = await readFile(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.match(cells, /class:is-numeric=\{isNumericListColumn\(column\.id\)\}/);
  assert.match(cells, /class:is-audio-format-header=\{column\.id === 'audioFormat'\}/);
  assert.match(cells, /class:is-numeric=\{isNumericListColumn\(cell\.id\)\}/);
  assert.match(cells, /class:is-audio-format-cell=\{cell\.id === 'audioFormat'\}/);
  assert.match(cells, /\.list-column\.is-numeric \.column-sort-button\s*\{\s*justify-content:\s*center;/);
  assert.match(cells, /\.list-column\.is-audio-format-header \.column-sort-button\s*\{\s*justify-content:\s*center;/);
  assert.match(cells, /\.list-column\.is-audio-format-cell\s*\{\s*text-align:\s*center;/);
  assert.match(cells, /\.column-index\.is-sortable \.column-sort-button\s*\{\s*justify-content:\s*center;/);
  assert.match(appCss, /\.list-column\.is-numeric\s*\{\s*text-align:\s*center;/);
  assert.match(appCss, /\.column-index,[\s\S]*?text-align:\s*center;/);
  for (const selector of ['list-column-year', 'list-column-duration', 'list-column-playCount']) {
    const rule = appCss.match(new RegExp(`\\.${selector}\\s*\\{([^}]+)\\}`))?.[1] ?? '';
    assert.doesNotMatch(rule, /text-align:\s*(?:left|right)/);
  }
});

test('audio format column is fixed at 163 CSS pixels', () => {
  const column = TRACK_COLUMN_DEFINITIONS.find(({ id }) => id === 'audioFormat');
  assert.deepEqual({ min: column?.min, minPx: column?.minPx }, { min: '163px', minPx: 163 });
  assert.equal(trackListGridTemplate(DEFAULT_TRACK_COLUMN_PREFERENCES).split(' ')[7], '163px');
});

test('list headers use only the shared bottom boundary and sort buttons fill their cells', async () => {
  const [appCss, trackList, columnCells, virtualList, albumIndex, playlistTree, app] = await Promise.all([
    readFile(new URL('../src/app.css', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/TrackList.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/TrackColumnCells.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/PagedVirtualList.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/AlbumIndex.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/PlaylistTree.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/App.svelte', import.meta.url), 'utf8'),
  ]);
  const tableHeader = appCss.match(/\.track-table-head\s*\{([^}]+)\}/)?.[1] ?? '';
  const listHeader = trackList.match(/:global\(\.track-list-header\)\s*\{([^}]+)\}/)?.[1] ?? '';
  const sortButton = columnCells.match(/\.column-sort-button\s*\{([^}]+)\}/)?.[1] ?? '';
  const virtualHeaderLine = appCss.match(/\.paged-virtual-header::after\s*\{([^}]+)\}/)?.[1] ?? '';
  const virtualFrame = virtualList.match(/\.paged-virtual-frame\s*\{([^}]+)\}/)?.[1] ?? '';
  const headerTextOffset = appCss.match(/\.track-table-head > \.column-index,[\s\S]*?\{([^}]+)\}/)?.[1] ?? '';

  assert.doesNotMatch(tableHeader, /background\s*:/);
  assert.doesNotMatch(tableHeader, /border-bottom\s*:/);
  assert.doesNotMatch(listHeader, /background\s*:/);
  assert.match(virtualHeaderLine, /bottom:\s*0/);
  assert.match(appCss, /--divider-width:\s*1px/);
  assert.match(virtualHeaderLine, /height:\s*0/);
  assert.match(virtualHeaderLine, /box-sizing:\s*content-box/);
  assert.match(virtualHeaderLine, /border-bottom:\s*var\(--divider-width\) solid var\(--divider-color\)/);
  assert.match(virtualHeaderLine, /background:\s*none/);
  assert.match(virtualFrame, /border:\s*1px solid var\(--line\)/);
  assert.doesNotMatch(albumIndex, /border-(?:top|bottom):\s*var\(--divider-width\)/);
  assert.match(playlistTree, /border-top:\s*var\(--divider-width\)/);
  assert.match(headerTextOffset, /top:\s*var\(--track-header-text-optical-offset,\s*-3\.25px\)/);
  assert.match(app, /-3\.25\s*\/\s*devicePixelRatio/);
  assert.match(app, /addEventListener\('resize', updateTrackHeaderTextOffset\)/);
  assert.doesNotMatch(app, /--track-header-line-height/);
  assert.match(sortButton, /width:\s*100%/);
  assert.match(sortButton, /height:\s*100%/);
});

test('default list columns are shared and keep the index as the only fixed column', () => {
  assert.deepEqual(DEFAULT_TRACK_COLUMN_PREFERENCES.map(({ id }) => id), [
    'title', 'artist', 'album', 'year', 'createdAt', 'duration', 'audioFormat', 'playCount',
  ]);
  assert.equal(trackListColumnCount(DEFAULT_TRACK_COLUMN_PREFERENCES), 9);
  assert.equal(trackListGridTemplate(DEFAULT_TRACK_COLUMN_PREFERENCES).split(' ').length, 9);
  assert.equal(trackListGridTemplate(DEFAULT_TRACK_COLUMN_PREFERENCES, DEFAULT_TRACK_COLUMN_WIDTHS).split(' ')[1], '184px');
});

test('flexible column widths clamp and feed the shared grid template', () => {
  const widths = setTrackColumnWidth(DEFAULT_TRACK_COLUMN_WIDTHS, 'artist', 12);
  assert.equal(widths.artist, 80);
  const wide = setTrackColumnWidth(widths, 'title', 1200);
  assert.equal(wide.title, 800);
  assert.deepEqual(normalizeTrackColumnWidths(null), DEFAULT_TRACK_COLUMN_WIDTHS);
  assert.match(trackListGridTemplate(DEFAULT_TRACK_COLUMN_PREFERENCES, wide), /^26px 800px /);
});

test('date, duration, and play count columns use the requested fixed fullwidth-character widths', () => {
  assert.deepEqual(
    TRACK_COLUMN_DEFINITIONS.filter(({ id }) => ['year', 'createdAt', 'duration', 'playCount'].includes(id))
      .map(({ id, min }) => [id, min]),
    [['year', '6em'], ['createdAt', '6em'], ['duration', '4em'], ['playCount', '5em']],
  );
});

test('settings columns can move directly to the long-press drag destination', () => {
  const moved = moveTrackColumnTo(DEFAULT_TRACK_COLUMN_PREFERENCES, 'year', 6);
  assert.deepEqual(moved.map(({ id }) => id), [
    'title', 'artist', 'album', 'createdAt', 'duration', 'audioFormat', 'year', 'playCount',
  ]);
  assert.deepEqual(moveTrackColumnTo(moved, 'year', -1), moved);
});

test('shared list row height defaults to 50px and clamps to the supported range', () => {
  assert.equal(DEFAULT_TRACK_ROW_HEIGHT_PX, 50);
  assert.equal(normalizeTrackRowHeightPx(undefined), 50);
  assert.equal(normalizeTrackRowHeightPx(30), 30);
  assert.equal(normalizeTrackRowHeightPx(60), 60);
  assert.equal(normalizeTrackRowHeightPx(12), 30);
  assert.equal(normalizeTrackRowHeightPx(75), 60);
  assert.equal(normalizeTrackRowHeightPx(Number.NaN), 50);
});

test('configured row height drives the three shared virtual lists and survives settings writes', async () => {
  const [app, css, ...lists] = await Promise.all([
    readFile(new URL('../src/App.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/app.css', import.meta.url), 'utf8'),
    ...['TrackList', 'PlaylistEntryList', 'PlaybackQueueList'].map((name) =>
      readFile(new URL(`../src/lib/${name}.svelte`, import.meta.url), 'utf8')),
  ]);
  assert.match(app, /min="30"\s+max="60"\s+step="1"/);
  assert.match(app, /rowHeightPx:\s*trackRowHeightPx/);
  assert.match(app, /trackRowHeightPx\s*=\s*normalizeTrackRowHeightPx\(stored\.rowHeightPx\)/);
  assert.ok([...app.matchAll(/rowHeightPx=\{trackRowHeightPx\}/g)].length >= 3);
  assert.match(app, /oninput=\{\(event\) => previewTrackRowHeight/);
  assert.match(app, /onchange=\{\(event\) => commitTrackRowHeight/);
  assert.match(css, /\.track-row\s*\{[^}]*min-height:\s*var\(--paged-row-height,\s*50px\)/s);
  assert.match(lists[0], /\.virtual-track-row\)\s*\{\s*min-height:\s*var\(--paged-row-height,\s*50px\)/);
  assert.equal(lists.length, 3);
  for (const list of lists) {
    assert.match(list, /rowHeightPx\s*=\s*DEFAULT_TRACK_ROW_HEIGHT_PX/);
    assert.match(list, /rowHeight=\{rowHeightPx\}/);
    assert.match(list, /compactRowHeight=\{rowHeightPx\}/);
    assert.match(list, /shortDesktopRowHeight=\{rowHeightPx\}/);
  }
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

test('playlist sort includes the original entry position and toggles direction', () => {
  assert.deepEqual(normalizePlaylistSort(null), DEFAULT_PLAYLIST_SORT);
  const byTitle = togglePlaylistSort(DEFAULT_PLAYLIST_SORT, 'title');
  assert.deepEqual(byTitle, { field: 'title', direction: 'asc' });
  assert.deepEqual(togglePlaylistSort(byTitle, 'title'), { field: 'title', direction: 'desc' });
  assert.deepEqual(togglePlaylistSort({ field: 'title', direction: 'desc' }, 'position'), {
    field: 'position',
    direction: 'asc',
  });
  assert.deepEqual(normalizePlaylistSort({ field: 'invalid', direction: 'desc' }), {
    field: 'position',
    direction: 'desc',
  });
});

test('column order can move the first and last information columns without moving the index', () => {
  const movedFirst = moveTrackColumn(DEFAULT_TRACK_COLUMN_PREFERENCES, 'title', 'down');
  assert.deepEqual(movedFirst.map(({ id }) => id), [
    'artist', 'title', 'album', 'year', 'createdAt', 'duration', 'audioFormat', 'playCount',
  ]);
  const movedLast = moveTrackColumn(movedFirst, 'playCount', 'up');
  assert.deepEqual(movedLast.map(({ id }) => id), [
    'artist', 'title', 'album', 'year', 'createdAt', 'duration', 'playCount', 'audioFormat',
  ]);
  assert.deepEqual(moveTrackColumn(movedLast, 'artist', 'up'), movedLast);
  assert.deepEqual(moveTrackColumn(movedLast, 'audioFormat', 'down'), movedLast);
  assert.equal(trackListGridTemplate(movedLast).split(' ').at(0), '26px');
  assert.notEqual(trackListGridTemplate(movedLast).split(' ').at(-1), '42px');
});

test('visibility omits any information column from rows, skeletons, header count and grid template', () => {
  let preferences = DEFAULT_TRACK_COLUMN_PREFERENCES.map((column) => ({ ...column }));
  for (const id of ['title', 'artist', 'album', 'year', 'createdAt', 'audioFormat', 'duration', 'playCount']) {
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

test('created date shows local YYYY-MM-DD only and legacy column preferences keep their order', () => {
  const timestamp = new Date(2026, 9, 14, 23, 59, 58).getTime();
  assert.equal(formatCreatedAt(timestamp), '2026-10-14');
  assert.equal(formatCreatedAt(null), '—');
  assert.doesNotMatch(formatCreatedAt(timestamp), /:/);
  assert.deepEqual(normalizeTrackColumnPreferences([
    { id: 'title', visible: true },
    { id: 'artist', visible: false },
    { id: 'album', visible: true },
    { id: 'year', visible: true },
    { id: 'audioFormat', visible: false },
    { id: 'duration', visible: true },
    { id: 'playCount', visible: true },
  ]), [
    { id: 'title', visible: true },
    { id: 'artist', visible: false },
    { id: 'album', visible: true },
    { id: 'year', visible: true },
    { id: 'createdAt', visible: true },
    { id: 'audioFormat', visible: false },
    { id: 'duration', visible: true },
    { id: 'playCount', visible: true },
  ]);
});

test('queue and album detail reuse shared sortable track-list headers', async () => {
  const [queue, album] = await Promise.all([
    readFile(new URL('../src/lib/PlaybackQueueList.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/AlbumDetail.svelte', import.meta.url), 'utf8'),
  ]);
  assert.match(queue, /sortable=\{true\}/);
  assert.match(queue, /onSort=\{\(columnId\) => onSort\?\.\(columnId as TrackListColumnPreference\['id'\]\)\}/);
  assert.match(queue, /longPressReorder=\{true\}/);
  assert.match(queue, /onReorder=\{handleReorder\}/);
  assert.match(queue, /invokeCommand\('playback_get_queue_page', request\)/);
  assert.match(queue, /hideSortDirection=\{hideSortDirection\}/);
  assert.match(album, /sortable=\{true\}/);
  assert.match(album, /sortableIndex=\{true\}/);
  assert.match(album, /DEFAULT_ALBUM_DETAIL_SORT/);
  assert.match(album, /settings_get_album_detail_sort/);
  assert.match(album, /settings_set_album_detail_sort/);
  assert.match(album, /sort: detailSort\.field === 'position' \? null/);
  assert.match(album, /indexSort: albumIndexSortActive \? detailSort\.direction : null/);
  assert.match(album, /handleIndexSort/);
});

test('album detail artwork header shows artists and both duration statistics without duplicating title/count', async () => {
  const album = await readFile(new URL('../src/lib/AlbumDetail.svelte', import.meta.url), 'utf8');
  assert.doesNotMatch(album, /包含的演出者/);
  assert.match(album, /artists\.length \? artists\.join\('．'\) : '未知'/);
  assert.match(album, /累積播放/);
  assert.match(album, /播放時長/);
  assert.match(album, /formatDurationHours\(album\.playedMs\)/);
  assert.match(album, /formatDurationHours\(album\.durationMs\)/);
  assert.doesNotMatch(album, /<strong>\{album\.displayName\}<\/strong>/);
  assert.doesNotMatch(album, /formatAlbumDetailSummary/);
});

test('paged list loading rows keep their cells visually blank', async () => {
  const paths = [
    '../src/lib/TrackColumnCells.svelte',
    '../src/lib/TrackList.svelte',
    '../src/lib/PlaylistEntryList.svelte',
    '../src/lib/PlaybackQueueList.svelte',
    '../src/lib/ArtistIndex.svelte',
    '../src/lib/AlbumIndex.svelte',
    '../src/lib/FavoritePopover.svelte',
  ];
  const sources = await Promise.all(paths.map((path) => readFile(new URL(path, import.meta.url), 'utf8')));
  const [cells, ...lists] = sources;
  assert.doesNotMatch(cells, /placeholderTitle|正在載入|aria-hidden="true">—/);
  assert.match(cells, /class="track-index column-index" role="gridcell" aria-hidden="true"><\/span>/);
  assert.ok(lists.every((source) => !source.includes('placeholderTitle')));
  assert.doesNotMatch(sources[4], /正在計算演出者資料/);
  assert.doesNotMatch(sources[5], /正在整理專輯/);
  assert.doesNotMatch(sources[6], /正在載入播放清單/);
});

test('field order settings use the shared 300ms long-press drag delay', async () => {
  const [app, appCss] = await Promise.all([
    readFile(new URL('../src/App.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/app.css', import.meta.url), 'utf8'),
  ]);
  assert.match(app, /QUEUE_REORDER_LONG_PRESS_MS/);
  assert.match(app, /setTimeout\([\s\S]*?QUEUE_REORDER_LONG_PRESS_MS/);
  assert.match(app, /moveTrackColumnTo\(trackColumnPreferences, column\.id, drag\.toIndex\)/);
  assert.match(app, /class:column-order-drop-before=/);
  assert.match(app, /class:column-order-drop-after=/);
  assert.match(app, /class="track-column-setting-row"[\s\S]*?onpointerdown=\{\(event\) => handleTrackColumnOrderPointerDown\(event, index\)\}/);
  assert.doesNotMatch(app, /track-column-order-grip|IconGripVertical/);
  assert.match(appCss, /\.track-column-setting-row\.column-order-drop-before::before/);
  assert.match(appCss, /\.track-column-setting-row\.column-order-drop-after::after/);
});

test('grouping mode setting persists and reaches all sortable list views', async () => {
  const [app, trackList, playlist, queue, artist, album, albumIndex] = await Promise.all([
    readFile(new URL('../src/App.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/TrackList.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/PlaylistEntryList.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/PlaybackQueueList.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/ArtistIndex.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/AlbumDetail.svelte', import.meta.url), 'utf8'),
    readFile(new URL('../src/lib/AlbumIndex.svelte', import.meta.url), 'utf8'),
  ]);
  assert.match(app, /啟用分組模式/);
  assert.match(app, /啟用後可能增加載入時間/);
  assert.match(app, /groupingEnabled,/);
  assert.equal([...app.matchAll(/groupingEnabled=\{groupingEnabled\}/g)].length, 5);
  // Play queue: shuffle hides group headers (order is random); shuffle off restores them.
  assert.match(app, /groupingEnabled=\{groupingEnabled && playback\?\.shuffle !== true\}/);
  for (const source of [trackList, playlist, queue, artist, album]) {
    assert.match(source, /groupingEnabled/);
  }
  assert.match(trackList, /groupingEnabled\s*&&\s*sortable/);
  assert.match(playlist, /groupingEnabled\s*&&\s*isGroupedSortField\(currentSort\.field\)/);
  assert.match(queue, /groupingEnabled\s*&&\s*currentSort/);
  assert.match(app, /<AlbumIndex[\s\S]*?\{groupingEnabled\}/);
  assert.match(albumIndex, /groupingEnabled: shouldGroupAlbumIndex\(\{ sortField, groupingEnabled \}\)/);
});

test('column drag pointer maps to final indices after removing the dragged row', () => {
  const centers = [20, 80, 140, 200];
  assert.equal(trackColumnDropIndexFromPointer(10, centers, 2), 0);
  assert.equal(trackColumnDropIndexFromPointer(100, centers, 0), 1);
  assert.equal(trackColumnDropIndexFromPointer(180, centers, 1), 2);
  assert.equal(trackColumnDropIndexFromPointer(100, centers, 2), 2);
  assert.equal(trackColumnDropIndexFromPointer(500, centers, 1), 3);
  assert.equal(trackColumnDropIndexFromPointer(100, [], 0), 0);
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
  assert.equal(
    formatTrackColumnValue('audioFormat', {
      codec: 'DSD64', sampleRateHz: 2_822_400, bitDepth: 1, bitrateBps: 5_644_800,
    }, formatDuration),
    'DSD64．2.8 MHz',
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
    createdAtUtcMs: null,
    codec: null,
    sampleRateHz: null,
    bitDepth: null,
    bitrateBps: null,
    durationMs: null,
  };
  for (const id of ['artist', 'album', 'year', 'createdAt', 'audioFormat', 'duration', 'playCount']) {
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

test('play count is played_ms / duration_ms with one decimal except zero', () => {
  assert.equal(formatPlayCount(130_000, 100_000), '1.3');
  assert.equal(formatPlayCount(100_000, 100_000), '1.0');
  assert.equal(formatPlayCount(110_000, 100_000), '1.1');
  assert.equal(formatPlayCount(200_000, 100_000), '2.0');
  assert.equal(formatPlayCount(0, 100_000), '0');
  assert.equal(formatPlayCount(50_000, 0), '—');
  assert.equal(formatPlayCount(50_000, null), '—');
  assert.equal(formatPlayCount(50_000, undefined), '—');
  assert.equal(formatPlayCount(undefined, 100_000), '0');
  assert.equal(formatTrackColumnValue('playCount', { playedMs: 130_000, durationMs: 100_000 }, formatDuration), '1.3');
  assert.equal(formatTrackColumnValue('playCount', { playedMs: 200_000, durationMs: 100_000 }, formatDuration), '2.0');
  assert.equal(formatTrackColumnValue('playCount', { playedMs: 10, durationMs: null }, formatDuration), '—');
});

test('Now Playing A and B keep artwork/lyrics order when stacked', () => {
  assert.deepEqual(nowPlayingLayoutOrder('a'), ['artwork', 'lyrics']);
  assert.deepEqual(nowPlayingLayoutOrder('b'), ['lyrics', 'artwork']);
  assert.deepEqual(nowPlayingLayoutOrder(undefined), ['artwork', 'lyrics']);
});
