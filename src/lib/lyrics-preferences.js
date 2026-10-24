/** @typedef {import('./ipc').LyricsPreferences} LyricsPreferences */

export const DEFAULT_LYRICS_PREFERENCES = Object.freeze({
  showTranslation: true,
  showRomanization: false,
  mainOpacityPercent: 100,
  secondaryOpacityPercent: 50,
  primaryFontSizePx: 20,
  auxiliaryScalePercent: 90,
  fontWeight: 550,
  lineGapPx: 10,
  textEffect: 'shadow',
  simplifiedToTraditional: true,
});

/**
 * Weight of the original lyric text, saved apart by each lyrics tab (playing page, taskbar, floating); the
 * translation and romanization rows are 50 lighter. Mirrors settings.rs `LYRICS_FONT_WEIGHT_RANGE`.
 */
export const LYRICS_FONT_WEIGHT = Object.freeze({ min: 100, max: 1000, step: 50, default: 550 });

/** The weight of translation and romanization rows: 50 below the original, never under 1. */
export function lyricsAuxiliaryWeight(/** @type {number} */ fontWeight) {
  return Math.max(1, fontWeight - 50);
}

/** @param {unknown} value Round and clamp to the allowed range; anything else becomes the default (550). */
export function normalizeLyricsFontWeight(value) {
  return typeof value === 'number' && Number.isFinite(value)
    ? Math.min(LYRICS_FONT_WEIGHT.max, Math.max(LYRICS_FONT_WEIGHT.min, Math.round(value)))
    : LYRICS_FONT_WEIGHT.default;
}

export const LYRICS_TEXT_EFFECTS = Object.freeze(['shadow', 'stroke', 'none']);

export const LYRICS_PREFERENCE_LIMITS = Object.freeze({
  mainOpacityPercent: Object.freeze({ min: 1, max: 100 }),
  secondaryOpacityPercent: Object.freeze({ min: 10, max: 100 }),
  primaryFontSizePx: Object.freeze({ min: 1, max: 50 }),
  auxiliaryScalePercent: Object.freeze({ min: 1, max: 100 }),
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
    mainOpacityPercent: normalizeInteger(
      candidate.mainOpacityPercent,
      LYRICS_PREFERENCE_LIMITS.mainOpacityPercent.min,
      LYRICS_PREFERENCE_LIMITS.mainOpacityPercent.max,
      DEFAULT_LYRICS_PREFERENCES.mainOpacityPercent,
    ),
    secondaryOpacityPercent: normalizeInteger(
      candidate.secondaryOpacityPercent,
      LYRICS_PREFERENCE_LIMITS.secondaryOpacityPercent.min,
      LYRICS_PREFERENCE_LIMITS.secondaryOpacityPercent.max,
      DEFAULT_LYRICS_PREFERENCES.secondaryOpacityPercent,
    ),
    primaryFontSizePx: normalizeInteger(
      candidate.primaryFontSizePx,
      LYRICS_PREFERENCE_LIMITS.primaryFontSizePx.min,
      LYRICS_PREFERENCE_LIMITS.primaryFontSizePx.max,
      DEFAULT_LYRICS_PREFERENCES.primaryFontSizePx,
    ),
    auxiliaryScalePercent: normalizeAuxiliaryScale(candidate),
    fontWeight: normalizeLyricsFontWeight(candidate.fontWeight),
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
 * Plain lyrics pass -1 and therefore never receive the secondary opacity.
 * @param {number} activeIndex
 * @param {string} playbackState
 */
export function getDisplayActiveLyricIndex(activeIndex, playbackState) {
  if ((playbackState !== 'playing' && playbackState !== 'paused' && playbackState !== 'buffering')
    || !Number.isInteger(activeIndex)
    || activeIndex < 0) {
    return -1;
  }
  return activeIndex;
}

/**
 * The opacity (0..1) of a timed line on the playing page: the main opacity for the current line, the secondary for
 * the others; no current line (`activeIndex` < 0: plain lyrics, before the first cue) shows every line at the main.
 * Absolute values, not relative to each other.
 * @param {number} lineIndex @param {number} activeIndex @param {number} mainOpacityPercent @param {number} secondaryOpacityPercent
 */
export function getLyricLineOpacity(lineIndex, activeIndex, mainOpacityPercent, secondaryOpacityPercent) {
  const limits = LYRICS_PREFERENCE_LIMITS;
  const main = normalizeInteger(mainOpacityPercent, limits.mainOpacityPercent.min, limits.mainOpacityPercent.max, DEFAULT_LYRICS_PREFERENCES.mainOpacityPercent);
  if (activeIndex < 0 || lineIndex === activeIndex) return main / 100;
  return normalizeInteger(secondaryOpacityPercent, limits.secondaryOpacityPercent.min, limits.secondaryOpacityPercent.max, DEFAULT_LYRICS_PREFERENCES.secondaryOpacityPercent) / 100;
}

/**
 * Translation/romanization size as a percent of the primary size. Older saves stored an
 * absolute `auxiliaryFontSizePx`; it migrates to round(aux / primary × 100) (18 / 20 → 90).
 * @param {Record<string, unknown>} candidate
 */
function normalizeAuxiliaryScale(candidate) {
  const { min, max } = LYRICS_PREFERENCE_LIMITS.auxiliaryScalePercent;
  if (typeof candidate.auxiliaryScalePercent === 'number' && Number.isFinite(candidate.auxiliaryScalePercent)) {
    return normalizeInteger(candidate.auxiliaryScalePercent, min, max, DEFAULT_LYRICS_PREFERENCES.auxiliaryScalePercent);
  }
  const auxiliary = candidate.auxiliaryFontSizePx;
  if (typeof auxiliary === 'number' && Number.isFinite(auxiliary)) {
    const primaryRaw = candidate.primaryFontSizePx;
    const primary = typeof primaryRaw === 'number' && Number.isFinite(primaryRaw) && primaryRaw >= 1
      ? primaryRaw : (typeof primaryRaw === 'number' && Number.isFinite(primaryRaw) ? 1 : DEFAULT_LYRICS_PREFERENCES.primaryFontSizePx);
    return normalizeInteger((auxiliary / primary) * 100, min, max, DEFAULT_LYRICS_PREFERENCES.auxiliaryScalePercent);
  }
  return DEFAULT_LYRICS_PREFERENCES.auxiliaryScalePercent;
}

/** Translation/romanization font size in px: primary × scale. @param {{ primaryFontSizePx: number, auxiliaryScalePercent: number }} preferences */
export function lyricsAuxiliaryFontSizePx(preferences) {
  return preferences.primaryFontSizePx * preferences.auxiliaryScalePercent / 100;
}

/** @param {unknown} value @param {number} min @param {number} max @param {number} fallback */
function normalizeInteger(value, min, max, fallback) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return fallback;
  return Math.max(min, Math.min(max, Math.round(value)));
}
