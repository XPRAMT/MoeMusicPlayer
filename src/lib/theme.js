/** @typedef {import('./ipc').ThemePreferences} ThemePreferences */

/** @type {Readonly<ThemePreferences>} */
export const DEFAULT_THEME_PREFERENCES = Object.freeze({
  backgroundHex: '#000000',
  accentHex: '#55D9FF',
  quickSettingsOpacityPercent: 70,
  mainBackgroundBlurPx: 20,
  mainBackgroundBrightnessPercent: 40,
});

const HEX_COLOR_PATTERN = /^#[0-9a-f]{6}$/i;

/** @param {unknown} value @returns {value is string} */
export function isHexColor(value) {
  return typeof value === 'string' && HEX_COLOR_PATTERN.test(value);
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
    quickSettingsOpacityPercent: normalizeOpacityPercent(candidate.quickSettingsOpacityPercent),
    mainBackgroundBlurPx: normalizeRange(candidate.mainBackgroundBlurPx, 0, 40, DEFAULT_THEME_PREFERENCES.mainBackgroundBlurPx),
    mainBackgroundBrightnessPercent: normalizeRange(
      candidate.mainBackgroundBrightnessPercent,
      0,
      100,
      DEFAULT_THEME_PREFERENCES.mainBackgroundBrightnessPercent,
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

/** @param {string} first @param {string} second */
export function contrastRatio(first, second) {
  const luminance = (/** @type {string} */ color) => {
    const [red, green, blue] = parseHexColor(color);
    return 0.2126 * linearize(red) + 0.7152 * linearize(green) + 0.0722 * linearize(blue);
  };
  const left = luminance(first);
  const right = luminance(second);
  const lighter = Math.max(left, right);
  const darker = Math.min(left, right);
  return (lighter + 0.05) / (darker + 0.05);
}

/** @param {string} backgroundHex */
export function contrastingTextColor(backgroundHex) {
  const background = isHexColor(backgroundHex) ? backgroundHex : DEFAULT_THEME_PREFERENCES.backgroundHex;
  const whiteContrast = contrastRatio(background, '#FFFFFF');
  const blackContrast = contrastRatio(background, '#000000');
  return whiteContrast >= blackContrast ? '#FFFFFF' : '#000000';
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

/** @param {ThemePreferences | unknown} value */
export function createThemeCssVariables(value) {
  const preferences = normalizeThemePreferences(value);
  const textColor = contrastingTextColor(preferences.backgroundHex);
  const textChannels = channels(textColor);
  const panel = readableSurfaceBlend(preferences.backgroundHex, textColor, 0.075);
  const panelSoft = readableSurfaceBlend(preferences.backgroundHex, textColor, 0.13);
  const sidebar = readableSurfaceBlend(preferences.backgroundHex, textColor, 0.035);
  const dock = readableSurfaceBlend(preferences.backgroundHex, textColor, 0.08);
  const textSurfaces = [preferences.backgroundHex, panel, panelSoft, sidebar, dock];
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
    '--page': preferences.backgroundHex,
    '--page-rgb': channels(preferences.backgroundHex),
    '--panel': panel,
    '--panel-soft': panelSoft,
    '--sidebar-bg': sidebar,
    '--dock-bg': dock,
    '--text': textColor,
    '--text-rgb': textChannels,
    '--text-soft': textColor,
    '--muted': textColor,
    '--quiet': textColor,
    '--line': `rgba(${textChannels}, 0.11)`,
    '--line-strong': `rgba(${textChannels}, 0.19)`,
    '--accent': preferences.accentHex,
    '--accent-rgb': channels(preferences.accentHex),
    '--accent-bright': blend(preferences.accentHex, textColor, 0.18),
    '--accent-text': accentText,
    '--accent-foreground': contrastingTextColor(preferences.accentHex),
    '--status-danger': statusText('#A61B1B'),
    '--status-success': statusText('#1F6B4F'),
    '--status-warning': statusText('#805500'),
    '--color-scheme': textColor === '#000000' ? 'light' : 'dark',
    '--preview-background': preferences.backgroundHex,
    '--preview-text': textColor,
    '--preview-accent': preferences.accentHex,
    '--preview-accent-text': contrastingTextColor(preferences.accentHex),
  };
}

/** @param {HTMLElement} root @param {ThemePreferences | unknown} preferences */
export function applyTheme(root, preferences) {
  const variables = createThemeCssVariables(preferences);
  for (const [name, value] of Object.entries(variables)) root.style.setProperty(name, value);
  return variables;
}
