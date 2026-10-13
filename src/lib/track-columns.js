/** @typedef {'title' | 'artist' | 'album' | 'year' | 'createdAt' | 'audioFormat' | 'duration' | 'playCount'} TrackColumnId */
/** @typedef {{ id: TrackColumnId, visible: boolean }} TrackColumnPreference */
/** @typedef {{ title: number, artist: number, album: number }} TrackColumnWidths */
/** @typedef {'asc' | 'desc'} LibrarySortDirection */
/** @typedef {{ field: TrackColumnId, direction: LibrarySortDirection }} LibraryListSort */
/** @typedef {'position' | TrackColumnId} PlaylistSortField */
/** @typedef {{ field: PlaylistSortField, direction: LibrarySortDirection }} PlaylistListSort */

const INDEX_COLUMN_PX = 26;
const FLEXIBLE_WIDTH_MIN_PX = 80;
const FLEXIBLE_WIDTH_MAX_PX = 800;

/** @type {ReadonlyArray<{ id: TrackColumnId, label: string, min: string, minPx: number }>} */
export const TRACK_COLUMN_DEFINITIONS = Object.freeze([
  { id: 'title', label: '曲目', min: 'minmax(150px,1.7fr)', minPx: 150 },
  { id: 'artist', label: '演出者', min: 'minmax(110px,1.15fr)', minPx: 110 },
  { id: 'album', label: '專輯', min: 'minmax(120px,1.25fr)', minPx: 120 },
  { id: 'year', label: '發行時間', min: '6em', minPx: 78 },
  { id: 'createdAt', label: '新增時間', min: '6em', minPx: 78 },
  { id: 'audioFormat', label: '音訊格式', min: '172px', minPx: 172 },
  { id: 'duration', label: '長度', min: '4em', minPx: 52 },
  { id: 'playCount', label: '播放次數', min: '5em', minPx: 65 },
]);

/** @type {TrackColumnPreference[]} */
export const DEFAULT_TRACK_COLUMN_PREFERENCES = TRACK_COLUMN_DEFINITIONS.map(({ id }) => ({ id, visible: true }));

/** @type {TrackColumnWidths} */
export const DEFAULT_TRACK_COLUMN_WIDTHS = Object.freeze({
  title: 220,
  artist: 150,
  album: 160,
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
export const DEFAULT_LIBRARY_SORT = Object.freeze({ field: 'title', direction: 'asc' });

/** @type {ReadonlySet<TrackColumnId>} */
export const RESIZABLE_TRACK_COLUMN_IDS = Object.freeze(new Set(/** @type {TrackColumnId[]} */ (['title', 'artist', 'album'])));

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
  return { field, direction: 'asc' };
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
    : { field, direction: 'asc' };
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
  'ALAC',
  'APE',
  'WV',
  'WAVPACK',
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
 */
export function trackListGridTemplate(preferences, widths) {
  const flexible = normalizeTrackColumnWidths(widths);
  const informationColumns = visibleTrackColumns(preferences).map(({ id }) => {
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
 */
export function trackListMinWidthPx(preferences, widths) {
  const flexible = normalizeTrackColumnWidths(widths);
  const visible = visibleTrackColumns(preferences);
  const information = visible.reduce((sum, column) => {
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
  return Number.isInteger(rounded) ? String(rounded) : rounded.toFixed(1);
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
  return LOSSLESS_CODECS.has(name);
}

/**
 * @param {{ codec?: string | null, sampleRateHz?: number | null, bitDepth?: number | null, bitrateBps?: number | null }} item
 * @param {{ omitLosslessBitrate?: boolean }} [options]
 */
export function formatAudioFormat(item, options = {}) {
  const omitLosslessBitrate = options.omitLosslessBitrate === true;
  const parts = [];
  if (typeof item.codec === 'string' && item.codec.trim()) {
    parts.push(item.codec.trim().toUpperCase());
  }
  if (typeof item.sampleRateHz === 'number' && Number.isFinite(item.sampleRateHz) && item.sampleRateHz > 0) {
    const kiloHertz = item.sampleRateHz / 1000;
    parts.push(`${Number.isInteger(kiloHertz) ? kiloHertz : kiloHertz.toFixed(1)} kHz`);
  }
  if (typeof item.bitDepth === 'number' && Number.isInteger(item.bitDepth) && item.bitDepth > 0) {
    parts.push(`${item.bitDepth} bit`);
  }
  const hideBitrate = omitLosslessBitrate && isLosslessCodec(item.codec);
  if (
    !hideBitrate
    && typeof item.bitrateBps === 'number'
    && Number.isFinite(item.bitrateBps)
    && item.bitrateBps > 0
  ) {
    parts.push(`${Math.round(item.bitrateBps / 1000)} kbps`);
  }
  return parts.length > 0 ? parts.join('．') : '—';
}

/**
 * Hi-Res badges use source bit depth only. Lossy decoder PCM depth is not a source-depth claim.
 * @param {{ sampleRateHz?: number | null, bitDepth?: number | null } | null | undefined} item
 */
export function isHiResTrack(item) {
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
 * @param {{ title?: string | null, fileName?: string | null, artist?: string | null, album?: string | null, releaseDate?: string | null, year?: number | string | null, createdAtUtcMs?: number | null, codec?: string | null, sampleRateHz?: number | null, bitDepth?: number | null, bitrateBps?: number | null, durationMs?: number | null, playedMs?: number | null }} item
 * @param {(durationMs: number | null | undefined) => string} formatDuration
 * @param {string} titleFallback
 */
export function formatTrackColumnValue(id, item, formatDuration, titleFallback = '未命名曲目') {
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
    case 'audioFormat':
      return formatAudioFormat(item, { omitLosslessBitrate: true });
    case 'duration':
      return item.durationMs === null || item.durationMs === undefined
        ? '—'
        : formatDuration(item.durationMs);
    case 'playCount':
      return formatPlayCount(item.playedMs, item.durationMs);
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
