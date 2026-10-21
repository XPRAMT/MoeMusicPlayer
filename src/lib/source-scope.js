// The top-bar switch that narrows the library lists to local songs, remote songs or both.

/** @typedef {'local' | 'all' | 'remote'} SourceScope */
/** Left to right on the switch. @type {readonly SourceScope[]} */
export const SOURCE_SCOPES = Object.freeze(['local', 'all', 'remote']);

/** The switch only appears when there are both kinds of enabled source. @param {{ kind: string, enabled: boolean }[]} sources */
export function shouldShowSourceScopeSwitch(sources) {
  const enabled = sources.filter((source) => source.enabled);
  return enabled.some((source) => source.kind === 'remote') && enabled.some((source) => source.kind !== 'remote');
}

/** Which of the three stops a horizontal position over the track is closest to (0 left, 2 right). @param {number} x @param {number} width */
export function getScopeIndexFromPosition(x, width) {
  if (!(width > 0) || !Number.isFinite(x)) return 1;
  return Math.min(2, Math.max(0, Math.floor((x / width) * 3)));
}

/** A stored or returned value that is not one of the three means everything. @param {unknown} value @returns {SourceScope} */
export function normalizeSourceScope(value) {
  return value === 'local' || value === 'remote' ? value : 'all';
}
