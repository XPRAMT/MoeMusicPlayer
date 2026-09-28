/** @typedef {import('./ipc').LyricsTrackResult} LyricsTrackResult */
/** @typedef {import('./ipc').TrackLyrics} TrackLyrics */

let requestSequence = 0;

/** @returns {string} */
function createRequestId() {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
  requestSequence += 1;
  return `lyrics-${Date.now().toString(36)}-${requestSequence.toString(36)}`;
}

/** @param {unknown} error */
function errorMessage(error) {
  if (error instanceof Error && error.message) return error.message;
  return typeof error === 'string' && error ? error : '歌詞服務目前無法使用。';
}

/**
 * @param {string | null} trackId
 * @param {LyricsTrackResult} result
 * @param {'load'|'search'|null} failedStage
 * @returns {LyricsControllerState}
 */
function stateFromResult(trackId, result, failedStage) {
  if (result.lyrics && result.lyrics.trackId !== trackId) {
    throw new Error('歌詞資料與目前播放曲目不一致。');
  }

  const candidates = Array.isArray(result.candidates) ? result.candidates : [];
  const phase = result.lyrics
    ? 'ready'
    : result.status === 'error'
      ? 'error'
      : candidates.length > 0 || result.status === 'candidates'
        ? 'candidates'
        : 'empty';

  return {
    trackId,
    phase,
    lyrics: result.lyrics ?? null,
    candidates,
    error: result.error,
    isSearching: false,
    selectingCandidateId: null,
    failedStage: phase === 'error' ? failedStage : null,
  };
}

/**
 * Owns one active track's local load and provider search lifecycle. The caller
 * should create one controller per mounted lyrics view and dispose it on exit.
 * @param {{
 *   api: {
 *     getTrack: (args: {trackId:string}) => Promise<LyricsTrackResult>,
 *     search: (args: {trackId:string, requestId:string}) => Promise<LyricsTrackResult>,
 *     selectCandidate: (args: {trackId:string, candidateId:string}) => Promise<TrackLyrics>,
 *     cancelSearch: (args: {requestId:string}) => Promise<void> | void
 *   },
 *   onChange: (state: LyricsControllerState) => void,
 *   createRequestId?: () => string
 * }} options
 */
export function createLyricsController({ api, onChange, createRequestId: makeRequestId = createRequestId }) {
  /** @type {LyricsControllerState} */
  let state = emptyState(null);
  /** @type {string | null} */
  let activeTrackId = null;
  /** @type {string | null} */
  let activeRequestId = null;
  let generation = 0;
  let disposed = false;

  function publish() {
    if (!disposed) onChange(state);
  }

  /** @param {LyricsControllerState} next */
  function update(next) {
    state = next;
    publish();
  }

  /** @param {string} requestId */
  function issueCancel(requestId) {
    try {
      void Promise.resolve(api.cancelSearch({ requestId })).catch(() => {});
    } catch {
      // Cancellation is best-effort; the generation fence still rejects late results.
    }
  }

  function cancelActiveRequest() {
    const requestId = activeRequestId;
    activeRequestId = null;
    if (requestId) issueCancel(requestId);
  }

  /** @param {string} trackId @param {number} requestGeneration */
  function isCurrent(trackId, requestGeneration) {
    return !disposed
      && activeTrackId === trackId
      && generation === requestGeneration;
  }

  /** @param {string} trackId @param {number} requestGeneration */
  async function startSearch(trackId, requestGeneration) {
    if (!isCurrent(trackId, requestGeneration)) return;

    const requestId = makeRequestId();
    activeRequestId = requestId;
    update({
      ...state,
      phase: 'searching',
      error: null,
      isSearching: true,
      failedStage: null,
    });

    try {
      const result = await api.search({ trackId, requestId });
      if (!isCurrent(trackId, requestGeneration) || activeRequestId !== requestId) return;
      activeRequestId = null;
      update(stateFromResult(trackId, result, 'search'));
    } catch (error) {
      if (!isCurrent(trackId, requestGeneration) || activeRequestId !== requestId) return;
      activeRequestId = null;
      update({
        ...state,
        phase: 'error',
        error: errorMessage(error),
        isSearching: false,
        failedStage: 'search',
      });
    }
  }

  /** @param {string} trackId @param {number} requestGeneration */
  async function loadTrack(trackId, requestGeneration) {
    try {
      const result = await api.getTrack({ trackId });
      if (!isCurrent(trackId, requestGeneration)) return;
      update(stateFromResult(trackId, result, 'load'));
      if (!result.lyrics && result.status !== 'error') {
        await startSearch(trackId, requestGeneration);
      }
    } catch (error) {
      if (!isCurrent(trackId, requestGeneration)) return;
      update({
        ...state,
        phase: 'error',
        error: errorMessage(error),
        isSearching: false,
        failedStage: 'load',
      });
    }
  }

  /** @param {string | null} trackId @returns {LyricsControllerState} */
  function emptyState(trackId) {
    return {
      trackId,
      phase: trackId ? 'loading' : 'idle',
      lyrics: null,
      candidates: [],
      error: null,
      isSearching: false,
      selectingCandidateId: null,
      failedStage: null,
    };
  }

  return {
    /** @returns {LyricsControllerState} */
    getState() {
      return state;
    },

    /** @param {string | null} trackId */
    async setTrack(trackId) {
      if (disposed || trackId === activeTrackId) return;
      generation += 1;
      activeTrackId = trackId;
      cancelActiveRequest();
      update(emptyState(trackId));
      if (trackId) await loadTrack(trackId, generation);
    },

    async retry() {
      if (disposed || !activeTrackId) return;
      if (state.failedStage === 'search') {
        await this.searchAgain();
        return;
      }
      generation += 1;
      cancelActiveRequest();
      const trackId = activeTrackId;
      update(emptyState(trackId));
      await loadTrack(trackId, generation);
    },

    async searchAgain() {
      if (disposed || !activeTrackId) return;
      generation += 1;
      cancelActiveRequest();
      const trackId = activeTrackId;
      update({
        ...state,
        phase: 'searching',
        error: null,
        isSearching: true,
        failedStage: null,
      });
      await startSearch(trackId, generation);
    },

    cancelSearch() {
      if (disposed || !activeRequestId) return;
      generation += 1;
      cancelActiveRequest();
      update({
        ...state,
        phase: state.candidates.length > 0 ? 'candidates' : 'empty',
        error: null,
        isSearching: false,
        failedStage: null,
      });
    },

    /** @param {string} candidateId */
    async selectCandidate(candidateId) {
      if (disposed || !activeTrackId || state.selectingCandidateId) return;
      generation += 1;
      cancelActiveRequest();
      const trackId = activeTrackId;
      const requestGeneration = generation;
      update({
        ...state,
        phase: 'loading',
        error: null,
        isSearching: false,
        selectingCandidateId: candidateId,
        failedStage: null,
      });

      try {
        const lyrics = await api.selectCandidate({ trackId, candidateId });
        if (!isCurrent(trackId, requestGeneration)) return;
        if (lyrics.trackId !== trackId) throw new Error('歌詞資料與目前播放曲目不一致。');
        update({
          trackId,
          phase: 'ready',
          lyrics,
          candidates: [],
          error: null,
          isSearching: false,
          selectingCandidateId: null,
          failedStage: null,
        });
      } catch (error) {
        if (!isCurrent(trackId, requestGeneration)) return;
        update({
          ...state,
          phase: 'error',
          error: errorMessage(error),
          isSearching: false,
          selectingCandidateId: null,
          failedStage: 'selection',
        });
      }
    },

    dispose() {
      if (disposed) return;
      generation += 1;
      disposed = true;
      cancelActiveRequest();
    },
  };
}

/** @typedef {{
 * trackId: string | null,
 * phase: 'idle'|'loading'|'ready'|'empty'|'candidates'|'searching'|'error',
 * lyrics: TrackLyrics | null,
 * candidates: import('./ipc').LyricsCandidate[],
 * error: string | null,
 * isSearching: boolean,
 * selectingCandidateId: string | null,
 * failedStage: 'load'|'search'|'selection'|null
 * }} LyricsControllerState
 */
