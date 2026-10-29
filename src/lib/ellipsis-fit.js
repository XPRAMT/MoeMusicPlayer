import { fitEndEllipsis, fitMiddleEllipsis } from './middle-ellipsis.js';

// Cached text fitting for the list cells (ADR 0037). The output is exactly what `fitMiddleEllipsis` / `fitEndEllipsis`
// return; this module only avoids repeating the work: one shared canvas context, a width cache and a result cache.

const SEPARATOR = '\u0000';
/** Joins a canvas font and its letter spacing in one font key (`canvasFontKey`). */
const SPACING_SEPARATOR = '\u0001';

/**
 * The canvas font of a computed style. Chromium returns an EMPTY `font` shorthand when the longhands come from
 * custom properties (the interface font is `var(--app-font-family)`), and assigning '' to `context.font` is ignored,
 * so measuring with the shorthand silently used the canvas default font. Build it from the longhands instead.
 * @param {{ font?: string, fontStyle?: string, fontVariant?: string, fontWeight?: string, fontStretch?: string, fontSize?: string, fontFamily?: string }} style
 */
export function canvasFontFromStyle(style) {
  const size = style.fontSize || '';
  const family = style.fontFamily || '';
  if (size && family) {
    const variant = style.fontVariant === 'small-caps' ? 'small-caps' : 'normal';
    const stretch = style.fontStretch && /^(normal|(ultra-|extra-|semi-)?(condensed|expanded))$/.test(style.fontStretch) ? style.fontStretch : 'normal';
    return [style.fontStyle || 'normal', variant, style.fontWeight || 'normal', stretch, size, family].join(' ');
  }
  return style.font || '';
}

/** Letter spacing in px of a computed style (`normal` is 0). @param {{ letterSpacing?: string }} style */
export function letterSpacingPx(style) {
  const value = Number.parseFloat(style.letterSpacing ?? '');
  return Number.isFinite(value) ? value : 0;
}

/** The fitter's font key: canvas font plus letter spacing. @param {CSSStyleDeclaration | Record<string, string>} style */
export function canvasFontKey(style) {
  const spacing = letterSpacingPx(style);
  return spacing ? canvasFontFromStyle(style) + SPACING_SEPARATOR + spacing : canvasFontFromStyle(style);
}
export const WIDTH_CACHE_SIZE = 4096;
export const FIT_CACHE_SIZE = 4096;

/** Opt-in counters (localStorage `moe.debug.ellipsis` = "1"); null when off so the hot path only tests one value. */
function createStats() {
  try {
    if (typeof localStorage === 'undefined' || localStorage.getItem('moe.debug.ellipsis') !== '1') return null;
  } catch {
    return null;
  }
  const stats = { measureText: 0, widthHits: 0, fitHits: 0, fitMisses: 0, fontReads: 0, layoutReads: 0, mounts: 0, destroys: 0, layoutRuns: 0, observerCallbacks: 0 };
  if (typeof window !== 'undefined') /** @type {any} */ (window).__moeEllipsisStats = stats;
  return stats;
}

class LruCache {
  /** @param {number} limit */
  constructor(limit) {
    this.limit = limit;
    /** @type {Map<string, any>} */
    this.map = new Map();
  }

  /** @param {string} key */
  get(key) {
    const map = this.map;
    if (!map.has(key)) return undefined;
    const value = map.get(key);
    map.delete(key);
    map.set(key, value);
    return value;
  }

  /** @param {string} key @param {any} value */
  set(key, value) {
    const map = this.map;
    if (map.has(key)) map.delete(key);
    else if (map.size >= this.limit) map.delete(/** @type {string} */ (map.keys().next().value));
    map.set(key, value);
  }

  get size() {
    return this.map.size;
  }
}

/**
 * @param {{
 *   getContext?: () => { font: string, measureText: (value: string) => { width: number } } | null,
 *   widthCacheSize?: number,
 *   fitCacheSize?: number,
 *   stats?: { measureText: number, widthHits: number, fitHits: number, fitMisses: number } | null,
 * }} [options]
 */
export function createEllipsisFitter(options = {}) {
  const getContext = options.getContext ?? (() => null);
  const widths = new LruCache(options.widthCacheSize ?? WIDTH_CACHE_SIZE);
  const fits = new LruCache(options.fitCacheSize ?? FIT_CACHE_SIZE);
  const stats = options.stats ?? null;
  let context = null;
  let lastFont = '';

  /** @param {string} font @param {string} value */
  function measureWidth(font, value) {
    const key = font + SEPARATOR + value;
    const cached = widths.get(key);
    if (cached !== undefined) {
      if (stats) stats.widthHits++;
      return cached;
    }
    context ??= getContext();
    let width;
    if (!context) {
      width = value.length;
    } else {
      const split = font.indexOf(SPACING_SEPARATOR);
      const canvasFont = split < 0 ? font : font.slice(0, split);
      const spacing = split < 0 ? 0 : Number(font.slice(split + 1)) || 0;
      if (canvasFont !== lastFont) {
        context.font = canvasFont;
        lastFont = canvasFont;
      }
      width = context.measureText(value).width + spacing * [...value].length;
      if (stats) stats.measureText++;
    }
    widths.set(key, width);
    return width;
  }

  /**
   * @param {string} text
   * @param {'middle' | 'end'} mode
   * @param {string} font canvas font shorthand (the host's computed `font`)
   * @param {number} maxWidth available width in CSS px
   * @returns {string}
   */
  function fit(text, mode, font, maxWidth) {
    if (!text) return '';
    if (!(maxWidth > 0)) return text;
    const key = font + SEPARATOR + mode + SEPARATOR + maxWidth + SEPARATOR + text;
    const cached = fits.get(key);
    if (cached !== undefined) {
      if (stats) stats.fitHits++;
      return cached;
    }
    if (stats) stats.fitMisses++;
    const measure = (/** @type {string} */ value) => measureWidth(font, value);
    // Most text fits: compare the (cached) full width first and skip the search.
    const result = measure(text) <= maxWidth
      ? text
      : (mode === 'end' ? fitEndEllipsis : fitMiddleEllipsis)(text, maxWidth, measure);
    fits.set(key, result);
    return result;
  }

  return { fit, measureWidth, widths, fits };
}

/** Width in CSS px that `clientWidth` would report for an unpadded, borderless host, read from a ResizeObserver entry. */
/** @param {any} entry */
export function hostWidthFromResizeEntry(entry) {
  const box = entry?.contentBoxSize;
  const size = Array.isArray(box) ? box[0] : box;
  const width = size ? size.inlineSize : entry?.contentRect?.width;
  return Math.round(width ?? 0);
}

const fontListeners = new Set();
/** @type {Map<string, string>} */
const fontCache = new Map();

/**
 * The computed `font` of a host, resolved once per font context instead of once per cell. The context is the caller's
 * `fontContext`, else the class names of the host's parent and grandparent (cells of one column share them).
 * @param {HTMLElement} host
 * @param {string | undefined} fontContext
 */
export function resolveHostFont(host, fontContext) {
  const parent = host.parentElement;
  const key = fontContext ?? `${parent?.tagName}.${parent?.className}|${parent?.parentElement?.className}`;
  let font = fontCache.get(key);
  if (font === undefined) {
    font = canvasFontKey(getComputedStyle(host));
    if (ellipsisStats) ellipsisStats.fontReads++;
    fontCache.set(key, font);
  }
  return font;
}

/** @type {WeakMap<Element, { width: number, callback: (width: number) => void }>} */
const observed = new WeakMap();
/** @type {ResizeObserver | null} */
let sharedObserver = null;

/**
 * One ResizeObserver for every cell. `callback` runs with the rounded width, only when that width changed.
 * @param {HTMLElement} host
 * @param {(width: number) => void} callback
 * @returns {() => void}
 */
export function observeHostWidth(host, callback) {
  sharedObserver ??= new ResizeObserver((entries) => {
    for (const entry of entries) {
      if (ellipsisStats) ellipsisStats.observerCallbacks++;
      const record = observed.get(entry.target);
      if (!record) continue;
      const width = hostWidthFromResizeEntry(entry);
      if (width === record.width) continue;
      record.width = width;
      record.callback(width);
    }
  });
  observed.set(host, { width: -1, callback });
  sharedObserver.observe(host);
  return () => {
    sharedObserver?.unobserve(host);
    observed.delete(host);
  };
}

/** Call when something that changes the drawn font changes (interface font, language, web fonts finishing loading). */
export function invalidateEllipsisFonts() {
  fontCache.clear();
  for (const listener of [...fontListeners]) listener();
}

/** @param {() => void} listener @returns {() => void} */
export function onEllipsisFontsInvalidated(listener) {
  fontListeners.add(listener);
  return () => fontListeners.delete(listener);
}

let sharedCanvas = null;

function sharedContext() {
  if (typeof document === 'undefined') return null;
  sharedCanvas ??= document.createElement('canvas');
  return sharedCanvas.getContext('2d');
}

export const ellipsisStats = createStats();

/** The app-wide fitter: one canvas context for every cell. */
export const sharedFitter = createEllipsisFitter({ getContext: sharedContext, stats: ellipsisStats });

if (typeof document !== 'undefined' && document.fonts?.addEventListener) {
  document.fonts.addEventListener('loadingdone', () => invalidateEllipsisFonts());
}

// The interface font and size settings are custom properties / classes on <html>: re-resolve fonts when they change.
let rootChangeScheduled = false;
if (typeof document !== 'undefined' && typeof MutationObserver !== 'undefined') {
  new MutationObserver(() => {
    if (rootChangeScheduled || typeof requestAnimationFrame === 'undefined') return;
    rootChangeScheduled = true;
    requestAnimationFrame(() => {
      rootChangeScheduled = false;
      invalidateEllipsisFonts();
    });
  }).observe(document.documentElement, { attributes: true, attributeFilter: ['style', 'class', 'lang'] });
}
