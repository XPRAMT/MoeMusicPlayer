/** Build quick-jump headings for albums with numbered discs (2 and above). */
/**
 * @param {Array<{ discNumber: number, count: number }>} discCounts
 * @param {number} unknownCount
 * @param {'asc' | 'desc'} [direction='asc']
 */
export function buildAlbumDiscQuickIndex(discCounts, unknownCount, direction = 'asc') {
  const discs = [...discCounts]
    .filter(({ discNumber, count }) => Number.isInteger(discNumber) && discNumber >= 2 && count > 0)
    .sort((left, right) => direction === 'desc'
      ? right.discNumber - left.discNumber
      : left.discNumber - right.discNumber);

  if (discs.length === 0) return { labels: [], groups: [] };

  const groups = [];
  let beforeIndex = 0;
  for (const { discNumber, count } of discs) {
    groups.push({ label: `DISC ${discNumber}`, beforeIndex });
    beforeIndex += count;
  }
  if (unknownCount > 0) groups.push({ label: '未知', beforeIndex });

  return { labels: groups.map(({ label }) => label), groups };
}
