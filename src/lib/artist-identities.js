import { getTraditionalIdentity } from './search-variants.js';

/** @typedef {{ name: string, key: string, traditional: string }} ArtistIdentityMapping */

/** @param {string[]} names @returns {Promise<ArtistIdentityMapping[]>} */
export async function buildArtistIdentityMappings(names) {
  const unique = [...new Set(names.map((name) => name.trim()).filter(Boolean))];
  return Promise.all(unique.map(async (name) => {
    const traditional = await getTraditionalIdentity(name);
    return {
      name,
      key: traditional.normalize('NFKC').toLowerCase(),
      traditional,
    };
  }));
}

/** @param {{name:string,count:number}[]} names @param {Map<string,string>} traditionalByName */
export function preferredArtistName(names, traditionalByName) {
  if (names.length === 0) return '';
  return [...names].sort((left, right) => {
    if (left.count !== right.count) return right.count - left.count;
    const leftTraditional = (traditionalByName.get(left.name) ?? left.name) === left.name;
    const rightTraditional = (traditionalByName.get(right.name) ?? right.name) === right.name;
    if (leftTraditional !== rightTraditional) return leftTraditional ? -1 : 1;
    return left.name.localeCompare(right.name, 'zh-Hant');
  })[0].name;
}
