import { writable } from 'svelte/store';
import { tr } from './i18n.js';
import { computeSourceColumnWidthPx, SOURCE_COLUMN_FALLBACK_PX } from './track-columns.js';

/** The label of every local track in the source column (the backend's `TRACK_SOURCE_NAME_SQL` fallback). */
export const LOCAL_SOURCE_LABEL = '本地';

/** Pixel width of the source column, shared by every track list. */
export const sourceColumnWidthPx = writable(SOURCE_COLUMN_FALLBACK_PX);

/** @type {string[]} */
let remoteNames = [];
/** @type {HTMLElement | null} */
let probe = null;
let scheduled = false;
let observing = false;

/** @param {CSSStyleDeclaration} style */
function fontOf(style) {
  return `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
}

function buildProbe() {
  if (probe || typeof document === 'undefined') return probe;
  probe = document.createElement('div');
  probe.setAttribute('aria-hidden', 'true');
  probe.style.cssText = 'position:absolute;visibility:hidden;pointer-events:none;left:-9999px;top:0;';
  probe.innerHTML = '<div class="track-row"><span class="list-column"></span></div><div class="track-table-head"><span class="list-column"></span></div>';
  document.body.appendChild(probe);
  return probe;
}

/** Measure with the list's real fonts: probes carry the same classes as the list's rows and header. */
function measurer() {
  const host = buildProbe();
  if (!host || typeof document === 'undefined') return null;
  const cellStyle = getComputedStyle(/** @type {Element} */ (host.querySelector('.track-row .list-column')));
  const headStyle = getComputedStyle(/** @type {Element} */ (host.querySelector('.track-table-head .list-column')));
  const context = document.createElement('canvas').getContext('2d');
  if (!context) return null;
  const spacing = Number.parseFloat(headStyle.letterSpacing);
  return (/** @type {string} */ text, /** @type {'cell' | 'header'} */ kind) => {
    const style = kind === 'header' ? headStyle : cellStyle;
    context.font = fontOf(style);
    const letters = kind === 'header' && Number.isFinite(spacing) ? spacing * [...text].length : 0;
    return context.measureText(text).width + letters;
  };
}

/** Recompute the width from the current labels and fonts. */
export function remeasureSourceColumn() {
  const measure = measurer();
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

function observeFonts() {
  if (observing || typeof document === 'undefined') return;
  observing = true;
  document.fonts?.addEventListener?.('loadingdone', scheduleRemeasure);
  new MutationObserver(scheduleRemeasure).observe(document.documentElement, { attributes: true, attributeFilter: ['style', 'class', 'lang'] });
  window.addEventListener('resize', scheduleRemeasure);
}

/**
 * Tell the column which servers exist (added, renamed or removed). Call with the names of all configured remote
 * accounts; the local label is always included.
 * @param {string[]} names
 */
export function setRemoteSourceNames(names) {
  remoteNames = names.filter((name) => typeof name === 'string' && name !== '');
  observeFonts();
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
