// Now-playing cover zoom: wheel (mouse over the cover) or two-finger pinch scales the cover itself (CSS transform on
// the cover element); double click / double tap restores. The layout reserves the zoomed cover size so the lyrics
// area is squeezed by the cover instead of scaling with it (see --np-cover-reserve in app.css). Pure math is exported
// for tests; `coverZoom` is the Svelte action that wires the events.

export const COVER_ZOOM_MIN = 0.5;
export const COVER_ZOOM_REST = 1;
export const COVER_ZOOM_MAX = 10;
export const COVER_ZOOM_WHEEL_STEP = 0.0015;
export const COVER_DOUBLE_TAP_MS = 300;
export const COVER_DOUBLE_TAP_SLOP_PX = 24;
export const COVER_PAN_SLOP_PX = 6;
/** No zoom input for this long = the zoom gesture is over: the lyrics re-wrap and re-centre once. */
export const COVER_ZOOM_SETTLE_MS = 150;
/** The cover keeps its own compositor layer (will-change) this long after the last input, then is re-rasterised. */
export const COVER_LAYER_IDLE_MS = 200;
const COVER_RESET_MS = 160;
const COVER_WHEEL_ANCHOR_SLOP_PX = 2;

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
 * forces nothing), then only writes (transform, data attributes, onZoom).
 * @param {HTMLElement} node
 * @param {{ enabled?: boolean, onZoom?: (state: CoverZoomState, size: { width: number, height: number }) => void, onZoomingChange?: (zooming: boolean) => void, initial?: CoverZoomState | ((node: HTMLElement) => CoverZoomState), syncScale?: number }} params
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
  /** After a scale change the layout moved: one more frame re-pins the anchor. @type {{ u: { x: number, y: number }, x: number, y: number } | null} */
  let settle = null;
  /** @type {{ x: number, y: number, t: number } | null} */
  let lastTap = null;
  let suppressClick = false;
  let dispatchingCancel = false;
  let resetting = false;
  let frameId = 0;
  /** @type {ReturnType<typeof setTimeout> | null} */
  let resetTimer = null;
  const reduced = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
  const active = () => options.enabled !== false;
  const hasRaf = () => typeof requestAnimationFrame === 'function';
  const raf = (/** @type {() => void} */ fn) => (hasRaf() ? requestAnimationFrame(fn) : (fn(), 0));

  // will-change: transform only while a gesture runs. Left on, Chromium keeps the raster scale it had when the
  // layer was promoted, so a cover zoomed in afterwards stays blurry; dropping it re-rasters at the settled scale.
  const layerGate = createIdleGate(COVER_LAYER_IDLE_MS, (on) => {
    if (on) node.dataset.coverGesture = '';
    else delete node.dataset.coverGesture;
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
    // A data attribute, not a class: Svelte rewrites the class list of this element when `has-artwork` toggles
    // (track change), which would drop a class added from here.
    if (state.scale !== COVER_ZOOM_REST) node.dataset.coverZoomed = '';
    else delete node.dataset.coverZoomed;
    if (notify) options.onZoom?.(state, size);
  }

  function schedule() {
    if (frameId) return;
    frameId = raf(frame);
  }

  function frame() {
    frameId = 0;
    if (resetting || !active()) { wheelPending = null; return; }
    // The only layout read of the frame: first, so nothing written yet can have dirtied the layout.
    const geo = coverGeometry(node.getBoundingClientRect(), state);
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
      if (!pinch.dirty && !settle) return;
      const [a, b] = [...pointers.values()];
      const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
      if (!pinch.u) pinch.u = contentOffsetAt(geo, state, pinch.startX, pinch.startY);
      pinch.dirty = false;
      pin = { u: pinch.u, x: mid.x, y: mid.y };
      next = anchoredZoom(geo, pin.u, pin, pinch.scale * (pointerDistance(a, b) / pinch.distance));
    } else if (pan && pointers.size === 1 && pan.dirty) {
      pan.dirty = false;
      next = clampCoverZoom({ scale: state.scale, x: pan.ox + (pan.cx - pan.x), y: pan.oy + (pan.cy - pan.y) }, geo.width, geo.height);
    } else if (settle) {
      next = anchoredZoom(geo, settle.u, settle, state.scale);
    }
    const scaleChanged = next !== null && next.scale !== state.scale;
    settle = null;
    if (next && (scaleChanged || Math.abs(next.x - state.x) >= 0.01 || Math.abs(next.y - state.y) >= 0.01)) apply(next, false);
    // The scale change moves the layout (squeeze) after this write: re-pin the anchor in one more frame.
    if (scaleChanged && pin) { settle = pin; schedule(); }
  }

  /** Back to the original size: the transform glides first, the layout (and onZoom) follows when it ends, and the cover is
   *  carried from where it was to where the new layout puts it. */
  function reset() {
    if (state.scale === COVER_ZOOM_REST || resetting) return;
    wheelPending = null; wheelAnchor = null; settle = null;
    if (reduced() || !hasRaf()) {
      apply({ ...COVER_ZOOM_IDENTITY }, false);
      return;
    }
    resetting = true;
    layerGate.hold();
    node.style.transition = `transform ${COVER_RESET_MS}ms ease`;
    node.style.transform = '';
    resetTimer = setTimeout(() => {
      resetTimer = null;
      const before = node.getBoundingClientRect(); // identity transform: the old layout centre (the one forced read)
      zoomGate.poke();
      apply({ ...COVER_ZOOM_IDENTITY }, false);
      raf(() => {
        const after = node.getBoundingClientRect(); // the new layout centre, after the owner's update
        const dx = (before.left + before.width / 2) - (after.left + after.width / 2);
        const dy = (before.top + before.height / 2) - (after.top + after.height / 2);
        if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5) { resetting = false; layerGate.release(); return; }
        node.style.transition = 'none';
        node.style.transform = `translate(${dx.toFixed(2)}px, ${dy.toFixed(2)}px)`;
        raf(() => {
          node.style.transition = `transform ${COVER_RESET_MS}ms ease`;
          node.style.transform = '';
          resetTimer = setTimeout(() => { resetTimer = null; resetting = false; layerGate.release(); }, COVER_RESET_MS + 20);
        });
      });
    }, COVER_RESET_MS + 20);
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
    if (pointers.size === 2) {
      const [a, b] = [...pointers.values()];
      pinch = { distance: Math.max(1, pointerDistance(a, b)), scale: state.scale, startX: (a.x + b.x) / 2, startY: (a.y + b.y) / 2, u: null, dirty: false };
      pan = null;
      lastTap = null;
      zoomGate.hold();
      layerGate.hold();
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
      layerGate.poke();
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
    if (pointers.size < 2 && pinch) { pinch = null; zoomGate.release(); layerGate.release(); }
    if (pointers.size === 0) pan = null;

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
        pointers.clear(); pinch = null; pan = null; lastTap = null; wheelPending = null; wheelAnchor = null; settle = null;
        apply({ ...COVER_ZOOM_IDENTITY }, false, false);
        layerGate.end();
      }
      // A disabled cover (the Now Playing page is closed) keeps its zoom: only a double click restores it.
      if (!active()) { pointers.clear(); pinch = null; pan = null; wheelPending = null; zoomGate.release(); layerGate.release(); }
    },
    destroy() {
      if (frameId && typeof cancelAnimationFrame === 'function') cancelAnimationFrame(frameId);
      if (resetTimer) clearTimeout(resetTimer);
      zoomGate.end();
      layerGate.end();
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
