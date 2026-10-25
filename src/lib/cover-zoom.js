// Now-playing cover zoom: wheel (mouse over the cover) or two-finger pinch scales the cover itself (CSS transform on
// the cover element); double click / double tap restores. The layout reserves the zoomed cover size so the lyrics
// area is squeezed by the cover instead of scaling with it (see --np-cover-reserve in app.css). Pure math is exported
// for tests; `coverZoom` is the Svelte action that wires the events.

import { coverDrawPlan, coverOverlayWanted, intersectRects, overlayDrawRegion, overlayFollowTransform, overlayOrigin, overlayTransformCss, parseSwipeFeedback, snapRectToDevicePixels, withoutSwipeFeedback } from './cover-overlay.js';

export const COVER_ZOOM_MIN = 0.5;
export const COVER_ZOOM_REST = 1;
export const COVER_ZOOM_MAX = 10;
export const COVER_ZOOM_WHEEL_STEP = 0.0015;
export const COVER_DOUBLE_TAP_MS = 300;
export const COVER_DOUBLE_TAP_SLOP_PX = 24;
export const COVER_PAN_SLOP_PX = 6;
/** No zoom input for this long = the zoom gesture is over: the lyrics re-wrap and re-centre once. */
export const COVER_ZOOM_SETTLE_MS = 50;
/** No input for this long = the cover has settled: the overlay canvas is redrawn sharp at the new scale (ADR 0036). */
export const COVER_LAYER_IDLE_MS = 50;
/** The double-click / double-tap reset glide. */
export const COVER_RESET_MS = 200;
const COVER_WHEEL_ANCHOR_SLOP_PX = 2;
/** The cover image bleeds this far past the stage edges (app.css `.cover-stage.has-artwork .cover-stage-image`). */
const COVER_IMAGE_BLEED_PX = 1;

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
 * The stage's painted client rect for `state`, from a geometry read in the same frame (its layout box scaled around
 * the centre and panned): what getBoundingClientRect would return after the write, without reading it.
 * @param {ReturnType<typeof coverGeometry>} geometry @param {CoverZoomState} state
 */
export function coverVisualRect(geometry, state) {
  const width = geometry.width * state.scale;
  const height = geometry.height * state.scale;
  return { left: geometry.layoutX + state.x - width / 2, top: geometry.layoutY + state.y - height / 2, width, height };
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
 * scale it was promoted at, so the action adds one persistent overlay <canvas> next to the stage (in the artwork pane):
 * after the cover settles at a scale other than the promote-time one, the visible part plus a bounded margin is drawn
 * into it at device resolution from the source-resolution image. Input never hides it: during a pan, zoom out or zoom
 * in it follows the stage by a compositor transform written together with the stage's own (frame / repin / resize),
 * and is redrawn sharp at the new scale once input stops for COVER_LAYER_IDLE_MS. The swipe gestures' damped feedback on
 * the stage (inline translate / scale / opacity / transition, swipe-dismiss.js) is copied onto the overlay in the same
 * frame (change observer), pivoting on the stage's centre, instead of hiding it; every geometry here is read without that
 * feedback (`stageRect`). The reset glide animates a copy drawn into the same canvas at the
 * screen-resolution thumbnail size instead of animating the stage (a running transform animation makes Chromium pick
 * the raster scale from the animation's largest scale, which on a phone is a ~3000 px tiled layer that is not ready in
 * time; a canvas is a texture, never tiled). The overlay is only shown/hidden (opacity) and the stage's image only
 * hidden (visibility) under the reset copy: paint-only changes.
 * @param {HTMLElement} node
 * @param {{ enabled?: boolean, onZoom?: (state: CoverZoomState, size: { width: number, height: number }) => void, onZoomingChange?: (zooming: boolean) => void, settleLayout?: () => unknown, initial?: CoverZoomState | ((node: HTMLElement) => CoverZoomState), syncScale?: number }} params
 */
export function coverZoom(node, params) {
  let options = params;
  /** @type {CoverZoomState} */
  let state = { ...COVER_ZOOM_IDENTITY };
  let size = { width: 0, height: 0 };
  /** @type {Map<number, { x: number, y: number }>} */
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
  /** @type {ResizeObserver | null} */
  let glideObserver = null;
  /** @type {{ x: number, y: number, t: number } | null} */
  let lastTap = null;
  let suppressClick = false;
  let dispatchingCancel = false;
  let resetting = false;
  let frameId = 0;
  /** The stage's inline style without the swipe feedback properties, as this action last left it: a style change that
   *  keeps it is the action's own transform write or swipe feedback (mirrored onto the overlay), not a foreign one. */
  let ownStyle = nonFeedbackStyle(node.style);
  /** The scale the stage's layer was promoted (first rasterised) at: when the action starts and each time the page opens. */
  let promotedScale = 1;
  let wasActive = params.enabled !== false;
  const reduced = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
  const active = () => options.enabled !== false;
  const hasRaf = () => typeof requestAnimationFrame === 'function';
  const raf = (/** @type {() => void} */ fn) => (hasRaf() ? requestAnimationFrame(fn) : (fn(), 0));

  const overlay = createOverlayCanvas(node);

  // Input activity: on at the first input of a gesture, off COVER_LAYER_IDLE_MS after the last one, when the settled
  // cover is redrawn sharp into the overlay. Input never hides the overlay: while it is on, an overlay that is shown
  // follows the stage by a compositor transform (`followOverlay`); without one the stage itself shows.
  const layerGate = createIdleGate(COVER_LAYER_IDLE_MS, (on) => {
    if (!on) drawSettled();
  });
  const zoomGate = createIdleGate(COVER_ZOOM_SETTLE_MS, (on) => {
    if (!on) wheelAnchor = null;
    options.onZoomingChange?.(on);
  });

  /** @param {CoverZoomState} next @param {boolean} animate @param {boolean} [notify] */
  function apply(next, animate, notify = true) {
    state = next;
    node.style.transition = animate && !reduced() ? `transform ${COVER_RESET_MS}ms ease` : 'none';
    node.style.transform = coverZoomTransform(state);
    ownStyle = nonFeedbackStyle(node.style);
    // A data attribute, not a class: Svelte rewrites the class list of this element when `has-artwork` toggles
    // (track change), which would drop a class added from here.
    if (state.scale !== COVER_ZOOM_REST) node.dataset.coverZoomed = '';
    else delete node.dataset.coverZoomed;
    if (notify) options.onZoom?.(state, size);
  }

  /** The decoded cover image, or null (no artwork: the fallback art is not drawn into the overlay). */
  function coverImage() {
    const image = /** @type {HTMLImageElement | null} */ (node.querySelector?.('img') ?? null);
    return image && image.complete && image.naturalWidth > 0 && image.naturalHeight > 0 ? image : null;
  }

  function hideOverlay() {
    if (!overlay) return;
    if (overlay.shown) overlay.hide();
    delete node.dataset.coverOverlay;
  }

  /**
   * Draw the cover into the overlay and show it (same task: it is never shown before it has been drawn). `visual` is the
   * stage's painted rect at `scale`, `visible` the client rect to cover. `hideStage` hides the stage's own image in the
   * same commit (the reset copy, which does not cover where the stage now is); the sharp view leaves it painted under
   * the overlay so its tiles stay ready for the next gesture.
   * @param {Rect} visual @param {number} scale @param {Rect} visible @param {boolean} hideStage
   */
  function drawOverlay(visual, scale, visible, hideStage) {
    const image = coverImage();
    if (!overlay || !image) return false;
    const dpr = typeof devicePixelRatio === 'number' && devicePixelRatio > 0 ? devicePixelRatio : 1;
    const snapped = snapRectToDevicePixels(visible, dpr);
    if (!snapped) return false;
    const plan = coverDrawPlan({ visual, scale, naturalWidth: image.naturalWidth, naturalHeight: image.naturalHeight, bleed: COVER_IMAGE_BLEED_PX, visible: snapped, dpr });
    if (!plan) return false;
    const radius = parseFloat(getComputedStyle(node).borderTopLeftRadius) || 0;
    if (!overlay.draw(image, plan, snapped, radius * scale * dpr, visual, layoutCentre(visual))) return false;
    if (hideStage) node.dataset.coverOverlay = '';
    else delete node.dataset.coverOverlay;
    return true;
  }

  /** The settled cover (no input for COVER_LAYER_IDLE_MS): draw its visible part plus a bounded margin, sharp, replacing
   *  whatever the overlay showed (one task: never a frame without either). At the promote-time scale none is needed. */
  function drawSettled() {
    if (destroyed || resetting || layerGate.active || !active()) return;
    if (!coverOverlayWanted(state.scale, promotedScale)) { hideOverlay(); return; }
    // Drawn for the stage without its swipe feedback: the mirrored feedback keeps applying on top of the new drawing.
    const visual = stageRect();
    const visible = visual.width > 0 ? clipToAncestors(node, visual) : null;
    const dpr = typeof devicePixelRatio === 'number' && devicePixelRatio > 0 ? devicePixelRatio : 1;
    const region = visible ? overlayDrawRegion({ visible, cover: visual, dpr }) : null;
    if (!region || !drawOverlay(visual, state.scale, region, false)) hideOverlay();
  }

  /** Keep a shown overlay on the stage now painted at `visual`: compositor transform only (no redraw). `paneBox` is the
   *  pane's client rect read at the same clean point as the stage's. @param {Rect} visual @param {Rect} paneBox */
  function followOverlay(visual, paneBox) {
    if (overlay?.following) overlay.follow(visual, paneBox, layoutCentre(visual));
  }

  /** The stage's un-transformed layout centre (its transform-origin, where the swipe feedback pivots) from its painted
   *  rect without feedback. @param {Rect} visual */
  function layoutCentre(visual) {
    return { x: visual.left + visual.width / 2 - state.x, y: visual.top + visual.height / 2 - state.y };
  }

  /** Whether the stage may carry swipe feedback now: an inline translate/scale, or its settle transition still running. */
  function feedbackLive() {
    const style = node.style;
    return Boolean(style.translate || style.scale || /translate|scale/.test(style.transition || ''));
  }

  /** The stage's painted client rect without the swipe feedback (`translate` / `scale`, which getBoundingClientRect
   *  includes): every geometry here is the zoom's own; the overlay gets the feedback by mirroring. The computed (current,
   *  also mid-transition) feedback is read only while there can be some, right after the rect read (style is clean). */
  function stageRect() {
    const rect = node.getBoundingClientRect();
    if (!feedbackLive() || typeof getComputedStyle !== 'function') return rect;
    const computed = getComputedStyle(node);
    return withoutSwipeFeedback(rect, parseSwipeFeedback(computed.translate, computed.scale), state);
  }

  /** Copy the stage's swipe feedback (translate, scale, opacity, transition) onto the overlay: called from the change
   *  observer (before paint) for every style change of the stage, so both move, fade and settle together. Not during the
   *  reset glide, whose copy runs its own animation. */
  function mirrorFeedback() {
    if (!overlay) return;
    const style = node.style;
    overlay.mirror(resetting ? null : { translate: style.translate || '', scale: style.scale || '', opacity: style.opacity || '', transition: style.transition || '' });
  }

  /** The cover or its pane was resized (e.g. the squeeze while zooming, a window resize). ResizeObserver runs after
   *  layout, before paint, so both reads are clean: a shown overlay is re-aligned in this frame, then redrawn once quiet. */
  function resized() {
    if (destroyed || resetting) return;
    if (!overlay?.following) { invalidateOverlay(); return; }
    followOverlay(stageRect(), overlay.paneRect());
    layerGate.poke();
  }

  /** Something outside the gesture moved or resized the cover (window resize, another style write, new image): fall
   *  back to the stage at once (same frame) and draw again once it has been quiet. */
  function invalidateOverlay() {
    if (destroyed || resetting) return;
    hideOverlay();
    if (coverOverlayWanted(state.scale, promotedScale)) layerGate.poke();
  }
  const overlayObservers = observeCoverChanges(node, invalidateOverlay, resized, mirrorFeedback, (record) => record.target === node && record.attributeName === 'style' && nonFeedbackStyle(node.style) === ownStyle);

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
    // Only with a shown overlay: the pane is read at the same clean point (no forced layout).
    const paneBox = overlay?.following ? overlay.paneRect() : null;
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
    // The overlay moves with the stage in the same write (pan, zoom out, zoom in: a sub-region of what was drawn sharp).
    if (paneBox) followOverlay(coverVisualRect(geo, state), paneBox);
    // The scale change moves the layout (squeeze) after this write: re-pin the anchor before this frame is painted.
    if (scaleChanged && pin) repin(pin);
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
      const paneBox = overlay?.following ? overlay.paneRect() : null; // layout is clean again after the read above
      const next = anchoredZoom(geo, pin.u, pin, state.scale);
      if (Math.abs(next.x - state.x) >= 0.01 || Math.abs(next.y - state.y) >= 0.01) apply(next, false);
      // Same task as the stage's compensating transform: the overlay never disagrees with the squeezed layout.
      if (paneBox) followOverlay(coverVisualRect(geo, state), paneBox);
    }, () => {});
  }

  /** Back to the original size (FLIP): the final layout is committed at once (the lyrics column and the squeeze snap at
   *  frame 0 together with the glide), and in the same task, before paint, the cover is put back where it was painted
   *  (inverse transform) and glides to identity. Never an uncompensated frame. */
  function reset() {
    if (state.scale === COVER_ZOOM_REST || resetting) return;
    wheelPending = null; wheelAnchor = null; repinToken += 1;
    if (reduced() || !hasRaf() || typeof node.animate !== 'function') {
      hideOverlay();
      apply({ ...COVER_ZOOM_IDENTITY }, false);
      layerGate.poke();
      return;
    }
    resetting = true;
    layerGate.hold();
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
    const start = flipTransform(from, to);
    overlay?.mirror(null); // the copy runs its own animation: no swipe feedback copied onto it
    // The copy at the screen-resolution thumbnail size glides, not the stage (see the action's doc comment). Without a
    // decoded image (fallback art) the stage itself glides.
    const target = overlay && drawOverlay(to, 1, to, true) ? overlay.element : node;
    const animation = target.animate(
      [{ transform: `translate(${start.x.toFixed(2)}px, ${start.y.toFixed(2)}px) scale(${start.scale.toFixed(4)})` }, { transform: 'translate(0px, 0px) scale(1)' }],
      { duration: COVER_RESET_MS, easing: 'ease' },
    );
    animation.currentTime = elapsedMs;
    animation.onfinish = endGlide;
    glide = animation;
  }

  /** @param {{ left: number, top: number, width: number, height: number }} from */
  function beginGlide(from) {
    if (destroyed || !resetting) return;
    hideOverlay(); // a sharp overlay from before the tap is replaced by the copy in this same task
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

  function endGlide() {
    glideObserver?.disconnect();
    glideObserver = null;
    glide = null;
    // Same task: the copy goes and the stage's image (already at its final scale) comes back in one commit.
    hideOverlay();
    delete node.dataset.coverResetting;
    resetting = false;
    mirrorFeedback();
    zoomGate.release();
    layerGate.release();
  }

  /** @param {WheelEvent} event */
  function wheel(event) {
    if (!active()) return;
    event.preventDefault();
    if (resetting) return;
    const factor = getWheelZoomFactor(event.deltaY, event.deltaMode);
    wheelPending = { factor: (wheelPending?.factor ?? 1) * factor, x: event.clientX, y: event.clientY };
    zoomGate.poke();
    layerGate.poke();
    schedule();
  }

  /** @param {PointerEvent} event */
  function down(event) {
    if (!active() || resetting) return;
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    // Any finger or button on the cover may move it (pinch, pan): no settled redraw until all are up. A shown overlay
    // stays and follows the stage.
    layerGate.hold();
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
    } else if (pointers.size === 1 && event.pointerType === 'mouse' && state.scale > COVER_ZOOM_REST) {
      pan = { x: event.clientX, y: event.clientY, ox: state.x, oy: state.y, cx: event.clientX, cy: event.clientY, moved: false, dirty: false };
      try { node.setPointerCapture(event.pointerId); } catch { /* gone */ }
    }
  }

  /** @param {PointerEvent} event */
  function move(event) {
    if (!pointers.has(event.pointerId)) return;
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
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
    if (pointers.size === 0) { pan = null; layerGate.release(); }

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
  promotedScale = state.scale;
  if (coverOverlayWanted(state.scale, promotedScale)) layerGate.poke();

  node.addEventListener('wheel', wheel, { passive: false });
  node.addEventListener('pointerdown', down);
  node.addEventListener('pointermove', move);
  node.addEventListener('pointerup', up);
  node.addEventListener('pointercancel', up);
  node.addEventListener('dblclick', dblclick);
  node.addEventListener('dragstart', dragstart);
  node.addEventListener('click', click, true);
  return {
    /** @param {typeof params} next */
    update(next) {
      options = next;
      // The owner can drop the zoom from outside (an enlarged cover is reset when the track changes): when it says
      // the scale is back at 1 while this element is still zoomed, follow it.
      if (next.syncScale === COVER_ZOOM_REST && state.scale !== COVER_ZOOM_REST && !resetting) {
        pointers.clear(); pinch = null; pan = null; lastTap = null; wheelPending = null; wheelAnchor = null; repinToken += 1;
        hideOverlay();
        apply({ ...COVER_ZOOM_IDENTITY }, false, false);
        if (coverOverlayWanted(state.scale, promotedScale)) layerGate.poke();
        else layerGate.end();
      }
      // The page (re)opened: its layers are created now, at the current scale.
      if (active() && !wasActive) {
        promotedScale = state.scale;
        hideOverlay();
        if (coverOverlayWanted(state.scale, promotedScale)) layerGate.poke();
      }
      wasActive = active();
      // A disabled cover (the Now Playing page is closed) keeps its zoom: only a double click restores it.
      if (!active()) { pointers.clear(); pinch = null; pan = null; wheelPending = null; zoomGate.release(); layerGate.release(); }
    },
    destroy() {
      if (frameId && typeof cancelAnimationFrame === 'function') cancelAnimationFrame(frameId);
      destroyed = true;
      glide?.cancel();
      glideObserver?.disconnect();
      overlayObservers.disconnect();
      overlay?.remove();
      delete node.dataset.coverResetting;
      delete node.dataset.coverOverlay;
      zoomGate.end();
      layerGate.dispose();
      node.removeEventListener('wheel', wheel);
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('pointerup', up);
      node.removeEventListener('pointercancel', up);
      node.removeEventListener('dragstart', dragstart);
      node.removeEventListener('dblclick', dblclick);
      node.removeEventListener('click', click, true);
      node.style.transform = '';
      delete node.dataset.coverZoomed;
    }
  };
}

/** @typedef {{ left: number, top: number, width: number, height: number }} Rect */

/**
 * The client rect of `rect` that is actually on screen: clipped by every ancestor that clips its overflow and by the
 * window. Read once when the cover settles, never per frame. @param {HTMLElement} node @param {Rect} rect
 */
function clipToAncestors(node, rect) {
  /** @type {Rect | null} */
  let visible = typeof innerWidth === 'number' ? intersectRects(rect, { left: 0, top: 0, width: innerWidth, height: innerHeight }) : rect;
  for (let element = node.parentElement; element && visible && typeof getComputedStyle === 'function'; element = element.parentElement) {
    const style = getComputedStyle(element);
    if (style.overflowX === 'visible' && style.overflowY === 'visible') continue;
    const box = element.getBoundingClientRect();
    visible = intersectRects(visible, { left: box.left + element.clientLeft, top: box.top + element.clientTop, width: element.clientWidth, height: element.clientHeight });
  }
  return visible;
}

/** The stage's swipe feedback properties (swipe-dismiss.js damped mode): mirrored onto the overlay, never a foreign change. */
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
 * resize: `onChange`) and for resizes of the cover or its pane (`onResize`, which may come from the zoom's own
 * squeeze); MutationObserver and ResizeObserver callbacks run before the next paint. `onStyle` runs first for every
 * batch of mutations (the swipe feedback is mirrored there, in the same frame as the stage's write). Mutation records
 * for which `isOwn` is true (the action's own transform writes and swipe feedback), and records of an attribute the
 * batch left at its old value (`unchangedInBatch`), do not count as a change.
 * @param {HTMLElement} node @param {() => void} onChange @param {() => void} onResize @param {() => void} onStyle
 * @param {(record: MutationRecord) => boolean} isOwn
 */
function observeCoverChanges(node, onChange, onResize, onStyle, isOwn) {
  /** @type {Array<() => void>} */
  const stops = [];
  if (typeof MutationObserver === 'function') {
    const mutations = new MutationObserver((records) => {
      onStyle();
      if (records.some((record) => !isOwn(record) && !unchangedInBatch(record, records))) onChange();
    });
    mutations.observe(node, { attributes: true, attributeFilter: ['style', 'class', 'src'], subtree: true, attributeOldValue: true });
    stops.push(() => mutations.disconnect());
  }
  if (typeof ResizeObserver === 'function') {
    let first = true;
    const sizes = new ResizeObserver(() => { if (first) { first = false; return; } onResize(); });
    sizes.observe(node);
    if (node.parentElement) sizes.observe(node.parentElement);
    stops.push(() => sizes.disconnect());
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

/**
 * The persistent overlay canvas, appended to the stage's parent (the artwork pane, `position: relative` in app.css).
 * It keeps its compositor layer for the stage's life (app.css `.cover-stage-overlay`); showing and hiding only changes
 * its opacity, and its backing store shrinks to 1x1 while hidden. It remembers where it was drawn (the stage's painted
 * rect, its own client rect, the pane's position) so `follow` can keep it on a moving stage with a transform only.
 * @param {HTMLElement} node
 */
function createOverlayCanvas(node) {
  const parent = node.parentElement;
  if (!parent || typeof document === 'undefined' || typeof document.createElement !== 'function') return null;
  const canvas = /** @type {HTMLCanvasElement} */ (document.createElement('canvas'));
  if (typeof canvas.getContext !== 'function') return null;
  canvas.className = 'cover-stage-overlay';
  canvas.setAttribute('aria-hidden', 'true');
  canvas.width = 1;
  canvas.height = 1;
  parent.appendChild(canvas);
  let shown = false;
  /** @type {{ visual: Rect, canvas: Rect, pane: { left: number, top: number } } | null} */
  let drawnAt = null;
  /** The stage's swipe feedback as last mirrored, or null (none / the reset copy). @type {{ translate: string, scale: string, opacity: string, transition: string } | null} */
  let feedback = null;
  const shownOpacity = () => feedback?.opacity || '1';
  /** @param {{ x: number, y: number }} origin */
  const setOrigin = (origin) => { canvas.style.transformOrigin = `${origin.x.toFixed(2)}px ${origin.y.toFixed(2)}px`; };
  return {
    element: canvas,
    get shown() { return shown; },
    /** Shown with a drawing that can follow the stage. */
    get following() { return shown && drawnAt !== null; },
    paneRect() { return parent.getBoundingClientRect(); },
    /**
     * @param {Rect} visual the stage's painted client rect now (without swipe feedback) @param {Rect} paneBox the pane's
     * client rect now @param {{ x: number, y: number }} centre the stage's layout centre now (client px)
     */
    follow(visual, paneBox, centre) {
      if (!shown || !drawnAt) return;
      const shift = { x: paneBox.left - drawnAt.pane.left, y: paneBox.top - drawnAt.pane.top };
      // Pivot on the stage's centre (mirrored swipe scale); the follow translate is expressed for that pivot.
      const origin = overlayOrigin(centre, { ...drawnAt.canvas, left: drawnAt.canvas.left + shift.x, top: drawnAt.canvas.top + shift.y });
      setOrigin(origin);
      canvas.style.transform = overlayTransformCss(overlayFollowTransform(drawnAt.visual, drawnAt.canvas, visual, shift, origin));
    },
    /**
     * The stage's swipe feedback, copied as is (same values, same transition, same frame); null clears it. While shown
     * the canvas's opacity is the stage's feedback opacity.
     * @param {{ translate: string, scale: string, opacity: string, transition: string } | null} next
     */
    mirror(next) {
      feedback = next;
      const values = next ?? { translate: '', scale: '', transition: '' };
      if ((canvas.style.transition ?? '') !== values.transition) canvas.style.transition = values.transition;
      if ((canvas.style.translate ?? '') !== values.translate) canvas.style.translate = values.translate;
      if ((canvas.style.scale ?? '') !== values.scale) canvas.style.scale = values.scale;
      if (shown && canvas.style.opacity !== shownOpacity()) canvas.style.opacity = shownOpacity();
    },
    /**
     * @param {HTMLImageElement} image @param {NonNullable<ReturnType<typeof coverDrawPlan>>} plan @param {Rect} visible
     * @param {number} radius the rounded clip radius in canvas px @param {Rect} visual the stage's painted client rect
     * @param {{ x: number, y: number }} centre the stage's layout centre (client px)
     */
    draw(image, plan, visible, radius, visual, centre) {
      const context = canvas.getContext('2d');
      if (!context) return false;
      const paneBox = parent.getBoundingClientRect();
      canvas.width = plan.canvas.width;
      canvas.height = plan.canvas.height;
      canvas.style.left = `${visible.left - paneBox.left - parent.clientLeft + parent.scrollLeft}px`;
      canvas.style.top = `${visible.top - paneBox.top - parent.clientTop + parent.scrollTop}px`;
      canvas.style.width = `${visible.width}px`;
      canvas.style.height = `${visible.height}px`;
      canvas.style.transform = '';
      setOrigin(overlayOrigin(centre, visible));
      drawnAt = { visual: { left: visual.left, top: visual.top, width: visual.width, height: visual.height }, canvas: { ...visible }, pane: { left: paneBox.left, top: paneBox.top } };
      context.imageSmoothingEnabled = true;
      context.imageSmoothingQuality = 'high';
      context.save();
      if (radius > 0) {
        context.beginPath();
        if (typeof context.roundRect === 'function') context.roundRect(plan.clip.x, plan.clip.y, plan.clip.width, plan.clip.height, radius);
        else context.rect(plan.clip.x, plan.clip.y, plan.clip.width, plan.clip.height);
        context.clip();
      }
      context.drawImage(image, plan.source.x, plan.source.y, plan.source.width, plan.source.height, plan.dest.x, plan.dest.y, plan.dest.width, plan.dest.height);
      context.restore();
      canvas.style.opacity = shownOpacity();
      shown = true;
      return true;
    },
    hide() {
      canvas.style.opacity = '0';
      canvas.style.transform = '';
      drawnAt = null;
      canvas.width = 1;
      canvas.height = 1;
      shown = false;
    },
    remove() { canvas.remove(); },
  };
}
