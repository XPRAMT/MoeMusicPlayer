// The sorts whose lists are cut into groups (with a heading and a quick-find panel): the names by their first
// character, the release and added times by year, and the audio format by format name. The backend builds the groups
// (crates/player-db: `quick_index_for_tracks`, `quick_index_for_playlist`, the queue's ordered keys).

export const GROUPED_SORT_FIELDS = Object.freeze(['title', 'artist', 'album', 'year', 'createdAt', 'audioFormat']);

/** @param {unknown} field */
export function isGroupedSortField(field) {
  return typeof field === 'string' && GROUPED_SORT_FIELDS.includes(field);
}
