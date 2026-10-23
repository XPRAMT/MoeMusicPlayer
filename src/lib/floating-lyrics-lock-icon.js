export const FLOATING_LYRICS_LOCK_CLOSED_PATH = 'M8 11V8a4 4 0 0 1 8 0v3';
export const FLOATING_LYRICS_LOCK_OPEN_PATH = 'M8 11V8a4 4 0 0 1 7.6-1.7';

/** The floating lyrics lock glyph reflects the current lock state. */
/** @param {boolean} locked */
export function floatingLyricsLockPath(locked) {
  return locked ? FLOATING_LYRICS_LOCK_CLOSED_PATH : FLOATING_LYRICS_LOCK_OPEN_PATH;
}
