/** Per-track lyrics display delay (seconds). Positive delays lyrics relative to audio. */

export const LYRICS_TIMING_OFFSET_MIN_SEC = -5;
export const LYRICS_TIMING_OFFSET_MAX_SEC = 5;
export const LYRICS_TIMING_OFFSET_STEP_SEC = 0.1;

const STORAGE_PREFIX = 'moemusicplayer.lyricsTimingOffsetSec:';

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

/** @param {string | null | undefined} trackId @returns {number} */
export function loadLyricsTimingOffsetSec(trackId) {
  if (!trackId || typeof localStorage === 'undefined') return 0;
  try {
    const raw = localStorage.getItem(STORAGE_PREFIX + trackId);
    if (raw === null) return 0;
    return clampLyricsTimingOffsetSec(Number(raw));
  } catch {
    return 0;
  }
}

/** @param {string | null | undefined} trackId @param {number} delaySec @returns {number} */
export function saveLyricsTimingOffsetSec(trackId, delaySec) {
  const clamped = clampLyricsTimingOffsetSec(delaySec);
  if (!trackId || typeof localStorage === 'undefined') return clamped;
  try {
    if (clamped === 0) localStorage.removeItem(STORAGE_PREFIX + trackId);
    else localStorage.setItem(STORAGE_PREFIX + trackId, String(clamped));
  } catch {
    // Quota / private mode: keep in-memory only.
  }
  return clamped;
}

/** @param {number} delaySec @returns {string} */
export function formatLyricsTimingOffsetSec(delaySec) {
  const value = clampLyricsTimingOffsetSec(delaySec);
  const sign = value > 0 ? '+' : '';
  return sign + value.toFixed(1) + ' 秒';
}