import { tr } from './i18n.js';
// A/B spectrogram comparison helpers (ADR 0013): shared view (zoom/pan), layer geometry
// on shared axes, level sampling for hover readout, difference image, drop targeting.


export const COMPARE_MODES = [
  { value: 'single', label: tr('單張') },
  { value: 'sideH', label: tr('並排（左右）') },
  { value: 'sideV', label: tr('並排（上下）') },
  { value: 'overlay', label: tr('疊圖') },
  { value: 'wipe', label: tr('分割滑桿') },
];

/** @typedef {{ x0: number, x1: number, y0: number, y1: number }} View  y is bottom-up */
/** @typedef {{ mode: string, opacityB: number, difference: boolean, wipe: number }} ComparePrefs */

/** @type {View} */
export const FULL_VIEW = { x0: 0, x1: 1, y0: 0, y1: 1 };
const MIN_SPAN = 0.002;

/** @param {unknown} value @returns {ComparePrefs} */
export function normalizeComparePrefs(value) {
  const raw = /** @type {Record<string, any>} */ (value && typeof value === 'object' ? value : {});
  const clamp01 = (/** @type {unknown} */ v, /** @type {number} */ d) => (Number.isFinite(Number(v)) ? Math.min(1, Math.max(0, Number(v))) : d);
  return {
    mode: COMPARE_MODES.some((m) => m.value === raw.mode) ? raw.mode : 'single',
    opacityB: clamp01(raw.opacityB, 0.5),
    difference: raw.difference === true,
    wipe: clamp01(raw.wipe, 0.5),
  };
}


/**
 * Zoom one axis around an anchor (0..1 in view space). factor < 1 zooms in.
 * @param {View} view @param {'x' | 'y'} axis @param {number} anchor @param {number} factor
 * @returns {View}
 */
export function zoomView(view, axis, anchor, factor) {
  const [a, b] = axis === 'x' ? [view.x0, view.x1] : [view.y0, view.y1];
  const span = Math.min(1, Math.max(MIN_SPAN, (b - a) * factor));
  const center = a + (b - a) * anchor;
  let lo = center - span * anchor;
  lo = Math.min(1 - span, Math.max(0, lo));
  return axis === 'x' ? { ...view, x0: lo, x1: lo + span } : { ...view, y0: lo, y1: lo + span };
}

/** The tick steps (seconds) the time axis picks from. */
const TIME_TICK_STEPS = [0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 30, 60, 120, 300, 600, 1200, 1800, 3600, 7200];
/** The time axis ticks every 10 seconds when the labels fit. */
export const PREFERRED_TIME_TICK_SECONDS = 10;
/** Closest two time labels may stand, in px ("59:50", the widest, is about 33 px wide at the axis font). */
export const MIN_TIME_TICK_SPACING_PX = 40;
/** Beyond this spacing (a zoomed-in view) the axis gets finer steps than 10 seconds. */
const MAX_TIME_TICK_SPACING_PX = 400;

/**
 * The time axis step in seconds: one tick every 10 seconds, coarser when that would crowd the labels (a long track) and
 * finer when the view is zoomed far in (the labels would be a long way apart).
 * @param {number} visibleSeconds time span on screen  @param {number} widthPx width of the plot
 */
export function timeTickStep(visibleSeconds, widthPx) {
  if (!(visibleSeconds > 0) || !(widthPx > 0)) return PREFERRED_TIME_TICK_SECONDS;
  const spacing = (/** @type {number} */ step) => (widthPx * step) / visibleSeconds;
  const preferred = spacing(PREFERRED_TIME_TICK_SECONDS);
  if (preferred >= MIN_TIME_TICK_SPACING_PX && preferred <= MAX_TIME_TICK_SPACING_PX) return PREFERRED_TIME_TICK_SECONDS;
  if (preferred < MIN_TIME_TICK_SPACING_PX) {
    return TIME_TICK_STEPS.find((step) => step > PREFERRED_TIME_TICK_SECONDS && spacing(step) >= MIN_TIME_TICK_SPACING_PX) ?? TIME_TICK_STEPS[TIME_TICK_STEPS.length - 1];
  }
  // Zoomed in: the largest finer step whose labels are no more than 400 px apart.
  const finer = TIME_TICK_STEPS.filter((step) => step < PREFERRED_TIME_TICK_SECONDS && spacing(step) <= MAX_TIME_TICK_SPACING_PX);
  return finer.length > 0 ? finer[finer.length - 1] : TIME_TICK_STEPS[0];
}

/** The tick times (seconds) from `from` to `to`, multiples of `step`. @param {number} from @param {number} to @param {number} step */
export function timeTickValues(from, to, step) {
  /** @type {number[]} */
  const ticks = [];
  if (!(step > 0) || !(to >= from)) return ticks;
  for (let index = Math.ceil(from / step - 1e-9); index * step <= to + 1e-9; index += 1) ticks.push(Number((index * step).toFixed(6)));
  return ticks;
}

/** Pan by a delta in view fractions (positive dx moves content left). @param {View} view @param {number} dx @param {number} dy */
export function panView(view, dx, dy) {
  const sx = view.x1 - view.x0;
  const sy = view.y1 - view.y0;
  const x0 = Math.min(1 - sx, Math.max(0, view.x0 + dx * sx));
  const y0 = Math.min(1 - sy, Math.max(0, view.y0 + dy * sy));
  return { x0, x1: x0 + sx, y0, y1: y0 + sy };
}

/**
 * A two-finger gesture step: zoom both axes by the change of the distance between the fingers around where their
 * midpoint was, then pan by the movement of the midpoint. Positions are in px inside the plot, `dist` is the distance
 * between the fingers.
 * @param {View} view
 * @param {{ x: number, y: number, dist: number }} prev @param {{ x: number, y: number, dist: number }} next
 * @param {number} width @param {number} height
 * @returns {View}
 */
export function pinchView(view, prev, next, width, height) {
  if (!(width > 0) || !(height > 0)) return view;
  let result = view;
  if (prev.dist > 0 && next.dist > 0) {
    const factor = prev.dist / next.dist;
    const vx = prev.x / width;
    const vy = 1 - prev.y / height;
    result = zoomView(zoomView(result, 'x', vx, factor), 'y', vy, factor);
  }
  return panView(result, -(next.x - prev.x) / width, (next.y - prev.y) / height);
}

/** CSS box (percent) placing the world inside the viewport. @param {View} view */
export function worldStyle(view) {
  const sx = view.x1 - view.x0;
  const sy = view.y1 - view.y0;
  return { left: (-view.x0 / sx) * 100, width: 100 / sx, bottom: (-view.y0 / sy) * 100, height: 100 / sy };
}

/**
 * Fraction of the shared frequency axis covered by a layer whose own top is `ownMaxHz`.
 * @param {number} ownMaxHz @param {number} sharedMaxHz @param {boolean} log
 */
export function layerHeightFraction(ownMaxHz, sharedMaxHz, log, minLog = 20) {
  if (ownMaxHz >= sharedMaxHz) return 1;
  if (!log) return ownMaxHz / sharedMaxHz;
  const lo = Math.min(minLog, sharedMaxHz / 2);
  return Math.log(ownMaxHz / lo) / Math.log(sharedMaxHz / lo);
}

/**
 * Sample a layer's quantized level at world coords (wx, wy bottom-up, 0..1).
 * @param {{ levels: Uint8Array, columns: number, height: number, widthFraction: number, heightFraction: number }} layer
 * @param {number} wx @param {number} wy
 * @returns {number | null}
 */
export function sampleLevel(layer, wx, wy) {
  if (wx < 0 || wy < 0 || wx >= layer.widthFraction || wy >= layer.heightFraction) return null;
  const col = Math.min(layer.columns - 1, Math.floor((wx / layer.widthFraction) * layer.columns));
  const row = Math.min(layer.height - 1, Math.floor((1 - wy / layer.heightFraction) * layer.height));
  return layer.levels[row * layer.columns + col] ?? null;
}

/** @param {number} level @param {number} floor @param {number} ceiling */
export function levelToDb(level, floor, ceiling) {
  return floor + (level / 255) * (ceiling - floor);
}

/**
 * Difference image B − A on the shared world grid as RGBA. Diverging blue–white–red;
 * full colour at ±half the dB range. Areas covered by neither stay black.
 * @param {Parameters<typeof sampleLevel>[0]} a @param {Parameters<typeof sampleLevel>[0]} b
 * @param {number} width @param {number} height
 */
export function differenceImage(a, b, width, height) {
  const out = new Uint8ClampedArray(width * height * 4);
  for (let y = 0; y < height; y += 1) {
    const wy = 1 - (y + 0.5) / height;
    for (let x = 0; x < width; x += 1) {
      const wx = (x + 0.5) / width;
      const va = sampleLevel(a, wx, wy);
      const vb = sampleLevel(b, wx, wy);
      const i = (y * width + x) * 4;
      out[i + 3] = 255;
      if (va === null && vb === null) continue;
      const t = Math.max(-1, Math.min(1, ((vb ?? 0) - (va ?? 0)) / 127.5));
      if (t >= 0) { out[i] = 255; out[i + 1] = Math.round(255 * (1 - t)); out[i + 2] = Math.round(255 * (1 - t)); }
      else { out[i] = Math.round(255 * (1 + t)); out[i + 1] = Math.round(255 * (1 + t)); out[i + 2] = 255; }
    }
  }
  return out;
}

/**
 * Where dropped files go. Two or more files fill A then B. One file goes to the hovered
 * side: explicit `[data-drop-side]` (side-by-side), else left/right half in overlay/wipe,
 * else A.
 * @param {string[]} paths @param {string} mode @param {'A' | 'B' | null} hoveredSide @param {number} xFraction
 * @returns {{ A?: string, B?: string }}
 */
export function dropAssignment(paths, mode, hoveredSide, xFraction) {
  if (paths.length === 0) return {};
  if (paths.length >= 2) return { A: paths[0], B: paths[1] };
  if (mode === 'single') return { A: paths[0] };
  if (hoveredSide) return { [hoveredSide]: paths[0] };
  if (mode === 'overlay' || mode === 'wipe') return { [xFraction < 0.5 ? 'A' : 'B']: paths[0] };
  return { A: paths[0] };
}
