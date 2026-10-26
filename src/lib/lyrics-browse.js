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
 * The row that shows the play button: the centre row while browsing, unless it is the current line or its seek
 * target (`seekPositionMs`) is at or past the end of the track (a seek there fails); an unknown or zero duration
 * does not hide it.
 * @param {{ browsing: boolean, centreIndex: number, currentIndex: number, seekPositionMs?: number, durationMs?: number | null }} input
 */
export function browseButtonIndex({ browsing, centreIndex, currentIndex, seekPositionMs, durationMs }) {
  if (!browsing || !Number.isInteger(centreIndex) || centreIndex < 0) return -1;
  if (typeof durationMs === 'number' && Number.isFinite(durationMs) && durationMs > 0
    && typeof seekPositionMs === 'number' && seekPositionMs >= durationMs) return -1;
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

/** After the user's scroll ends, line changes do not pull the view back for this long (each new scroll restarts it). */
export const BROWSE_HOLD_MS = 3000;

/**
 * The hold after a scroll signal (`scrollend`, input release, the wheel guard ending): while the user is still
 * scrolling there is no hold yet (`null`); once idle it ends `BROWSE_HOLD_MS` after the first idle signal (later
 * signals of the same scroll do not extend it). A new scroll sets it back to `null`.
 * @param {number | null} holdUntil @param {{ userActive: boolean, now: number }} input
 * @returns {number | null}
 */
export function browseHoldAfterScroll(holdUntil, { userActive, now }) {
  if (userActive) return null;
  return holdUntil ?? now + BROWSE_HOLD_MS;
}

/**
 * What browsing does when the current line changes: `keep` the user's position, or `return` (end browsing and
 * scroll back to the current line). Only a line change after the hold has ended returns (a change inside the hold is
 * ignored, and the hold ending does not return by itself); a seek or track change (`jumped`) returns at once unless
 * the user is scrolling.
 * @param {{ browsing: boolean, cueChanged: boolean, jumped?: boolean, userActive: boolean, holdUntil: number | null, now: number }} input
 * @returns {'none' | 'keep' | 'return'}
 */
export function browseCueAction({ browsing, cueChanged, jumped = false, userActive, holdUntil, now }) {
  if (!browsing) return 'none';
  if (!cueChanged || userActive) return 'keep';
  if (jumped) return 'return';
  return holdUntil !== null && now >= holdUntil ? 'return' : 'keep';
}
