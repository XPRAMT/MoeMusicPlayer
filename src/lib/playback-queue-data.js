/** @typedef {import('./ipc').PlaybackQueuePage} PlaybackQueuePage */
/** @typedef {import('./ipc').PlaybackQueuePageItem} PlaybackQueuePageItem */

/**
 * Adapt the playback queue IPC shape to the shared virtual paging controller.
 * @param {PlaybackQueuePage} page
 * @param {{ offset: number, limit: number }} request
 * @returns {{ items: PlaybackQueuePageItem[], offset: number, limit: number, totalCount: number }}
 */
export function adaptPlaybackQueuePage(page, request) {
  const totalCount = Number(page?.total);
  if (!Number.isSafeInteger(totalCount) || totalCount < 0) {
    throw new Error('播放佇列項目數量超出支援範圍。');
  }
  if (
    !page
    || !Array.isArray(page.items)
    || page.offset !== request.offset
    || !Number.isSafeInteger(page.offset)
    || page.offset < 0
    || page.items.length > request.limit
    || page.offset + page.items.length > totalCount
    || (page.items.length === 0 && page.offset < totalCount)
  ) {
    throw new Error('播放佇列分頁資料格式不符，請重新載入。');
  }
  return {
    items: page.items,
    offset: page.offset,
    limit: request.limit,
    totalCount,
  };
}

/** @param {PlaybackQueuePage} page */
export function playbackQueueCurrentEntryPosition(page) {
  if (page?.currentEntryPosition === null || page?.currentEntryPosition === undefined) return null;
  const position = Number(page.currentEntryPosition);
  if (!Number.isSafeInteger(position) || position < 0) {
    throw new Error('播放佇列目前項目位置超出支援範圍。');
  }
  return position;
}

/**
 * Entry position identifies repeated queue slots; track ID does not.
 * @param {PlaybackQueuePageItem} item
 * @param {number | null} currentEntryPosition
 */
export function isCurrentPlaybackQueueEntry(item, currentEntryPosition) {
  return currentEntryPosition === null
    ? item.isCurrent
    : item.entryPosition === currentEntryPosition;
}
