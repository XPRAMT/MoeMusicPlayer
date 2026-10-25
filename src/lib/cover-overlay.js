// Now-playing cover overlay canvas (ADR 0036): pure geometry for drawing the cover into a <canvas> that sits above the
// cover stage. Two uses, one element: (1) the reset glide animates a copy drawn at the screen-resolution thumbnail size
// (the cover's on-screen size at zoom 1, in device pixels) instead of the stage itself; (2) after a zoom/pan settles,
// only the visible part of the cover is drawn at device resolution from the source-resolution image, so the zoomed
// cover is sharp without a giant compositor layer. `cover-zoom.js` owns the element; everything here is testable math.

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

/**
 * Whether the settled cover is shown through the overlay: any scale but 1, or scale 1 on a stage whose compositor layer
 * was promoted at another scale (a will-change layer keeps its promote-time raster scale, so its own pixels are soft).
 * @param {number} scale @param {number} promotedScale
 */
export function coverOverlayWanted(scale, promotedScale) {
  return scale !== 1 || promotedScale !== 1;
}
