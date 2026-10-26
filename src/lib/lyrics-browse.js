// Browse and seek on the Now Playing synced lyrics (ADR 0029): while the user scrolls the lyrics, the line at the
// centre of the viewport (when it is not the current line) offers a play button that seeks to that line. Pure
// functions; LyricsView owns the state and the DOM.
import { findLyricsRowAtOffset } from './lyrics-window.js';

/**
 * Timeline index of the row under the viewport's vertical centre (the head spacer above the first row excluded),
 * from the layout already used for the virtual window: a binary search over the Fenwick tree, no DOM reads.
 * @param {import('./lyrics-window.js').LyricsLayout} layout @param {number} scrollTop @param {number} clientHeight
 * @param {number} edgePadding
 */
export function findBrowseCentreRow(layout, scrollTop, clientHeight, edgePadding) {
  if (!layout || layout.visibleIndices.length === 0) return -1;
  const centre = (Number.isFinite(scrollTop) ? scrollTop : 0)
    + (Number.isFinite(clientHeight) ? clientHeight : 0) / 2
    - (Number.isFinite(edgePadding) ? edgePadding : 0);
  return findLyricsRowAtOffset(layout, centre);
}

/**
 * The row that shows the play button: the centre row while browsing, unless it is the current line.
 * @param {{ browsing: boolean, centreIndex: number, currentIndex: number }} input
 */
export function browseButtonIndex({ browsing, centreIndex, currentIndex }) {
  if (!browsing || !Number.isInteger(centreIndex) || centreIndex < 0) return -1;
  return centreIndex === currentIndex ? -1 : centreIndex;
}

/**
 * Where a seek to a line lands: the moment it becomes the current line (its start shifted by the display offset,
 * as `findActiveLyricIndex` decides it; no cue lead), rounded up so the line is already current there.
 * @param {number} startMs @param {number} offsetMs
 */
export function browseSeekPositionMs(startMs, offsetMs) {
  if (!Number.isFinite(startMs)) return 0;
  return Math.max(0, Math.ceil(startMs - (Number.isFinite(offsetMs) ? offsetMs : 0)));
}

/**
 * What browsing does when the current line changes or a scroll ends: `keep` (nothing to end), `defer` (the user is
 * still scrolling; pull back when the scroll ends) or `return` (end browsing and scroll back to the current line).
 * @param {{ browsing: boolean, cueChanged: boolean, pendingReturn: boolean, userActive: boolean }} input
 * @returns {'none' | 'keep' | 'defer' | 'return'}
 */
export function browseReturnAction({ browsing, cueChanged, pendingReturn, userActive }) {
  if (!browsing) return 'none';
  if (!cueChanged && !pendingReturn) return 'keep';
  return userActive ? 'defer' : 'return';
}
