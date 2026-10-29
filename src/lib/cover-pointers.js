// Stale pointer rules for the now-playing cover zoom (ADR 0036): a mouse press whose release never reaches the cover
// node (released outside it or the window, or captured by another handler) must not stay in the pointer map, because a
// non-empty map counts as an active gesture and postpones the re-raster nudge forever. Pure decisions only;
// `cover-zoom.js` owns the map and the listeners.

/** @typedef {{ x: number, y: number, type?: string }} PointerEntry */

/** Only a mouse can be released without the node seeing it; touch and pen are captured to their contact. @param {PointerEntry | undefined} entry */
export function isMouseEntry(entry) {
  return entry !== undefined && entry.type === 'mouse';
}

/** A window-level pointerup / pointercancel releases a tracked mouse entry. @param {PointerEntry | undefined} entry */
export function releasesOnWindowEvent(entry) {
  return isMouseEntry(entry);
}

/** A mouse move with no button held means its release was missed. Touch and pen are never judged by `buttons`. @param {PointerEntry | undefined} entry @param {number} buttons */
export function releasesOnMove(entry, buttons) {
  return isMouseEntry(entry) && buttons === 0;
}
