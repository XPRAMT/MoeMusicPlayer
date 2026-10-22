// Settings of the lyrics drawn on the Windows taskbar (ADR 0018). The backend (settings.rs `TaskbarLyricsSettings`)
// enforces the same ranges.

export const TASKBAR_LYRICS_FONT_SIZE_PX = Object.freeze({ min: 1, max: 50, default: 20 });
/** The translation line as a percentage of the main line (100 = the same size). */
export const TASKBAR_LYRICS_TRANSLATION_SCALE_PERCENT = Object.freeze({ min: 1, max: 100, default: 100 });
export const TASKBAR_LYRICS_ALIGNMENTS = Object.freeze(['left', 'center', 'right']);
export const TASKBAR_LYRICS_DEFAULT_CUSTOM_COLOR = '#00e1ff';

export const TASKBAR_LYRICS_TEXT_EFFECTS = Object.freeze(['shadow', 'stroke', 'none']);
/** Which taskbars show lyrics (ADR 0031): every monitor, the player window's monitor, or the chosen monitors. */
export const TASKBAR_LYRICS_DISPLAY_MODES = Object.freeze(['all', 'playerMonitor', 'custom']);
/** Same cap as settings.rs `MAX_CUSTOM_MONITORS`. */
export const TASKBAR_LYRICS_MAX_CUSTOM_MONITORS = 32;

/** @typedef {{ enabled: boolean, align: 'left'|'center'|'right', fontSizePx: number, translationScalePercent: number, useCustomColor: boolean, customColor: string, showTranslation: boolean, textEffect: 'shadow'|'stroke'|'none', displayMode: 'all'|'playerMonitor'|'custom', customMonitors: string[] }} TaskbarLyricsSettings */

/** @type {Readonly<TaskbarLyricsSettings>} */
export const DEFAULT_TASKBAR_LYRICS_SETTINGS = Object.freeze({
  enabled: true,
  align: 'center',
  fontSizePx: TASKBAR_LYRICS_FONT_SIZE_PX.default,
  translationScalePercent: TASKBAR_LYRICS_TRANSLATION_SCALE_PERCENT.default,
  useCustomColor: false,
  customColor: TASKBAR_LYRICS_DEFAULT_CUSTOM_COLOR,
  showTranslation: false,
  textEffect: 'shadow',
  displayMode: 'all',
  customMonitors: /** @type {string[]} */ (/** @type {unknown} */ (Object.freeze([]))),
});

/**
 * Stable monitor ids, trimmed and de-duplicated in order (offline monitors are kept: they are just ids).
 * @param {unknown} value
 * @returns {string[]}
 */
export function normalizeTaskbarLyricsMonitorIds(value) {
  if (!Array.isArray(value)) return [];
  /** @type {string[]} */
  const ids = [];
  for (const item of value) {
    if (typeof item !== 'string') continue;
    const id = item.trim();
    if (id !== '' && id.length <= 512 && !ids.includes(id)) ids.push(id);
  }
  return ids.slice(0, TASKBAR_LYRICS_MAX_CUSTOM_MONITORS);
}

/**
 * Turn one monitor on or off in the custom choice; the ids of unplugged monitors stay as they are.
 * @param {readonly string[]} chosen @param {string} id
 * @returns {string[]}
 */
export function toggleTaskbarLyricsMonitor(chosen, id) {
  const ids = normalizeTaskbarLyricsMonitorIds(chosen);
  return ids.includes(id) ? ids.filter((value) => value !== id) : normalizeTaskbarLyricsMonitorIds([...ids, id]);
}

/** @param {unknown} value */
export function isHexColor(value) {
  return typeof value === 'string' && /^#[0-9a-fA-F]{6}$/.test(value);
}

/**
 * Repair a settings object field by field: unknown choices and non-booleans become the defaults, the font size is
 * rounded and clamped, a colour that is not `#rrggbb` becomes white.
 * @param {unknown} value
 * @returns {TaskbarLyricsSettings}
 */
export function normalizeTaskbarLyricsSettings(value) {
  const source = typeof value === 'object' && value !== null ? /** @type {Record<string, unknown>} */ (value) : {};
  const defaults = DEFAULT_TASKBAR_LYRICS_SETTINGS;
  const size = typeof source.fontSizePx === 'number' && Number.isFinite(source.fontSizePx)
    ? Math.min(TASKBAR_LYRICS_FONT_SIZE_PX.max, Math.max(TASKBAR_LYRICS_FONT_SIZE_PX.min, Math.round(source.fontSizePx)))
    : defaults.fontSizePx;
  const scale = typeof source.translationScalePercent === 'number' && Number.isFinite(source.translationScalePercent)
    ? Math.min(TASKBAR_LYRICS_TRANSLATION_SCALE_PERCENT.max, Math.max(TASKBAR_LYRICS_TRANSLATION_SCALE_PERCENT.min, Math.round(source.translationScalePercent)))
    : defaults.translationScalePercent;
  return {
    enabled: typeof source.enabled === 'boolean' ? source.enabled : defaults.enabled,
    align: TASKBAR_LYRICS_ALIGNMENTS.includes(/** @type {string} */ (source.align))
      ? /** @type {'left'|'center'|'right'} */ (source.align)
      : defaults.align,
    fontSizePx: size,
    translationScalePercent: scale,
    useCustomColor: typeof source.useCustomColor === 'boolean' ? source.useCustomColor : defaults.useCustomColor,
    customColor: isHexColor(source.customColor) ? /** @type {string} */ (source.customColor).toLowerCase() : defaults.customColor,
    showTranslation: typeof source.showTranslation === 'boolean' ? source.showTranslation : defaults.showTranslation,
    textEffect: TASKBAR_LYRICS_TEXT_EFFECTS.includes(/** @type {string} */ (source.textEffect))
      ? /** @type {'shadow'|'stroke'|'none'} */ (source.textEffect)
      : defaults.textEffect,
    displayMode: TASKBAR_LYRICS_DISPLAY_MODES.includes(/** @type {string} */ (source.displayMode))
      ? /** @type {'all'|'playerMonitor'|'custom'} */ (source.displayMode)
      : defaults.displayMode,
    customMonitors: normalizeTaskbarLyricsMonitorIds(source.customMonitors),
  };
}

/**
 * Text colour of the taskbar lyrics: the custom colour, or black or white to contrast with the taskbar's own theme.
 * @param {{ useCustomColor: boolean, customColor: string }} settings
 * @param {boolean} taskbarIsLight
 */
export function getTaskbarLyricsColor(settings, taskbarIsLight) {
  if (settings.useCustomColor) return settings.customColor;
  return taskbarIsLight ? '#000000' : '#ffffff';
}

/**
 * The colour of the shadow or outline, as `r, g, b`: the opposite of the text, so it separates the text from whatever
 * is behind it (black behind light text, white behind dark text; the same idea as the Now Playing lyrics, which use
 * the inverse of the text colour).
 * @param {{ useCustomColor: boolean, customColor: string }} settings
 * @param {boolean} taskbarIsLight
 */
export function getTaskbarLyricsEffectRgb(settings, taskbarIsLight) {
  const text = getTaskbarLyricsColor(settings, taskbarIsLight);
  const red = parseInt(text.slice(1, 3), 16);
  const green = parseInt(text.slice(3, 5), 16);
  const blue = parseInt(text.slice(5, 7), 16);
  // Perceived brightness (Rec. 601 weights); light text gets a dark effect and the other way round.
  const brightness = 0.299 * red + 0.587 * green + 0.114 * blue;
  return brightness >= 128 ? '0, 0, 0' : '255, 255, 255';
}

/**
 * Font size that fits the window: the chosen size, but never so large that one line (or a line plus the translation
 * line at `translationScalePercent` of its size) does not fit the window height.
 * @param {number} fontSizePx @param {number} heightPx @param {number} translationScalePercent 0 when there is no translation line
 */
export function fitTaskbarLyricsFontSize(fontSizePx, heightPx, translationScalePercent) {
  if (!(heightPx > 0)) return fontSizePx;
  // Line box = 1.4 em (the font's content box is 1.33 em, so descenders are not clipped).
  const lines = 1.4 * (1 + Math.max(0, translationScalePercent) / 100);
  return Math.max(1, Math.min(fontSizePx, Math.floor(heightPx / lines)));
}

/**
 * How far the line has scrolled (0 to 1) when it is wider than the window: it scrolls over the time the line is shown,
 * and rests for the last tenth so the end of the line can be read before the next line comes.
 * @param {number} elapsedMs time since the line started @param {number} durationMs how long the line is shown
 */
export function getLyricScrollProgress(elapsedMs, durationMs) {
  if (!(durationMs > 0) || !Number.isFinite(elapsedMs)) return 0;
  return Math.min(1, Math.max(0, elapsedMs / (durationMs * 0.9)));
}
