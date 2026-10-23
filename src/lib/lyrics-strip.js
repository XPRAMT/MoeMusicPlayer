// Continuous animation of the taskbar / floating lyrics (ADR 0032): every line joined into one strip, separated by an
// ideographic space (U+3000), that scrolls with playback so the current line sits at the chosen alignment. The
// translation strip is laid out with the same cell widths, so each translation stays under its own line.

import { normalizeLyricsPreferences } from './lyrics-preferences.js';

/** The separator between lines (one em wide). */
export const LYRICS_STRIP_SEPARATOR = '\u3000';
export const LYRICS_ANIMATIONS = Object.freeze(['none', 'continuous']);
/** The continuous strip anchors a line by its LEFT edge ... */
export const LYRICS_STRIP_ALIGN = 'left';
/** ... on the lyric area's horizontal CENTRE, whatever the text alignment setting and the panel width. */
export const LYRICS_STRIP_TARGET = 'center';
/** The terminal line advances at an estimated 500 ms for each grapheme. */
export const LYRICS_MS_PER_GRAPHEME = 500;

/** @param {string} value */
export function lyricsGraphemeCount(value) {
  if (typeof Intl.Segmenter === 'function') {
    return Array.from(new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(value)).length;
  }
  return Array.from(value).length;
}

/** @param {number} lineStartMs @param {string} text @param {number|null} [trackEndMs] */
export function lyricsStripFinalArrivalMs(lineStartMs, text, trackEndMs = null) {
  const estimated = lineStartMs + Math.max(1, lyricsGraphemeCount(text)) * LYRICS_MS_PER_GRAPHEME;
  return typeof trackEndMs === 'number' && Number.isFinite(trackEndMs) ? Math.min(estimated, trackEndMs) : estimated;
}

/** @param {unknown} value @returns {'none' | 'continuous'} */
export function normalizeLyricsAnimation(value) {
  return value === 'continuous' ? 'continuous' : 'none';
}

/** Avoid blur rasterization on the moving strip while preserving the selected effect in other modes.
 * @param {string} animation
 * @param {string} textEffect
 */
export function effectiveLyricsTextEffect(animation, textEffect) {
  return animation === 'continuous' && textEffect === 'shadow' ? 'stroke' : textEffect;
}

/**
 * The lines of the strip: timed lines with some text (main or translation), in time order.
 * @param {{ startMs: number, line: { text?: string | null, translation?: string | null } }[]} timeline sorted
 * @param {boolean} showTranslation
 * @returns {{ startMs: number, text: string, translation: string }[]}
 */
export function buildLyricsStripLines(timeline, showTranslation) {
  if (!Array.isArray(timeline)) return [];
  return timeline.flatMap((entry) => {
    const text = entry?.line?.text?.trim() ?? '';
    const translation = showTranslation ? (entry?.line?.translation?.trim() ?? '') : '';
    return text === '' && translation === '' ? [] : [{ startMs: entry.startMs, text, translation }];
  });
}

/**
 * Cell of each line along the strip (px): as wide as the wider of its main text and its translation, one separator
 * apart.
 * @param {readonly number[]} mainWidths @param {readonly number[]} translationWidths @param {number} gapPx
 * @returns {{ starts: number[], widths: number[] }}
 */
export function layoutLyricsStrip(mainWidths, translationWidths, gapPx) {
  const starts = [];
  const widths = [];
  let x = 0;
  for (let index = 0; index < mainWidths.length; index += 1) {
    const width = Math.max(0, mainWidths[index] || 0, translationWidths[index] || 0);
    starts.push(x);
    widths.push(width);
    x += width + Math.max(0, gapPx);
  }
  return { starts, widths };
}

/**
 * The point of a cell that sits on the alignment point of the window.
 * @param {{ starts: number[], widths: number[] }} layout @param {number} index @param {'left'|'center'|'right'} align
 */
export function lyricsStripAnchor(layout, index, align) {
  const start = layout.starts[index] ?? 0;
  const width = layout.widths[index] ?? 0;
  return align === 'right' ? start + width : align === 'center' ? start + width / 2 : start;
}

/**
 * Horizontal shift of the strip (px) at a lyrics position: the current line's anchor on the window's alignment point,
 * gliding linearly to the next line's anchor until the next line starts. Before the first line it rests on the first.
 * When the track duration and last main-text glyph centre are known, the final line glides until that glyph's centre
 * reaches the target at track end.
 * @param {readonly number[]} startTimesMs one per strip line, sorted
 * @param {{ starts: number[], widths: number[] }} layout
 * @param {'left'|'center'|'right'} align
 * @param {number} positionMs lyrics time (playback position plus the display offset)
 * @param {number} viewWidthPx
 * @param {'left'|'center'|'right'} [targetAlign] the point of the window the anchor sits on (default: the same as `align`)
 * @param {{endMs?:number|null,lastGlyphCenter?:number}|null} [finalLine]
 */
export function lyricsStripOffset(startTimesMs, layout, align, positionMs, viewWidthPx, targetAlign = align, finalLine = null) {
  const count = Math.min(startTimesMs.length, layout.starts.length);
  if (count === 0) return 0;
  const target = targetAlign === 'right' ? viewWidthPx : targetAlign === 'center' ? viewWidthPx / 2 : 0;
  let index = -1;
  for (let i = 0; i < count && startTimesMs[i] <= positionMs; i += 1) index = i;
  let anchor;
  if (index < 0) anchor = lyricsStripAnchor(layout, 0, align);
  else if (index >= count - 1) {
    const last = count - 1;
    const start = startTimesMs[last];
    const endMs = finalLine?.endMs;
    const lastGlyphCenter = Math.max(0, finalLine?.lastGlyphCenter ?? layout.widths[last] ?? 0);
    if (typeof endMs === 'number' && Number.isFinite(endMs) && endMs > start && positionMs > start) {
      const progress = Math.min(1, Math.max(0, (positionMs - start) / (endMs - start)));
      const from = layout.starts[last] ?? 0;
      const to = from + lastGlyphCenter;
      anchor = from + (to - from) * progress;
    } else {
      anchor = lyricsStripAnchor(layout, last, align);
    }
  }
  else {
    const span = startTimesMs[index + 1] - startTimesMs[index];
    const fraction = span > 0 ? Math.min(1, Math.max(0, (positionMs - startTimesMs[index]) / span)) : 1;
    const from = lyricsStripAnchor(layout, index, align);
    anchor = from + (lyricsStripAnchor(layout, index + 1, align) - from) * fraction;
  }
  return target - anchor;
}

/**
 * The cells to draw (ADR 0032): only the lines that can show while the strip glides from `fromOffsetPx` to
 * `toOffsetPx`, plus `marginPx` on both sides, so the strip never holds the whole song. Cells keep their place in the
 * layout, so swapping the window at a line change moves nothing on screen. Returns an empty range (last < first) when
 * there is nothing to draw.
 * @param {{ starts: number[], widths: number[] }} layout cells in order
 * @param {number} fromOffsetPx @param {number} toOffsetPx @param {number} viewWidthPx @param {number} marginPx
 * @returns {{ first: number, last: number }}
 */
export function lyricsStripWindow(layout, fromOffsetPx, toOffsetPx, viewWidthPx, marginPx) {
  const count = layout.starts.length;
  const left = -Math.max(fromOffsetPx, toOffsetPx) - Math.max(0, marginPx);
  const right = -Math.min(fromOffsetPx, toOffsetPx) + Math.max(0, viewWidthPx) + Math.max(0, marginPx);
  let first = 0;
  while (first < count && layout.starts[first] + (layout.widths[first] ?? 0) < left) first += 1;
  let last = first - 1;
  while (last + 1 < count && layout.starts[last + 1] <= right) last += 1;
  return { first, last };
}

/**
 * The next lyrics time at which the glide changes direction (the next line's start), or null when the strip rests.
 * @param {readonly number[]} startTimesMs @param {number} positionMs @param {number|null} [finalArrivalMs]
 */
export function nextLyricsStripBreak(startTimesMs, positionMs, finalArrivalMs = null) {
  if (startTimesMs.length === 0) return null;
  if (positionMs < startTimesMs[0]) return startTimesMs[0];
  for (const start of startTimesMs) if (start > positionMs) return start;
  const lastStart = startTimesMs[startTimesMs.length - 1];
  if (typeof finalArrivalMs === 'number' && Number.isFinite(finalArrivalMs) && finalArrivalMs > lastStart && finalArrivalMs > positionMs) return finalArrivalMs;
  return null;
}

/** How long a line takes to fade between "current" and "not current": it ends exactly when the next line starts. */
export const LYRICS_FADE_MS = 200;

/**
 * Which lines are mid-fade at a lyrics position (ADR 0032). The current line fades out and the next one fades in over
 * the last `fadeMs` before the next line starts (shortened to the current line's own length when it is shorter), so
 * both finish at the very moment the glide reaches the next line. Before the first line nothing is dimmed (like Now
 * Playing); in the `fadeMs` before the first line starts (never reaching back before position 0) every line but the
 * first fades out to "not current", so the first line's start is no jump. After the last line's start nothing fades. A pure function of the position, so seeks and pauses need no event.
 * @param {readonly number[]} startTimesMs one per strip line, sorted
 * @param {number} positionMs lyrics time (playback position plus the display offset)
 * @param {number} [fadeMs]
 * @returns {{ current: number, incoming: number, progress: number, fadeStartMs: number | null, fadeMs: number }}
 *   `current` is -1 before the first line; `incoming` is -1 after the last line starts.
 */
export function lyricsStripFocus(startTimesMs, positionMs, fadeMs = LYRICS_FADE_MS) {
  let current = -1;
  for (let i = 0; i < startTimesMs.length && startTimesMs[i] <= positionMs; i += 1) current = i;
  const incoming = current + 1 < startTimesMs.length ? current + 1 : -1;
  if (incoming < 0) return { current, incoming: -1, progress: 0, fadeStartMs: null, fadeMs: 0 };
  const next = startTimesMs[incoming];
  // Before the first line the fade may not start before position 0 (the lead-in is the intro, like Now Playing).
  const length = current >= 0 ? next - startTimesMs[current] : Math.max(1, next);
  const duration = Math.max(1, Math.min(Math.max(0, fadeMs), length));
  const fadeStartMs = next - duration;
  const progress = Math.min(1, Math.max(0, (positionMs - fadeStartMs) / duration));
  return { current, incoming, progress, fadeStartMs, fadeMs: duration };
}

/**
 * How "current" a line is, 0 (not current) to 1 (current): the fade progress for the two lines in the fade. Before the
 * first line every line counts as current (1) except in the fade before the first line starts, where all lines but the
 * first go 1 -> 0 and the first stays 1.
 * @param {{ current: number, incoming: number, progress: number }} focus from `lyricsStripFocus`
 * @param {number} index
 */
export function lyricsLineWeight(focus, index) {
  if (focus.current < 0) return focus.incoming < 0 || index === focus.incoming ? 1 : 1 - focus.progress;
  if (index === focus.current) return 1 - focus.progress;
  if (index === focus.incoming) return focus.progress;
  return 0;
}

/**
 * The opacity of a line relative to the window's own opacity setting (the container already applies that one, so the
 * product is main x not-current): 1 for a current line, `inactiveOpacityPercent / 100` for a line that is not,
 * linear in between. Only the continuous animation applies it; "none" leaves every line as it is.
 * @param {number} weight from `lyricsLineWeight`
 * @param {unknown} inactiveOpacityPercent the shared lyrics setting (10-100, normalized here)
 * @param {unknown} [animation]
 */
export function lyricsLineOpacityFactor(weight, inactiveOpacityPercent, animation = 'continuous') {
  if (normalizeLyricsAnimation(animation) !== 'continuous') return 1;
  const inactive = normalizeLyricsPreferences({ inactiveOpacityPercent }).inactiveOpacityPercent / 100;
  const clamped = Math.min(1, Math.max(0, weight));
  return inactive + (1 - inactive) * clamped;
}
