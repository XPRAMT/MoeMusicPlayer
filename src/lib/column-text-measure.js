/** Text measurement with the track lists' real fonts, shared by the self-sizing columns (source, audio format). */
import { canvasFontFromStyle, letterSpacingPx, onEllipsisFontsInvalidated } from './ellipsis-fit.js';

/** @type {HTMLElement | null} */
let probe = null;

function buildProbe() {
  if (probe || typeof document === 'undefined') return probe;
  probe = document.createElement('div');
  probe.setAttribute('aria-hidden', 'true');
  probe.style.cssText = 'position:absolute;visibility:hidden;pointer-events:none;left:-9999px;top:0;';
  probe.innerHTML =
    '<div class="track-row"><span class="list-column"></span></div><div class="track-table-head"><span class="list-column"></span></div>' +
    '<div class="track-row"><span class="list-column is-numeric"><span data-tabular style="white-space:pre;font-variant-numeric:tabular-nums"></span></span></div>';
  document.body.appendChild(probe);
  return probe;
}

/**
 * A measuring function using the list's real fonts: probes carry the same classes as the list's rows and header.
 * Returns null outside a document.
 * With `tabularNums`, cells are measured in the DOM with `font-variant-numeric: tabular-nums` (as the numeric
 * columns render them): canvas ignores that property, and a font's tabular digits can be wider than its
 * proportional ones.
 * @param {{ tabularNums?: boolean }} [options]
 * @returns {((text: string, kind: 'cell' | 'header') => number) | null}
 */
export function createColumnTextMeasurer({ tabularNums = false } = {}) {
  const host = buildProbe();
  if (!host || typeof document === 'undefined') return null;
  const cellStyle = getComputedStyle(/** @type {Element} */ (host.querySelector('.track-row .list-column')));
  const headStyle = getComputedStyle(/** @type {Element} */ (host.querySelector('.track-table-head .list-column')));
  const context = document.createElement('canvas').getContext('2d');
  if (!context) return null;
  const tabular = /** @type {HTMLElement | null} */ (host.querySelector('[data-tabular]'));
  return (text, kind) => {
    if (tabularNums && kind === 'cell' && tabular) {
      tabular.textContent = text;
      return tabular.getBoundingClientRect().width;
    }
    const style = kind === 'header' ? headStyle : cellStyle;
    context.font = canvasFontFromStyle(style);
    return context.measureText(text).width + letterSpacingPx(style) * [...text].length;
  };
}

/** @type {Set<() => void>} */
const fontListeners = new Set();
let observing = false;
let scheduled = false;

function notifyFontChange() {
  if (scheduled || typeof requestAnimationFrame === 'undefined') return;
  scheduled = true;
  requestAnimationFrame(() => {
    scheduled = false;
    for (const listener of fontListeners) listener();
  });
}

/**
 * Call `listener` (once per frame at most) whenever the list fonts may have changed: web fonts loaded, the font
 * settings or language changed (style/class/lang on <html>), or the window resized.
 * @param {() => void} listener
 */
export function onColumnFontsChange(listener) {
  fontListeners.add(listener);
  if (observing || typeof document === 'undefined') return;
  observing = true;
  document.fonts?.addEventListener?.('loadingdone', notifyFontChange);
  new MutationObserver(notifyFontChange).observe(document.documentElement, { attributes: true, attributeFilter: ['style', 'class', 'lang'] });
  window.addEventListener('resize', notifyFontChange);
  // Interface font / size setting changes (App invalidates the ellipsis fonts then).
  onEllipsisFontsInvalidated(notifyFontChange);
}
