// Settings of the lyrics drawn on the Windows taskbar (ADR 0018). The backend (settings.rs `TaskbarLyricsSettings`)
// enforces the same ranges.

export const TASKBAR_LYRICS_FONT_SIZE_PX = Object.freeze({ min: 10, max: 32, default: 16 });
export const TASKBAR_LYRICS_ALIGNMENTS = Object.freeze(['left', 'center', 'right']);
export const TASKBAR_LYRICS_DEFAULT_CUSTOM_COLOR = '#ffffff';

/** @typedef {{ enabled: boolean, align: 'left'|'center'|'right', fontSizePx: number, useCustomColor: boolean, customColor: string, showTranslation: boolean }} TaskbarLyricsSettings */

/** @type {Readonly<TaskbarLyricsSettings>} */
export const DEFAULT_TASKBAR_LYRICS_SETTINGS = Object.freeze({
  enabled: false,
  align: 'left',
  fontSizePx: TASKBAR_LYRICS_FONT_SIZE_PX.default,
  useCustomColor: false,
  customColor: TASKBAR_LYRICS_DEFAULT_CUSTOM_COLOR,
  showTranslation: false,
});

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
  return {
    enabled: typeof source.enabled === 'boolean' ? source.enabled : defaults.enabled,
    align: TASKBAR_LYRICS_ALIGNMENTS.includes(/** @type {string} */ (source.align))
      ? /** @type {'left'|'center'|'right'} */ (source.align)
      : defaults.align,
    fontSizePx: size,
    useCustomColor: typeof source.useCustomColor === 'boolean' ? source.useCustomColor : defaults.useCustomColor,
    customColor: isHexColor(source.customColor) ? /** @type {string} */ (source.customColor).toLowerCase() : defaults.customColor,
    showTranslation: typeof source.showTranslation === 'boolean' ? source.showTranslation : defaults.showTranslation,
  };
}

/**
 * Text colour of the taskbar lyrics: the custom colour, or black or white to contrast with the taskbar's own theme.
 * @param {TaskbarLyricsSettings} settings
 * @param {boolean} taskbarIsLight
 */
export function getTaskbarLyricsColor(settings, taskbarIsLight) {
  if (settings.useCustomColor) return settings.customColor;
  return taskbarIsLight ? '#000000' : '#ffffff';
}

/**
 * Font size that fits the window: the chosen size, but never so large that one line (or a line plus a smaller
 * translation line) does not fit the window height.
 * @param {number} fontSizePx @param {number} heightPx @param {boolean} withTranslation
 */
export function fitTaskbarLyricsFontSize(fontSizePx, heightPx, withTranslation) {
  if (!(heightPx > 0)) return fontSizePx;
  // Line box = 1.4 em (the font's content box is 1.33 em, so descenders are not clipped); the translation line is 0.75 of the main size.
  const lines = withTranslation ? 1.4 * 1.75 : 1.4;
  return Math.max(8, Math.min(fontSizePx, Math.floor(heightPx / lines)));
}
