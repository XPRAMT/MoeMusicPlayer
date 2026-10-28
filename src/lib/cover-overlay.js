// Now-playing cover overlay canvas (ADR 0036): pure geometry for drawing the cover into a <canvas> that sits above the
// cover stage. Two uses, one element: (1) the reset glide animates a copy drawn at the screen-resolution thumbnail size
// (the cover's on-screen size at zoom 1, in device pixels) instead of the stage itself; (2) after a zoom/pan settles,
// the visible part of the cover plus a bounded margin is drawn at device resolution from the source-resolution image, so
// the zoomed cover is sharp without a giant compositor layer; during later input the drawn canvas follows the stage by a
// compositor transform (`overlayFollowTransform`) until the next settled draw. `cover-zoom.js` owns the element;
// everything here is testable math.

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

/** The settled draw covers at most this many times the visible area's device pixels (visible part + margin). */
export const COVER_OVERLAY_MARGIN_PIXEL_RATIO = 2;
/** The overlay canvas backing never exceeds this many device pixels on a side (unless the visible part alone does). */
export const COVER_OVERLAY_MAX_DIMENSION = 4096;

/**
 * The client rect the settled overlay is drawn for: the visible part of the cover grown by a margin, so content that a
 * pan or zoom-out brings into view is already sharp. The margin is the same on every side, clamped to the cover's own
 * painted rect (a side that hits the cover edge stops, the others keep growing), and grows only while the region stays
 * within `pixelRatio` times the visible area and `maxDimension` device pixels per side.
 * @param {{ visible: Rect, cover: Rect, dpr: number, pixelRatio?: number, maxDimension?: number }} input @returns {Rect}
 */
export function overlayDrawRegion({ visible, cover, dpr, pixelRatio = COVER_OVERLAY_MARGIN_PIXEL_RATIO, maxDimension = COVER_OVERLAY_MAX_DIMENSION }) {
  const base = intersectRects(visible, cover) ?? visible;
  const ratio = dpr > 0 ? dpr : 1;
  const maxArea = base.width * base.height * Math.max(1, pixelRatio);
  const maxWidth = Math.max(base.width, maxDimension / ratio);
  const maxHeight = Math.max(base.height, maxDimension / ratio);
  /** @param {number} m */
  const grow = (m) => intersectRects({ left: base.left - m, top: base.top - m, width: base.width + 2 * m, height: base.height + 2 * m }, cover) ?? base;
  /** @param {Rect} r */
  const fits = (r) => r.width * r.height <= maxArea * (1 + 1e-9) && r.width <= maxWidth + 1e-9 && r.height <= maxHeight + 1e-9;
  const limit = Math.max(
    base.left - cover.left,
    base.top - cover.top,
    cover.left + cover.width - (base.left + base.width),
    cover.top + cover.height - (base.top + base.height),
  );
  if (!(limit > 0)) return base;
  const whole = grow(limit);
  if (fits(whole)) return whole;
  let lo = 0;
  let hi = limit;
  for (let i = 0; i < 40; i += 1) {
    const mid = (lo + hi) / 2;
    if (fits(grow(mid))) lo = mid; else hi = mid;
  }
  return grow(lo);
}

/**
 * The compositor transform that keeps an overlay drawn for the stage at `drawnVisual` (its painted client rect then)
 * aligned with the stage now painted at `visual`. `drawnCanvas` is the canvas's client rect when it was drawn;
 * `paneShift` is how far its containing pane has moved since (the canvas is positioned in the pane, so it moves with
 * it). The cover content is mapped by the same translate + scale as the stage:
 * p' = visual.left + (p - drawnVisual.left) * visual.width / drawnVisual.width.
 * `origin` is the canvas's transform-origin in its own box (px from its top-left; default its centre): the overlay
 * pivots the mirrored swipe `scale` on the stage's centre (`overlayOrigin`), and the translate is adjusted so the
 * result is the same map whatever the origin: translate(a_K) scale(k) around the box centre K equals
 * translate(a_K + (1 - k) (K - O)) scale(k) around O.
 * @param {Rect} drawnVisual @param {Rect} drawnCanvas @param {Rect} visual @param {{ x: number, y: number }} paneShift
 * @param {{ x: number, y: number }} [origin]
 * @returns {{ x: number, y: number, scaleX: number, scaleY: number }}
 */
export function overlayFollowTransform(drawnVisual, drawnCanvas, visual, paneShift, origin) {
  const scaleX = drawnVisual.width > 0 ? visual.width / drawnVisual.width : 1;
  const scaleY = drawnVisual.height > 0 ? visual.height / drawnVisual.height : 1;
  const drawnCentreX = drawnCanvas.left + drawnCanvas.width / 2;
  const drawnCentreY = drawnCanvas.top + drawnCanvas.height / 2;
  const x = visual.left - (drawnCentreX + paneShift.x) + scaleX * (drawnCentreX - drawnVisual.left);
  const y = visual.top - (drawnCentreY + paneShift.y) + scaleY * (drawnCentreY - drawnVisual.top);
  if (!origin) return { x, y, scaleX, scaleY };
  return {
    x: x + (1 - scaleX) * (drawnCanvas.width / 2 - origin.x),
    y: y + (1 - scaleY) * (drawnCanvas.height / 2 - origin.y),
    scaleX,
    scaleY,
  };
}

/**
 * The overlay's transform-origin (px in its own box) that puts its pivot on the stage's layout centre `centre` (client
 * px; the stage's own transform-origin is its centre). `canvasBox` is the canvas's un-transformed client rect now.
 * With it the stage's swipe feedback (`translate`, `scale`) copied onto the canvas maps every client point exactly like
 * it does on the stage: CSS applies translate, then scale, then transform, all around transform-origin, so both become
 * p -> C + t + s (q - C) of the point q they would paint at without the feedback.
 * @param {{ x: number, y: number }} centre @param {Rect} canvasBox
 */
export function overlayOrigin(centre, canvasBox) {
  return { x: centre.x - canvasBox.left, y: centre.y - canvasBox.top };
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

/** CSS for `overlayFollowTransform` ('' at identity). @param {{ x: number, y: number, scaleX: number, scaleY: number }} t */
export function overlayTransformCss(t) {
  if (Math.abs(t.x) < 0.005 && Math.abs(t.y) < 0.005 && Math.abs(t.scaleX - 1) < 1e-6 && Math.abs(t.scaleY - 1) < 1e-6) return '';
  return `translate(${t.x.toFixed(2)}px, ${t.y.toFixed(2)}px) scale(${t.scaleX.toFixed(5)}, ${t.scaleY.toFixed(5)})`;
}

/**
 * Whether the settled cover is shown through the overlay: any scale but 1, or scale 1 on a stage whose compositor layer
 * was promoted at another scale (a will-change layer keeps its promote-time raster scale, so its own pixels are soft).
 * @param {number} scale @param {number} promotedScale
 */
export function coverOverlayWanted(scale, promotedScale) {
  return scale !== 1 || promotedScale !== 1;
}

/** @typedef {{ translate: string, scale: string, opacity: string, transition: string }} MirroredFeedback */

/** Whether a mirrored swipe feedback carries a displacement, scale or fade (the settle transition alone does not count). @param {MirroredFeedback | null | undefined} f */
export function swipeFeedbackActive(f) {
  return Boolean(f && (f.translate || f.scale || f.opacity));
}

/**
 * The stage's feedback just ended (it was displaced, faded or scaled and is back at rest, the bounce-back settling by a
 * transition). The overlay must not run its own copy of that bounce: it is hidden in the same write with every feedback
 * property cleared, and redrawn once the stage has settled (ADR 0036).
 * @param {MirroredFeedback | null | undefined} previous @param {MirroredFeedback | null | undefined} next
 */
export function swipeFeedbackReleased(previous, next) {
  return swipeFeedbackActive(previous) && !swipeFeedbackActive(next);
}

/** The inline feedback values a (re)drawn overlay carries: exactly the stage's now, no transition. @param {MirroredFeedback | null | undefined} f */
export function overlayFeedbackStyle(f) {
  return { translate: f?.translate || '', scale: f?.scale || '', opacity: f?.opacity || '' };
}

/**
 * Whether a shown overlay still belongs to the cover image now on the stage (same source it was drawn from). A different
 * or unknown current source means the drawing is stale: it is dropped and drawn again.
 * @param {string | null | undefined} drawnSource @param {string | null | undefined} currentSource
 */
export function overlayMatchesSource(drawnSource, currentSource) {
  return (drawnSource || "") === (currentSource || "");
}
