import { writable } from 'svelte/store';
import { tr } from './i18n.js';
import {
  AUDIO_FORMAT_COLUMN_FALLBACK_PX,
  audioFormatEntryKey,
  computeAudioFormatColumnWidthPx,
} from './track-columns.js';
import { createColumnTextMeasurer, onColumnFontsChange } from './column-text-measure.js';

/**
 * Pixel width of the audio format column, shared by every track list. It fits the widest format text (plus the
 * Hi-Res badge on rows that have one) of every row the lists have shown this session, measured with the real font.
 * It only grows while the app runs; a font or language change measures everything again.
 */
export const audioFormatColumnWidthPx = writable(AUDIO_FORMAT_COLUMN_FALLBACK_PX);

/** Every distinct (text, Hi-Res) pair seen, keyed by `audioFormatEntryKey`. @type {Map<string, { text: string, hiRes: boolean }>} */
const entries = new Map();
/** Entries not measured yet. @type {{ text: string, hiRes: boolean }[]} */
let pending = [];
let widestPx = 0;
let scheduled = false;
let observing = false;

/** Measure what is pending (or, with `all`, everything) and publish the width when it changed. */
function measureEntries(all = false) {
  const measure = createColumnTextMeasurer();
  if (!measure) return;
  const batch = all ? [...entries.values()] : pending;
  pending = [];
  const width = computeAudioFormatColumnWidthPx(batch, tr('音訊格式'), measure);
  widestPx = all ? width : Math.max(widestPx, width);
  audioFormatColumnWidthPx.update((current) => (current === widestPx ? current : widestPx));
}

function scheduleMeasure() {
  if (scheduled || typeof requestAnimationFrame === 'undefined') return;
  scheduled = true;
  requestAnimationFrame(() => {
    scheduled = false;
    measureEntries();
  });
}

/**
 * Report the audio format cells a list row renders. Already seen values cost one Map lookup; new ones are measured
 * on the next frame.
 * @param {{ id: string, text: string, hiRes?: boolean }[]} cells
 */
export function noteAudioFormatCells(cells) {
  for (const cell of cells) {
    if (cell.id !== 'audioFormat' || typeof cell.text !== 'string' || cell.text === '') continue;
    const entry = { text: cell.text, hiRes: cell.hiRes === true };
    const key = audioFormatEntryKey(entry);
    if (entries.has(key)) continue;
    entries.set(key, entry);
    pending.push(entry);
    if (!observing) {
      observing = true;
      onColumnFontsChange(() => measureEntries(true));
    }
    scheduleMeasure();
  }
}
