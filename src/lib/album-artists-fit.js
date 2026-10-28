/**
 * Whole-lines fit of the album detail artist list. `line-clamp` does not work in the Android WebView, so the text is cut in
 * script: exactly the available whole lines are shown and the text ends with an ellipsis. Pure logic, the DOM is injected.
 */
export const ELLIPSIS = '…';
/** A new line count must be this far past the boundary (in lines) before it replaces the current one. */
export const HYSTERESIS_MARGIN_LINES = 0.02;

/**
 * Lines (fractional) that fit beside the stats block: (summary max height - stats height - gap + 0.5 px) / line height.
 * @param {number} maxHeight @param {number} statsHeight @param {number} gap @param {number} lineHeight
 * @returns {number}
 */
/** @param {number} maxHeight @param {number} statsHeight @param {number} gap @param {number} lineHeight */
export function rawLinesAvailable(maxHeight, statsHeight, gap, lineHeight) {
  if (!(lineHeight > 0)) return 1;
  const free = maxHeight - statsHeight - gap + 0.5;
  return Number.isFinite(free) ? free / lineHeight : 1;
}

/** Whole lines available, at least 1. @param {number} maxHeight @param {number} statsHeight @param {number} gap @param {number} lineHeight */
export function linesAvailable(maxHeight, statsHeight, gap, lineHeight) {
  return Math.max(1, Math.floor(rawLinesAvailable(maxHeight, statsHeight, gap, lineHeight)));
}

/**
 * Hysteresis: the first value is taken at once; a different count must be computed on two consecutive runs (and be past the
 * boundary by a small margin) before it replaces the current one, so a 1 px scroll wiggle near a threshold cannot flip it.
 */
export function createLinesHysteresis() {
  /** @type {number | null} */ let lines = null;
  /** @type {number | null} */ let pending = null;
  return {
    /** @param {number} raw fractional lines available @returns {number} */
    next(raw) {
      const want = Math.max(1, Math.floor(raw));
      if (lines === null) { lines = want; return lines; }
      const pastMargin = want > lines ? raw >= want + HYSTERESIS_MARGIN_LINES : raw < lines - HYSTERESIS_MARGIN_LINES;
      if (want === lines || !pastMargin) { pending = null; return lines; }
      if (pending === want) { lines = want; pending = null; } else pending = want;
      return lines;
    },
    reset() { lines = null; pending = null; },
    get current() { return lines; },
  };
}

/** Cut text: no trailing list separator or space before the ellipsis. @param {string} text */
export function ellipsize(text) {
  return text.replace(/[．\s]+$/, '') + ELLIPSIS;
}

/**
 * The full text when it fits `lines`; otherwise the longest prefix (binary search) that, followed by an ellipsis, still fits.
 * @param {string} full @param {number} lines @param {(text: string) => number} countLines rendered line count of a text
 * @returns {string}
 */
export function cutToLines(full, lines, countLines) {
  if (countLines(full) <= lines) return full;
  let lo = 0;
  let hi = full.length;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (countLines(ellipsize(full.slice(0, mid))) <= lines) lo = mid; else hi = mid - 1;
  }
  return ellipsize(full.slice(0, lo));
}
