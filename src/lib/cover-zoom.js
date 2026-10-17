// Now-playing cover zoom: wheel (mouse over the cover) or two-finger pinch scales the cover itself (CSS transform on
// the cover element); double click / double tap restores. The layout reserves the zoomed cover size so the lyrics
// area is squeezed by the cover instead of scaling with it (see --np-cover-zoom in app.css). Pure math is exported
// for tests; `coverZoom` is the Svelte action that wires the events.

export const COVER_ZOOM_MIN = 0.4;
export const COVER_ZOOM_REST = 1;
export const COVER_ZOOM_MAX = 4;
export const COVER_ZOOM_WHEEL_STEP = 0.0015;
export const COVER_DOUBLE_TAP_MS = 300;
export const COVER_DOUBLE_TAP_SLOP_PX = 24;
export const COVER_PAN_SLOP_PX = 6;

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
 * use:coverZoom={{ enabled, onZoom, initial, syncScale }} on the cover element, which is the element that gets transformed. Mouse: wheel
 * over the cover zooms at the cursor, drag pans while zoomed. Touch/pen: pinch zooms, one finger pans while zoomed.
 * Double click / double tap restores the original size. `onZoom` receives the current state
 * ({ scale, x, y }; scale 1 = not zoomed). The cover element is re-created on track changes (the artwork pane is removed
 * while the next cover loads), so the owner keeps the last state and passes it back as `initial` to restore it.
 * @param {HTMLElement} node
 * @param {{ enabled?: boolean, onZoom?: (state: CoverZoomState, size: { width: number, height: number }) => void, initial?: CoverZoomState | ((node: HTMLElement) => CoverZoomState), syncScale?: number }} params
 */
export function coverZoom(node, params) {
  let options = params;
  /** @type {CoverZoomState} */
  let state = { ...COVER_ZOOM_IDENTITY };
  /** @type {Map<number, { x: number, y: number }>} */
  const pointers = new Map();
  /** @type {{ distance: number, scale: number, midX: number, midY: number } | null} */
  let pinch = null;
  /** @type {{ x: number, y: number, ox: number, oy: number, moved: boolean } | null} */
  let pan = null;
  /** @type {{ x: number, y: number, t: number } | null} */
  let lastTap = null;
  let suppressClick = false;
  let dispatchingCancel = false;
  const reduced = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
  const active = () => options.enabled !== false;

  function layerSize() {
    return { width: node.offsetWidth, height: node.offsetHeight };
  }

  /** @param {CoverZoomState} next @param {boolean} animate @param {boolean} [notify] */
  function apply(next, animate, notify = true) {
    state = next;
    node.style.transition = animate && !reduced() ? 'transform 160ms ease' : 'none';
    node.style.transform = coverZoomTransform(state);
    // A data attribute, not a class: Svelte rewrites the class list of this element when `has-artwork` toggles
    // (track change), which would drop a class added from here.
    if (state.scale !== COVER_ZOOM_REST) node.dataset.coverZoomed = '';
    else delete node.dataset.coverZoomed;
    if (notify) options.onZoom?.(state, { width: node.offsetWidth, height: node.offsetHeight });
  }

  /** Anchor relative to the cover's visual centre. @param {number} clientX @param {number} clientY */
  function anchorOf(clientX, clientY) {
    const rect = node.getBoundingClientRect();
    return { x: clientX - (rect.left + rect.width / 2), y: clientY - (rect.top + rect.height / 2) };
  }

  let settleFrame = 0;

  /**
   * Zooming changes the layout (the cover's column grows/shrinks, the lyrics get squeezed), which moves the cover's
   * un-transformed centre. After the layout settles, shift the translation so the content point `u` (offset from the
   * cover centre, in cover px) is again under `target` (client coords).
   * @param {{ x: number, y: number }} target @param {{ x: number, y: number }} u
   */
  function settleAnchor(target, u) {
    if (typeof requestAnimationFrame !== 'function') return;
    cancelAnimationFrame(settleFrame);
    settleFrame = requestAnimationFrame(() => {
      settleFrame = 0;
      if (state.scale === COVER_ZOOM_REST) return;
      const rect = node.getBoundingClientRect();
      const centreX = rect.left + rect.width / 2;
      const centreY = rect.top + rect.height / 2;
      const { width, height } = layerSize();
      const dx = target.x - u.x * state.scale - centreX;
      const dy = target.y - u.y * state.scale - centreY;
      if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5) return;
      apply(clampCoverZoom({ scale: state.scale, x: state.x + dx, y: state.y + dy }, width, height), false);
    });
  }

  /** Content offset from the cover centre (cover px) of the point currently at client (x, y). @param {number} clientX @param {number} clientY */
  function contentOffsetAt(clientX, clientY) {
    const a = anchorOf(clientX, clientY);
    return { x: a.x / state.scale, y: a.y / state.scale };
  }

  /** @param {number} scale @param {number} clientX @param {number} clientY */
  function zoomTo(scale, clientX, clientY) {
    const { width, height } = layerSize();
    const a = anchorOf(clientX, clientY);
    const u = contentOffsetAt(clientX, clientY);
    apply(zoomCoverAt(state, scale, a.x, a.y, width, height), false);
    settleAnchor({ x: clientX, y: clientY }, u);
  }

  function reset() {
    if (state.scale !== COVER_ZOOM_REST) apply({ ...COVER_ZOOM_IDENTITY }, true);
  }

  /** @param {WheelEvent} event */
  function wheel(event) {
    if (!active()) return;
    event.preventDefault();
    zoomTo(state.scale * getWheelZoomFactor(event.deltaY, event.deltaMode), event.clientX, event.clientY);
  }

  /** @param {PointerEvent} event */
  function down(event) {
    if (!active()) return;
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    if (pointers.size === 2) {
      const [a, b] = [...pointers.values()];
      pinch = { distance: Math.max(1, pointerDistance(a, b)), scale: state.scale, midX: (a.x + b.x) / 2, midY: (a.y + b.y) / 2 };
      pan = null;
      lastTap = null;
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
      pan = { x: event.clientX, y: event.clientY, ox: state.x, oy: state.y, moved: false };
      try { node.setPointerCapture(event.pointerId); } catch { /* gone */ }
    }
  }

  /** @param {PointerEvent} event */
  function move(event) {
    if (!pointers.has(event.pointerId)) return;
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    if (pinch && pointers.size >= 2) {
      const [a, b] = [...pointers.values()];
      const midX = (a.x + b.x) / 2;
      const midY = (a.y + b.y) / 2;
      const mid = anchorOf(midX, midY);
      const u = contentOffsetAt(pinch.midX, pinch.midY);
      const { width, height } = layerSize();
      const zoomed = zoomCoverAt(state, pinch.scale * (pointerDistance(a, b) / pinch.distance), mid.x, mid.y, width, height);
      // Two fingers moving together pan the zoomed cover.
      apply(clampCoverZoom({ scale: zoomed.scale, x: zoomed.x + (midX - pinch.midX), y: zoomed.y + (midY - pinch.midY) }, width, height), false);
      pinch.midX = midX;
      pinch.midY = midY;
      settleAnchor({ x: midX, y: midY }, u);
      suppressClick = true;
      if (event.cancelable) event.preventDefault();
    } else if (pan && pointers.size === 1) {
      const dx = event.clientX - pan.x;
      const dy = event.clientY - pan.y;
      if (!pan.moved && Math.hypot(dx, dy) < COVER_PAN_SLOP_PX) return;
      pan.moved = true;
      suppressClick = true;
      const { width, height } = layerSize();
      apply(clampCoverZoom({ scale: state.scale, x: pan.ox + dx, y: pan.oy + dy }, width, height), false);
      if (event.cancelable) event.preventDefault();
    }
  }

  /** @param {PointerEvent} event */
  function up(event) {
    if (dispatchingCancel || !pointers.has(event.pointerId)) return;
    const wasPinching = pinch !== null;
    const moved = pan?.moved === true;
    pointers.delete(event.pointerId);
    if (pointers.size < 2) pinch = null;
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
  if (initial && initial.scale !== COVER_ZOOM_REST) apply({ ...initial }, false, false);

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
      if (next.syncScale === COVER_ZOOM_REST && state.scale !== COVER_ZOOM_REST) {
        pointers.clear(); pinch = null; pan = null; lastTap = null;
        apply({ ...COVER_ZOOM_IDENTITY }, false, false);
      }
      // A disabled cover (the Now Playing page is closed) keeps its zoom: only a double click restores it.
      if (!active()) { pointers.clear(); pinch = null; pan = null; }
    },
    destroy() {
      cancelAnimationFrame(settleFrame);
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
