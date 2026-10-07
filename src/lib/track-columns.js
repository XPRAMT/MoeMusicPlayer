/** @typedef {'title' | 'artist' | 'album' | 'year' | 'audioFormat' | 'duration' | 'playCount'} TrackColumnId */
/** @typedef {{ id: TrackColumnId, visible: boolean }} TrackColumnPreference */

const INDEX_COLUMN_PX = 26;
const ACTION_COLUMN_PX = 42;

/** @type {ReadonlyArray<{ id: TrackColumnId, label: string, min: string, minPx: number }>} */
export const TRACK_COLUMN_DEFINITIONS = Object.freeze([
  { id: 'title', label: '曲目', min: 'minmax(150px,1.7fr)', minPx: 150 },
  { id: 'artist', label: '演出者', min: 'minmax(110px,1.15fr)', minPx: 110 },
  { id: 'album', label: '專輯', min: 'minmax(120px,1.25fr)', minPx: 120 },
  { id: 'year', label: '年份', min: '86px', minPx: 86 },
  { id: 'audioFormat', label: '音訊格式', min: '172px', minPx: 172 },
  { id: 'duration', label: '長度', min: '72px', minPx: 72 },
  { id: 'playCount', label: '播放次數', min: '78px', minPx: 78 },
]);

/** @type {TrackColumnPreference[]} */
export const DEFAULT_TRACK_COLUMN_PREFERENCES = TRACK_COLUMN_DEFINITIONS.map(({ id }) => ({ id, visible: true }));

/** @type {Set<TrackColumnId>} */
const COLUMN_IDS = new Set(TRACK_COLUMN_DEFINITIONS.map(({ id }) => id));
const COLUMN_DEFINITION_BY_ID = new Map(TRACK_COLUMN_DEFINITIONS.map((column) => [column.id, column]));

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
 * Fixed row number and play operation are always present.
 * @param {TrackColumnPreference[]} preferences
 */
export function trackListColumnCount(preferences) {
  return visibleTrackColumns(preferences).length + 2;
}

/** @param {TrackColumnPreference[]} preferences */
export function trackListGridTemplate(preferences) {
  const informationColumns = visibleTrackColumns(preferences)
    .map(({ id }) => COLUMN_DEFINITION_BY_ID.get(id)?.min ?? 'minmax(100px,1fr)');
  return [`${INDEX_COLUMN_PX}px`, ...informationColumns, `${ACTION_COLUMN_PX}px`].join(' ');
}

/**
 * Width the shared grid needs before a long cell is allowed to grow the row.
 * Includes the index column, the action column, horizontal padding, and gaps.
 * @param {TrackColumnPreference[]} preferences
 */
export function trackListMinWidthPx(preferences) {
  const visible = visibleTrackColumns(preferences);
  const information = visible.reduce(
    (sum, column) => sum + (COLUMN_DEFINITION_BY_ID.get(column.id)?.minPx ?? 100),
    0,
  );
  const gaps = (visible.length + 1) * 10;
  const padding = 28;
  return INDEX_COLUMN_PX + information + ACTION_COLUMN_PX + gaps + padding;
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

/** @param {unknown} value */
export function formatYear(value) {
  if (typeof value === 'number') {
    return Number.isInteger(value) && value >= 0 && value <= 9999
      ? value.toString().padStart(4, '0')
      : '—';
  }
  if (typeof value !== 'string') return '—';
  const match = /^(\d{4})(?:-(\d{2})-(\d{2}))?$/.exec(value.trim());
  if (!match) return '—';
  if (match[2] === undefined) return match[1];
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  if (year === 0 || month < 1 || month > 12 || day < 1 || day > 31) return '—';
  const date = new Date(Date.UTC(year, month - 1, day));
  return date.getUTCFullYear() === year && date.getUTCMonth() === month - 1 && date.getUTCDate() === day
    ? match[1]
    : '—';
}

/**
 * @param {{ codec?: string | null, sampleRateHz?: number | null, bitDepth?: number | null, bitrateBps?: number | null }} item
 */
export function formatAudioFormat(item) {
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
  if (typeof item.bitrateBps === 'number' && Number.isFinite(item.bitrateBps) && item.bitrateBps > 0) {
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
 * @param {TrackColumnId} id
 * @param {{ title?: string | null, artist?: string | null, album?: string | null, year?: number | string | null, codec?: string | null, sampleRateHz?: number | null, bitDepth?: number | null, bitrateBps?: number | null, durationMs?: number | null, playedMs?: number | null }} item
 * @param {(durationMs: number | null | undefined) => string} formatDuration
 * @param {string} titleFallback
 */
export function formatTrackColumnValue(id, item, formatDuration, titleFallback = '未命名曲目') {
  switch (id) {
    case 'title':
      return typeof item.title === 'string' && item.title.trim() ? item.title.trim() : titleFallback;
    case 'artist':
      return typeof item.artist === 'string' && item.artist.trim() ? item.artist.trim() : '—';
    case 'album':
      return typeof item.album === 'string' && item.album.trim() ? item.album.trim() : '—';
    case 'year':
      return formatYear(item.year);
    case 'audioFormat':
      return formatAudioFormat(item);
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
