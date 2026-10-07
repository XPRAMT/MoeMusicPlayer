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
 * @param {{ selectionMode?: boolean }} [options]
 * @returns {LyricsControllerState}
 */
function stateFromResult(trackId, result, failedStage, options = {}) {
  if (result.lyrics && result.lyrics.trackId !== trackId) {
    throw new Error('歌詞資料與目前播放曲目不一致。');
  }

  const candidates = Array.isArray(result.candidates) ? result.candidates : [];
  const selectionMode = Boolean(options.selectionMode);
  // Manual selection must stay on the candidate UI even when prior lyrics are preserved.
  const phase = candidates.length > 0 || result.status === 'candidates'
    ? (candidates.length > 0 ? 'candidates' : 'empty')
    : selectionMode && result.lyrics
      ? 'empty'
      : result.lyrics
        ? 'ready'
        : result.status === 'error'
          ? 'error'
          : 'empty';

  return {
    trackId,
    phase,
    lyrics: result.lyrics ?? null,
    candidates,
    error: result.error,
    isSearching: false,
    selectingCandidateId: null,
    selectionMode: selectionMode || phase === 'candidates' || (selectionMode && phase === 'empty'),
    failedStage: phase === 'error' ? failedStage : null,
    pickerClosed: false,
  };
}

/**
 * Owns one active track's local load and provider search lifecycle. The caller
 * should create one controller per mounted lyrics view and dispose it on exit.
 * @param {{
 *   api: {
 *     getTrack: (args: {trackId:string}) => Promise<LyricsTrackResult>,
 *     search: (args: {trackId:string, requestId:string, manual?:boolean, query?:string}) => Promise<LyricsTrackResult>,
 *     selectCandidate: (args: {trackId:string, candidateId:string}) => Promise<TrackLyrics>,
 *     clearTrack: (args: {trackId:string}) => Promise<LyricsTrackResult>,
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

  /** @param {string} trackId @param {number} requestGeneration @param {{manual?: boolean, query?: string}} [options] */
  async function startSearch(trackId, requestGeneration, options = {}) {
    if (!isCurrent(trackId, requestGeneration)) return;

    const manual = Boolean(options.manual);
    const query = typeof options.query === 'string' ? options.query.trim() : '';
    const requestId = makeRequestId();
    activeRequestId = requestId;
    update({
      ...state,
      phase: 'searching',
      error: null,
      isSearching: true,
      selectionMode: manual || state.selectionMode,
      failedStage: null,
    });

    try {
      /** @type {{trackId:string, requestId:string, manual?:boolean, query?:string}} */
      const args = { trackId, requestId };
      if (manual) args.manual = true;
      if (query) args.query = query;
      const result = await api.search(args);
      if (!isCurrent(trackId, requestGeneration) || activeRequestId !== requestId) return;
      activeRequestId = null;
      const next = stateFromResult(trackId, result, 'search', { selectionMode: manual || state.selectionMode });
      if (!next.lyrics && state.lyrics && (manual || state.selectionMode)) {
        next.lyrics = state.lyrics;
      }
      update(next);
    } catch (error) {
      if (!isCurrent(trackId, requestGeneration) || activeRequestId !== requestId) return;
      activeRequestId = null;
      update({
        ...state,
        phase: 'error',
        error: errorMessage(error),
        isSearching: false,
        selectionMode: manual || state.selectionMode,
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
      selectionMode: false,
      failedStage: null,
      pickerClosed: false,
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

    /**
     * @param {{ query?: string }} [options]
     */
    async searchAgain(options = {}) {
      if (disposed || !activeTrackId) return;
      generation += 1;
      cancelActiveRequest();
      const trackId = activeTrackId;
      const query = typeof options.query === 'string' ? options.query.trim() : '';
      update({
        ...state,
        phase: 'searching',
        error: null,
      isSearching: true,
      selectionMode: true,
      failedStage: null,
      pickerClosed: false,
    });
      await startSearch(trackId, generation, { manual: true, query: query || undefined });
    },

    cancelSearch() {
      if (disposed) return;
      if (activeRequestId) {
        generation += 1;
        cancelActiveRequest();
      }
      // Dismiss selection UI and restore playback lyrics when the user cancels.
      if (state.lyrics) {
        update({
          ...state,
          phase: 'ready',
          candidates: [],
          error: null,
          isSearching: false,
          selectionMode: false,
          failedStage: null,
        });
        return;
      }
      update({
        ...state,
        phase: state.candidates.length > 0 ? 'candidates' : 'empty',
        error: null,
        isSearching: false,
        selectionMode: state.candidates.length > 0,
        failedStage: null,
      });
    },


    async removeLyrics() {
      if (disposed || !activeTrackId) return;
      if (typeof api.clearTrack !== 'function') {
        update({
          ...state,
          error: '目前環境不支援移除歌詞。',
          failedStage: 'selection',
        });
        return;
      }
      generation += 1;
      cancelActiveRequest();
      const trackId = activeTrackId;
      const requestGeneration = generation;
      update({
        ...state,
        phase: 'loading',
        error: null,
        isSearching: false,
        selectingCandidateId: null,
        failedStage: null,
      });
      try {
        const result = await api.clearTrack({ trackId });
        if (!isCurrent(trackId, requestGeneration)) return;
        // Do not auto-search after an explicit remove; leave empty or rediscovered local/sidecar lyrics.
        update({
          ...stateFromResult(trackId, result, null, { selectionMode: false }),
          selectionMode: false,
          selectingCandidateId: null,
          isSearching: false,
          candidates: [],
          failedStage: null,
          pickerClosed: true,
        });
      } catch (error) {
        if (!isCurrent(trackId, requestGeneration)) return;
        update({
          ...state,
          phase: 'candidates',
          error: errorMessage(error),
          isSearching: false,
          selectionMode: true,
          selectingCandidateId: null,
          failedStage: 'selection',
        });
      }
    },

    dismissSelection() {
      if (disposed || !state.trackId) return;
      generation += 1;
      cancelActiveRequest();
      update({
        ...state,
        phase: state.lyrics ? 'ready' : 'empty',
        candidates: [],
        error: null,
        isSearching: false,
        selectionMode: false,
        selectingCandidateId: null,
        failedStage: null,
        pickerClosed: true,
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
          selectionMode: false,
          failedStage: null,
          pickerClosed: true,
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
 * selectionMode: boolean,
 * failedStage: 'load'|'search'|'selection'|null,
 * pickerClosed: boolean
 * }} LyricsControllerState
 */
