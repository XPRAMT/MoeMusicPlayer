// Now-playing cover zoom: wheel (mouse over the cover) or two-finger pinch scales the cover itself (CSS transform on
// the cover element); double click / double tap restores. The layout reserves the zoomed cover size so the lyrics
// area is squeezed by the cover instead of scaling with it (see --np-cover-zoom in app.css). Pure math is exported
// for tests; `coverZoom` is the Svelte action that wires the events.

export const COVER_ZOOM_MIN = 1;
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
 * Keep the scaled cover covering its own unscaled box: |offset| <= (scale - 1) * size / 2.
 * @param {CoverZoomState} state @param {number} width @param {number} height @returns {CoverZoomState}
 */
export function clampCoverZoom(state, width, height) {
  const scale = clamp(Number.isFinite(state.scale) ? state.scale : 1, COVER_ZOOM_MIN, COVER_ZOOM_MAX);
  if (scale <= COVER_ZOOM_MIN) return { scale: COVER_ZOOM_MIN, x: 0, y: 0 };
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

/** CSS transform for the cover (transform-origin: center). @param {CoverZoomState} state */
export function coverZoomTransform(state) {
  if (state.scale <= COVER_ZOOM_MIN) return '';
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
 * use:coverZoom={{ enabled, onZoom }} on the cover element, which is the element that gets transformed. Mouse: wheel
 * over the cover zooms at the cursor, drag pans while zoomed. Touch/pen: pinch zooms, one finger pans while zoomed.
 * Double click / double tap resets. `onZoom` receives the current scale (1 = not zoomed).
 * @param {HTMLElement} node
 * @param {{ enabled?: boolean, onZoom?: (scale: number) => void }} params
 */
export function coverZoom(node, params) {
  let options = params;
  /** @type {CoverZoomState} */
  let state = { ...COVER_ZOOM_IDENTITY };
  /** @type {Map<number, { x: number, y: number }>} */
  const pointers = new Map();
  /** @type {{ distance: number, scale: number } | null} */
  let pinch = null;
  /** @type {{ x: number, y: number, ox: number, oy: number, moved: boolean } | null} */
  let pan = null;
  /** @type {{ x: number, y: number, t: number } | null} */
  let lastTap = null;
  let suppressClick = false;
  const reduced = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
  const active = () => options.enabled !== false;

  function layerSize() {
    return { width: node.offsetWidth, height: node.offsetHeight };
  }

  /** @param {CoverZoomState} next @param {boolean} animate */
  function apply(next, animate) {
    state = next;
    node.style.transition = animate && !reduced() ? 'transform 160ms ease' : 'none';
    node.style.transform = coverZoomTransform(state);
    node.classList.toggle('is-cover-zoomed', state.scale > COVER_ZOOM_MIN);
    options.onZoom?.(state.scale);
  }

  /** Anchor relative to the cover's visual centre. @param {number} clientX @param {number} clientY */
  function anchorOf(clientX, clientY) {
    const rect = node.getBoundingClientRect();
    return { x: clientX - (rect.left + rect.width / 2), y: clientY - (rect.top + rect.height / 2) };
  }

  /** @param {number} scale @param {number} clientX @param {number} clientY */
  function zoomTo(scale, clientX, clientY) {
    const { width, height } = layerSize();
    const a = anchorOf(clientX, clientY);
    apply(zoomCoverAt(state, scale, a.x, a.y, width, height), false);
  }

  function reset() {
    if (state.scale > COVER_ZOOM_MIN) apply({ ...COVER_ZOOM_IDENTITY }, true);
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
      pinch = { distance: Math.max(1, pointerDistance(a, b)), scale: state.scale };
      pan = null;
      lastTap = null;
      // Let the swipe-to-skip gesture on the same element drop the first finger (it ignores pointercancel dismissals).
      const [firstId] = [...pointers.keys()];
      try { node.dispatchEvent(new PointerEvent('pointercancel', { pointerId: firstId, bubbles: true })); } catch { /* unsupported */ }
      for (const id of pointers.keys()) {
        try { node.setPointerCapture(id); } catch { /* gone */ }
      }
    } else if (pointers.size === 1 && state.scale > COVER_ZOOM_MIN) {
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
      const mid = anchorOf((a.x + b.x) / 2, (a.y + b.y) / 2);
      const { width, height } = layerSize();
      apply(zoomCoverAt(state, pinch.scale * (pointerDistance(a, b) / pinch.distance), mid.x, mid.y, width, height), false);
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
    if (!pointers.has(event.pointerId)) return;
    const wasPinching = pinch !== null;
    const moved = pan?.moved === true;
    pointers.delete(event.pointerId);
    if (pointers.size < 2) pinch = null;
    if (pointers.size === 0) pan = null;
    else if (pointers.size === 1 && state.scale > COVER_ZOOM_MIN) {
      const [rest] = [...pointers.values()];
      pan = { x: rest.x, y: rest.y, ox: state.x, oy: state.y, moved: true };
    }
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

  node.addEventListener('wheel', wheel, { passive: false });
  node.addEventListener('pointerdown', down);
  node.addEventListener('pointermove', move);
  node.addEventListener('pointerup', up);
  node.addEventListener('pointercancel', up);
  node.addEventListener('dblclick', dblclick);
  node.addEventListener('click', click, true);
  return {
    /** @param {typeof params} next */
    update(next) {
      options = next;
      if (!active()) { pointers.clear(); pinch = null; pan = null; reset(); }
    },
    destroy() {
      node.removeEventListener('wheel', wheel);
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('pointerup', up);
      node.removeEventListener('pointercancel', up);
      node.removeEventListener('dblclick', dblclick);
      node.removeEventListener('click', click, true);
      node.style.transform = '';
      node.classList.remove('is-cover-zoomed');
    }
  };
}
