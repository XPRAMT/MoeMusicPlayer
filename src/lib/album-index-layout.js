/** @param {{ sortField: string, groupingEnabled: boolean }} options */
export function shouldGroupAlbumIndex({ sortField, groupingEnabled }) {
  return groupingEnabled && sortField === 'title';
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
