// Now-playing cover zoom: wheel (mouse over the cover) or two-finger pinch scales the cover itself (CSS transform on
// the cover element); double click / double tap restores. The layout reserves the zoomed cover size so the lyrics
// area is squeezed by the cover instead of scaling with it (see --np-cover-reserve in app.css). Pure math is exported
// for tests; `coverZoom` is the Svelte action that wires the events.

import { parseSwipeFeedback, withoutSwipeFeedback } from './cover-swipe-feedback.js';
import { releasesOnMove, releasesOnWindowEvent } from './cover-pointers.js';
import { COVER_RERASTER_RESTORE_MS, COVER_RERASTER_TRANSITION, COVER_RERASTER_TRANSLATE, createRerasterState, invalidateReraster, isRerasterTranslate, markRerastered, observeRerasterScale, planReraster } from './cover-reraster.js';

export const COVER_ZOOM_MIN = 0.5;
export const COVER_ZOOM_REST = 1;
export const COVER_ZOOM_MAX = 10;
export const COVER_ZOOM_WHEEL_STEP = 0.0015;
export const COVER_DOUBLE_TAP_MS = 300;
export const COVER_DOUBLE_TAP_SLOP_PX = 24;
export const COVER_PAN_SLOP_PX = 6;
/** No zoom input for this long = the zoom gesture is over: the lyrics re-wrap and re-centre once. */
export const COVER_ZOOM_SETTLE_MS = 50;
/** The scale stable for this long (and no gesture) = the zoom is over: the stage gets one re-raster nudge (ADR 0036). */
export const COVER_LAYER_IDLE_MS = 50;
/** The double-click / double-tap reset glide (a Web Animation on the stage itself). */
export const COVER_RESET_MS = 200;
export const COVER_RESET_EASING = 'ease';
const COVER_WHEEL_ANCHOR_SLOP_PX = 2;
/** A timer may fire a hair early: the stability check allows this much (ms) under the idle time. */
const COVER_TIMER_SLACK_MS = 2;
/** A nudge waits for a swipe's feedback to end at most this many timer periods (the swipe's own transition re-rasters). */
const COVER_FEEDBACK_WAIT_LIMIT = 40;

/** @typedef {{ scale: number, x: number, y: number }} CoverZoomState */
/** @type {CoverZoomState} */
export const COVER_ZOOM_IDENTITY = Object.freeze({ scale: 1, x: 0, y: 0 });

/** @param {number} value @param {number} min @param {number} max */
function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

/**
 * Zoom range is COVER_ZOOM_MIN..COVER_ZOOM_MAX (1 = original size; below 1 the cover shrinks in place). Keep an enlarged cover covering its own unscaled box: |offset| <= (scale - 1) * size / 2.
 * @param {CoverZoomState} state @param {number} width @param {number} height @returns {CoverZoomState}
 */
export function clampCoverZoom(state, width, height) {
  const scale = clamp(Number.isFinite(state.scale) ? state.scale : 1, COVER_ZOOM_MIN, COVER_ZOOM_MAX);
  if (scale <= COVER_ZOOM_REST) return { scale, x: 0, y: 0 };
  const maxX = ((scale - 1) * Math.max(0, width)) / 2;
  const maxY = ((scale - 1) * Math.max(0, height)) / 2;
  return { scale, x: clamp(state.x, -maxX, maxX), y: clamp(state.y, -maxY, maxY) };
}

/**
 * Zoom to `nextScale` keeping the screen point under the cursor/fingers fixed.
 * `anchorX/Y` are relative to the cover's current visual centre (screen px).
 * @param {CoverZoomState} state @param {number} nextScale @param {number} anchorX @param {number} anchorY @param {number} width @param {number} height
 */
export function zoomCoverAt(state, nextScale, anchorX, anchorY, width, height) {
  const scale = clamp(nextScale, COVER_ZOOM_MIN, COVER_ZOOM_MAX);
  const ratio = 1 - scale / state.scale;
  return clampCoverZoom({ scale, x: state.x + anchorX * ratio, y: state.y + anchorY * ratio }, width, height);
}

/**
 * Wheel delta (pixels; line/page modes are scaled up) to a multiplicative zoom factor. Wheel up zooms in.
 * @param {number} deltaY @param {number} [deltaMode]
 */
export function getWheelZoomFactor(deltaY, deltaMode = 0) {
  const pixels = deltaMode === 1 ? deltaY * 16 : deltaMode === 2 ? deltaY * 100 : deltaY;
  return Math.exp(-clamp(pixels, -240, 240) * COVER_ZOOM_WHEEL_STEP);
}

/** @param {{ x: number, y: number }} a @param {{ x: number, y: number }} b */
export function pointerDistance(a, b) {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

/**
 * The pan as fractions of the cover's own size, for settings.json (so it survives a window resize).
 * @param {CoverZoomState} state @param {number} width @param {number} height
 */
export function coverZoomToFractions(state, width, height) {
  return {
    scale: state.scale,
    offsetX: width > 0 ? state.x / width : 0,
    offsetY: height > 0 ? state.y / height : 0,
  };
}

/**
 * A saved zoom back into pixels for a cover of `width` x `height`, repaired and clamped.
 * @param {{ scale?: number, offsetX?: number, offsetY?: number } | null | undefined} saved @param {number} width @param {number} height
 * @returns {CoverZoomState}
 */
export function coverZoomFromFractions(saved, width, height) {
  const scale = typeof saved?.scale === 'number' && Number.isFinite(saved.scale) ? saved.scale : COVER_ZOOM_REST;
  const x = typeof saved?.offsetX === 'number' && Number.isFinite(saved.offsetX) ? saved.offsetX * width : 0;
  const y = typeof saved?.offsetY === 'number' && Number.isFinite(saved.offsetY) ? saved.offsetY * height : 0;
  return clampCoverZoom({ scale, x, y }, width, height);
}

/** CSS transform for the cover (transform-origin: center). @param {CoverZoomState} state */
export function coverZoomTransform(state) {
  if (state.scale === COVER_ZOOM_REST) return '';
  return `translate(${state.x.toFixed(2)}px, ${state.y.toFixed(2)}px) scale(${state.scale.toFixed(4)})`;
}

/**
 * A mouse drag pans the cover only while it is enlarged; shrunk or at rest it is a swipe (next / previous / close).
 * Marked on the stage as `data-cover-pannable`, which `swipeDismiss` reads. @param {number} scale
 */
export function isCoverPannable(scale) {
  return scale > COVER_ZOOM_REST;
}

/**
 * Second tap close enough in time and place to the first.
 * @param {{ x: number, y: number, t: number } | null} previous @param {{ x: number, y: number, t: number }} next
 */
export function isDoubleTap(previous, next) {
  if (!previous) return false;
  return next.t - previous.t <= COVER_DOUBLE_TAP_MS && pointerDistance(previous, next) <= COVER_DOUBLE_TAP_SLOP_PX;
}

/**
 * The cover's geometry from ONE getBoundingClientRect() taken while the layout is clean (start of an animation
 * frame, before any write): visual centre, un-transformed (layout) centre and un-scaled size. `state` is the transform
 * that was applied when the rect was read.
 * @param {{ left: number, top: number, width: number, height: number }} rect @param {CoverZoomState} state
 */
export function coverGeometry(rect, state) {
  const scale = state.scale > 0 ? state.scale : 1;
  const centreX = rect.left + rect.width / 2;
  const centreY = rect.top + rect.height / 2;
  return { centreX, centreY, layoutX: centreX - state.x, layoutY: centreY - state.y, width: rect.width / scale, height: rect.height / scale };
}

/**
 * Content offset from the cover centre (cover px) of the point currently at client (x, y).
 * @param {ReturnType<typeof coverGeometry>} geometry @param {CoverZoomState} state @param {number} clientX @param {number} clientY
 */
export function contentOffsetAt(geometry, state, clientX, clientY) {
  const scale = state.scale > 0 ? state.scale : 1;
  return { x: (clientX - geometry.centreX) / scale, y: (clientY - geometry.centreY) / scale };
}

/**
 * The transform that puts the content point `u` (cover px from its centre) at client `target` at `nextScale`, given the
 * cover's CURRENT layout centre. Absolute, so a layout shift caused by the previous step (the column grows or shrinks)
 * is corrected in the same write and the point under the fingers/cursor never drifts.
 * @param {ReturnType<typeof coverGeometry>} geometry @param {{ x: number, y: number }} u @param {{ x: number, y: number }} target @param {number} nextScale
 * @returns {CoverZoomState}
 */
export function anchoredZoom(geometry, u, target, nextScale) {
  const scale = clamp(nextScale, COVER_ZOOM_MIN, COVER_ZOOM_MAX);
  return clampCoverZoom({ scale, x: target.x - geometry.layoutX - u.x * scale, y: target.y - geometry.layoutY - u.y * scale }, geometry.width, geometry.height);
}

/**
 * FLIP: the transform that makes a cover whose layout has already moved/resized to `to` (its un-transformed box) paint
 * exactly where `from` (the box it was painted at) was. Both are client rects read in the same frame.
 * @param {{ left: number, top: number, width: number, height: number }} from @param {{ left: number, top: number, width: number, height: number }} to
 * @returns {CoverZoomState}
 */
export function flipTransform(from, to) {
  return {
    scale: to.width > 0 ? from.width / to.width : 1,
    x: from.left + from.width / 2 - (to.left + to.width / 2),
    y: from.top + from.height / 2 - (to.top + to.height / 2),
  };
}

/**
 * The restore glide's Web Animation: from the inverse (FLIP) transform, which paints the stage where it was before the
 * restore, to the identity of the final layout. Matching function lists, so the interpolation is translate + scale.
 * `fill: 'none'`: when it ends or is cancelled nothing of it stays on the stage.
 * @param {CoverZoomState} start the inverse transform (`flipTransform(from, to)`)
 */
export function restoreGlide(start) {
  return {
    keyframes: [
      { transform: `translate(${start.x.toFixed(2)}px, ${start.y.toFixed(2)}px) scale(${start.scale.toFixed(4)})` },
      { transform: 'translate(0px, 0px) scale(1)' },
    ],
    options: { duration: COVER_RESET_MS, easing: COVER_RESET_EASING, fill: /** @type {FillMode} */ ('none') },
  };
}

/**
 * The zoom state that paints exactly `visual` (a glide interrupted half-way) on a stage whose un-transformed box is
 * `layout` (both client rects). Scale 1 or below cannot carry an offset (the model recentres it).
 * @param {{ left: number, top: number, width: number, height: number }} visual @param {{ left: number, top: number, width: number, height: number }} layout
 * @returns {CoverZoomState}
 */
export function stateFromVisual(visual, layout) {
  return clampCoverZoom(flipTransform(visual, layout), layout.width, layout.height);
}

/**
 * An activity flag that turns on at the first `poke()` and off `idleMs` after the last one (or `end()` at once);
 * `hold()` keeps it on until `release()`. `onChange(active)` fires only on a real change.
 * @param {number} idleMs @param {(active: boolean) => void} onChange
 * @param {{ set: (fn: () => void, ms: number) => unknown, clear: (id: unknown) => void }} [timers]
 */
export function createIdleGate(idleMs, onChange, timers = { set: (fn, ms) => setTimeout(fn, ms), clear: (id) => clearTimeout(/** @type {any} */ (id)) }) {
  let active = false;
  let held = false;
  /** @type {unknown} */
  let timer = null;
  const stop = () => { if (timer !== null) { timers.clear(timer); timer = null; } };
  const set = (/** @type {boolean} */ value) => { if (active !== value) { active = value; onChange(value); } };
  const arm = () => {
    stop();
    if (held) return;
    timer = timers.set(() => { timer = null; if (!held) set(false); }, idleMs);
  };
  return {
    poke() { set(true); arm(); },
    hold() { held = true; set(true); stop(); },
    release() { held = false; if (active) arm(); },
    end() { held = false; stop(); set(false); },
    dispose() { held = false; stop(); },
    get active() { return active; },
  };
}

/**
 * use:coverZoom={{ enabled, onZoom, onZoomingChange, initial, syncScale }} on the cover element, which is the element that gets
 * transformed. Mouse: wheel over the cover zooms at the cursor, drag pans while zoomed. Touch/pen: pinch zooms, one
 * finger pans while zoomed. Double click / double tap restores the original size. `onZoom` receives the current state
 * ({ scale, x, y }; scale 1 = not zoomed). `onZoomingChange(true)` fires at the first wheel/pinch step and
 * `onZoomingChange(false)` COVER_ZOOM_SETTLE_MS after the last one: the lyrics freeze their text layout between the two
 * and re-wrap once at the end. The cover element is re-created on track changes (the artwork pane is removed while the
 * next cover loads), so the owner keeps the last state and passes it back as `initial` to restore it.
 *
 * Cost per animation frame (ADR 0036): all input (wheel factors, both fingers, pan) is accumulated and applied by ONE
 * requestAnimationFrame callback, which reads the cover rect once at the very start (layout is clean there, so the read
 * forces nothing), then only writes (transform, data attributes, onZoom). A scale change moves the layout (the lyrics
 * column is squeezed); the owner's `settleLayout()` (Svelte `tick`) resolves once that layout is committed, and the
 * transform is re-pinned in the SAME frame, in a microtask before paint, so no frame is ever painted uncompensated.
 *
 * Layers (ADR 0036): the stage is a compositor layer for its whole life (app.css: permanent `will-change: transform`
 * and `z-index`), so no gesture or animation ever creates or tears down a layer. A will-change layer keeps the raster
 * scale it was promoted at, so a zoomed-in cover is soft until Chrome re-rasters the layer. That is done by a re-raster
 * nudge (cover-reraster.js): once the scale above 1 has been stable for COVER_LAYER_IDLE_MS and no gesture or swipe
 * feedback is active, the stage is moved by 0.01 px and back with a 180 ms translate transition (inline `translate` and
 * `transition` only, restored afterwards). A gesture that starts cancels a pending nudge and wins over one in flight:
 * the nudge only clears what it set itself. The double-tap restore (reset) is a FLIP on the stage itself: the final
 * layout is committed at once and a Web Animation (inverse transform to identity, `fill: 'none'`) glides the stage; a
 * gesture during it cancels the animation and carries on from the visual it had reached. No second element is involved.
 * @param {HTMLElement} node
 * @param {{ enabled?: boolean, onZoom?: (state: CoverZoomState, size: { width: number, height: number }) => void, onZoomingChange?: (zooming: boolean) => void, settleLayout?: () => unknown, initial?: CoverZoomState | ((node: HTMLElement) => CoverZoomState), syncScale?: number }} params
 */
export function coverZoom(node, params) {
  let options = params;
  /** @type {CoverZoomState} */
  let state = { ...COVER_ZOOM_IDENTITY };
  let size = { width: 0, height: 0 };
  /** @type {Map<number, { x: number, y: number }>} */
  /** @type {Map<number, import("./cover-pointers.js").PointerEntry>} */
  const pointers = new Map();
  /** @type {{ distance: number, scale: number, startX: number, startY: number, u: { x: number, y: number } | null, dirty: boolean } | null} */
  let pinch = null;
  /** @type {{ x: number, y: number, ox: number, oy: number, cx: number, cy: number, moved: boolean, dirty: boolean } | null} */
  let pan = null;
  /** @type {{ factor: number, x: number, y: number } | null} */
  let wheelPending = null;
  /** The content point under the cursor for the current wheel run; reused until the cursor really moves. @type {{ u: { x: number, y: number }, x: number, y: number } | null} */
  let wheelAnchor = null;
  let repinToken = 0;
  let destroyed = false;
  /** The running reset glide (Web Animation) and the observer that re-aims it if the layout moves again before paint. @type {Animation | null} */
  let glide = null;
  /** The cover image source the glide started with. */
  let glideSource = '';
  /** @type {ResizeObserver | null} */
  let glideObserver = null;
  /** @type {{ x: number, y: number, t: number } | null} */
  let lastTap = null;
  let suppressClick = false;
  let dispatchingCancel = false;
  let resetting = false;
  let frameId = 0;
  /** The stage's inline style without the swipe feedback properties, as this action last left it: a style change that
   *  keeps it is the action's own transform write or swipe feedback (or the nudge), not a foreign one. */
  let ownStyle = nonFeedbackStyle(node.style);
  const debugSwipe = (() => { try { return globalThis.localStorage?.getItem('moe.debug.coverSwipe') === '1'; } catch { return false; } })();
  let wasActive = params.enabled !== false;
  const reduced = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
  const active = () => options.enabled !== false;
  const hasRaf = () => typeof requestAnimationFrame === 'function';
  const raf = (/** @type {() => void} */ fn) => (hasRaf() ? requestAnimationFrame(fn) : (fn(), 0));
  const clock = () => (typeof performance !== 'undefined' && typeof performance.now === 'function' ? performance.now() : Date.now());

  // Re-raster nudge state (cover-reraster.js): the stability clock, the single timer and the nudge in flight.
  let reraster = createRerasterState(1, clock());
  /** @type {ReturnType<typeof setTimeout> | null} */
  let rerasterTimer = null;
  /** The nudge in flight: the transition to restore, what it last wrote as `transition`, its restore timer. @type {{ prev: string, transition: string, restoreTimer: ReturnType<typeof setTimeout> | null, guardTimer: ReturnType<typeof setTimeout> | null } | null} */
  let nudge = null;
  /** Re-checks of the nudge timer made while swipe feedback was on the stage. */
  let feedbackWaits = 0;

  const zoomGate = createIdleGate(COVER_ZOOM_SETTLE_MS, (on) => {
    if (!on) wheelAnchor = null;
    options.onZoomingChange?.(on);
  });

  /** @param {CoverZoomState} next @param {boolean} animate @param {boolean} [notify] */
  function apply(next, animate, notify = true) {
    cancelNudge(true); // a new transform write always wins over a nudge (it sets `transition` itself)
    state = next;
    node.style.transition = animate && !reduced() ? `transform ${COVER_RESET_MS}ms ease` : 'none';
    node.style.transform = coverZoomTransform(state);
    ownStyle = nonFeedbackStyle(node.style);
    // A data attribute, not a class: Svelte rewrites the class list of this element when `has-artwork` toggles
    // (track change), which would drop a class added from here.
    if (state.scale !== COVER_ZOOM_REST) node.dataset.coverZoomed = '';
    else delete node.dataset.coverZoomed;
    // Only an enlarged cover owns a mouse drag (it pans); shrunk or at rest, the drag is the page's swipe.
    if (isCoverPannable(state.scale)) node.dataset.coverPannable = '';
    else delete node.dataset.coverPannable;
    reraster = observeRerasterScale(reraster, state.scale, clock());
    scheduleReraster();
    if (notify) options.onZoom?.(state, size);
  }

  /** (Re)arm the nudge timer; nothing is armed when no nudge is due (scale at rest, or already nudged at this scale).
   *  @param {number} [delay] @param {boolean} [retry] a re-check by the timer itself (does not reset the feedback wait budget) */
  function scheduleReraster(delay = COVER_LAYER_IDLE_MS, retry = false) {
    if (rerasterTimer !== null) { clearTimeout(rerasterTimer); rerasterTimer = null; }
    if (!retry) feedbackWaits = 0;
    if (destroyed) return;
    if (planReraster(reraster, { now: clock(), idleMs: COVER_LAYER_IDLE_MS - COVER_TIMER_SLACK_MS, busy: false, inFlight: false }) === 'skip') return;
    rerasterTimer = setTimeout(rerasterTick, delay);
  }

  /** A finger / wheel / frame is in progress: the nudge resumes from its end (pointer up, the applied frame). */
  function gestureActive() {
    return pointers.size > 0 || wheelPending !== null || frameId !== 0 || resetting;
  }

  /** A swipe's feedback (or its bounce-back) is on the stage. */
  function feedbackBusy() {
    const style = node.style;
    if (style.translate || style.scale || style.opacity || /translate|scale/.test(style.transition || '')) return true;
    return feedbackTransitionRunning();
  }

  function rerasterTick() {
    rerasterTimer = null;
    if (destroyed || !active()) return;
    const gesture = gestureActive();
    const feedback = !gesture && nudge === null && feedbackBusy();
    const plan = planReraster(reraster, { now: clock(), idleMs: COVER_LAYER_IDLE_MS - COVER_TIMER_SLACK_MS, busy: gesture || feedback, inFlight: nudge !== null });
    if (plan === 'run') { runNudge(); return; }
    if (plan !== 'wait') return;
    // Under a finger the timer stops (up() re-arms it); under swipe feedback it re-checks a bounded number of times.
    if (gesture && nudge === null) return;
    if (feedback && (feedbackWaits += 1) > COVER_FEEDBACK_WAIT_LIMIT) return;
    scheduleReraster(COVER_LAYER_IDLE_MS, true);
  }

  /** The nudge: translate 0.01px with no transition, style forced, then (next frame) translate cleared with a 180 ms
   *  transition; the previous transition string comes back COVER_RERASTER_RESTORE_MS later. Reduced motion changes
   *  nothing: the move is a hundredth of a pixel. */
  function runNudge() {
    reraster = markRerastered(reraster);
    const style = node.style;
    const mine = { prev: style.transition || '', transition: 'none', restoreTimer: /** @type {ReturnType<typeof setTimeout> | null} */ (null), guardTimer: /** @type {ReturnType<typeof setTimeout> | null} */ (null) };
    nudge = mine;
    style.transition = 'none';
    mine.transition = style.transition || 'none'; // compared against the read-back: a browser may serialise what was written
    style.translate = COVER_RERASTER_TRANSLATE;
    if (typeof getComputedStyle === 'function') void getComputedStyle(node).translate; // force style: the transition starts from here
    if (debugSwipe) console.log('[cover-reraster]', JSON.stringify({ scale: state.scale, prev: mine.prev }));
    raf(() => {
      if (nudge !== mine) return;
      // A gesture wrote the stage's properties in between (swipe feedback): it wins, nothing of ours is cleared.
      if (!isRerasterTranslate(style.translate) || (style.transition || '') !== mine.transition) { nudge = null; return; }
      style.transition = COVER_RERASTER_TRANSITION;
      mine.transition = style.transition || COVER_RERASTER_TRANSITION;
      style.translate = '';
      mine.restoreTimer = setTimeout(() => {
        if (nudge !== mine) return;
        nudge = null;
        if (mine.guardTimer !== null) clearTimeout(mine.guardTimer);
        if (isRerasterTranslate(style.translate)) style.translate = ''; // safety: never leave the nudge's own value behind
        if ((style.transition || '') === mine.transition) style.transition = mine.prev;
      }, COVER_RERASTER_RESTORE_MS);
    });
    // Safety net (a frame that never comes, e.g. a hidden page): whatever the nudge still owns is taken back.
    mine.guardTimer = setTimeout(() => { if (nudge === mine) cancelNudge(true); }, COVER_RERASTER_RESTORE_MS + 200);
  }

  /** Stop a nudge in flight. `clean`: also take back what the nudge wrote and still owns; whatever a gesture replaced
   *  is left alone either way. @param {boolean} clean */
  function cancelNudge(clean) {
    if (!nudge) return;
    const mine = nudge;
    nudge = null;
    if (mine.restoreTimer !== null) clearTimeout(mine.restoreTimer);
    if (mine.guardTimer !== null) clearTimeout(mine.guardTimer);
    if (!clean) return;
    const style = node.style;
    if (isRerasterTranslate(style.translate)) style.translate = '';
    if ((style.transition || '') === mine.transition) style.transition = mine.prev;
  }

  /** A feedback (translate / scale / opacity) CSS transition of the stage is still running. */
  function feedbackTransitionRunning() {
    if (typeof node.getAnimations !== 'function') return false;
    return node.getAnimations().some((/** @type {any} */ animation) => typeof animation.transitionProperty === 'string'
      && /^(translate|scale|opacity)$/.test(animation.transitionProperty)
      && animation.playState !== 'finished' && animation.playState !== 'idle');
  }

  /** Whether the stage may carry swipe feedback now: an inline translate/scale, or its settle transition string. */
  function feedbackLive() {
    const style = node.style;
    return Boolean(style.translate || style.scale || /translate|scale/.test(style.transition || ''));
  }

  /** The stage's painted client rect without the swipe feedback (`translate` / `scale`, which getBoundingClientRect
   *  includes): every geometry here is the zoom's own. The computed (current, also mid-transition) feedback is read only
   *  while there can be some, right after the rect read (style is clean). */
  function stageRect() {
    const rect = node.getBoundingClientRect();
    if (!feedbackLive() || typeof getComputedStyle !== 'function') return rect;
    const computed = getComputedStyle(node);
    return withoutSwipeFeedback(rect, parseSwipeFeedback(computed.translate, computed.scale), state);
  }

  /** The source of the cover image on the stage now ('' without one). */
  function currentSource() {
    const image = /** @type {HTMLImageElement | null} */ (node.querySelector?.('img') ?? null);
    return image ? image.currentSrc || image.src || '' : '';
  }

  /** Every style write of the stage (before paint): swipe feedback written over a nudge in flight takes the stage's
   *  properties, so the nudge is dropped without clearing anything. */
  function styleChanged() {
    if (!nudge) {
      if (rerasterTimer === null) scheduleReraster(); // swipe feedback just ended (transition cleared): look again
      return;
    }
    const style = node.style;
    if (style.scale || style.opacity || ((style.translate || '') !== '' && !isRerasterTranslate(style.translate))) cancelNudge(false);
  }

  /** Something outside the gesture changed the cover (another style write, class, a new image, a window resize): the
   *  restore glide ends if the image changed, otherwise the new content needs a nudge once the scale is stable. */
  function foreignChange() {
    if (destroyed) return;
    if (resetting) {
      // The glide re-aims itself on a layout change (ResizeObserver); a different cover image ends it.
      if (glide && glideSource !== currentSource()) { glide.cancel(); endGlide(); }
      return;
    }
    reraster = invalidateReraster(reraster, clock());
    scheduleReraster();
  }
  const coverObservers = observeCoverChanges(node, foreignChange, styleChanged, (record) => record.target === node && record.attributeName === 'style' && nonFeedbackStyle(node.style) === ownStyle);

  function schedule() {
    if (frameId) return;
    frameId = raf(frame);
  }

  function frame() {
    frameId = 0;
    if (resetting || !active()) { wheelPending = null; return; }
    // The only layout read of the frame: first, so nothing written yet can have dirtied the layout.
    const geo = coverGeometry(stageRect(), state);
    size = { width: geo.width, height: geo.height };
    /** @type {CoverZoomState | null} */
    let next = null;
    /** @type {{ u: { x: number, y: number }, x: number, y: number } | null} */
    let pin = null;
    if (wheelPending) {
      const { factor, x, y } = wheelPending;
      wheelPending = null;
      if (!wheelAnchor || Math.hypot(x - wheelAnchor.x, y - wheelAnchor.y) > COVER_WHEEL_ANCHOR_SLOP_PX) {
        wheelAnchor = { u: contentOffsetAt(geo, state, x, y), x, y };
      }
      pin = { u: wheelAnchor.u, x, y };
      next = anchoredZoom(geo, pin.u, pin, state.scale * factor);
    } else if (pinch && pointers.size >= 2) {
      if (!pinch.dirty) return;
      const [a, b] = [...pointers.values()];
      const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
      if (!pinch.u) pinch.u = contentOffsetAt(geo, state, pinch.startX, pinch.startY);
      pinch.dirty = false;
      pin = { u: pinch.u, x: mid.x, y: mid.y };
      next = anchoredZoom(geo, pin.u, pin, pinch.scale * (pointerDistance(a, b) / pinch.distance));
    } else if (pan && pointers.size === 1 && pan.dirty) {
      pan.dirty = false;
      next = clampCoverZoom({ scale: state.scale, x: pan.ox + (pan.cx - pan.x), y: pan.oy + (pan.cy - pan.y) }, geo.width, geo.height);
    }
    const scaleChanged = next !== null && next.scale !== state.scale;
    if (next && (scaleChanged || Math.abs(next.x - state.x) >= 0.01 || Math.abs(next.y - state.y) >= 0.01)) apply(next, false);
    // The scale change moves the layout (squeeze) after this write: re-pin the anchor before this frame is painted.
    if (scaleChanged && pin) repin(pin);
    scheduleReraster(); // a frame that applied nothing (clamped zoom) must not leave a due nudge unarmed
  }

  /** Once the owner has committed the layout this scale change caused (a microtask, still before paint), measure the
   *  cover again and put the anchor point back under the cursor/fingers: the squeeze never shows for a frame.
   *  @param {{ u: { x: number, y: number }, x: number, y: number }} pin */
  function repin(pin) {
    const token = ++repinToken;
    Promise.resolve(options.settleLayout?.()).then(() => {
      if (destroyed || token !== repinToken || resetting || !active()) return;
      const geo = coverGeometry(stageRect(), state);
      size = { width: geo.width, height: geo.height };
      const next = anchoredZoom(geo, pin.u, pin, state.scale);
      if (Math.abs(next.x - state.x) >= 0.01 || Math.abs(next.y - state.y) >= 0.01) apply(next, false);
    }, () => {});
  }

  /** Back to the original size (FLIP): the final layout is committed at once (the lyrics column and the squeeze snap at
   *  frame 0 together with the glide), and in the same task, before paint, the cover is put back where it was painted
   *  (inverse transform) and glides to identity. Never an uncompensated frame. */
  function reset() {
    if (state.scale === COVER_ZOOM_REST || resetting) return;
    wheelPending = null; wheelAnchor = null; repinToken += 1;
    if (reduced() || !hasRaf() || typeof node.animate !== 'function') {
      apply({ ...COVER_ZOOM_IDENTITY }, false);
      return;
    }
    resetting = true;
    zoomGate.hold();
    const from = node.getBoundingClientRect(); // where the cover is painted now (zoomed layout, current transform)
    node.dataset.coverResetting = '';
    apply({ ...COVER_ZOOM_IDENTITY }, false); // final layout: transform cleared, attributes dropped, owner told
    Promise.resolve(options.settleLayout?.()).then(() => beginGlide(from), () => beginGlide(from));
  }

  /** @param {{ left: number, top: number, width: number, height: number }} from @param {number} [elapsedMs] */
  function startGlide(from, elapsedMs = 0) {
    glide?.cancel();
    glide = null;
    const to = node.getBoundingClientRect(); // animation cancelled: the un-transformed layout box
    const plan = restoreGlide(flipTransform(from, to));
    const animation = node.animate(plan.keyframes, plan.options);
    animation.currentTime = elapsedMs;
    animation.onfinish = endGlide;
    glide = animation;
  }

  /** @param {{ left: number, top: number, width: number, height: number }} from */
  function beginGlide(from) {
    if (destroyed || !resetting) return;
    glideSource = currentSource();
    startGlide(from);
    if (typeof ResizeObserver === 'function') {
      const started = performance.now();
      let first = true;
      // The owner can still resize the cover/pane in this frame's rendering step (e.g. its own ResizeObserver): re-aim
      // the glide from the same starting box before anything is painted.
      glideObserver = new ResizeObserver(() => {
        if (first) { first = false; return; }
        if (glide) startGlide(from, performance.now() - started);
      });
      glideObserver.observe(node);
      if (node.parentElement) glideObserver.observe(node.parentElement);
    }
  }

  /** The glide finished or was cancelled: the stage's transform is the real final state again (the animation has
   *  `fill: none`, so nothing of it stays; the inline value is the state's own). */
  function endGlide() {
    glideObserver?.disconnect();
    glideObserver = null;
    glide = null;
    if (node.style.transform !== coverZoomTransform(state)) node.style.transform = coverZoomTransform(state);
    delete node.dataset.coverResetting;
    resetting = false;
    zoomGate.release();
  }

  /** A new gesture during the glide wins: the animation is cancelled and the stage carries on from the visual it had
   *  reached (a state that paints exactly that, re-pinned in the same frame once the owner has laid out the new scale). */
  function interruptGlide() {
    if (!resetting) return;
    const visual = stageRect(); // includes the running animation
    glide?.cancel();
    endGlide();
    const layout = stageRect(); // animation gone, identity state: the layout box
    const next = stateFromVisual(visual, layout);
    if (Math.abs(next.scale - 1) < 0.0005 && Math.abs(next.x) < 0.01 && Math.abs(next.y) < 0.01) return;
    apply(next, false);
    const token = ++repinToken;
    Promise.resolve(options.settleLayout?.()).then(() => {
      if (destroyed || token !== repinToken || resetting || !active()) return;
      const settled = coverGeometry(stageRect(), state);
      size = { width: settled.width, height: settled.height };
      const pinned = clampCoverZoom({ scale: state.scale, x: visual.left + visual.width / 2 - settled.layoutX, y: visual.top + visual.height / 2 - settled.layoutY }, settled.width, settled.height);
      if (Math.abs(pinned.x - state.x) >= 0.01 || Math.abs(pinned.y - state.y) >= 0.01) apply(pinned, false);
    }, () => {});
  }

  /** @param {WheelEvent} event */
  function wheel(event) {
    if (!active()) return;
    event.preventDefault();
    interruptGlide();
    cancelNudge(true);
    const factor = getWheelZoomFactor(event.deltaY, event.deltaMode);
    wheelPending = { factor: (wheelPending?.factor ?? 1) * factor, x: event.clientX, y: event.clientY };
    zoomGate.poke();
    schedule();
  }

  /** @param {PointerEvent} event */
  function down(event) {
    if (!active()) return;
    interruptGlide();
    cancelNudge(true);
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY, type: event.pointerType });
    if (pointers.size === 2) {
      const [a, b] = [...pointers.values()];
      pinch = { distance: Math.max(1, pointerDistance(a, b)), scale: state.scale, startX: (a.x + b.x) / 2, startY: (a.y + b.y) / 2, u: null, dirty: false };
      pan = null;
      lastTap = null;
      zoomGate.hold();
      // Let the swipe-to-skip gesture on the same element drop the first finger (it ignores pointercancel dismissals).
      const [firstId] = [...pointers.keys()];
      // Our own up() listens for pointercancel too, so mark this one as synthetic and ignore it there.
      dispatchingCancel = true;
      try { node.dispatchEvent(new PointerEvent('pointercancel', { pointerId: firstId, bubbles: true })); } catch { /* unsupported */ }
      dispatchingCancel = false;
      for (const id of pointers.keys()) {
        try { node.setPointerCapture(id); } catch { /* gone */ }
      }
    } else if (pointers.size === 1 && event.pointerType === 'mouse' && isCoverPannable(state.scale)) {
      pan = { x: event.clientX, y: event.clientY, ox: state.x, oy: state.y, cx: event.clientX, cy: event.clientY, moved: false, dirty: false };
      try { node.setPointerCapture(event.pointerId); } catch { /* gone */ }
    }
  }

  /** @param {PointerEvent} event */
  function move(event) {
    const entry = pointers.get(event.pointerId);
    if (!entry) return;
    // A mouse move with no button held: its release was missed (outside the node / window), so release it now.
    if (releasesOnMove(entry, event.buttons)) { up(event); return; }
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY, type: entry.type });
    if (pinch && pointers.size >= 2) {
      pinch.dirty = true;
      suppressClick = true;
      if (event.cancelable) event.preventDefault();
      schedule();
    } else if (pan && pointers.size === 1) {
      const dx = event.clientX - pan.x;
      const dy = event.clientY - pan.y;
      if (!pan.moved && Math.hypot(dx, dy) < COVER_PAN_SLOP_PX) return;
      pan.moved = true;
      pan.cx = event.clientX;
      pan.cy = event.clientY;
      pan.dirty = true;
      suppressClick = true;
      if (event.cancelable) event.preventDefault();
      schedule();
    }
  }

  /** @param {PointerEvent} event */
  function up(event) {
    if (dispatchingCancel || !pointers.has(event.pointerId)) return;
    const wasPinching = pinch !== null;
    const moved = pan?.moved === true;
    pointers.delete(event.pointerId);
    if (pointers.size < 2 && pinch) { pinch = null; zoomGate.release(); }
    if (pointers.size === 0) { pan = null; scheduleReraster(); }

    if (wasPinching || moved) {
      setTimeout(() => { suppressClick = false; }, 0);
      lastTap = null;
      return;
    }
    if (event.type !== 'pointerup' || event.pointerType === 'mouse') return;
    // Touch/pen double tap (dblclick is not reliably synthesized by the WebView with touch-action rules).
    const tap = { x: event.clientX, y: event.clientY, t: event.timeStamp };
    if (isDoubleTap(lastTap, tap)) { lastTap = null; reset(); } else lastTap = tap;
  }

  /** Capture-phase window pointerup / pointercancel: releases a mouse entry the node never saw the release of. Idempotent
   *  with the node's own listener (`up` ignores an id that is no longer tracked). @param {PointerEvent} event */
  function windowUp(event) {
    if (releasesOnWindowEvent(pointers.get(event.pointerId))) up(event);
  }

  /** The browser's own image drag would start on the first mouse move and cancel the pointer (no panning). @param {Event} event */
  function dragstart(event) {
    event.preventDefault();
  }

  function dblclick() {
    if (active()) reset();
  }

  /** @param {MouseEvent} event */
  function click(event) {
    if (!suppressClick) return;
    suppressClick = false;
    event.preventDefault();
    event.stopPropagation();
  }

  const initial = typeof options.initial === 'function' ? options.initial(node) : options.initial;
  // Restoring the owner's saved state must not report it back (it would overwrite the saved pan while the cover
  // has no size yet).
  if (initial && initial.scale !== COVER_ZOOM_REST) {
    size = { width: node.offsetWidth, height: node.offsetHeight };
    apply({ ...initial }, false, false);
  }

  node.addEventListener('wheel', wheel, { passive: false });
  node.addEventListener('pointerdown', down);
  node.addEventListener('pointermove', move);
  node.addEventListener('pointerup', up);
  node.addEventListener('pointercancel', up);
  node.addEventListener('dblclick', dblclick);
  node.addEventListener('dragstart', dragstart);
  node.addEventListener('click', click, true);
  const hasWindow = typeof globalThis.addEventListener === 'function';
  if (hasWindow) {
    globalThis.addEventListener('pointerup', windowUp, true);
    globalThis.addEventListener('pointercancel', windowUp, true);
  }
  return {
    /** @param {typeof params} next */
    update(next) {
      options = next;
      // The owner can drop the zoom from outside (an enlarged cover is reset when the track changes): when it says
      // the scale is back at 1 while this element is still zoomed, follow it.
      if (next.syncScale === COVER_ZOOM_REST && state.scale !== COVER_ZOOM_REST && !resetting) {
        pointers.clear(); pinch = null; pan = null; lastTap = null; wheelPending = null; wheelAnchor = null; repinToken += 1;
        apply({ ...COVER_ZOOM_IDENTITY }, false, false);
      }
      // The page (re)opened: its layers are created now, at the current scale.
      if (active() && !wasActive) {
        reraster = invalidateReraster(reraster, clock());
        scheduleReraster();
      }
      wasActive = active();
      // A disabled cover (the Now Playing page is closed) keeps its zoom: only a double click restores it.
      if (!active()) { pointers.clear(); pinch = null; pan = null; wheelPending = null; zoomGate.release(); cancelNudge(true); }
    },
    destroy() {
      if (frameId && typeof cancelAnimationFrame === 'function') cancelAnimationFrame(frameId);
      destroyed = true;
      cancelNudge(true);
      if (rerasterTimer !== null) clearTimeout(rerasterTimer);
      glide?.cancel();
      glideObserver?.disconnect();
      coverObservers.disconnect();
      delete node.dataset.coverResetting;
      zoomGate.end();
      node.removeEventListener('wheel', wheel);
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('pointerup', up);
      node.removeEventListener('pointercancel', up);
      node.removeEventListener('dragstart', dragstart);
      node.removeEventListener('dblclick', dblclick);
      node.removeEventListener('click', click, true);
      if (hasWindow) {
        globalThis.removeEventListener('pointerup', windowUp, true);
        globalThis.removeEventListener('pointercancel', windowUp, true);
      }
      node.style.transform = '';
      delete node.dataset.coverZoomed;
      delete node.dataset.coverPannable;
    }
  };
}

/** @typedef {{ left: number, top: number, width: number, height: number }} Rect */

/** The stage's swipe feedback and nudge properties (swipe-dismiss.js damped mode, cover-reraster.js): never a foreign change. */
const FEEDBACK_PROPERTY = /^(translate|scale|opacity|transition(-.*)?)$/;

/**
 * A signature of the inline style without the swipe feedback properties (and their longhands).
 * @param {CSSStyleDeclaration | Record<string, unknown>} style
 */
function nonFeedbackStyle(style) {
  const declaration = /** @type {CSSStyleDeclaration} */ (style);
  if (typeof declaration?.getPropertyValue === 'function' && typeof declaration.length === 'number') {
    const parts = [];
    for (let i = 0; i < declaration.length; i += 1) {
      const name = declaration[i];
      if (!FEEDBACK_PROPERTY.test(name)) parts.push(`${name}:${declaration.getPropertyValue(name)}`);
    }
    return parts.join(';');
  }
  return JSON.stringify(Object.entries(style ?? {}).filter(([name, value]) => !FEEDBACK_PROPERTY.test(name) && typeof value !== 'function' && value !== ''));
}

/**
 * Watch for changes the gesture did not make (the owner's style writes, class changes, a new image source, a window
 * resize: `onChange`); MutationObserver callbacks run before the next paint. `onStyle` runs first for every batch of
 * mutations (a swipe's feedback written over a nudge in flight is noticed there, in the same frame as the write).
 * Mutation records for which `isOwn` is true (the action's own transform writes, swipe feedback and the nudge), and
 * records of an attribute the batch left at its old value (`unchangedInBatch`), do not count as a change.
 * @param {HTMLElement} node @param {() => void} onChange @param {() => void} onStyle
 * @param {(record: MutationRecord) => boolean} isOwn
 */
function observeCoverChanges(node, onChange, onStyle, isOwn) {
  /** @type {Array<() => void>} */
  const stops = [];
  if (typeof MutationObserver === 'function') {
    const mutations = new MutationObserver((records) => {
      onStyle();
      if (records.some((record) => record.type === 'childList' || (!isOwn(record) && !unchangedInBatch(record, records)))) onChange();
    });
    mutations.observe(node, { attributes: true, attributeFilter: ['style', 'class', 'src'], subtree: true, attributeOldValue: true, childList: true });
    stops.push(() => mutations.disconnect());
  }
  if (typeof addEventListener === 'function') {
    addEventListener('resize', onChange);
    stops.push(() => removeEventListener('resize', onChange));
  }
  if (typeof node.addEventListener === 'function') {
    node.addEventListener('load', onChange, true); // the <img> got its (new) pixels
    stops.push(() => node.removeEventListener('load', onChange, true));
  }
  return { disconnect() { for (const stop of stops) stop(); } };
}

/**
 * Whether the batch made no net change to `record`'s attribute: the old value of the batch's first record for that
 * element and attribute equals the attribute's value now. The browser emits a record for every write, even one that
 * changes nothing (swipe-dismiss's `classList.remove('is-swiping')` on a stage that never had the class, at every
 * finger lift and at the pinch's synthetic pointercancel); such records are not a change of the cover. A record
 * without an old value (attributeOldValue unsupported) counts as a change.
 * @param {MutationRecord} record @param {MutationRecord[]} records
 */
function unchangedInBatch(record, records) {
  const name = record.attributeName;
  const target = /** @type {Element} */ (record.target);
  if (!name || typeof target?.getAttribute !== 'function') return false;
  const first = records.find((other) => other.target === target && other.attributeName === name) ?? record;
  return first.oldValue !== undefined && first.oldValue === target.getAttribute(name);
}
