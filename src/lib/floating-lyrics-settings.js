// Floating lyrics (ADR 0032): the taskbar lyrics' look options saved separately (settings.rs `FloatingLyricsSettings`),
// plus the move/fixed mode and the window position. The line, translation and marquee rules are the taskbar ones.

import {
  TASKBAR_LYRICS_ALIGNMENTS,
  TASKBAR_LYRICS_DEFAULT_CUSTOM_COLOR,
  TASKBAR_LYRICS_TEXT_EFFECTS,
  isHexColor,
  normalizeTaskbarLyricsSettings,
} from './taskbar-lyrics-settings.js';

/**
 * Keep in step with floating_lyrics.rs (padding, line box, lock button). The window is as tall as the lines it draws:
 * each line box is the text's own height (1.25 em) with a 10% buffer.
 */
export const FLOATING_LYRICS_LAYOUT = Object.freeze({ paddingPx: 8, minHeightPx: 44, lineHeight: 1.375, buttonPx: 32, buttonInsetPx: 6 });
/** Panel width: CSS px on Windows, dp on Android (settings.rs `FloatingLyricsSettings::WIDTH_RANGE_PX`). */
export const FLOATING_LYRICS_WIDTH_PX = Object.freeze({ min: 200, max: 4000, step: 10, default: 800 });
export const FLOATING_LYRICS_POSITION_LIMIT = 1_000_000;

/** @typedef {import('./ipc').FloatingLyricsSettings} FloatingLyricsSettings */

/** @type {Readonly<FloatingLyricsSettings>} */
export const DEFAULT_FLOATING_LYRICS_SETTINGS = Object.freeze({
  enabled: false,
  align: 'center',
  fontSizePx: 28,
  translationScalePercent: 80,
  opacityPercent: 100,
  widthPx: FLOATING_LYRICS_WIDTH_PX.default,
  landscapeWidthPx: FLOATING_LYRICS_WIDTH_PX.default,
  useCustomColor: false,
  customColor: TASKBAR_LYRICS_DEFAULT_CUSTOM_COLOR,
  showTranslation: true,
  textEffect: 'shadow',
  locked: false,
  position: null,
});

/** @param {unknown} value @returns {number} */
export function normalizeFloatingLyricsWidth(value) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return FLOATING_LYRICS_WIDTH_PX.default;
  return Math.max(FLOATING_LYRICS_WIDTH_PX.min, Math.min(FLOATING_LYRICS_WIDTH_PX.max, Math.round(value)));
}

/**
 * Which width field the slider edits: Android keeps portrait (`widthPx`) and landscape (`landscapeWidthPx`) apart;
 * Windows has one.
 * @param {boolean} isAndroid @param {boolean} landscape @returns {'widthPx' | 'landscapeWidthPx'}
 */
export function floatingLyricsWidthField(isAndroid, landscape) {
  return isAndroid && landscape ? 'landscapeWidthPx' : 'widthPx';
}

/** @param {unknown} value @returns {{ x: number, y: number } | null} */
export function normalizeFloatingLyricsPosition(value) {
  if (typeof value !== 'object' || value === null) return null;
  const { x, y } = /** @type {Record<string, unknown>} */ (value);
  if (!Number.isInteger(x) || !Number.isInteger(y)) return null;
  if (Math.abs(/** @type {number} */ (x)) > FLOATING_LYRICS_POSITION_LIMIT || Math.abs(/** @type {number} */ (y)) > FLOATING_LYRICS_POSITION_LIMIT) return null;
  return { x: /** @type {number} */ (x), y: /** @type {number} */ (y) };
}

/**
 * Repair field by field: a missing or malformed field takes the floating default (not the taskbar one); ranges and
 * colours follow the taskbar rules.
 * @param {unknown} value
 * @returns {FloatingLyricsSettings}
 */
export function normalizeFloatingLyricsSettings(value) {
  const source = typeof value === 'object' && value !== null ? /** @type {Record<string, unknown>} */ (value) : {};
  const defaults = DEFAULT_FLOATING_LYRICS_SETTINGS;
  /** @param {string} key @param {(item: unknown) => boolean} valid */
  const pick = (key, valid) => (valid(source[key]) ? source[key] : /** @type {Record<string, unknown>} */ (/** @type {unknown} */ (defaults))[key]);
  const finite = (/** @type {unknown} */ item) => typeof item === 'number' && Number.isFinite(item);
  const bool = (/** @type {unknown} */ item) => typeof item === 'boolean';
  const look = normalizeTaskbarLyricsSettings({
    align: pick('align', (item) => TASKBAR_LYRICS_ALIGNMENTS.includes(/** @type {string} */ (item))),
    fontSizePx: pick('fontSizePx', finite),
    translationScalePercent: pick('translationScalePercent', finite),
    opacityPercent: pick('opacityPercent', finite),
    useCustomColor: pick('useCustomColor', bool),
    customColor: pick('customColor', isHexColor),
    showTranslation: pick('showTranslation', bool),
    textEffect: pick('textEffect', (item) => TASKBAR_LYRICS_TEXT_EFFECTS.includes(/** @type {string} */ (item))),
  });
  return {
    enabled: typeof source.enabled === 'boolean' ? source.enabled : defaults.enabled,
    align: look.align,
    fontSizePx: look.fontSizePx,
    translationScalePercent: look.translationScalePercent,
    opacityPercent: look.opacityPercent,
    widthPx: normalizeFloatingLyricsWidth(source.widthPx),
    landscapeWidthPx: normalizeFloatingLyricsWidth(source.landscapeWidthPx),
    useCustomColor: look.useCustomColor,
    customColor: look.customColor,
    showTranslation: look.showTranslation,
    textEffect: look.textEffect,
    locked: typeof source.locked === 'boolean' ? source.locked : defaults.locked,
    position: normalizeFloatingLyricsPosition(source.position),
  };
}

/**
 * Lyrics for the Android overlay: timed lines only, sorted, with the lyrics' own offset and the per-track delay folded
 * into one offset (the overlay draws the line at `position + offsetMs`, like `findActiveLyricIndex`).
 * @param {{ trackId: string | null, offsetMs?: number | null, lines: { startMs?: number | null, text?: string | null, translation?: string | null }[] } | null} data
 * @param {number} combinedOffsetMs from `lyricsDisplayOffsetMs`
 * @param {number | null} durationMs
 */
export function buildFloatingLyricsNativeData(data, combinedOffsetMs, durationMs = null) {
  if (!data || typeof data.trackId !== 'string') return { trackId: null, offsetMs: 0, durationMs: null, lines: [] };
  const lines = (Array.isArray(data.lines) ? data.lines : [])
    .map((line, index) => ({ line, index }))
    .filter(({ line }) => typeof line?.startMs === 'number' && Number.isFinite(line.startMs))
    .sort((left, right) => /** @type {number} */ (left.line.startMs) - /** @type {number} */ (right.line.startMs) || left.index - right.index)
    .map(({ line }) => ({
      startMs: Math.round(/** @type {number} */ (line.startMs)),
      text: (line.text ?? '').trim(),
      translation: (line.translation ?? '').trim(),
    }));
  return {
    trackId: data.trackId,
    offsetMs: Number.isFinite(combinedOffsetMs) ? Math.round(combinedOffsetMs) : 0,
    durationMs: typeof durationMs === 'number' && Number.isFinite(durationMs) && durationMs > 0 ? Math.round(durationMs) : null,
    lines,
  };
}

/** The settings tabs of the lyrics section; the taskbar tab exists on Windows only. */
export const LYRICS_SETTINGS_TABS = Object.freeze(['general', 'taskbar', 'floating']);
export const LYRICS_SETTINGS_TAB_STORAGE_KEY = 'moe.settings.lyricsTab';

/**
 * @param {unknown} value the remembered tab
 * @param {boolean} isWindows
 * @returns {'general' | 'taskbar' | 'floating'}
 */
export function normalizeLyricsSettingsTab(value, isWindows) {
  if (value === 'floating') return 'floating';
  if (value === 'taskbar' && isWindows) return 'taskbar';
  return 'general';
}