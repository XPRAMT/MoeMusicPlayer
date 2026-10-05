/** @typedef {{ trackId: string | null, basePlayedMs: number, lastPositionMs: number, sessionCreditedMs: number, isPlaying: boolean }} LivePlayCountState */
/** @typedef {{ trackId: string, playedMs: number }} LivePlayCountView */

/** @returns {LivePlayCountState} */
export function createLivePlayCountState() {
  return /** @type {LivePlayCountState} */ ({
    trackId: null,
    basePlayedMs: 0,
    lastPositionMs: 0,
    sessionCreditedMs: 0,
    isPlaying: false,
  });
}

/**
 * Natural position advances while Playing credit listening time.
 * Seek jumps (>2s), backward moves, and paused gaps do not credit.
 * @param {LivePlayCountState} state
 * @param {{ trackId: string | null, playedMs?: number | null, positionMs: number, isPlaying: boolean }} sample
 * @returns {LivePlayCountState}
 */
export function applyLivePlayCountSample(state, sample) {
  const trackId = sample.trackId ?? null;
  const playedMs = typeof sample.playedMs === 'number' && Number.isFinite(sample.playedMs) && sample.playedMs > 0
    ? sample.playedMs
    : 0;
  const positionMs = typeof sample.positionMs === 'number' && Number.isFinite(sample.positionMs)
    ? Math.max(0, sample.positionMs)
    : 0;
  const isPlaying = Boolean(sample.isPlaying);

  if (trackId == null) {
    return createLivePlayCountState();
  }

  if (state.trackId !== trackId) {
    return {
      trackId,
      basePlayedMs: playedMs,
      lastPositionMs: positionMs,
      sessionCreditedMs: 0,
      isPlaying,
    };
  }

  let basePlayedMs = state.basePlayedMs;
  let sessionCreditedMs = state.sessionCreditedMs;
  let rebasedFromPersisted = false;
  if (playedMs > basePlayedMs + sessionCreditedMs) {
    basePlayedMs = playedMs;
    sessionCreditedMs = 0;
    rebasedFromPersisted = true;
  } else if (playedMs > basePlayedMs) {
    const absorbed = playedMs - basePlayedMs;
    basePlayedMs = playedMs;
    sessionCreditedMs = Math.max(0, sessionCreditedMs - absorbed);
  }

  let nextSession = sessionCreditedMs;
  // Skip position credit on the same sample that absorbed a persisted catch-up,
  // so DB flush does not double-count the overlapping interval.
  if (!rebasedFromPersisted && isPlaying && state.isPlaying) {
    const delta = positionMs - state.lastPositionMs;
    if (delta > 0 && delta <= 2000) {
      nextSession += delta;
    }
  }

  return {
    trackId,
    basePlayedMs,
    lastPositionMs: positionMs,
    sessionCreditedMs: nextSession,
    isPlaying,
  };
}

/** @param {LivePlayCountState} state */
export function getLivePlayedMs(state) {
  if (!state.trackId) return null;
  return state.basePlayedMs + state.sessionCreditedMs;
}

/** @param {LivePlayCountState} state @returns {LivePlayCountView | null} */
export function toLivePlayCountView(state) {
  const playedMs = getLivePlayedMs(state);
  if (!state.trackId || playedMs == null) return null;
  return { trackId: state.trackId, playedMs };
}

/**
 * Overlay live playedMs onto a list/Now Playing item for the current track only.
 * @template {{ playedMs?: number | null, id?: string | null, trackId?: string | null }} T
 * @param {T} item
 * @param {LivePlayCountView | null | undefined} live
 * @returns {T}
 */
export function withLivePlayedMs(item, live) {
  if (!live || !item) return item;
  const id = item.id ?? item.trackId ?? null;
  if (!id || id !== live.trackId) return item;
  if (item.playedMs === live.playedMs) return item;
  return { ...item, playedMs: live.playedMs };
}
