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
 * @typedef {{ family: string, displayName: string, group: 'traditional'|'simplified'|'japanese'|'latin', faces: FontFace[], subset?: string }} FontEntry
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
 * Group headings for `rows` (from `fontPickerRows`): one before the first font of each group, in the backend's group
 * order (the OS region's), so a group without a matching font has no heading while searching. 「預設字型」 (row 0)
 * stays above the first heading.
 * @param {ReadonlyArray<{ font: FontEntry | null }>} rows
 * @returns {Array<{ group: FontEntry['group'], beforeIndex: number }>}
 */
export function fontPickerGroups(rows) {
  /** @type {Array<{ group: FontEntry['group'], beforeIndex: number }>} */
  const groups = [];
  rows.forEach((row, index) => {
    const group = row.font?.group;
    if (group && groups[groups.length - 1]?.group !== group) groups.push({ group, beforeIndex: index });
  });
  return groups;
}

/**
 * The family names of a CSS `font-family` list, unquoted; generic families (`sans-serif`) stay as they are.
 * @param {string} stack
 */
export function cssStackFamilies(stack) {
  return String(stack)
    .split(',')
    .map((part) => part.trim().replace(/^(['"])(.*)\1$/, '$2').trim())
    .filter(Boolean);
}

const GENERIC_FAMILIES = new Set(['serif', 'sans-serif', 'monospace', 'cursive', 'fantasy', 'system-ui']);

/**
 * The family the Windows native lyrics draw CJK text in when no font is chosen (`player-lyrics-native` win.rs:
 * Segoe UI, whose missing CJK and kana go to this family through the font fallback the renderer builds).
 */
export const WINDOWS_NATIVE_LYRICS_CJK_FAMILY = 'Microsoft JhengHei';

/**
 * The installed family 「預設字型」 actually draws a picker target's (CJK) text in, for the 「預設（…）」 label, or null when
 * it cannot be told.
 * - `interface`: the first family of the default stack that is installed and has CJK glyphs (Segoe UI has none, so
 *   Chinese and Japanese text falls through to Microsoft JhengHei, the family that draws most of the interface's
 *   text); else the first installed one; reaching the generic `sans-serif` means the system default (Android).
 * - `lyrics` (playing page): follows the interface font (`--app-font-stack`), so the chosen interface font when it is
 *   listed, else as `interface`.
 * - `taskbar` / `floating`: Windows: the native renderer's CJK family; Android (floating): the system default.
 * @param {{ target: 'interface' | 'lyrics' | 'taskbar' | 'floating', platform: string, fonts: readonly FontEntry[], systemDefault?: string | null, interfaceFamily?: string | null, stack?: string }} input
 * @returns {FontEntry | null}
 */
export function defaultFontFamily({ target, platform, fonts, systemDefault = null, interfaceFamily = null, stack = DEFAULT_INTERFACE_FONT_STACK }) {
  const find = (/** @type {string | null | undefined} */ name) => {
    const lower = normalizeFontFamily(name)?.toLowerCase();
    return lower ? fonts.find((font) => font.family.toLowerCase() === lower) ?? null : null;
  };
  if (target === 'taskbar' || target === 'floating') {
    return platform === 'windows' ? find(WINDOWS_NATIVE_LYRICS_CJK_FAMILY) : find(systemDefault);
  }
  if (target === 'lyrics') {
    const inherited = find(interfaceFamily);
    if (inherited) return inherited;
  }
  /** @type {FontEntry | null} */
  let firstInstalled = null;
  for (const name of cssStackFamilies(stack)) {
    if (GENERIC_FAMILIES.has(name.toLowerCase())) return firstInstalled ?? find(systemDefault);
    const font = find(name);
    if (font && font.group !== 'latin') return font;
    firstInstalled ??= font;
  }
  return firstInstalled ?? find(systemDefault);
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
