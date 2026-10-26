// Font pickers (ADR 0042): the four font settings (main interface, playing-page lyrics, taskbar lyrics, floating
// lyrics) store only a family name; null is 「預設字型」, today's font. The list comes from the backend (`fonts_list`),
// already grouped by the OS region and sorted with the app's collation; these pure helpers search it, window it and
// turn a family into CSS.

/** Same limit as settings.rs `FONT_FAMILY_MAX_CHARS`. */
export const FONT_FAMILY_MAX_CHARS = 256;

/** The main interface's default stack (app.css `--app-default-font`). */
export const DEFAULT_INTERFACE_FONT_STACK = "'Segoe UI', 'Microsoft JhengHei', sans-serif";

/** The interface's current stack (app.css `--app-font-stack`: the chosen interface font, else the default). */
export const APP_FONT_STACK_CSS = 'var(--app-font-stack)';

/** Height of one picker row (CSS px); the list is windowed on it. */
export const FONT_PICKER_ROW_PX = 36;

/**
 * @param {unknown} value
 * @returns {string | null} The trimmed family name, or null (the default font) for anything unusable.
 */
export function normalizeFontFamily(value) {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  return trimmed && [...trimmed].length <= FONT_FAMILY_MAX_CHARS ? trimmed : null;
}

/** @param {string} name A family name as a CSS string (quotes and backslashes escaped). */
export function quoteCssFamily(name) {
  return `"${String(name).replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/[\n\r\f]/g, ' ')}"`;
}

/** The `@font-face` family the Android WebView loads `family`'s files under (ADR 0042). */
export function androidFontAlias(/** @type {string} */ family) {
  return `MoeFont ${family}`;
}

/**
 * The CSS `font-family` value for a chosen family, falling back to `fallback` (a missing font draws in the default).
 * On Android the family is first tried through its `@font-face` alias.
 * @param {string | null | undefined} family
 * @param {string} fallback
 * @param {{ android?: boolean }} [options]
 */
export function fontFamilyCss(family, fallback, options = {}) {
  const name = normalizeFontFamily(family);
  if (!name) return fallback;
  const names = options.android ? [androidFontAlias(name), name] : [name];
  return [...names.map(quoteCssFamily), fallback].join(', ');
}

/** @param {string} value Lower-case, width-folded (NFKC) and without spaces, for matching. */
function fold(value) {
  return String(value).normalize('NFKC').toLowerCase().replace(/\s+/g, '');
}

/**
 * @typedef {{ path: string, faceIndex: number, weight: number, italic: boolean }} FontFace
 * @typedef {{ family: string, displayName: string, group: 'traditional'|'simplified'|'japanese'|'latin', faces: FontFace[] }} FontEntry
 */

/**
 * The fonts whose shown or stored name contains every word of `query` (case, width and spaces ignored); the order
 * stays the backend's.
 * @param {readonly FontEntry[]} fonts
 * @param {string} query
 */
export function filterFonts(fonts, query) {
  const words = String(query ?? '').trim().split(/\s+/).filter(Boolean).map(fold);
  if (!words.length) return [...fonts];
  return fonts.filter((font) => {
    const haystack = `${fold(font.displayName)}\u0000${fold(font.family)}`;
    return words.every((word) => haystack.includes(word));
  });
}

/**
 * The picker's rows: 「預設字型」 first (always, so it can be chosen while searching), then the matching fonts.
 * @param {readonly FontEntry[]} fonts
 * @param {string} query
 * @returns {Array<{ key: string, family: string | null, font: FontEntry | null }>}
 */
export function fontPickerRows(fonts, query) {
  return [
    { key: '\u0000default', family: null, font: null },
    ...filterFonts(fonts, query).map((font) => ({ key: font.family, family: font.family, font })),
  ];
}

/**
 * The rows to render for a scroll position: those in view plus `overscan` on each side.
 * @param {number} scrollTop
 * @param {number} viewportHeight
 * @param {number} rowHeight
 * @param {number} count
 * @param {number} [overscan]
 * @returns {{ start: number, end: number }} `start` inclusive, `end` exclusive.
 */
export function visibleRowRange(scrollTop, viewportHeight, rowHeight, count, overscan = 4) {
  if (count <= 0 || rowHeight <= 0) return { start: 0, end: 0 };
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const last = Math.ceil((Math.max(0, scrollTop) + Math.max(0, viewportHeight)) / rowHeight);
  return { start: Math.max(0, first - overscan), end: Math.min(count, Math.max(last, first + 1) + overscan) };
}

/**
 * The name shown for the current setting: the font's localized name, the stored name when it is not (or not yet)
 * listed, or null for the default font.
 * @param {readonly FontEntry[]} fonts
 * @param {string | null | undefined} family
 */
export function fontDisplayName(fonts, family) {
  const name = normalizeFontFamily(family);
  if (!name) return null;
  const lower = name.toLowerCase();
  return fonts.find((font) => font.family.toLowerCase() === lower)?.displayName ?? name;
}

/**
 * Replace the list after a background refresh: only when it changed (an unchanged refresh sends no list).
 * @param {FontEntry[]} current
 * @param {{ fonts?: FontEntry[], changed?: boolean } | null | undefined} update
 */
export function applyFontListUpdate(current, update) {
  return update?.changed && Array.isArray(update.fonts) ? update.fonts : current;
}

/**
 * The `@font-face` rules for `font` on Android: one per face, by weight and style, from its file (the WebView loads
 * a face only when text uses it). A TrueType collection is read at its first face (CSS has no face index).
 * @param {FontEntry} font
 * @param {(path: string) => string} toUrl `convertFileSrc`
 */
export function androidFontFaceSources(font, toUrl) {
  return font.faces.map((face) => ({
    family: androidFontAlias(font.family),
    source: `url(${JSON.stringify(toUrl(face.path))})`,
    descriptors: { weight: String(face.weight), style: face.italic ? 'italic' : 'normal' },
  }));
}
