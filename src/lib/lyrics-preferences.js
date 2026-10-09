/** @typedef {import('./ipc').LyricsPreferences} LyricsPreferences */

export const DEFAULT_LYRICS_PREFERENCES = Object.freeze({
  showTranslation: false,
  showRomanization: false,
  inactiveOpacityPercent: 70,
  primaryFontSizePx: 14,
  auxiliaryFontSizePx: 10,
  lineGapPx: 24,
  textEffect: 'shadow',
  simplifiedToTraditional: false,
});

export const LYRICS_TEXT_EFFECTS = Object.freeze(['shadow', 'stroke', 'none']);

export const LYRICS_PREFERENCE_LIMITS = Object.freeze({
  inactiveOpacityPercent: Object.freeze({ min: 10, max: 100 }),
  primaryFontSizePx: Object.freeze({ min: 12, max: 36 }),
  auxiliaryFontSizePx: Object.freeze({ min: 9, max: 24 }),
  lineGapPx: Object.freeze({ min: 0, max: 50 }),
});

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
    lineGapPx: normalizeInteger(
      candidate.lineGapPx,
      LYRICS_PREFERENCE_LIMITS.lineGapPx.min,
      LYRICS_PREFERENCE_LIMITS.lineGapPx.max,
      DEFAULT_LYRICS_PREFERENCES.lineGapPx,
    ),
    textEffect: LYRICS_TEXT_EFFECTS.includes(/** @type {string} */ (candidate.textEffect))
      ? /** @type {import('./ipc').LyricsTextEffect} */ (candidate.textEffect)
      : DEFAULT_LYRICS_PREFERENCES.textEffect,
    simplifiedToTraditional: typeof candidate.simplifiedToTraditional === 'boolean'
      ? candidate.simplifiedToTraditional
      : DEFAULT_LYRICS_PREFERENCES.simplifiedToTraditional,
  };
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
