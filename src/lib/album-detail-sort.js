const ALBUM_DETAIL_SORT_FIELDS = new Set([
  'position', 'title', 'artist', 'album', 'year', 'createdAt', 'modifiedAt', 'audioFormat', 'duration', 'playCount',
]);

export const DEFAULT_ALBUM_DETAIL_SORT = Object.freeze({ field: 'position', direction: 'asc' });

/** @param {unknown} value @returns {import('./ipc').AlbumDetailSort} */
export function normalizeAlbumDetailSort(value) {
  if (!value || typeof value !== 'object') return { ...DEFAULT_ALBUM_DETAIL_SORT };
  const record = /** @type {Record<string, unknown>} */ (value);
  return {
    field: ALBUM_DETAIL_SORT_FIELDS.has(/** @type {string} */ (record.field))
      ? /** @type {import('./ipc').AlbumDetailSortField} */ (record.field)
      : DEFAULT_ALBUM_DETAIL_SORT.field,
    direction: record.direction === 'desc' ? 'desc' : 'asc',
  };
}

/** @param {ReturnType<typeof normalizeAlbumDetailSort>} current @param {import('./ipc').AlbumDetailSortField} field @returns {import('./ipc').AlbumDetailSort} */
export function toggleAlbumDetailSort(current, field) {
  const normalized = normalizeAlbumDetailSort(current);
  return {
    field,
    direction: normalized.field === field && normalized.direction === 'asc' ? 'desc' : 'asc',
  };
}
