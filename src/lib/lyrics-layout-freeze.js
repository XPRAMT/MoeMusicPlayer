// While the cover is being zoomed the lyrics column changes width on every step. The text layout (line wrapping, row
// heights, centring) is then kept at the last settled size and the column only clips it; one re-layout happens when the
// zoom activity stops (cover-zoom.js `onZoomingChange(false)`). Pure helpers for LyricsView.

/**
 * The size the layout should use: the settled one while frozen, the measured one otherwise.
 * @param {boolean} frozen @param {{ width: number, height: number }} settled @param {{ width: number, height: number }} measured
 */
export function lyricsLayoutSize(frozen, settled, measured) {
  return frozen ? settled : measured;
}

/** The fixed row width (CSS) while frozen; undefined when not frozen or when no width has been measured yet. @param {boolean} frozen @param {number} width */
export function frozenRowWidth(frozen, width) {
  return frozen && width > 0 ? `${width}px` : undefined;
}
