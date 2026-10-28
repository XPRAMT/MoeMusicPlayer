/**
 * Remembering the page the app was on and opening it again at the next start (ADR 0053).
 *
 * The Rust side stores the value in settings.json (`lastPage`, `LastPageSettings` in settings.rs); everything here
 * is a pure function so the rules can be tested: reading a stored value, writing the current state, the fallback
 * when the target is gone, launch intents that win over the memory, and the debounce.
 */

/** Same list as `LAST_PAGE_VIEWS` in src-tauri/src/settings.rs (tests/start-page.test.mjs compares them). */
export const START_PAGE_VIEWS = Object.freeze([
  'library',
  'artists',
  'artist-detail',
  'albums',
  'album-detail',
  'playlists',
  'queue',
  'settings',
]);

/** Same list as `LAST_PAGE_SETTINGS_SECTIONS` in settings.rs. */
export const START_PAGE_SETTINGS_SECTIONS = Object.freeze([
  'appearance',
  'track-columns',
  'now-playing',
  'lyrics',
  'playback',
  'shortcuts',
  'sources',
  'about',
]);

/** The page a fresh install (and every fallback) starts on. Same values as `LastPageSettings::default()`. */
export const DEFAULT_START_PAGE = Object.freeze(/** @type {StartPage} */ ({
  view: 'library',
  playlistId: null,
  settingsSection: null,
  artistKey: null,
  albumKey: null,
  nowPlaying: false,
}));

/** Quiet time after the last navigation before it is written to settings.json. */
export const START_PAGE_SAVE_DELAY_MS = 400;
/** How long a remembered Now Playing page waits for the playback snapshot to name a current track. */
export const START_PAGE_NOW_PLAYING_WAIT_MS = 5000;

const KEY_MAX_CHARS = 1024;

/** @typedef {{ view: string, playlistId: string | null, settingsSection: string | null, artistKey: string | null, albumKey: string | null, nowPlaying: boolean }} StartPage */

/** @param {unknown} value @returns {string | null} */
function key(value) {
  return typeof value === 'string' && value.length > 0 && value.length <= KEY_MAX_CHARS ? value : null;
}

/**
 * Reads any stored value: an unknown view or a view whose target is missing is the default page. Only the
 * fields the view needs are kept. Mirrors `LastPageSettings::normalized` in Rust.
 * @param {unknown} raw
 * @returns {StartPage}
 */
export function normalizeStartPage(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return { ...DEFAULT_START_PAGE };
  const source = /** @type {Record<string, unknown>} */ (raw);
  const view = source.view;
  if (typeof view !== 'string' || !START_PAGE_VIEWS.includes(view)) return { ...DEFAULT_START_PAGE };
  /** @type {StartPage} */
  const page = { ...DEFAULT_START_PAGE, view, nowPlaying: source.nowPlaying === true };
  if (view === 'playlists') page.playlistId = key(source.playlistId);
  else if (view === 'artist-detail') page.artistKey = key(source.artistKey);
  else if (view === 'album-detail') page.albumKey = key(source.albumKey);
  else if (view === 'settings') {
    const section = source.settingsSection;
    page.settingsSection =
      typeof section === 'string' && START_PAGE_SETTINGS_SECTIONS.includes(section) ? section : null;
  }
  const missingTarget =
    (view === 'playlists' && !page.playlistId) ||
    (view === 'artist-detail' && !page.artistKey) ||
    (view === 'album-detail' && !page.albumKey);
  return missingTarget ? { ...DEFAULT_START_PAGE } : page;
}

/**
 * The value to write for the current navigation state. The page underneath the Now Playing overlay is the page
 * (`activeView` keeps it while the overlay is open); the flag says whether the overlay was open.
 * @param {{ activeView: string, selectedPlaylistId?: string | null, settingsSection?: string | null,
 *   artistKey?: string | null, albumKey?: string | null, nowPlayingOpen?: boolean }} state
 * @returns {StartPage}
 */
export function startPageFromState(state) {
  return normalizeStartPage({
    view: state.activeView,
    playlistId: state.selectedPlaylistId,
    settingsSection: state.settingsSection,
    artistKey: state.artistKey,
    albumKey: state.albumKey,
    nowPlaying: state.nowPlayingOpen === true,
  });
}

/** @param {StartPage} a @param {StartPage} b */
export function startPagesEqual(a, b) {
  return (
    a.view === b.view &&
    a.playlistId === b.playlistId &&
    a.settingsSection === b.settingsSection &&
    a.artistKey === b.artistKey &&
    a.albumKey === b.albumKey &&
    a.nowPlaying === b.nowPlaying
  );
}

/**
 * Decides whether the remembered page is opened at this start. An explicit launch intent (the program was started
 * to play a file) wins over the memory; a stored default page needs no restoring.
 * @param {unknown} stored
 * @param {{ externalLaunch?: boolean }} [launch]
 * @returns {{ restore: boolean, page: StartPage }}
 */
export function planStartRestore(stored, launch = {}) {
  const page = normalizeStartPage(stored);
  if (launch.externalLaunch === true) return { restore: false, page };
  return { restore: !startPagesEqual(page, DEFAULT_START_PAGE), page };
}

/**
 * Pages that need no lookup open at once; the others (a playlist, an artist, an album) wait for their target to be
 * checked, so that nothing flashes and a missing target never shows.
 * @param {StartPage} page
 * @returns {'direct' | 'playlist' | 'artist' | 'album'}
 */
export function startPageKind(page) {
  if (page.view === 'playlists') return 'playlist';
  if (page.view === 'artist-detail') return 'artist';
  if (page.view === 'album-detail') return 'album';
  return 'direct';
}

/**
 * After the lookup: the page itself when its target still exists, otherwise the default start page (silently,
 * including the Now Playing flag).
 * @param {StartPage} page
 * @param {{ playlistIds?: readonly string[], artistFound?: boolean, albumFound?: boolean }} found
 * @returns {StartPage}
 */
export function resolveStartPage(page, found) {
  const kind = startPageKind(page);
  if (kind === 'playlist' && !(found.playlistIds ?? []).includes(page.playlistId ?? '')) return { ...DEFAULT_START_PAGE };
  if (kind === 'artist' && found.artistFound !== true) return { ...DEFAULT_START_PAGE };
  if (kind === 'album' && found.albumFound !== true) return { ...DEFAULT_START_PAGE };
  return page;
}

/**
 * A late restore (after a lookup) must not undo a navigation the user made in the meantime.
 * @param {string} currentView @param {string} viewAtStart
 */
export function lateRestoreStillWanted(currentView, viewAtStart) {
  return currentView === viewAtStart;
}

/**
 * The remembered Now Playing page opens once the playback snapshot names a current track (the session may still be
 * restoring when the first snapshot arrives, so it waits); it is dropped when the wait ran out.
 * @param {{ hasCurrentTrack: boolean, timedOut: boolean }} state
 * @returns {'wait' | 'open' | 'drop'}
 */
export function nowPlayingRestoreAction({ hasCurrentTrack, timedOut }) {
  if (hasCurrentTrack) return 'open';
  return timedOut ? 'drop' : 'wait';
}

/**
 * Debounced writer. `schedule` keeps only the latest page and writes it after `delayMs` of quiet; `flush` writes
 * a pending page at once (hidden page, closing); an unchanged page is never written. `baseline` is what is already
 * stored.
 * @param {{ delayMs: number, save: (page: StartPage) => unknown, baseline?: StartPage,
 *   setTimer?: (callback: () => void, delayMs: number) => unknown, clearTimer?: (handle: unknown) => void }} options
 */
export function createStartPageSaver({
  delayMs,
  save,
  baseline = DEFAULT_START_PAGE,
  setTimer = (callback, ms) => setTimeout(callback, ms),
  clearTimer = (handle) => clearTimeout(/** @type {any} */ (handle)),
}) {
  /** @type {StartPage} */
  let saved = { ...baseline };
  /** @type {StartPage | null} */
  let pending = null;
  /** @type {unknown} */
  let timer = null;

  function stop() {
    if (timer !== null) clearTimer(timer);
    timer = null;
  }
  function write() {
    stop();
    const page = pending;
    pending = null;
    if (!page || startPagesEqual(page, saved)) return false;
    saved = page;
    save(page);
    return true;
  }
  return {
    /** @param {StartPage} page */
    schedule(page) {
      if (startPagesEqual(page, pending ?? saved)) return;
      pending = page;
      stop();
      timer = setTimer(write, delayMs);
    },
    flush() {
      return write();
    },
    cancel() {
      stop();
      pending = null;
    },
    /** @param {StartPage} page */
    setBaseline(page) {
      saved = { ...page };
    },
    hasPending() {
      return pending !== null;
    },
  };
}
