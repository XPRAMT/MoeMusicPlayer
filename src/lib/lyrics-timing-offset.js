import { invoke, isTauri } from '@tauri-apps/api/core';
import { tr } from './i18n.js';
/** Per-track lyrics shift (歌詞同步, seconds). +N s is added to every lyric timestamp (lines show N s later);
 * −N s shows them earlier. */

export const LYRICS_TIMING_OFFSET_MIN_SEC = -10;
export const LYRICS_TIMING_OFFSET_MAX_SEC = 10;
export const LYRICS_TIMING_OFFSET_STEP_SEC = 0.1;

/** Key prefix of the old browser-storage copy, kept only to move it into the library database once. */
const LEGACY_STORAGE_PREFIX = 'moemusicplayer.lyricsTimingOffsetSec:';

/** @param {unknown} value @returns {number} */
export function clampLyricsTimingOffsetSec(value) {
  const numeric = typeof value === 'number' ? value : Number(value);
  if (!Number.isFinite(numeric)) return 0;
  // Work in tenths of a second to avoid 0.15 / 0.1 floating-point traps.
  const tenths = Math.round(numeric * 10);
  const clampedTenths = Math.max(
    LYRICS_TIMING_OFFSET_MIN_SEC * 10,
    Math.min(LYRICS_TIMING_OFFSET_MAX_SEC * 10, tenths),
  );
  const clamped = clampedTenths / 10;
  return Object.is(clamped, -0) ? 0 : clamped;
}

/** Convert UI delay seconds to findActiveLyricIndex offsetMs. Positive delay => lyrics later.
 * @param {number} delaySec @returns {number} */
export function lyricsTimingDelaySecToOffsetMs(delaySec) {
  const offsetMs = Math.round(-clampLyricsTimingOffsetSec(delaySec) * 1000);
  return Object.is(offsetMs, -0) ? 0 : offsetMs;
}

/** Event (Tauri, app-wide) sent when the per-track delay changes: `{ trackId, delaySec }`. Other lyric surfaces
 * (Windows taskbar lyrics, later the Android status-bar lyrics) follow it at once. */
export const LYRICS_TIMING_OFFSET_EVENT = 'lyrics-timing-offset-changed';

/**
 * The one offset every lyric surface uses: the lyrics' own `offsetMs` (LRC [offset:]) plus the user's per-track
 * delay. Pass it to findActiveLyricIndex / nextCuePosition. The Now Playing page's LYRIC_CUE_LEAD_MS is added on top
 * by displayCuePositions only (it exists for the page's line-change animation).
 * @param {number | null | undefined} lyricsOffsetMs @param {number} delaySec @returns {number}
 */
export function lyricsDisplayOffsetMs(lyricsOffsetMs, delaySec) {
  const own = typeof lyricsOffsetMs === 'number' && Number.isFinite(lyricsOffsetMs) ? lyricsOffsetMs : 0;
  const total = own + lyricsTimingDelaySecToOffsetMs(delaySec);
  return Object.is(total, -0) ? 0 : total;
}

/** Where the delay is kept: the library database, through the Tauri commands (one Rust implementation for Windows
 * and Android). Replaceable for tests. Outside Tauri nothing is kept. */
let backend = {
  available: () => isTauri(),
  /** @param {string} command @param {Record<string, unknown>} args */
  invoke: (command, args) => invoke(command, args),
};

/** @param {{ available: () => boolean, invoke: (command: string, args: Record<string, unknown>) => Promise<any> }} next */
export function setLyricsTimingOffsetBackendForTests(next) {
  backend = next;
  legacyMigration = null;
}

/** @type {Promise<void> | null} */
let legacyMigration = null;

/** Old browser-storage delays (`${LEGACY_STORAGE_PREFIX}<trackId>`), read once and handed to the database; the keys
 * are removed only after the database accepted them (a failure leaves them for the next start).
 * @returns {Promise<void>} */
export function migrateLegacyLyricsTimingOffsets() {
  if (!legacyMigration) legacyMigration = runLegacyMigration();
  return legacyMigration;
}

async function runLegacyMigration() {
  if (!backend.available() || typeof localStorage === 'undefined') return;
  /** @type {string[]} */
  const keys = [];
  /** @type {{ trackId: string, delaySec: number }[]} */
  const entries = [];
  try {
    for (let index = 0; index < localStorage.length; index += 1) {
      const key = localStorage.key(index);
      if (!key || !key.startsWith(LEGACY_STORAGE_PREFIX)) continue;
      keys.push(key);
      const trackId = key.slice(LEGACY_STORAGE_PREFIX.length);
      const delaySec = clampLyricsTimingOffsetSec(Number(localStorage.getItem(key)));
      if (trackId && delaySec !== 0) entries.push({ trackId, delaySec });
    }
  } catch {
    return;
  }
  if (keys.length === 0) return;
  try {
    if (entries.length > 0) await backend.invoke('lyrics_timing_offsets_import', { entries });
  } catch {
    legacyMigration = null;
    return;
  }
  for (const key of keys) {
    try { localStorage.removeItem(key); } catch { /* ignore */ }
  }
}

/** The stored delay for a track (0 when none or unavailable).
 * @param {string | null | undefined} trackId @returns {Promise<number>} */
export async function loadLyricsTimingOffsetSec(trackId) {
  if (!trackId || !backend.available()) return 0;
  await migrateLegacyLyricsTimingOffsets();
  try {
    return clampLyricsTimingOffsetSec(await backend.invoke('lyrics_timing_offset_get', { trackId }));
  } catch {
    return 0;
  }
}

/** Store the delay (0 removes it); resolves with the value now in effect.
 * @param {string | null | undefined} trackId @param {number} delaySec @returns {Promise<number>} */
export function saveLyricsTimingOffsetSec(trackId, delaySec) {
  const clamped = clampLyricsTimingOffsetSec(delaySec);
  if (!trackId || !backend.available()) return Promise.resolve(clamped);
  // One write at a time, in the order asked (a dragged slider sends many): the last value is the one kept.
  const write = saveChain.then(async () => {
    await migrateLegacyLyricsTimingOffsets();
    try {
      return clampLyricsTimingOffsetSec(await backend.invoke('lyrics_timing_offset_set', { trackId, delaySec: clamped }));
    } catch {
      return clamped;
    }
  });
  saveChain = write.then(() => {}, () => {});
  return write;
}

/** @type {Promise<void>} */
let saveChain = Promise.resolve();

/** @param {number} delaySec @returns {string} */
export function formatLyricsTimingOffsetSec(delaySec) {
  const value = clampLyricsTimingOffsetSec(delaySec);
  const sign = value > 0 ? '+' : '';
  return tr('{0} 秒', [sign + value.toFixed(1)]);
}

/** The 歌詞同步 panel shows the stored delay negated (shown = −stored): lyrics 2 s early → the user sets −2 (stored +2,
 * which shows each line 2 s later); lyrics 3 s late → +3. The stored value and all timing math are unchanged.
 * @param {number} storedSec @returns {number} */
export function lyricsShiftShownSec(storedSec) {
  return clampLyricsTimingOffsetSec(-clampLyricsTimingOffsetSec(storedSec));
}

/** Inverse of {@link lyricsShiftShownSec}: the stored delay for a value the panel shows.
 * @param {number} shownSec @returns {number} */
export function lyricsShiftStoredSec(shownSec) {
  return clampLyricsTimingOffsetSec(-clampLyricsTimingOffsetSec(shownSec));
}
