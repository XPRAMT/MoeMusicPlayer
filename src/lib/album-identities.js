import { getTraditionalIdentity } from './search-variants.js';
import { preferredArtistName } from './artist-identities.js';

const UNMARKED_ALBUM_KEY = 'album:unmarked';

/**
 * @typedef {{ name: string | null, trackCount: number }} AlbumIndexFacet
 * @typedef {{ key: string, displayName: string, aliases: string[], trackCount: number, unmarked: boolean }} AlbumEntry
 */

/** @param {string | null | undefined} value */
export function albumIdentityKey(value) {
  const name = typeof value === 'string' ? value.trim() : '';
  return name ? `album:${name.normalize('NFKC').toLowerCase()}` : UNMARKED_ALBUM_KEY;
}

/** @param {AlbumIndexFacet[]} facets @returns {Promise<AlbumEntry[]>} */
export async function buildAlbumEntries(facets) {
  const names = [...new Set(facets.map((facet) => facet.name?.trim() ?? '').filter(Boolean))];
  const traditionalNames = new Map(await Promise.all(names.map(async (name) => (
    /** @type {[string, string]} */ ([name, await getTraditionalIdentity(name)])
  ))));
  /** @type {Map<string, { aliases: Map<string, number>, queryAliases: Set<string>, trackCount: number, unmarked: boolean }>} */
  const groups = new Map();
  for (const facet of facets) {
    const original = typeof facet.name === 'string' ? facet.name : '';
    const raw = original.trim();
    const key = raw ? albumIdentityKey(traditionalNames.get(raw) ?? raw) : UNMARKED_ALBUM_KEY;
    let group = groups.get(key);
    if (!group) {
      group = { aliases: new Map(), queryAliases: new Set(), trackCount: 0, unmarked: !raw };
      groups.set(key, group);
    }
    group.trackCount += Math.max(0, Number(facet.trackCount) || 0);
    group.queryAliases.add(original);
    group.queryAliases.add(raw);
    if (raw) group.aliases.set(raw, (group.aliases.get(raw) ?? 0) + Math.max(0, Number(facet.trackCount) || 0));
  }

  return [...groups].map(([key, group]) => {
    const aliases = [...group.aliases].map(([name, count]) => ({ name, count }));
    const traditionalByName = new Map(aliases.map(({ name }) => [name, traditionalNames.get(name) ?? name]));
    return {
      key,
      displayName: group.unmarked ? '其它' : preferredArtistName(aliases, traditionalByName),
      aliases: [...group.queryAliases],
      trackCount: group.trackCount,
      unmarked: group.unmarked,
    };
  }).sort((left, right) => left.displayName.localeCompare(right.displayName, 'zh-Hant', { numeric: true, sensitivity: 'base' })
    || Number(left.unmarked) - Number(right.unmarked));
}

/** @param {string} name @param {AlbumEntry[]} entries @returns {Promise<AlbumEntry | null>} */
export async function findAlbumEntryByName(name, entries) {
  const normalized = await getTraditionalIdentity(name.trim());
  const key = albumIdentityKey(normalized);
  return entries.find((entry) => entry.key === key || entry.aliases.some((alias) => alias.normalize('NFKC').toLowerCase() === name.trim().normalize('NFKC').toLowerCase())) ?? null;
}
