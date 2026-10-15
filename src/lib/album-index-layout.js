/** @param {{ sortField: string, groupingEnabled: boolean }} options */
export function shouldGroupAlbumIndex({ sortField, groupingEnabled }) {
  return groupingEnabled && sortField === 'title';
}

/** Convert a screen-physical-pixel distance to CSS pixels. */
/** @param {number} physicalPixels @param {number} devicePixelRatio */
export function physicalPixelsToCssPixels(physicalPixels, devicePixelRatio) {
  const pixels = Number.isFinite(physicalPixels) ? Math.max(0, physicalPixels) : 0;
  const ratio = Number.isFinite(devicePixelRatio) && devicePixelRatio > 0 ? devicePixelRatio : 1;
  return pixels / ratio;
}

/** Card text follows the shared body/caption tokens; include the two CSS flex gaps. */
/** @param {number} bodyFontSize @param {number} captionFontSize */
export function getAlbumCardTextHeight(bodyFontSize, captionFontSize) {
  const body = Number.isFinite(bodyFontSize) && bodyFontSize > 0 ? bodyFontSize : 13;
  const caption = Number.isFinite(captionFontSize) && captionFontSize > 0 ? captionFontSize : 12;
  return 8 + body * 1.4 + caption * 1.4;
}

/**
 * Build the virtual row geometry for the responsive album-card grid.
 * Group headings are full-width rows; cards remain in fixed-size grid rows.
 * @param {{ totalCount: number, columns: number, cardRowHeight: number, groups?: Array<{ label: string, beforeIndex: number }>, groupHeadingHeight?: number, groupCardGap?: number }} options
 */
export function buildAlbumIndexRows({
  totalCount,
  columns,
  cardRowHeight,
  groups = [],
  groupHeadingHeight = 37,
  groupCardGap = 0,
}) {
  const count = Number.isSafeInteger(totalCount) ? Math.max(0, totalCount) : 0;
  const columnCount = Number.isSafeInteger(columns) ? Math.max(1, columns) : 1;
  const cardHeight = Number.isFinite(cardRowHeight) ? Math.max(1, cardRowHeight) : 1;
  const headingHeight = Number.isFinite(groupHeadingHeight) ? Math.max(1, groupHeadingHeight) : 1;
  const headingCardGap = Number.isFinite(groupCardGap) ? Math.max(0, groupCardGap) : 0;
  const normalizedGroups = groups
    .filter((group) => Number.isSafeInteger(group.beforeIndex) && group.beforeIndex >= 0 && group.beforeIndex < count)
    .slice()
    .sort((left, right) => left.beforeIndex - right.beforeIndex)
    .filter((group, index, all) => index === 0 || group.beforeIndex !== all[index - 1].beforeIndex);

  /** @type {Array<{kind:'group',label:string,beforeIndex:number,top:number,height:number}|{kind:'cards',start:number,end:number,top:number,height:number}>} */
  const rows = [];
  let top = 0;
  /** @param {number} start @param {number} end */
  const appendCards = (start, end) => {
    for (let index = start; index < end; index += columnCount) {
      /** @type {ReturnType<typeof buildAlbumIndexRows>['rows'][number]} */
      const row = { kind: 'cards', start: index, end: Math.min(end, index + columnCount), top, height: cardHeight };
      rows.push(row);
      top += cardHeight;
    }
  };

  if (normalizedGroups.length === 0) {
    appendCards(0, count);
  } else {
    let cursor = 0;
    for (let index = 0; index < normalizedGroups.length; index += 1) {
      const group = normalizedGroups[index];
      if (group.beforeIndex > cursor) appendCards(cursor, group.beforeIndex);
      const end = normalizedGroups[index + 1]?.beforeIndex ?? count;
      rows.push({ kind: 'group', label: group.label, beforeIndex: group.beforeIndex, top, height: headingHeight });
      top += headingHeight + headingCardGap;
      appendCards(group.beforeIndex, end);
      cursor = end;
    }
    if (cursor < count) appendCards(cursor, count);
  }

  return { rows, totalHeight: top };
}

/**
 * @param {ReturnType<typeof buildAlbumIndexRows>['rows']} rows
 * @param {number} scrollTop
 * @param {number} viewportHeight
 * @param {number} [overscan]
 */
export function findVisibleAlbumIndexRows(rows, scrollTop, viewportHeight, overscan = 0) {
  const start = Math.max(0, scrollTop - Math.max(0, overscan));
  const end = Math.max(start, scrollTop + Math.max(0, viewportHeight) + Math.max(0, overscan));
  let low = 0;
  let high = rows.length;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (rows[middle].top + rows[middle].height <= start) low = middle + 1;
    else high = middle;
  }
  const visible = [];
  for (let index = low; index < rows.length && rows[index].top < end; index += 1) visible.push(rows[index]);
  return visible;
}

/**
 * Materialize the non-reactive page-cache items used by the virtual Svelte grid.
 * Every snapshot revision creates fresh card-row item arrays, even when row
 * geometry and index ranges are unchanged, so newly fetched pages replace their
 * loading placeholders without requiring a scroll event.
 * @template {{ key: string }} Item
 * @param {Array<ReturnType<typeof buildAlbumIndexRows>['rows'][number]>} rows
 * @param {(index: number) => Item | null} itemAt
 * @param {number} revision
 * @param {number} refreshKey
 * @returns {Array<(ReturnType<typeof buildAlbumIndexRows>['rows'][number] & { revision:number, items?: Array<{index:number,album:Item|null,key:string|number}> })>}
 */
export function buildAlbumIndexRenderRows(rows, itemAt, revision, refreshKey = 0) {
  return rows.map((row) => {
    if (row.kind !== 'cards') return { ...row, revision };
    const items = Array.from({ length: row.end - row.start }, (_, offset) => {
      const index = row.start + offset;
      const album = itemAt(index);
      return {
        index,
        album,
        key: album?.key ?? `loading:${index}:${refreshKey}`,
      };
    });
    return { ...row, revision, items };
  });
}

export const DEFAULT_ALBUM_CARD_MAX_WIDTH_PX = 180;
export const MIN_ALBUM_CARD_MAX_WIDTH_PX = 120;
export const MAX_ALBUM_CARD_MAX_WIDTH_PX = 320;

/** @param {unknown} value @returns {number} */
export function normalizeAlbumCardMaxWidthPx(value) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return DEFAULT_ALBUM_CARD_MAX_WIDTH_PX;
  return Math.min(MAX_ALBUM_CARD_MAX_WIDTH_PX, Math.max(MIN_ALBUM_CARD_MAX_WIDTH_PX, Math.round(value)));
}

/**
 * Fill the available row width with album cards no wider than `maxCardWidth`.
 * Columns = ceil((available + gap) / (max + gap)), at least 1; the cards then
 * share the remaining width equally, so there is no trailing gap and each card
 * is at most `maxCardWidth` (narrower only when one card exceeds the row).
 * @param {number} availableWidth @param {number} maxCardWidth @param {number} gap
 * @returns {{ columns: number, cardWidth: number }}
 */
export function getAlbumCardGrid(availableWidth, maxCardWidth, gap) {
  const available = Number.isFinite(availableWidth) ? Math.max(0, availableWidth) : 0;
  const max = Number.isFinite(maxCardWidth) && maxCardWidth > 0 ? maxCardWidth : DEFAULT_ALBUM_CARD_MAX_WIDTH_PX;
  const spacing = Number.isFinite(gap) ? Math.max(0, gap) : 0;
  const columns = Math.max(1, Math.ceil((available + spacing) / (max + spacing) - 1e-9));
  const cardWidth = available > 0 ? Math.min(max, (available - spacing * (columns - 1)) / columns) : max;
  return { columns, cardWidth: Math.max(1, cardWidth) };
}
