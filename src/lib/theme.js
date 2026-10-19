/** @typedef {import('./ipc').ThemePreferences} ThemePreferences */

/** @type {Readonly<ThemePreferences>} */
export const DEFAULT_THEME_PREFERENCES = Object.freeze({
  backgroundHex: '#000000',
  accentHex: '#A9C3FE',
  borderOpacityPercent: 10,
  dividerOpacityPercent: 10,
  colorMode: 'autoBackgroundOpacity',
  backgroundOpacityTextColor: 'white',
  mainBackgroundClarityPercent: 65,
  coverBackgroundClarityPercent: 50,
  quickSettingsOpacityPercent: 20,
  quickSettingsBlurPx: 5,
  mainBackgroundBlurPx: 5,
  mainBackgroundOpacityPercent: 60,
  mainBackgroundMode: 'solid',
  slideshowFolder: null,
  slideshowIntervalMinutes: 2,
  slideshowOrder: 'sequential',
  autoTextColorThresholdPercent: 50,
});

const HEX_COLOR_PATTERN = /^#[0-9a-f]{6}$/i;

/** @param {unknown} value @returns {value is string} */
export function isHexColor(value) {
  return typeof value === 'string' && HEX_COLOR_PATTERN.test(value);
}

/** @param {unknown} value @returns {value is ThemePreferences['mainBackgroundMode']} */
function isMainBackgroundMode(value) {
  return value === 'solid'
    || value === 'singleImage'
    || value === 'slideshow'
    || value === 'albumCover'
    || value === 'systemWallpaper';
}

/** @param {unknown} value @returns {ThemePreferences} */
export function normalizeThemePreferences(value) {
  const candidate = value && typeof value === 'object'
    ? /** @type {Record<string, unknown>} */ (value)
    : {};

  return {
    backgroundHex: isHexColor(candidate.backgroundHex)
      ? candidate.backgroundHex.toUpperCase()
      : DEFAULT_THEME_PREFERENCES.backgroundHex,
    accentHex: isHexColor(candidate.accentHex)
      ? candidate.accentHex.toUpperCase()
      : DEFAULT_THEME_PREFERENCES.accentHex,
    borderOpacityPercent: normalizeRange(
      candidate.borderOpacityPercent,
      0,
      100,
      DEFAULT_THEME_PREFERENCES.borderOpacityPercent,
    ),
    dividerOpacityPercent: normalizeRange(
      candidate.dividerOpacityPercent,
      0,
      100,
      DEFAULT_THEME_PREFERENCES.dividerOpacityPercent,
    ),
    colorMode: candidate.colorMode === 'autoBackgroundOpacity' || candidate.colorMode === 'autoTextColor'
      ? candidate.colorMode
      : DEFAULT_THEME_PREFERENCES.colorMode,
    backgroundOpacityTextColor: candidate.backgroundOpacityTextColor === 'black'
      ? 'black'
      : DEFAULT_THEME_PREFERENCES.backgroundOpacityTextColor,
    mainBackgroundClarityPercent: normalizeRange(
      candidate.mainBackgroundClarityPercent ?? candidate.backgroundClarityPercent,
      0,
      100,
      DEFAULT_THEME_PREFERENCES.mainBackgroundClarityPercent,
    ),
    coverBackgroundClarityPercent: normalizeRange(
      candidate.coverBackgroundClarityPercent ?? candidate.backgroundClarityPercent,
      0,
      100,
      DEFAULT_THEME_PREFERENCES.coverBackgroundClarityPercent,
    ),
    quickSettingsOpacityPercent: normalizeOpacityPercent(candidate.quickSettingsOpacityPercent),
    quickSettingsBlurPx: normalizeRange(
      candidate.quickSettingsBlurPx,
      0,
      50,
      DEFAULT_THEME_PREFERENCES.quickSettingsBlurPx,
    ),
    mainBackgroundBlurPx: normalizeRange(candidate.mainBackgroundBlurPx, 0, 50, DEFAULT_THEME_PREFERENCES.mainBackgroundBlurPx),
    mainBackgroundOpacityPercent: normalizeRange(
      candidate.mainBackgroundOpacityPercent ?? candidate.mainBackgroundBrightnessPercent,
      0,
      100,
      DEFAULT_THEME_PREFERENCES.mainBackgroundOpacityPercent,
    ),
    mainBackgroundMode: isMainBackgroundMode(candidate.mainBackgroundMode)
      ? candidate.mainBackgroundMode
      : DEFAULT_THEME_PREFERENCES.mainBackgroundMode,
    slideshowFolder: typeof candidate.slideshowFolder === 'string' && candidate.slideshowFolder.length > 0
      ? candidate.slideshowFolder
      : null,
    slideshowIntervalMinutes: normalizeRange(
      candidate.slideshowIntervalMinutes,
      1,
      120,
      DEFAULT_THEME_PREFERENCES.slideshowIntervalMinutes,
    ),
    slideshowOrder: candidate.slideshowOrder === 'random'
      ? 'random'
      : DEFAULT_THEME_PREFERENCES.slideshowOrder,
    autoTextColorThresholdPercent: normalizeRange(
      candidate.autoTextColorThresholdPercent,
      0,
      100,
      DEFAULT_THEME_PREFERENCES.autoTextColorThresholdPercent,
    ),
  };
}

/** @param {unknown} value */
function normalizeOpacityPercent(value) {
  return normalizeRange(value, 0, 100, DEFAULT_THEME_PREFERENCES.quickSettingsOpacityPercent);
}

/** @param {unknown} value @param {number} min @param {number} max @param {number} fallback */
function normalizeRange(value, min, max, fallback) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return fallback;
  return Math.max(min, Math.min(max, Math.round(value)));
}

/** @param {string} hex */
function parseHexColor(hex) {
  const normalized = isHexColor(hex) ? hex.slice(1) : '000000';
  return [
    Number.parseInt(normalized.slice(0, 2), 16),
    Number.parseInt(normalized.slice(2, 4), 16),
    Number.parseInt(normalized.slice(4, 6), 16),
  ];
}

/** @param {number} channel */
function linearize(channel) {
  const value = channel / 255;
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

/**
 * WCAG relative luminance Y for an sRGB hex color (0–1).
 * @param {string} hex
 */
export function relativeLuminance(hex) {
  const [red, green, blue] = parseHexColor(isHexColor(hex) ? hex : DEFAULT_THEME_PREFERENCES.backgroundHex);
  return 0.2126 * linearize(red) + 0.7152 * linearize(green) + 0.0722 * linearize(blue);
}

/** @param {string} first @param {string} second */
export function contrastRatio(first, second) {
  const left = relativeLuminance(first);
  const right = relativeLuminance(second);
  const lighter = Math.max(left, right);
  const darker = Math.min(left, right);
  return (lighter + 0.05) / (darker + 0.05);
}

/**
 * Pick black or white text from relative luminance Y vs a 0–100 threshold.
 * Y at or above the threshold → black; below → white.
 * @param {string} backgroundHex
 * @param {number} [thresholdPercent]
 */
export function contrastingTextColor(backgroundHex, thresholdPercent = DEFAULT_THEME_PREFERENCES.autoTextColorThresholdPercent) {
  const background = isHexColor(backgroundHex) ? backgroundHex : DEFAULT_THEME_PREFERENCES.backgroundHex;
  const threshold = normalizeRange(
    thresholdPercent,
    0,
    100,
    DEFAULT_THEME_PREFERENCES.autoTextColorThresholdPercent,
  ) / 100;
  return relativeLuminance(background) >= threshold ? '#000000' : '#FFFFFF';
}

/** @param {number[]} channels */
function channelsToHex(channels) {
  return `#${channels.map((value) => Math.round(value).toString(16).padStart(2, '0')).join('')}`.toUpperCase();
}

/** @param {string} color @param {string} tint @param {number} amount */
function blend(color, tint, amount) {
  const source = parseHexColor(color);
  const target = parseHexColor(tint);
  return channelsToHex(source.map((channel, index) => channel + (target[index] - channel) * amount));
}

/**
 * Page color under an image at the given opacity percent (0–100).
 * @param {string} pageHex
 * @param {string | null | undefined} imageAverageHex
 * @param {number} opacityPercent
 */
export function compositeBackgroundHex(pageHex, imageAverageHex, opacityPercent) {
  const page = isHexColor(pageHex) ? pageHex.toUpperCase() : DEFAULT_THEME_PREFERENCES.backgroundHex;
  const image = isHexColor(imageAverageHex) ? imageAverageHex.toUpperCase() : null;
  const amount = typeof opacityPercent === 'number' && Number.isFinite(opacityPercent)
    ? Math.max(0, Math.min(100, opacityPercent)) / 100
    : 0;
  if (!image || amount <= 0) return page;
  if (amount >= 1) return image;
  return blend(page, image, amount);
}

/** @param {string} background @param {string} foreground @param {number} requestedAmount */
function readableSurfaceBlend(background, foreground, requestedAmount) {
  const requested = blend(background, foreground, requestedAmount);
  if (contrastRatio(requested, foreground) >= 4.5) return requested;

  let safeAmount = 0;
  let unsafeAmount = requestedAmount;
  for (let iteration = 0; iteration < 24; iteration += 1) {
    const candidateAmount = (safeAmount + unsafeAmount) / 2;
    const candidate = blend(background, foreground, candidateAmount);
    if (contrastRatio(candidate, foreground) >= 4.5) safeAmount = candidateAmount;
    else unsafeAmount = candidateAmount;
  }
  return blend(background, foreground, safeAmount);
}

/** @param {string} color */
function channels(color) {
  return parseHexColor(color).join(', ');
}

/**
 * @typedef {{ contrastBackgroundHex?: string | null, pageBackgroundHex?: string | null }} ThemeCssOptions
 * `contrastBackgroundHex` is the visual average after page+image mix; `--page` stays the solid theme color.
 */

/** @param {ThemePreferences | unknown} value @param {ThemeCssOptions} [options] */
export function createThemeCssVariables(value, options = {}) {
  const preferences = normalizeThemePreferences(value);
  const contrastBackground = isHexColor(options.contrastBackgroundHex)
    ? options.contrastBackgroundHex.toUpperCase()
    : preferences.backgroundHex;
  const pageBackground = isHexColor(options.pageBackgroundHex)
    ? options.pageBackgroundHex.toUpperCase()
    : preferences.backgroundHex;
  const threshold = preferences.autoTextColorThresholdPercent;
  const textColor = preferences.colorMode === 'autoBackgroundOpacity'
    ? (preferences.backgroundOpacityTextColor === 'black' ? '#000000' : '#FFFFFF')
    : contrastingTextColor(contrastBackground, threshold);
  // Inverse is always the other of black/white; do not re-run the Y threshold on the text color.
  const textInverse = textColor === '#000000' ? '#FFFFFF' : '#000000';
  const textChannels = channels(textColor);
  const textInverseChannels = channels(textInverse);
  const panel = readableSurfaceBlend(contrastBackground, textColor, 0.075);
  const panelSoft = readableSurfaceBlend(contrastBackground, textColor, 0.13);
  const dock = readableSurfaceBlend(contrastBackground, textColor, 0.08);
  const textSurfaces = [contrastBackground, panel, panelSoft, dock];
  /** @param {string} foreground */
  const readableOnSurfaces = (foreground) => textSurfaces.every(
    (surface) => contrastRatio(foreground, surface) >= 4.5,
  );
  const accentText = readableOnSurfaces(preferences.accentHex)
    ? preferences.accentHex
    : textColor;
  /** @param {string} foreground */
  const statusText = (foreground) => readableOnSurfaces(foreground) ? foreground : textColor;

  return {
    '--page': pageBackground,
    '--page-rgb': channels(pageBackground),
    '--panel': panel,
    '--panel-soft': panelSoft,
    '--dock-bg': dock,
    '--text': textColor,
    '--text-rgb': textChannels,
    '--text-inverse': textInverse,
    '--text-inverse-rgb': textInverseChannels,
    '--text-soft': textColor,
    '--muted': textColor,
    '--quiet': textColor,
    '--line': `rgba(${textChannels}, ${preferences.borderOpacityPercent / 100})`,
    '--divider-color': `rgba(${textChannels}, ${preferences.dividerOpacityPercent / 100})`,
    '--accent': preferences.accentHex,
    '--accent-rgb': channels(preferences.accentHex),
    '--accent-bright': blend(preferences.accentHex, textColor, 0.18),
    '--accent-text': accentText,
    '--accent-foreground': contrastingTextColor(preferences.accentHex, threshold),
    '--status-danger': statusText('#A61B1B'),
    '--status-success': statusText('#1F6B4F'),
    '--status-warning': statusText('#805500'),
    '--color-scheme': textColor === '#000000' ? 'light' : 'dark',
  };
}

/** @param {HTMLElement} root @param {ThemePreferences | unknown} preferences @param {ThemeCssOptions} [options] */
export function applyTheme(root, preferences, options = {}) {
  const variables = createThemeCssVariables(preferences, options);
  for (const [name, value] of Object.entries(variables)) root.style.setProperty(name, value);
  return variables;
}
