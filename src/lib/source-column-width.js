import { writable } from 'svelte/store';
import { tr } from './i18n.js';
import { computeSourceColumnWidthPx, SOURCE_COLUMN_FALLBACK_PX } from './track-columns.js';
import { createColumnTextMeasurer, onColumnFontsChange } from './column-text-measure.js';

/** The label of every local track in the source column (the backend's `TRACK_SOURCE_NAME_SQL` fallback). */
export const LOCAL_SOURCE_LABEL = '本地';

/** Pixel width of the source column, shared by every track list. */
export const sourceColumnWidthPx = writable(SOURCE_COLUMN_FALLBACK_PX);

/** @type {string[]} */
let remoteNames = [];
let scheduled = false;
let observing = false;

/** Recompute the width from the current labels and fonts. */
export function remeasureSourceColumn() {
  const measure = createColumnTextMeasurer();
  if (!measure) return;
  sourceColumnWidthPx.set(computeSourceColumnWidthPx([tr(LOCAL_SOURCE_LABEL), ...remoteNames], tr('來源'), measure));
}

function scheduleRemeasure() {
  if (scheduled || typeof requestAnimationFrame === 'undefined') return;
  scheduled = true;
  requestAnimationFrame(() => {
    scheduled = false;
    remeasureSourceColumn();
  });
}

/**
 * Tell the column which servers exist (added, renamed or removed). Call with the names of all configured remote
 * accounts; the local label is always included.
 * @param {string[]} names
 */
export function setRemoteSourceNames(names) {
  remoteNames = names.filter((name) => typeof name === 'string' && name !== '');
  if (!observing) {
    observing = true;
    onColumnFontsChange(remeasureSourceColumn);
  }
  scheduleRemeasure();
}

/** Read the configured servers from the backend and size the column to them. Failures keep the previous width. */
export async function refreshRemoteSourceNames() {
  try {
    const { invokeCommand } = await import('./ipc');
    const accounts = await invokeCommand('remote_accounts_list', {});
    setRemoteSourceNames(accounts.map((account) => account.name));
  } catch {
    // keep the current width
  }
}
