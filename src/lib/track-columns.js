/** @typedef {'title' | 'artist' | 'album' | 'year' | 'audioFormat' | 'duration' | 'playCount'} TrackColumnId */
/** @typedef {{ id: TrackColumnId, visible: boolean }} TrackColumnPreference */

const INDEX_COLUMN_PX = 26;

/** @type {ReadonlyArray<{ id: TrackColumnId, label: string, min: string, minPx: number }>} */
export const TRACK_COLUMN_DEFINITIONS = Object.freeze([
  { id: 'title', label: '曲目', min: 'minmax(150px,1.7fr)', minPx: 150 },
  { id: 'artist', label: '演出者', min: 'minmax(110px,1.15fr)', minPx: 110 },
  { id: 'album', label: '專輯', min: 'minmax(120px,1.25fr)', minPx: 120 },
  { id: 'year', label: '發行時間', min: '108px', minPx: 108 },
  { id: 'audioFormat', label: '音訊格式', min: '172px', minPx: 172 },
  { id: 'duration', label: '長度', min: '72px', minPx: 72 },
  { id: 'playCount', label: '播放次數', min: '78px', minPx: 78 },
]);

/** @type {TrackColumnPreference[]} */
export const DEFAULT_TRACK_COLUMN_PREFERENCES = TRACK_COLUMN_DEFINITIONS.map(({ id }) => ({ id, visible: true }));

/** @type {Set<TrackColumnId>} */
const COLUMN_IDS = new Set(TRACK_COLUMN_DEFINITIONS.map(({ id }) => id));
const COLUMN_DEFINITION_BY_ID = new Map(TRACK_COLUMN_DEFINITIONS.map((column) => [column.id, column]));

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
  if (!Array.isArray(value) || value.length !== TRACK_COLUMN_DEFINITIONS.length) {
    return DEFAULT_TRACK_COLUMN_PREFERENCES.map((column) => ({ ...column }));
  }

  const seen = new Set();
  for (const item of value) {
    if (!item || typeof item !== 'object' || !COLUMN_IDS.has(item.id) || typeof item.visible !== 'boolean' || seen.has(item.id)) {
      return DEFAULT_TRACK_COLUMN_PREFERENCES.map((column) => ({ ...column }));
    }
    seen.add(item.id);
  }

  return value.map((item) => ({
    id: /** @type {TrackColumnId} */ (item.id),
    visible: item.visible,
  }));
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

/** @param {TrackColumnPreference[]} preferences */
export function trackListGridTemplate(preferences) {
  const informationColumns = visibleTrackColumns(preferences)
    .map(({ id }) => COLUMN_DEFINITION_BY_ID.get(id)?.min ?? 'minmax(100px,1fr)');
  return [`${INDEX_COLUMN_PX}px`, ...informationColumns].join(' ');
}

/**
 * Width the shared grid needs before a long cell is allowed to grow the row.
 * Includes the index column, horizontal padding, and gaps.
 * @param {TrackColumnPreference[]} preferences
 */
export function trackListMinWidthPx(preferences) {
  const visible = visibleTrackColumns(preferences);
  const information = visible.reduce(
    (sum, column) => sum + (COLUMN_DEFINITION_BY_ID.get(column.id)?.minPx ?? 100),
    0,
  );
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
 * @param {{ title?: string | null, fileName?: string | null, artist?: string | null, album?: string | null, releaseDate?: string | null, year?: number | string | null, codec?: string | null, sampleRateHz?: number | null, bitDepth?: number | null, bitrateBps?: number | null, durationMs?: number | null, playedMs?: number | null }} item
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

/** The same helper is used by Now Playing A/B and its narrow stacked layout. */
/** @param {'a' | 'b' | undefined} layout */
export function nowPlayingLayoutOrder(layout) {
  return layout === 'b' ? ['lyrics', 'artwork'] : ['artwork', 'lyrics'];
}
