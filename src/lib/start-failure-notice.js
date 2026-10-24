/**
 * Notices for a track that could not be started (ADR 0036, 0026).
 *
 * A different track starts only after it has been opened; when it cannot be, nothing changes (the music that plays goes
 * on) and the user is told by an auto-dismissing notice: the server could not be reached, or the song cannot be played.
 * The texts are the stable markers of Rust (`REMOTE_LINK_LOST`, `REMOTE_UNREACHABLE`, `TRACK_UNPLAYABLE`,
 * `LOAD_SUPERSEDED` in `player-core`; `tests/start-failure-notice.test.mjs` checks they have not drifted).
 */

/** Two reports of the same failed attempt (the command's error and the snapshot's notice) are one notice. */
export const START_NOTICE_DEDUPE_MS = 2000;

/** How long the notice stays on screen. */
export const START_NOTICE_VISIBLE_MS = 4000;

const UNREACHABLE_MARKERS = ['伺服器無法連線', '無法連線到伺服器'];
const UNPLAYABLE_MARKER = '無法播放這首歌';

/**
 * @param {unknown} message
 * @returns {boolean}
 */
export function isRemoteUnreachableMessage(message) {
  return typeof message === 'string' && UNREACHABLE_MARKERS.some((marker) => message.includes(marker));
}

/**
 * What a rejected playback command was about.
 * @param {unknown} message
 * @returns {'unreachable' | 'unplayable' | 'superseded' | null} `null` for any other error (shown as an error).
 */
export function classifyStartError(message) {
  if (typeof message !== 'string') return null;
  if (isRemoteUnreachableMessage(message)) return 'unreachable';
  if (message.includes(UNPLAYABLE_MARKER)) return 'unplayable';
  // A start that a newer choice of track replaced: nothing shows it.
  if (message === 'superseded' || message.includes('load superseded')) return 'superseded';
  return null;
}

/**
 * Whether a snapshot's `lastError` is a new unreachable-server failure (the error text changed to one): the stream of
 * the playing remote track gave up.
 * @param {string | null | undefined} previousError
 * @param {string | null | undefined} nextError
 * @returns {boolean}
 */
export function snapshotReportsNewUnreachable(previousError, nextError) {
  return isRemoteUnreachableMessage(nextError) && previousError !== nextError;
}

/**
 * The failure a snapshot reports that was not seen before. The backend numbers the failed starts (`startFailure.sequence`
 * grows with each one), whichever way the start was asked for: a tap, a media key, the end of the previous track.
 * `lastSequence` is `null` before the first snapshot, which only sets the baseline (a reloaded page does not replay an
 * old failure).
 * @param {number | null} lastSequence
 * @param {{ kind?: string, sequence?: number } | null | undefined} notice
 * @returns {{ sequence: number, kind: 'unreachable' | 'unplayable' | null }} the sequence to remember, and the kind to
 *   tell the user about (`null`: nothing new)
 */
export function readStartFailure(lastSequence, notice) {
  const sequence = typeof notice?.sequence === 'number' ? notice.sequence : (lastSequence ?? 0);
  if (lastSequence === null) return { sequence, kind: null };
  if (!notice || sequence <= lastSequence) return { sequence: Math.max(sequence, lastSequence), kind: null };
  const kind = notice.kind === 'unreachable' || notice.kind === 'unplayable' ? notice.kind : null;
  return { sequence, kind };
}

/**
 * Decide whether a failure shows the notice now. `lastShownMs` is when the notice was last shown (or null).
 * @param {number | null} lastShownMs
 * @param {number} nowMs
 * @returns {boolean}
 */
export function shouldShowStartNotice(lastShownMs, nowMs) {
  return lastShownMs === null || nowMs - lastShownMs >= START_NOTICE_DEDUPE_MS;
}
