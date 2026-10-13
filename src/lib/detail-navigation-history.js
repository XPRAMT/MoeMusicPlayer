const DETAIL_NAVIGATION_STATE_KEY = '__moeDetailNavigation';

const VALID_VIEWS = new Set([
  'library',
  'artists',
  'artist-detail',
  'albums',
  'album-detail',
  'playlists',
  'queue',
  'settings',
]);

/** @typedef {'library' | 'artists' | 'artist-detail' | 'albums' | 'album-detail' | 'playlists' | 'queue' | 'settings'} DetailNavigationView */
/** @typedef {{ view: DetailNavigationView, artistKey?: string, albumKey?: string, preserveIndex?: boolean }} DetailNavigationRoute */

/**
 * Keep host state fields intact while adding a small, serializable app route.
 * History.state may be null, an object, or a primitive supplied by a host.
 * @param {unknown} currentState
 * @param {DetailNavigationRoute} route
 */
export function withDetailNavigationRoute(currentState, route) {
  const base = currentState && typeof currentState === 'object' && !Array.isArray(currentState)
    ? currentState
    : {};
  return {
    ...base,
    [DETAIL_NAVIGATION_STATE_KEY]: route,
  };
}

/** @param {unknown} state @returns {DetailNavigationRoute | null} */
export function readDetailNavigationRoute(state) {
  if (!state || typeof state !== 'object' || Array.isArray(state)) return null;
  const route = /** @type {Record<string, unknown>} */ (state)[DETAIL_NAVIGATION_STATE_KEY];
  if (!route || typeof route !== 'object' || Array.isArray(route)) return null;
  const candidate = /** @type {Record<string, unknown>} */ (route);
  if (typeof candidate.view !== 'string' || !VALID_VIEWS.has(candidate.view)) return null;
  if (candidate.view === 'artist-detail' && typeof candidate.artistKey !== 'string') return null;
  if (candidate.view === 'album-detail' && typeof candidate.albumKey !== 'string') return null;
  return /** @type {DetailNavigationRoute} */ ({
    view: candidate.view,
    ...(typeof candidate.artistKey === 'string' ? { artistKey: candidate.artistKey } : {}),
    ...(typeof candidate.albumKey === 'string' ? { albumKey: candidate.albumKey } : {}),
    ...(typeof candidate.preserveIndex === 'boolean' ? { preserveIndex: candidate.preserveIndex } : {}),
  });
}
