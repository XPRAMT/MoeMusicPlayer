/**
 * Shift a panel centered on its anchor only as far as needed to fit both edges.
 * @param {number} anchorCenterX
 * @param {number} panelWidth
 * @param {number} viewportWidth
 * @param {number} [insetPx]
 * @returns {number}
 */
export function getPopoverHorizontalShiftPx(anchorCenterX, panelWidth, viewportWidth, insetPx = 8) {
  const centeredLeft = anchorCenterX - panelWidth / 2;
  const centeredRight = anchorCenterX + panelWidth / 2;
  if (centeredLeft < insetPx) return insetPx - centeredLeft;
  if (centeredRight > viewportWidth - insetPx) return viewportWidth - insetPx - centeredRight;
  return 0;
}

/**
 * Return the leftward overflow for a volume panel anchored near the right edge.
 * @param {number} anchorCenterX
 * @param {number} panelWidth
 * @param {number} viewportWidth
 * @param {number} [insetPx]
 * @returns {number}
 */
export function getVolumePopoverLeftShiftPx(anchorCenterX, panelWidth, viewportWidth, insetPx = 8) {
  return Math.max(0, -getPopoverHorizontalShiftPx(anchorCenterX, panelWidth, viewportWidth, insetPx));
}
