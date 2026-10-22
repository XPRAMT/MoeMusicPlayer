// Continuous animation of the taskbar / floating lyrics (ADR 0032): every line joined into one strip, separated by an
// ideographic space (U+3000), that scrolls with playback so the current line sits at the chosen alignment. The
// translation strip is laid out with the same cell widths, so each translation stays under its own line.

/** The separator between lines (one em wide). */
export const LYRICS_STRIP_SEPARATOR = '\u3000';
export const LYRICS_ANIMATIONS = Object.freeze(['none', 'continuous']);
/** The continuous strip always anchors the current line by its right edge, whatever the text alignment setting. */
export const LYRICS_STRIP_ALIGN = 'right';

/** @param {unknown} value @returns {'none' | 'continuous'} */
export function normalizeLyricsAnimation(value) {
  return value === 'continuous' ? 'continuous' : 'none';
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
 * gliding linearly to the next line's anchor until the next line starts. Before the first line it rests on the first;
 * the last line rests where it is.
 * @param {readonly number[]} startTimesMs one per strip line, sorted
 * @param {{ starts: number[], widths: number[] }} layout
 * @param {'left'|'center'|'right'} align
 * @param {number} positionMs lyrics time (playback position plus the display offset)
 * @param {number} viewWidthPx
 */
export function lyricsStripOffset(startTimesMs, layout, align, positionMs, viewWidthPx) {
  const count = Math.min(startTimesMs.length, layout.starts.length);
  if (count === 0) return 0;
  const target = align === 'right' ? viewWidthPx : align === 'center' ? viewWidthPx / 2 : 0;
  let index = -1;
  for (let i = 0; i < count && startTimesMs[i] <= positionMs; i += 1) index = i;
  let anchor;
  if (index < 0) anchor = lyricsStripAnchor(layout, 0, align);
  else if (index >= count - 1) anchor = lyricsStripAnchor(layout, count - 1, align);
  else {
    const span = startTimesMs[index + 1] - startTimesMs[index];
    const fraction = span > 0 ? Math.min(1, Math.max(0, (positionMs - startTimesMs[index]) / span)) : 1;
    const from = lyricsStripAnchor(layout, index, align);
    anchor = from + (lyricsStripAnchor(layout, index + 1, align) - from) * fraction;
  }
  return target - anchor;
}

/**
 * The next lyrics time at which the glide changes direction (the next line's start), or null when the strip rests.
 * @param {readonly number[]} startTimesMs @param {number} positionMs
 */
export function nextLyricsStripBreak(startTimesMs, positionMs) {
  if (startTimesMs.length < 2 || positionMs < startTimesMs[0]) return startTimesMs.length > 0 && positionMs < startTimesMs[0] ? startTimesMs[0] : null;
  for (const start of startTimesMs) if (start > positionMs) return start;
  return null;
}
