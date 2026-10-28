import { tr } from './i18n.js';
import { displayUnknown } from './unknown-label.js';
import { formatBitrateKbps } from './track-details-format.js';
/** @typedef {'title' | 'artist' | 'album' | 'year' | 'createdAt' | 'modifiedAt' | 'audioFormat' | 'duration' | 'playCount' | 'source'} TrackColumnId */
/** @typedef {{ id: TrackColumnId, visible: boolean }} TrackColumnPreference */
/** @typedef {{ title: number, artist: number, album: number, source: number }} TrackColumnWidths */
/** @typedef {'asc' | 'desc'} LibrarySortDirection */
/** @typedef {{ field: TrackColumnId, direction: LibrarySortDirection }} LibraryListSort */
/** @typedef {'position' | TrackColumnId} PlaylistSortField */
/** @typedef {{ field: PlaylistSortField, direction: LibrarySortDirection }} PlaylistListSort */

const INDEX_COLUMN_PX = 26;
const FLEXIBLE_WIDTH_MIN_PX = 80;
const FLEXIBLE_WIDTH_MAX_PX = 800;

/** @type {ReadonlyArray<{ id: TrackColumnId, label: string, min: string, minPx: number }>} */
export const TRACK_COLUMN_DEFINITIONS = Object.freeze([
  { id: 'title', label: tr('曲目'), min: 'minmax(150px,1.7fr)', minPx: 150 },
  { id: 'artist', label: tr('演出者'), min: 'minmax(110px,1.15fr)', minPx: 110 },
  { id: 'album', label: tr('專輯'), min: 'minmax(120px,1.25fr)', minPx: 120 },
  { id: 'year', label: tr('發行時間'), min: '6em', minPx: 78 },
  { id: 'createdAt', label: tr('新增時間'), min: '6em', minPx: 78 },
  { id: 'modifiedAt', label: tr('修改時間'), min: '6em', minPx: 78 },
  { id: 'audioFormat', label: tr('音訊格式'), min: '163px', minPx: 163 },
  { id: 'duration', label: tr('長度'), min: '4em', minPx: 52 },
  { id: 'playCount', label: tr('播放次數'), min: '5em', minPx: 65 },
  { id: 'source', label: tr('來源'), min: 'auto', minPx: 64 },
]);

/** Column values rendered as dates, counts, or durations use centered alignment. */
const NUMERIC_LIST_COLUMN_IDS = new Set([
  'year', 'createdAt', 'modifiedAt', 'duration', 'playCount',
  'trackCount', 'albumCount', 'trackDuration', 'played', 'source',
]);

/** @param {string} id */
export function isNumericListColumn(id) {
  return NUMERIC_LIST_COLUMN_IDS.has(id);
}

/**
 * Whether this runs in the Android app (its WebView says so). Android's storage records no creation time, so there the
 * "added" column (新增時間) is off by default and the modified time is on; on Windows it is the other way round. The
 * backend (`settings.rs`, `default_created_column_visible`) makes the same choice for the saved settings.
 */
function isAndroidWebView() {
  return typeof navigator !== 'undefined' && /Android/i.test(navigator.userAgent ?? '');
}

/** @param {TrackColumnId} id */
function defaultColumnVisible(id) {
  if (id === 'source') return false;
  if (id === 'createdAt') return !isAndroidWebView();
  if (id === 'modifiedAt') return isAndroidWebView();
  return true;
}

/** @type {TrackColumnPreference[]} */
export const DEFAULT_TRACK_COLUMN_PREFERENCES = ['title', 'artist', 'album', 'year', 'createdAt', 'modifiedAt', 'duration', 'audioFormat', 'playCount', 'source'].map((id) => ({ id: /** @type {TrackColumnId} */ (id), visible: defaultColumnVisible(/** @type {TrackColumnId} */ (id)) }));

/** @type {TrackColumnWidths} */
export const DEFAULT_TRACK_COLUMN_WIDTHS = Object.freeze({
  title: 184,
  artist: 144,
  album: 257,
  source: 64,
});

export const DEFAULT_TRACK_ROW_HEIGHT_PX = 50;
export const MIN_TRACK_ROW_HEIGHT_PX = 30;
export const MAX_TRACK_ROW_HEIGHT_PX = 60;

/** @param {unknown} value */
export function normalizeTrackRowHeightPx(value) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return DEFAULT_TRACK_ROW_HEIGHT_PX;
  return Math.min(MAX_TRACK_ROW_HEIGHT_PX, Math.max(MIN_TRACK_ROW_HEIGHT_PX, Math.round(value)));
}

/** @type {LibraryListSort} */
export const DEFAULT_LIBRARY_SORT = Object.freeze({ field: 'createdAt', direction: 'asc' });

/** @type {ReadonlySet<TrackColumnId>} */
export const RESIZABLE_TRACK_COLUMN_IDS = Object.freeze(new Set(/** @type {TrackColumnId[]} */ (['title', 'artist', 'album'])));

/** Width of the source column before its labels were measured (and what the settings file keeps in `widths.source`; the stored value is never read). */
export const SOURCE_COLUMN_FALLBACK_PX = 64;
/** Room the header keeps next to its title for the sort indicator (gap 4 px + the 9 px arrow). */
export const SOURCE_HEADER_INDICATOR_PX = 14;
/** Horizontal padding inside the source cell and a rounding margin so the widest label never reaches the ellipsis. */
export const SOURCE_CELL_PADDING_PX = 4;

/**
 * The smallest width at which the source column shows all of its text: the widest of every label the list can show
 * (the whole set of source names, not the rows on screen) and the header title with its sort indicator.
 * @param {string[]} labels every source label
 * @param {string} headerLabel the column title
 * @param {(text: string, kind: 'cell' | 'header') => number} measure text width in px with the list's real font
 */
export function computeSourceColumnWidthPx(labels, headerLabel, measure) {
  let widest = 0;
  for (const label of labels) {
    if (typeof label !== 'string' || label === '') continue;
    widest = Math.max(widest, measure(label, 'cell'));
  }
  const header = measure(headerLabel, 'header') + SOURCE_HEADER_INDICATOR_PX;
  const width = Math.ceil(Math.max(widest, header)) + SOURCE_CELL_PADDING_PX;
  return Number.isFinite(width) && width > 0 ? width : SOURCE_COLUMN_FALLBACK_PX;
}

/** @param {unknown} value */
function sourceWidthOrFallback(value) {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? Math.ceil(value) : SOURCE_COLUMN_FALLBACK_PX;
}

/** @type {Set<TrackColumnId>} */
const COLUMN_IDS = new Set(TRACK_COLUMN_DEFINITIONS.map(({ id }) => id));
const COLUMN_DEFINITION_BY_ID = new Map(TRACK_COLUMN_DEFINITIONS.map((column) => [column.id, column]));

/**
 * @param {unknown} value
 * @returns {number}
 */
function clampFlexibleWidth(value) {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return DEFAULT_TRACK_COLUMN_WIDTHS.title;
  }
  return Math.min(FLEXIBLE_WIDTH_MAX_PX, Math.max(FLEXIBLE_WIDTH_MIN_PX, Math.round(value)));
}

/**
 * @param {unknown} value
 * @returns {TrackColumnWidths}
 */
export function normalizeTrackColumnWidths(value) {
  if (!value || typeof value !== 'object') {
    return { ...DEFAULT_TRACK_COLUMN_WIDTHS };
  }
  const record = /** @type {Record<string, unknown>} */ (value);
  return {
    title: typeof record.title === 'number' && Number.isFinite(record.title)
      ? clampFlexibleWidth(record.title)
      : DEFAULT_TRACK_COLUMN_WIDTHS.title,
    artist: typeof record.artist === 'number' && Number.isFinite(record.artist)
      ? clampFlexibleWidth(record.artist)
      : DEFAULT_TRACK_COLUMN_WIDTHS.artist,
    album: typeof record.album === 'number' && Number.isFinite(record.album)
      ? clampFlexibleWidth(record.album)
      : DEFAULT_TRACK_COLUMN_WIDTHS.album,
    // The source column sizes itself; a stored width is ignored.
    source: DEFAULT_TRACK_COLUMN_WIDTHS.source,
  };
}

/**
 * @param {unknown} value
 * @returns {LibraryListSort}
 */
export function normalizeLibrarySort(value) {
  if (!value || typeof value !== 'object') {
    return { ...DEFAULT_LIBRARY_SORT };
  }
  const record = /** @type {Record<string, unknown>} */ (value);
  const field = COLUMN_IDS.has(/** @type {TrackColumnId} */ (record.field))
    ? /** @type {TrackColumnId} */ (record.field)
    : DEFAULT_LIBRARY_SORT.field;
  const direction = record.direction === 'desc' ? 'desc' : 'asc';
  return { field, direction };
}

/** Columns whose first click sorts from the largest to the smallest: the dates, the length, the play count and the quality. */
const DESCENDING_FIRST_COLUMNS = new Set(['year', 'createdAt', 'modifiedAt', 'duration', 'playCount', 'audioFormat']);

/** The direction the first click on a column sorts in. @param {string} field @returns {'asc' | 'desc'} */
export function getFirstSortDirection(field) {
  return DESCENDING_FIRST_COLUMNS.has(field) ? 'desc' : 'asc';
}

/**
 * @param {LibraryListSort} current
 * @param {TrackColumnId} field
 * @returns {LibraryListSort}
 */
export function toggleLibrarySort(current, field) {
  const normalized = normalizeLibrarySort(current);
  if (normalized.field === field) {
    return { field, direction: normalized.direction === 'asc' ? 'desc' : 'asc' };
  }
  return { field, direction: getFirstSortDirection(field) };
}

/** @type {PlaylistListSort} */
export const DEFAULT_PLAYLIST_SORT = Object.freeze({ field: 'position', direction: 'asc' });

/** @param {unknown} value @returns {PlaylistListSort} */
export function normalizePlaylistSort(value) {
  if (!value || typeof value !== 'object') return { ...DEFAULT_PLAYLIST_SORT };
  const record = /** @type {Record<string, unknown>} */ (value);
  const field = record.field === 'position' || COLUMN_IDS.has(/** @type {TrackColumnId} */ (record.field))
    ? /** @type {PlaylistSortField} */ (record.field)
    : DEFAULT_PLAYLIST_SORT.field;
  return { field, direction: record.direction === 'desc' ? 'desc' : 'asc' };
}

/** @param {PlaylistListSort} current @param {PlaylistSortField} field @returns {PlaylistListSort} */
export function togglePlaylistSort(current, field) {
  const normalized = normalizePlaylistSort(current);
  return normalized.field === field
    ? { field, direction: normalized.direction === 'asc' ? 'desc' : 'asc' }
    : { field, direction: getFirstSortDirection(field) };
}

/**
 * @param {TrackColumnWidths} widths
 * @param {TrackColumnId} id
 * @param {number} widthPx
 * @returns {TrackColumnWidths}
 */
export function setTrackColumnWidth(widths, id, widthPx) {
  if (!RESIZABLE_TRACK_COLUMN_IDS.has(id)) {
    return normalizeTrackColumnWidths(widths);
  }
  return normalizeTrackColumnWidths({
    ...normalizeTrackColumnWidths(widths),
    [id]: widthPx,
  });
}
/** Codecs that store source PCM without a useful average bitrate in the list. */
const LOSSLESS_CODECS = new Set([
  'FLAC',
  'WAV',
  'WAVE',
  'AIFF',
  'AIF',
  'AIFC',
  'ALAC',
  'APE',
  'WV',
  'WAVPACK',
  'WVLL',
  'WVDSD',
  'PCM',
  'TAK',
  'TTA',
]);

/**
 * Normalize the persisted preference boundary without trusting its order or fields.
 * Missing/malformed preferences fall back as a whole to the stable default order.
 * @param {unknown} value
 * @returns {TrackColumnPreference[]}
 */
export function normalizeTrackColumnPreferences(value) {
  if (!Array.isArray(value)) {
    return DEFAULT_TRACK_COLUMN_PREFERENCES.map((column) => ({ ...column }));
  }

  const seen = new Set();
  for (const item of value) {
    if (!item || typeof item !== 'object' || (item.id !== 'createdAt' && !COLUMN_IDS.has(item.id)) || typeof item.visible !== 'boolean' || seen.has(item.id)) {
      return DEFAULT_TRACK_COLUMN_PREFERENCES.map((column) => ({ ...column }));
    }
    seen.add(item.id);
  }

  const migrated = value.map((item) => ({
    id: /** @type {TrackColumnId} */ (item.id),
    visible: item.visible,
  }));
  if (!seen.has('createdAt')) {
    const audioIndex = migrated.findIndex((column) => column.id === 'audioFormat');
    migrated.splice(audioIndex < 0 ? migrated.length : audioIndex, 0, { id: 'createdAt', visible: true });
  }
  // The modified column was added after the created one: preferences saved without it get it right after the created
  // column, on by default on Android only.
  if (!seen.has('modifiedAt')) {
    const createdIndex = migrated.findIndex((column) => column.id === 'createdAt');
    migrated.splice(createdIndex < 0 ? migrated.length : createdIndex + 1, 0, { id: 'modifiedAt', visible: defaultColumnVisible('modifiedAt') });
  }
  // The source column is the newest: preferences saved without it get it, hidden, at the end.
  if (!seen.has('source')) migrated.push({ id: 'source', visible: false });
  if (migrated.length !== TRACK_COLUMN_DEFINITIONS.length) {
    return DEFAULT_TRACK_COLUMN_PREFERENCES.map((column) => ({ ...column }));
  }
  return migrated;
}

/** @param {TrackColumnPreference[]} preferences */
export function visibleTrackColumns(preferences) {
  return preferences.filter((column) => column.visible);
}

/**
 * The row number is always present. Playback starts from a click on the row.
 * @param {TrackColumnPreference[]} preferences
 */
export function trackListColumnCount(preferences) {
  return visibleTrackColumns(preferences).length + 1;
}

/**
 * @param {TrackColumnPreference[]} preferences
 * @param {TrackColumnWidths | null | undefined} [widths]
 * @param {number} [sourceWidthPx] measured width of the source column (`computeSourceColumnWidthPx`)
 */
export function trackListGridTemplate(preferences, widths, sourceWidthPx) {
  const flexible = normalizeTrackColumnWidths(widths);
  const informationColumns = visibleTrackColumns(preferences).map(({ id }) => {
    if (id === 'source') return `${sourceWidthOrFallback(sourceWidthPx)}px`;
    if (RESIZABLE_TRACK_COLUMN_IDS.has(id)) {
      return `${flexible[/** @type {'title' | 'artist' | 'album'} */ (id)]}px`;
    }
    return COLUMN_DEFINITION_BY_ID.get(id)?.min ?? 'minmax(100px,1fr)';
  });
  return [`${INDEX_COLUMN_PX}px`, ...informationColumns].join(' ');
}

/**
 * Width the shared grid needs before a long cell is allowed to grow the row.
 * Includes the index column, horizontal padding, and gaps.
 * @param {TrackColumnPreference[]} preferences
 * @param {TrackColumnWidths | null | undefined} [widths]
 * @param {number} [sourceWidthPx]
 */
export function trackListMinWidthPx(preferences, widths, sourceWidthPx) {
  const flexible = normalizeTrackColumnWidths(widths);
  const visible = visibleTrackColumns(preferences);
  const information = visible.reduce((sum, column) => {
    if (column.id === 'source') return sum + sourceWidthOrFallback(sourceWidthPx);
    if (RESIZABLE_TRACK_COLUMN_IDS.has(column.id)) {
      return sum + flexible[/** @type {'title' | 'artist' | 'album'} */ (column.id)];
    }
    return sum + (COLUMN_DEFINITION_BY_ID.get(column.id)?.minPx ?? 100);
  }, 0);
  const gaps = visible.length * 10;
  const padding = 28;
  return INDEX_COLUMN_PX + information + gaps + padding;
}

/**
 * Return a new order with one information column moved. Boundary moves are no-ops.
 * @param {TrackColumnPreference[]} preferences
 * @param {TrackColumnId} id
 * @param {'up' | 'down'} direction
 * @returns {TrackColumnPreference[]}
 */
export function moveTrackColumn(preferences, id, direction) {
  const current = normalizeTrackColumnPreferences(preferences);
  const index = current.findIndex((column) => column.id === id);
  const destination = index + (direction === 'up' ? -1 : 1);
  if (index < 0 || destination < 0 || destination >= current.length) return current;
  [current[index], current[destination]] = [current[destination], current[index]];
  return current;
}

/** Move one configured column directly to a zero-based position. */
/** @param {TrackColumnPreference[]} preferences @param {TrackColumnId} id @param {number} destination */
export function moveTrackColumnTo(preferences, id, destination) {
  const current = normalizeTrackColumnPreferences(preferences);
  const index = current.findIndex((column) => column.id === id);
  if (!Number.isSafeInteger(destination) || index < 0 || destination < 0 || destination >= current.length || index === destination) {
    return current;
  }
  const [column] = current.splice(index, 1);
  current.splice(destination, 0, column);
  return current;
}

/**
 * Convert a pointer Y coordinate to the final index after removing the dragged column.
 * Row centers are sorted in the same order as `preferences`.
 * @param {number} pointerY
 * @param {number[]} rowCenters
 * @param {number} fromIndex
 */
export function trackColumnDropIndexFromPointer(pointerY, rowCenters, fromIndex) {
  const length = rowCenters.length;
  if (length === 0 || !Number.isFinite(pointerY) || !Number.isSafeInteger(fromIndex)) return 0;

  const from = Math.max(0, Math.min(length - 1, fromIndex));
  const insertionIndex = rowCenters.findIndex((center) => pointerY < center);
  const insertion = insertionIndex < 0 ? length : insertionIndex;
  const destination = insertion > from ? insertion - 1 : insertion;
  return Math.max(0, Math.min(length - 1, destination));
}

/**
 * @param {TrackColumnPreference[]} preferences
 * @param {TrackColumnId} id
 * @param {boolean} visible
 */
export function setTrackColumnVisibility(preferences, id, visible) {
  return normalizeTrackColumnPreferences(preferences).map((column) =>
    column.id === id ? { ...column, visible } : column,
  );
}

/**
 * play_count = played_ms / duration_ms. Unknown or zero duration shows an em dash.
 * @param {unknown} playedMs
 * @param {unknown} durationMs
 */
export function formatPlayCount(playedMs, durationMs) {
  if (typeof durationMs !== 'number' || !Number.isFinite(durationMs) || durationMs <= 0) {
    return '—';
  }
  const played = typeof playedMs === 'number' && Number.isFinite(playedMs) && playedMs > 0
    ? playedMs
    : 0;
  const ratio = played / durationMs;
  if (!Number.isFinite(ratio) || ratio < 0) return '—';
  const rounded = Math.round(ratio * 10) / 10;
  return rounded === 0 ? '0' : rounded.toFixed(1);
}

/**
 * Display release date as YYYY, YYYY-MM, or YYYY-MM-DD.
 * Accepts a normalized releaseDate string, a legacy year number, or YYYY[-MM[-DD]].
 * @param {unknown} releaseDate
 * @param {unknown} [year]
 */
/**
 * @param {unknown} releaseDate
 * @param {unknown} [year]
 */
export function formatReleaseDate(releaseDate, year) {
  if (typeof releaseDate === 'number' && (year === undefined || year === null)) {
    return formatReleaseDate(null, releaseDate);
  }
  if (typeof releaseDate === 'string') {
    const match = /^(\d{4})(?:-(\d{2})(?:-(\d{2}))?)?$/.exec(releaseDate.trim());
    if (!match) return '—';
    const y = Number(match[1]);
    if (y === 0 || y > 9999) return '—';
    if (match[2] === undefined) return match[1];
    const month = Number(match[2]);
    if (month < 1 || month > 12) return '—';
    if (match[3] === undefined) return `${match[1]}-${match[2]}`;
    const day = Number(match[3]);
    if (day < 1 || day > 31) return '—';
    const date = new Date(Date.UTC(y, month - 1, day));
    return date.getUTCFullYear() === y && date.getUTCMonth() === month - 1 && date.getUTCDate() === day
      ? `${match[1]}-${match[2]}-${match[3]}`
      : '—';
  }
  if (typeof year === 'number') {
    return Number.isInteger(year) && year >= 1 && year <= 9999
      ? year.toString().padStart(4, '0')
      : '—';
  }
  if (typeof year === 'string') {
    return formatReleaseDate(year);
  }
  return '—';
}

/** @deprecated Prefer formatReleaseDate; kept for callers that only have a year. */
/** @param {unknown} value */
export function formatYear(value) {
  return formatReleaseDate(null, value);
}

/**
 * @param {unknown} codec
 */
export function isLosslessCodec(codec) {
  if (typeof codec !== 'string') return false;
  const name = codec.trim().toUpperCase();
  return LOSSLESS_CODECS.has(name) || isDsdCodec(name);
}

/**
 * DSD64 / DSD128 / DSD256 / DSD512 (the codec label the DSF reader reports) and WVDSD (DSD inside WavPack).
 * @param {unknown} codec
 */
export function isDsdCodec(codec) {
  return typeof codec === 'string' && /^(DSD\d+|WVDSD)$/i.test(codec.trim());
}

/**
 * @param {{ codec?: string | null, sampleRateHz?: number | null, bitDepth?: number | null, bitrateBps?: number | null }} item
 * @param {{ omitLosslessBitrate?: boolean, omitDsdBitDepth?: boolean }} [options]
 */
export function formatAudioFormat(item, options = {}) {
  // The track list passes `omitLosslessBitrate: true`: lossless formats (including DSD) show no average bitrate there.
  // Everywhere else (the Now Playing top bar) the bitrate is shown.
  const omitLosslessBitrate = options.omitLosslessBitrate === true;
  const parts = [];
  if (typeof item.codec === 'string' && item.codec.trim()) {
    parts.push(item.codec.trim().toUpperCase());
  }
  if (typeof item.sampleRateHz === 'number' && Number.isFinite(item.sampleRateHz) && item.sampleRateHz > 0) {
    if (isDsdCodec(item.codec)) {
      // DSD rates are in the megahertz range (2.8224 MHz for DSD64).
      parts.push(`${(item.sampleRateHz / 1_000_000).toFixed(1)} MHz`);
    } else {
      const kiloHertz = item.sampleRateHz / 1000;
      parts.push(`${Number.isInteger(kiloHertz) ? kiloHertz : kiloHertz.toFixed(1)} kHz`);
    }
  }
  // DSD is always 1 bit, so the list leaves it out (`omitDsdBitDepth`).
  const hideBitDepth = options.omitDsdBitDepth === true && isDsdCodec(item.codec);
  if (!hideBitDepth && typeof item.bitDepth === 'number' && Number.isInteger(item.bitDepth) && item.bitDepth > 0) {
    parts.push(`${item.bitDepth} bit`);
  }
  const hideBitrate = omitLosslessBitrate && isLosslessCodec(item.codec);
  if (
    !hideBitrate
    && typeof item.bitrateBps === 'number'
    && Number.isFinite(item.bitrateBps)
    && item.bitrateBps > 0
  ) {
    // Same rule as the details panel; the compact top bar and list keep one Mbps decimal.
    parts.push(formatBitrateKbps(item.bitrateBps / 1000, 1));
  }
  return parts.length > 0 ? parts.join('．') : '—';
}

/**
 * Hi-Res badges use source bit depth only. Lossy decoder PCM depth is not a source-depth claim. Every DSD file
 * counts as Hi-Res (its 1-bit depth is not comparable with PCM bit depth).
 * @param {{ codec?: string | null, sampleRateHz?: number | null, bitDepth?: number | null } | null | undefined} item
 */
export function isHiResTrack(item) {
  if (isDsdCodec(item?.codec)) return true;
  return typeof item?.sampleRateHz === 'number'
    && item.sampleRateHz >= 48_000
    && typeof item.bitDepth === 'number'
    && item.bitDepth >= 24;
}

/**
 * Title tag when it has text; otherwise the file stem. Artist and album are never used as a title.
 * @param {{ title?: string | null, fileName?: string | null } | null | undefined} item
 * @param {string} fallback
 */
export function displayTrackTitle(item, fallback = '') {
  const title = typeof item?.title === 'string' ? item.title.trim() : '';
  if (title) return title;
  const fileName = typeof item?.fileName === 'string' ? item.fileName.trim() : '';
  return fileName || fallback;
}

/**
 * @param {TrackColumnId} id
 * @param {{ title?: string | null, fileName?: string | null, artist?: string | null, album?: string | null, releaseDate?: string | null, year?: number | string | null, createdAtUtcMs?: number | null, modifiedAtUtcMs?: number | null, codec?: string | null, sampleRateHz?: number | null, bitDepth?: number | null, bitrateBps?: number | null, durationMs?: number | null, playedMs?: number | null, sourceName?: string | null }} item
 * @param {(durationMs: number | null | undefined) => string} formatDuration
 * @param {string} titleFallback
 */
export function formatTrackColumnValue(id, item, formatDuration, titleFallback = tr('未命名曲目')) {
  switch (id) {
    case 'title':
      return displayTrackTitle(item, titleFallback);
    case 'artist':
      return typeof item.artist === 'string' && item.artist.trim() ? item.artist.trim() : '—';
    case 'album':
      return typeof item.album === 'string' && item.album.trim() ? item.album.trim() : '—';
    case 'year':
      return formatReleaseDate(item.releaseDate, item.year);
    case 'createdAt':
      return formatCreatedAt(item.createdAtUtcMs);
    case 'modifiedAt':
      return formatCreatedAt(item.modifiedAtUtcMs);
    case 'audioFormat':
      return formatAudioFormat(item, { omitLosslessBitrate: true, omitDsdBitDepth: true });
    case 'duration':
      return item.durationMs === null || item.durationMs === undefined
        ? '—'
        : formatDuration(item.durationMs);
    case 'playCount':
      return formatPlayCount(item.playedMs, item.durationMs);
    case 'source':
      return typeof item.sourceName === 'string' && item.sourceName.trim() ? displayUnknown(item.sourceName.trim()) : '—';
  }
}

/** Render a UTC epoch timestamp as the user's local calendar date, never as a time. */
/** @param {number | null | undefined} timestamp */
export function formatCreatedAt(timestamp) {
  if (typeof timestamp !== 'number' || !Number.isFinite(timestamp)) return '—';
  const date = new Date(timestamp);
  if (!Number.isFinite(date.getTime())) return '—';
  const year = String(date.getFullYear()).padStart(4, '0');
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

/** The same helper is used by Now Playing A/B and its narrow stacked layout. */
/** @param {'a' | 'b' | undefined} layout */
export function nowPlayingLayoutOrder(layout) {
  return layout === 'b' ? ['lyrics', 'artwork'] : ['artwork', 'lyrics'];
}
