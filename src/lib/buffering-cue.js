// Buffering cue (ADR 0028): the progress bar thumb pulses for as long as the playback state is `buffering`, starting
// and stopping with the state (no delay). The cue is the pulse alone; no text changes.

/**
 * Whether the thumb should pulse for this playback snapshot state.
 * @param {string | null | undefined} state playback snapshot state
 * @returns {boolean}
 */
export function bufferingCueVisible(state) {
  return state === 'buffering';
}
