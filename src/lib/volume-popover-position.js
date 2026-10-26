/** @typedef {{ left?: number, right?: number }} HorizontalSafeInsets */

/**
 * Shift a panel centered on its anchor only as far as needed to fit both edges of the safe area
 * (the window minus the system-bar insets, see `safe-viewport.js`).
 * @param {number} anchorCenterX
 * @param {number} panelWidth
 * @param {number} viewportWidth
 * @param {number} [insetPx]
 * @param {HorizontalSafeInsets} [safeInsets]
 * @returns {number}
 */
export function getPopoverHorizontalShiftPx(anchorCenterX, panelWidth, viewportWidth, insetPx = 8, safeInsets = {}) {
  const minLeft = (safeInsets.left ?? 0) + insetPx;
  const maxRight = viewportWidth - (safeInsets.right ?? 0) - insetPx;
  const centeredLeft = anchorCenterX - panelWidth / 2;
  const centeredRight = anchorCenterX + panelWidth / 2;
  if (centeredLeft < minLeft) return minLeft - centeredLeft;
  if (centeredRight > maxRight) return maxRight - centeredRight;
  return 0;
}

/**
 * Return the leftward overflow for a volume panel anchored near the right edge.
 * @param {number} anchorCenterX
 * @param {number} panelWidth
 * @param {number} viewportWidth
 * @param {number} [insetPx]
 * @param {HorizontalSafeInsets} [safeInsets]
 * @returns {number}
 */
export function getVolumePopoverLeftShiftPx(anchorCenterX, panelWidth, viewportWidth, insetPx = 8, safeInsets = {}) {
  return Math.max(0, -getPopoverHorizontalShiftPx(anchorCenterX, panelWidth, viewportWidth, insetPx, safeInsets));
}
