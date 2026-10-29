/**
 * Optical vertical centring of the list header labels (--track-header-text-optical-offset).
 * The header font falls back per language (the interface font may lack CJK glyphs, and the fallback font depends on lang), so the
 * line box and the glyphs' place in it differ by language. The offset is therefore measured from the text actually
 * shown: the current language's header labels, drawn with the header's font, letter spacing and lang. All list headers
 * share the `.track-table-head` font, so one value serves every list (and Android, same frontend).
 * Recomputed only when the list fonts may have changed (start, language, interface font or size, window/dpr), never
 * while scrolling.
 */
import { canvasFontFromStyle } from './ellipsis-fit.js';
import { onColumnFontsChange } from './column-text-measure.js';
import { TRACK_COLUMN_DEFINITIONS } from './track-columns.js';

/**
 * The shift (CSS px, negative = up) that puts the glyphs' ink centre on the centre of the label's line box (the box is
 * itself centred in the header row), snapped to whole device pixels.
 * @param {{ boxHeight: number, baseline: number, ascent: number, descent: number, devicePixelRatio: number }} metrics
 *   `baseline`: the baseline's distance from the top of the line box; `ascent`/`descent`: the ink above/below it.
 * @returns {number}
 */
export function computeHeaderOpticalOffsetPx({ boxHeight, baseline, ascent, descent, devicePixelRatio }) {
  const ratio = devicePixelRatio > 0 ? devicePixelRatio : 1;
  const inkCentre = baseline - (ascent - descent) / 2;
  const shift = boxHeight / 2 - inkCentre;
  if (!Number.isFinite(shift)) return 0;
  const snapped = Math.round(shift * ratio) / ratio;
  return Object.is(snapped, -0) ? 0 : snapped;
}

const INK_SCALE = 8;

/** @type {HTMLElement | null} */
let probe = null;

/** Measures the current language's header labels and sets the offset on <html>. */
export function remeasureHeaderTextOffset() {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  const text = TRACK_COLUMN_DEFINITIONS.map(({ label }) => label).join('');
  if (!probe) {
    probe = document.createElement('div');
    probe.setAttribute('aria-hidden', 'true');
    probe.style.cssText = 'position:absolute;visibility:hidden;pointer-events:none;left:-9999px;top:0;';
    probe.innerHTML = '<div class="track-table-head"><span class="column-sort-label" style="display:block;white-space:nowrap;overflow:hidden"><span></span><span style="display:inline-block;width:0;height:0"></span></span></div><canvas width="1" height="1"></canvas>';
    document.body.appendChild(probe);
  }
  // The real label is a flex item (blockified, nowrap, overflow hidden): its box is the line box, so the probe is too.
  const label = /** @type {HTMLElement} */ (probe.querySelector('.column-sort-label'));
  const [textNode, baselineMark] = /** @type {HTMLElement[]} */ ([...label.children]);
  const canvas = /** @type {HTMLCanvasElement} */ (probe.querySelector('canvas'));
  textNode.textContent = text;
  canvas.lang = root.lang;
  const style = getComputedStyle(label);
  const context = canvas.getContext('2d');
  if (!context) return;
  // Glyph bounds are reported in whole pixels; measuring at 8x the size keeps an eighth-pixel precision.
  const size = parseFloat(style.fontSize) || 13;
  context.font = canvasFontFromStyle(style).replace(style.fontSize, `${size * INK_SCALE}px`);
  const ink = context.measureText(text);
  const box = label.getBoundingClientRect();
  const offset = computeHeaderOpticalOffsetPx({
    boxHeight: box.height,
    baseline: baselineMark.getBoundingClientRect().bottom - box.top,
    ascent: ink.actualBoundingBoxAscent / INK_SCALE,
    descent: ink.actualBoundingBoxDescent / INK_SCALE,
    devicePixelRatio: window.devicePixelRatio,
  });
  const value = `${offset}px`;
  if (root.style.getPropertyValue('--track-header-text-optical-offset') !== value) {
    root.style.setProperty('--track-header-text-optical-offset', value);
  }
}

if (typeof requestAnimationFrame !== 'undefined' && typeof document !== 'undefined') {
  requestAnimationFrame(remeasureHeaderTextOffset);
  onColumnFontsChange(remeasureHeaderTextOffset);
}
