/**
 * Fit an artwork frame inside its available pane while preserving the source image ratio.
 * The returned dimensions include the border around the artwork (border is per side).
 * @param {{ sourceWidth: number, sourceHeight: number, availableWidth: number, availableHeight: number, maxWidth: number, maxHeight: number, border?: number }} input
 */
export function calculateArtworkFrame(input) {
  const { sourceWidth, sourceHeight, availableWidth, availableHeight, maxWidth, maxHeight } = input;
  const border = Math.max(0, input.border ?? 0);
  if (![sourceWidth, sourceHeight, availableWidth, availableHeight, maxWidth, maxHeight].every((value) => Number.isFinite(value) && value > 0)) {
    return null;
  }

  const innerMaxWidth = Math.max(0, Math.min(availableWidth, maxWidth) - border * 2);
  const innerMaxHeight = Math.max(0, Math.min(availableHeight, maxHeight) - border * 2);
  const scale = Math.min(innerMaxWidth / sourceWidth, innerMaxHeight / sourceHeight);
  if (!Number.isFinite(scale) || scale <= 0) return null;

  const width = sourceWidth * scale + border * 2;
  const height = sourceHeight * scale + border * 2;
  return { width, height, imageWidth: sourceWidth * scale, imageHeight: sourceHeight * scale };
}
