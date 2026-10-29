// Floating lyrics (ADR 0032): the taskbar lyrics' look options saved separately (settings.rs `FloatingLyricsSettings`),
// plus the move/fixed mode and the panel position. The line, translation and marquee rules are the taskbar ones.
// `locked`, `position` and `landscapePosition` are only shown here: Rust keeps the stored ones whatever the page sends.

import {
  LYRICS_HIDE_WHEN_PAUSED_DEFAULT,
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
export const FLOATING_LYRICS_LAYOUT = Object.freeze({
  lineHeight: 1.375,
  // The window is text height x 1.5 (= the lyric area x 1.5 / 1.1); the area is centred and the room around it is
  // transparent, for the shadow / glow. player-lyrics-native FLOATING_PANEL_LINE_HEIGHT / SIDE_MARGIN_EM.
  panelLineHeight: 1.875,
  sideMarginEm: 0.25,
});
/**
 * Lyric area width (the panel is this plus the lock gutter on its right): CSS px on Windows, dp on Android. Stored
 * within settings.rs `FloatingLyricsSettings::WIDTH_RANGE_PX`; the slider's maximum is the current screen's width.
 */
export const FLOATING_LYRICS_WIDTH_PX = Object.freeze({ min: 100, max: 8192, step: 10, default: 800 });

/** The lock button and its transparent gutter (CSS px): as tall as the main line, so it scales with the font. */
/** @param {unknown} fontSizePx */
export function floatingLyricsLockPx(fontSizePx) {
  const font = typeof fontSizePx === 'number' && Number.isFinite(fontSizePx) && fontSizePx > 0 ? fontSizePx : 1;
  return Math.max(1, Math.ceil(font * FLOATING_LYRICS_LAYOUT.lineHeight));
}

/** Transparent side margin (CSS px) left of the lyric area and right of the lock (floating_lyrics.rs `side_margin_css`). @param {unknown} fontSizePx */
export function floatingLyricsMarginPx(fontSizePx) {
  const font = typeof fontSizePx === 'number' && Number.isFinite(fontSizePx) && fontSizePx > 0 ? fontSizePx : 1;
  return Math.max(1, Math.ceil(font * FLOATING_LYRICS_LAYOUT.sideMarginEm));
}

/** Share of the window height that is the lyric area (1.1 / 1.5); the rest is split above and below it. */
export const FLOATING_LYRICS_AREA_SHARE = FLOATING_LYRICS_LAYOUT.lineHeight / FLOATING_LYRICS_LAYOUT.panelLineHeight;

/** The width slider's maximum: the current screen's width (Windows: the monitor holding the panel), never below min. */
/**
 * The width is the lyric area: with `fontSizePx`, the lock gutter and both side margins are taken off the screen width
 * (the whole window has to fit; floating_lyrics.rs `lyric_width_css`).
 * @param {unknown} screenWidth @param {unknown} [fontSizePx]
 */
export function floatingLyricsWidthMax(screenWidth, fontSizePx) {
  if (typeof screenWidth !== 'number' || !Number.isFinite(screenWidth) || screenWidth <= 0) return FLOATING_LYRICS_WIDTH_PX.max;
  const chrome = fontSizePx === undefined ? 0 : floatingLyricsLockPx(fontSizePx) + 2 * floatingLyricsMarginPx(fontSizePx);
  return Math.max(FLOATING_LYRICS_WIDTH_PX.min, Math.min(FLOATING_LYRICS_WIDTH_PX.max, Math.floor(screenWidth - chrome)));
}

/** D-pad timing: a tap moves 1 px; held past `delayMs` it repeats every `intervalMs` until released. */
export const FLOATING_LYRICS_NUDGE_REPEAT = Object.freeze({ delayMs: 300, intervalMs: 100 / 3 });

/**
 * Press a D-pad arrow: `step()` at once, then again every intervalMs once held past delayMs. Returns `release`, which
 * stops the repeat and calls `done()` once (where the position is saved).
 * @param {() => void} step @param {() => void} done
 * @param {{ setTimeout: (fn: () => void, ms: number) => any, clearTimeout: (id: any) => void, setInterval: (fn: () => void, ms: number) => any, clearInterval: (id: any) => void }} [timers]
 */
export function pressFloatingLyricsNudge(step, done, timers = globalThis) {
  /** @type {any} */
  let interval = null;
  let released = false;
  step();
  const delay = timers.setTimeout(() => {
    if (!released) interval = timers.setInterval(step, FLOATING_LYRICS_NUDGE_REPEAT.intervalMs);
  }, FLOATING_LYRICS_NUDGE_REPEAT.delayMs);
  return () => {
    if (released) return;
    released = true;
    timers.clearTimeout(delay);
    if (interval !== null) timers.clearInterval(interval);
    done();
  };
}
/**
 * The saved position is the lyric area's top-left as a ratio of the screen (settings.rs `FloatingLyricsPosition`,
 * `player_lyrics_rules::position_ratio`); values outside this range are dropped like in Rust.
 */
export const FLOATING_LYRICS_POSITION_RATIO_RANGE = Object.freeze({ min: -1, max: 2 });

/** @typedef {import('./ipc').FloatingLyricsSettings} FloatingLyricsSettings */

/** @type {Readonly<FloatingLyricsSettings>} */
export const DEFAULT_FLOATING_LYRICS_SETTINGS = Object.freeze({
  enabled: false,
  align: 'center',
  fontSizePx: 28,
  translationScalePercent: 80,
  mainOpacityPercent: 100,
  secondaryOpacityPercent: 50,
  fontWeight: 500,
  fontFamily: null,
  widthPx: FLOATING_LYRICS_WIDTH_PX.default,
  landscapeWidthPx: FLOATING_LYRICS_WIDTH_PX.default,
  useCustomColor: false,
  customColor: TASKBAR_LYRICS_DEFAULT_CUSTOM_COLOR,
  showTranslation: true,
  textEffect: 'shadow',
  animation: 'none',
  hideWhenPaused: LYRICS_HIDE_WHEN_PAUSED_DEFAULT,
  locked: false,
  position: null,
  landscapePosition: null,
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

/** A saved position (screen ratio, see FLOATING_LYRICS_POSITION_RATIO_RANGE). @param {unknown} value @returns {{ x: number, y: number } | null} */
export function normalizeFloatingLyricsPosition(value) {
  if (typeof value !== 'object' || value === null) return null;
  const { x, y } = /** @type {Record<string, unknown>} */ (value);
  const { min, max } = FLOATING_LYRICS_POSITION_RATIO_RANGE;
  const valid = (/** @type {unknown} */ item) => typeof item === 'number' && Number.isFinite(item) && item >= min && item <= max;
  if (!valid(x) || !valid(y)) return null;
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
    mainOpacityPercent: pick('mainOpacityPercent', finite),
    secondaryOpacityPercent: pick('secondaryOpacityPercent', finite),
    fontWeight: pick('fontWeight', finite),
    fontFamily: source.fontFamily,
    useCustomColor: pick('useCustomColor', bool),
    customColor: pick('customColor', isHexColor),
    showTranslation: pick('showTranslation', bool),
    textEffect: pick('textEffect', (item) => TASKBAR_LYRICS_TEXT_EFFECTS.includes(/** @type {string} */ (item))),
    animation: source.animation,
  });
  return {
    enabled: typeof source.enabled === 'boolean' ? source.enabled : defaults.enabled,
    align: look.align,
    fontSizePx: look.fontSizePx,
    translationScalePercent: look.translationScalePercent,
    mainOpacityPercent: look.mainOpacityPercent,
    secondaryOpacityPercent: look.secondaryOpacityPercent,
    fontWeight: look.fontWeight,
    fontFamily: look.fontFamily,
    widthPx: normalizeFloatingLyricsWidth(source.widthPx),
    landscapeWidthPx: normalizeFloatingLyricsWidth(source.landscapeWidthPx),
    useCustomColor: look.useCustomColor,
    customColor: look.customColor,
    showTranslation: look.showTranslation,
    textEffect: look.textEffect,
    animation: look.animation,
    hideWhenPaused: typeof source.hideWhenPaused === 'boolean' ? source.hideWhenPaused : defaults.hideWhenPaused,
    locked: typeof source.locked === 'boolean' ? source.locked : defaults.locked,
    position: normalizeFloatingLyricsPosition(source.position),
    landscapePosition: normalizeFloatingLyricsPosition(source.landscapePosition),
  };
}

/**
 * Lyrics for the Android overlay: timed lines only, sorted, with the lyrics' own offset and the per-track delay folded
 * into one offset (the overlay draws the line at `position + offsetMs`, like `findActiveLyricIndex`).
 * @param {{ trackId: string | null, offsetMs?: number | null, lines: { startMs?: number | null, endMs?: number | null, text?: string | null, translation?: string | null }[] } | null} data
 * @param {number} combinedOffsetMs from `lyricsDisplayOffsetMs`
 * @param {number | null} durationMs
 */
export function buildFloatingLyricsNativeData(data, combinedOffsetMs, durationMs = null) {
  if (!data || typeof data.trackId !== 'string') return { trackId: null, offsetMs: 0, durationMs: null, lines: [] };
  const lines = (Array.isArray(data.lines) ? data.lines : [])
    .map((line, index) => ({ line, index }))
    .filter(({ line }) => typeof line?.startMs === 'number' && Number.isFinite(line.startMs))
    .sort((left, right) => /** @type {number} */ (left.line.startMs) - /** @type {number} */ (right.line.startMs) || left.index - right.index)
    .map(({ line }) => {
      /** @type {{ startMs: number, endMs?: number, text: string, translation: string, words?: { s: number, e: number, t: string }[], lead?: number }} */
      const out = {
        startMs: Math.round(/** @type {number} */ (line.startMs)),
        text: (line.text ?? '').trim(),
        translation: (line.translation ?? '').trim(),
      };
      // The line's own end (ADR 0041 D10), in the same time as `startMs` (the offset applies to both alike).
      const endMs = /** @type {{ endMs?: number | null }} */ (line).endMs;
      if (typeof endMs === 'number' && Number.isFinite(endMs)) out.endMs = Math.round(endMs);
      // Word timing (ADR 0041), untrimmed like the line text it spells; `lead` = UTF-16 units trimmed off the front.
      const words = /** @type {{ words?: { s: number, e: number, t: string }[] | null }} */ (line).words;
      if (Array.isArray(words) && words.length > 0 && words.map((word) => word.t).join('') === (line.text ?? '')) {
        out.words = words.map((word) => ({ s: Math.round(word.s), e: Math.round(word.e), t: word.t }));
        out.lead = (line.text ?? '').length - (line.text ?? '').trimStart().length;
      }
      return out;
    });
  return {
    trackId: data.trackId,
    offsetMs: Number.isFinite(combinedOffsetMs) ? Math.round(combinedOffsetMs) : 0,
    durationMs: typeof durationMs === 'number' && Number.isFinite(durationMs) && durationMs > 0 ? Math.round(durationMs) : null,
    lines,
  };
}

/**
 * 「顯示在其他應用程式上層」 notice of the floating lyrics tab: Android only, and only once the status has been
 * read and says the permission is missing (granted or still unknown → nothing, so it never flashes).
 * @param {string | undefined | null} platform
 * @param {{ canDrawOverlays?: boolean } | null | undefined} status
 */
export function floatingLyricsOverlayNoticeVisible(platform, status) {
  return platform === 'android' && status != null && status.canDrawOverlays !== true;
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