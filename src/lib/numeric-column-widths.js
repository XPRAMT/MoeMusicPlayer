import { writable } from 'svelte/store';
import { tr } from './i18n.js';
import {
  computeNumericColumnWidthPx,
  NUMERIC_COLUMN_SAMPLE_PATTERNS,
  TRACK_COLUMN_DEFINITIONS,
  widestDigit,
} from './track-columns.js';
import { createColumnTextMeasurer, onColumnFontsChange } from './column-text-measure.js';

/**
 * Time/number columns sized from one sample value each (the widest realistic value in the column's display format,
 * drawn with the font's widest digit), not from the rows: O(1) measurements per column, redone only when the list
 * fonts, font size or language change, never while scrolling.
 * Keys: the track list column ids, and `artist:*` for the artist index.
 */
const SPECS = [
  ...TRACK_COLUMN_DEFINITIONS
    .filter(({ id }) => id in NUMERIC_COLUMN_SAMPLE_PATTERNS)
    .map(({ id, label, minPx }) => ({ key: id, header: label, pattern: NUMERIC_COLUMN_SAMPLE_PATTERNS[/** @type {keyof typeof NUMERIC_COLUMN_SAMPLE_PATTERNS} */ (id)], fallbackPx: minPx })),
  { key: 'artist:trackCount', header: tr('歌曲數量'), pattern: NUMERIC_COLUMN_SAMPLE_PATTERNS.count, fallbackPx: 78 },
  { key: 'artist:albumCount', header: tr('專輯數量'), pattern: NUMERIC_COLUMN_SAMPLE_PATTERNS.smallCount, fallbackPx: 78 },
  { key: 'artist:played', header: tr('累積播放'), pattern: NUMERIC_COLUMN_SAMPLE_PATTERNS.hours, fallbackPx: 78 },
  { key: 'artist:trackDuration', header: tr('播放時長'), pattern: NUMERIC_COLUMN_SAMPLE_PATTERNS.hours, fallbackPx: 78 },
];

/**
 * @param {(text: string, kind: 'cell' | 'header') => number} measure
 * @returns {Record<string, number>}
 */
export function computeNumericColumnWidths(measure) {
  const digit = widestDigit(measure);
  /** @type {Record<string, number>} */
  const widths = {};
  for (const spec of SPECS) widths[spec.key] = computeNumericColumnWidthPx(spec.pattern, spec.header, digit, measure, spec.fallbackPx);
  return widths;
}

/** Measured widths by key; empty until the first frame (the lists keep their previous fixed widths until then). */
export const numericColumnWidths = writable(/** @type {Record<string, number>} */ ({}));

export function remeasureNumericColumns() {
  const measure = createColumnTextMeasurer();
  if (!measure) return;
  const next = computeNumericColumnWidths(measure);
  numericColumnWidths.update((current) => (Object.keys(next).every((key) => current[key] === next[key]) ? current : next));
}

if (typeof requestAnimationFrame !== 'undefined' && typeof document !== 'undefined') {
  requestAnimationFrame(remeasureNumericColumns);
  onColumnFontsChange(remeasureNumericColumns);
}
