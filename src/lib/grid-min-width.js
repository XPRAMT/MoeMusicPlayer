// Smallest width a track-table row needs so a CSS grid template never collapses below its fixed /
// minmax() minimums. Rows use `gap: 10px` and `padding: 0 14px` (app.css `.track-row`).
export const TRACK_ROW_GAP_PX = 10;
export const TRACK_ROW_PADDING_X_PX = 28;

/** Split a grid-template-columns value at top-level whitespace (keeps minmax(...) intact). @param {string} template */
export function splitGridTemplate(template) {
  /** @type {string[]} */
  const tokens = [];
  let depth = 0;
  let current = '';
  for (const char of String(template ?? '')) {
    if (char === '(') depth += 1;
    if (char === ')') depth = Math.max(0, depth - 1);
    if (/\s/.test(char) && depth === 0) {
      if (current) tokens.push(current);
      current = '';
    } else {
      current += char;
    }
  }
  if (current) tokens.push(current);
  return tokens;
}

/** Minimum px of one track: px / em / rem lengths, minmax(min, max) uses min; fr / auto / % count as 0. @param {string} token @param {number} emPx */
export function getGridTrackMinPx(token, emPx) {
  const value = token.trim();
  const minmax = /^minmax\((.*),(.*)\)$/i.exec(value.replace(/\s+/g, ''));
  if (minmax) return getGridTrackMinPx(minmax[1], emPx);
  const length = /^(-?\d*\.?\d+)(px|r?em)$/i.exec(value);
  if (!length) return 0;
  const amount = Number(length[1]);
  return Math.max(0, length[2].toLowerCase() === 'px' ? amount : amount * emPx);
}

/** @param {string} template @param {number} emPx */
export function getGridTemplateMinWidthPx(template, emPx = 16) {
  const tokens = splitGridTemplate(template);
  if (tokens.length === 0) return 0;
  const tracks = tokens.reduce((sum, token) => sum + getGridTrackMinPx(token, emPx), 0);
  return Math.ceil(tracks + (tokens.length - 1) * TRACK_ROW_GAP_PX + TRACK_ROW_PADDING_X_PX);
}
