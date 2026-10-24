/**
 * Which playback command wins when the user acts again before an earlier one has answered (ADR 0036).
 *
 * Choosing a track to play (from the library, an artist or album page, a playlist or the queue) is never dropped
 * because an earlier command is still in flight: the latest choice wins. A choice on a server that does not answer can
 * take seconds; the next tap on a local song must start at once. The older command is superseded: its late result or
 * error is ignored (no snapshot applied, no message), and it leaves the busy flag to the newer one. The controls'
 * own duplicate-click guard (play/pause, shuffle, repeat, previous/next, seek) is separate and unchanged.
 */

/**
 * Whether a click that chooses a track is accepted. A command that is merely in flight does not block it; a seek in
 * flight still does (ADR 0036: the transport ignores clicks while seeking).
 * @param {{ playbackReady: boolean, isSeeking: boolean }} state
 * @returns {boolean}
 */
export function acceptsTrackChoice({ playbackReady, isSeeking }) {
  return playbackReady && !isSeeking;
}

/**
 * Numbers the non-seek playback commands in the order they were sent.
 * @returns {{ begin: () => number, isLatest: (token: number) => boolean }}
 */
export function createPlaybackCommandOrder() {
  let latest = 0;
  return {
    /** A command is sent: its token. Every earlier token is superseded from now on. */
    begin: () => ++latest,
    /** Whether `token` is still the newest command (its result may be applied, its busy flag cleared). */
    isLatest: (token) => token === latest,
  };
}
