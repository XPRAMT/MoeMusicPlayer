// Now-playing cover overlay canvas (ADR 0036): pure geometry for drawing the cover into a <canvas> that sits above the
// cover stage. Its only use is the double-tap restore glide: a copy drawn at the screen-resolution thumbnail size (the
// cover's on-screen size at zoom 1, in device pixels) animates instead of the stage itself. Sharpness after a zoom comes
// from the re-raster nudge (cover-reraster.js), not from this canvas. `cover-zoom.js` owns the element; everything here
// is testable math.

/** @typedef {{ left: number, top: number, width: number, height: number }} Rect */

/** @param {Rect} a @param {Rect} b @returns {Rect | null} */
export function intersectRects(a, b) {
  const left = Math.max(a.left, b.left);
  const top = Math.max(a.top, b.top);
  const right = Math.min(a.left + a.width, b.left + b.width);
  const bottom = Math.min(a.top + a.height, b.top + b.height);
  if (!(right > left && bottom > top)) return null;
  return { left, top, width: right - left, height: bottom - top };
}

/**
 * Snap a client rect to whole device pixels (each edge rounded), so the canvas backing maps 1:1 to device pixels.
 * @param {Rect} rect @param {number} dpr @returns {Rect | null}
 */
export function snapRectToDevicePixels(rect, dpr) {
  const ratio = dpr > 0 ? dpr : 1;
  const left = Math.round(rect.left * ratio);
  const top = Math.round(rect.top * ratio);
  const right = Math.round((rect.left + rect.width) * ratio);
  const bottom = Math.round((rect.top + rect.height) * ratio);
  if (right <= left || bottom <= top) return null;
  return { left: left / ratio, top: top / ratio, width: (right - left) / ratio, height: (bottom - top) / ratio };
}

/**
 * Where an `object-fit: cover` image (centred, object-position 50% 50%) sits inside a cover box of `width` x `height`
 * whose image element bleeds `bleed` px past each edge (app.css: `.cover-stage.has-artwork .cover-stage-image`).
 * Returns the image's top-left in box coordinates and the source-to-box scale `k`.
 * @param {number} width @param {number} height @param {number} naturalWidth @param {number} naturalHeight @param {number} [bleed]
 */
export function objectFitCoverPlacement(width, height, naturalWidth, naturalHeight, bleed = 0) {
  const boxWidth = width + 2 * bleed;
  const boxHeight = height + 2 * bleed;
  const k = Math.max(boxWidth / naturalWidth, boxHeight / naturalHeight);
  return { k, left: -bleed + (boxWidth - naturalWidth * k) / 2, top: -bleed + (boxHeight - naturalHeight * k) / 2 };
}

/**
 * The drawImage plan for the overlay canvas. `visual` is the stage's painted client rect (its layout box scaled by
 * `scale` around the centre, plus any pan: the rect getBoundingClientRect returns), `visible` the client rect the canvas
 * covers (already snapped to device pixels). Returns the canvas backing size (device px), the source rect (image px)
 * and the destination rect (canvas px), clamped to the image; null when nothing would be drawn.
 * @param {{ visual: Rect, scale: number, naturalWidth: number, naturalHeight: number, bleed?: number, visible: Rect, dpr: number }} input
 */
export function coverDrawPlan({ visual, scale, naturalWidth, naturalHeight, bleed = 0, visible, dpr }) {
  if (!(scale > 0) || !(naturalWidth > 0) || !(naturalHeight > 0) || !(visual.width > 0) || !(visual.height > 0)) return null;
  const ratio = dpr > 0 ? dpr : 1;
  const placement = objectFitCoverPlacement(visual.width / scale, visual.height / scale, naturalWidth, naturalHeight, bleed);
  // The whole image in client coordinates.
  const image = {
    left: visual.left + placement.left * scale,
    top: visual.top + placement.top * scale,
    width: naturalWidth * placement.k * scale,
    height: naturalHeight * placement.k * scale,
  };
  const drawn = intersectRects(image, visible);
  if (!drawn) return null;
  const perSource = placement.k * scale; // client px per source px
  return {
    canvas: { width: Math.max(1, Math.round(visible.width * ratio)), height: Math.max(1, Math.round(visible.height * ratio)) },
    source: {
      x: (drawn.left - image.left) / perSource,
      y: (drawn.top - image.top) / perSource,
      width: drawn.width / perSource,
      height: drawn.height / perSource,
    },
    dest: {
      x: (drawn.left - visible.left) * ratio,
      y: (drawn.top - visible.top) * ratio,
      width: drawn.width * ratio,
      height: drawn.height * ratio,
    },
    /** The stage's rounded clip in canvas px (the radius scales with the zoom like the stage's own clip). */
    clip: {
      x: (visual.left - visible.left) * ratio,
      y: (visual.top - visible.top) * ratio,
      width: visual.width * ratio,
      height: visual.height * ratio,
    },
  };
}

/** @typedef {{ x: number, y: number, scale: number }} SwipeFeedback */

/**
 * The swipe feedback (`swipe-dismiss.js` damped mode) from the computed `translate` and `scale` values ('none', '',
 * '12px', '12px 3px', '0.97', '0.97 0.97').
 * @param {string | undefined | null} translate @param {string | undefined | null} scale @returns {SwipeFeedback}
 */
export function parseSwipeFeedback(translate, scale) {
  const t = typeof translate === 'string' && translate !== 'none' ? translate.trim().split(/\s+/).map((part) => parseFloat(part)) : [];
  const s = typeof scale === 'string' && scale !== 'none' ? parseFloat(scale.trim().split(/\s+/)[0]) : NaN;
  return { x: Number.isFinite(t[0]) ? t[0] : 0, y: Number.isFinite(t[1]) ? t[1] : 0, scale: Number.isFinite(s) && s > 0 ? s : 1 };
}

/**
 * The stage's painted client rect without the swipe feedback, from a rect read with it (`getBoundingClientRect`
 * includes the individual `translate` and `scale`). The feedback pivots on the layout centre C: R = C + t + s (V - C);
 * `pan` is the stage's own transform translate (V's centre - C).
 * @param {Rect} rect @param {SwipeFeedback} feedback @param {{ x: number, y: number }} pan @returns {Rect}
 */
export function withoutSwipeFeedback(rect, feedback, pan) {
  if (feedback.x === 0 && feedback.y === 0 && feedback.scale === 1) return rect;
  const s = feedback.scale;
  const width = rect.width / s;
  const height = rect.height / s;
  const centreX = rect.left + rect.width / 2 - feedback.x - s * pan.x + pan.x;
  const centreY = rect.top + rect.height / 2 - feedback.y - s * pan.y + pan.y;
  return { left: centreX - width / 2, top: centreY - height / 2, width, height };
}

/**
 * Whether a shown overlay still belongs to the cover image now on the stage (same source it was drawn from). A different
 * or unknown current source means the drawing is stale: it is dropped and drawn again.
 * @param {string | null | undefined} drawnSource @param {string | null | undefined} currentSource
 */
export function overlayMatchesSource(drawnSource, currentSource) {
  return (drawnSource || "") === (currentSource || "");
}
