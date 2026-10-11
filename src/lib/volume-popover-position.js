/**
 * Measure only the right-side overflow of a panel centered on its anchor.
 * @param {number} anchorCenterX
 * @param {number} panelWidth
 * @param {number} viewportWidth
 * @param {number} [insetPx]
 * @returns {number}
 */
export function getVolumePopoverLeftShiftPx(anchorCenterX, panelWidth, viewportWidth, insetPx = 8) {
  const centeredRight = anchorCenterX + panelWidth / 2;
  return Math.max(0, centeredRight - (viewportWidth - insetPx));
}
