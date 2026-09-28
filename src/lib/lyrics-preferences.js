/** @typedef {import('./ipc').LyricsPreferences} LyricsPreferences */

export const DEFAULT_LYRICS_PREFERENCES = Object.freeze({
  showTranslation: false,
  showRomanization: false,
  inactiveOpacityPercent: 70,
  primaryFontSizePx: 14,
  auxiliaryFontSizePx: 10,
});

export const LYRICS_PREFERENCE_LIMITS = Object.freeze({
  inactiveOpacityPercent: Object.freeze({ min: 10, max: 100 }),
  primaryFontSizePx: Object.freeze({ min: 12, max: 36 }),
  auxiliaryFontSizePx: Object.freeze({ min: 9, max: 24 }),
});

export const LYRIC_PRIMARY_MAX_LINES = 2;
export const LYRIC_PRIMARY_LINE_HEIGHT = 1.35;
export const LYRIC_AUXILIARY_LINE_HEIGHT = 1.25;
export const LYRIC_ROW_GAP_PX = 2;
export const LYRIC_ROW_VERTICAL_PADDING_PX = 14;
export const LYRIC_ROW_BORDER_PX = 1;
export const LYRIC_MIN_ROW_HEIGHT_PX = 76;

/** @param {unknown} value @returns {LyricsPreferences} */
export function normalizeLyricsPreferences(value) {
  const candidate = value && typeof value === 'object'
    ? /** @type {Record<string, unknown>} */ (value)
    : {};

  return {
    showTranslation: typeof candidate.showTranslation === 'boolean'
      ? candidate.showTranslation
      : DEFAULT_LYRICS_PREFERENCES.showTranslation,
    showRomanization: typeof candidate.showRomanization === 'boolean'
      ? candidate.showRomanization
      : DEFAULT_LYRICS_PREFERENCES.showRomanization,
    inactiveOpacityPercent: normalizeInteger(
      candidate.inactiveOpacityPercent,
      LYRICS_PREFERENCE_LIMITS.inactiveOpacityPercent.min,
      LYRICS_PREFERENCE_LIMITS.inactiveOpacityPercent.max,
      DEFAULT_LYRICS_PREFERENCES.inactiveOpacityPercent,
    ),
    primaryFontSizePx: normalizeInteger(
      candidate.primaryFontSizePx,
      LYRICS_PREFERENCE_LIMITS.primaryFontSizePx.min,
      LYRICS_PREFERENCE_LIMITS.primaryFontSizePx.max,
      DEFAULT_LYRICS_PREFERENCES.primaryFontSizePx,
    ),
    auxiliaryFontSizePx: normalizeInteger(
      candidate.auxiliaryFontSizePx,
      LYRICS_PREFERENCE_LIMITS.auxiliaryFontSizePx.min,
      LYRICS_PREFERENCE_LIMITS.auxiliaryFontSizePx.max,
      DEFAULT_LYRICS_PREFERENCES.auxiliaryFontSizePx,
    ),
  };
}

/**
 * Fixed virtual rows must hold the largest possible primary text (two lines)
 * plus every enabled auxiliary line and the exact CSS spacing/box edges.
 * @param {LyricsPreferences | unknown} value
 */
export function getLyricRowHeight(value) {
  const preferences = normalizeLyricsPreferences(value);
  const auxiliaryCount = Number(preferences.showTranslation) + Number(preferences.showRomanization);
  const contentRows = 1 + auxiliaryCount;
  const primaryHeight = preferences.primaryFontSizePx
    * LYRIC_PRIMARY_LINE_HEIGHT
    * LYRIC_PRIMARY_MAX_LINES;
  const auxiliaryHeight = auxiliaryCount
    * preferences.auxiliaryFontSizePx
    * LYRIC_AUXILIARY_LINE_HEIGHT;
  const gaps = Math.max(0, contentRows - 1) * LYRIC_ROW_GAP_PX;
  const boxHeight = primaryHeight
    + auxiliaryHeight
    + gaps
    + LYRIC_ROW_VERTICAL_PADDING_PX
    + LYRIC_ROW_BORDER_PX;

  return Math.max(LYRIC_MIN_ROW_HEIGHT_PX, Math.ceil(boxHeight));
}

/**
 * Paused playback keeps its cue. A stopped/ready track or a position before
 * its first cue has no focused row, so all timed lines remain fully readable.
 * Plain lyrics pass -1 and therefore never receive inactive opacity.
 * @param {number} activeIndex
 * @param {string} playbackState
 */
export function getDisplayActiveLyricIndex(activeIndex, playbackState) {
  if ((playbackState !== 'playing' && playbackState !== 'paused')
    || !Number.isInteger(activeIndex)
    || activeIndex < 0) {
    return -1;
  }
  return activeIndex;
}

/** @param {number} lineIndex @param {number} activeIndex @param {number} opacityPercent */
export function getLyricLineOpacity(lineIndex, activeIndex, opacityPercent) {
  if (activeIndex < 0 || lineIndex === activeIndex) return 1;
  const safeOpacity = normalizeInteger(
    opacityPercent,
    LYRICS_PREFERENCE_LIMITS.inactiveOpacityPercent.min,
    LYRICS_PREFERENCE_LIMITS.inactiveOpacityPercent.max,
    DEFAULT_LYRICS_PREFERENCES.inactiveOpacityPercent,
  );
  return safeOpacity / 100;
}

/** @param {unknown} value @param {number} min @param {number} max @param {number} fallback */
function normalizeInteger(value, min, max, fallback) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return fallback;
  return Math.max(min, Math.min(max, Math.round(value)));
}
