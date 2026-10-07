/** Separators for multiple names stored in one ARTIST tag: `| \ / ; ,` and space. */
const ARTIST_SEPARATOR = /[|\\/;, ]+/;

/**
 * Split one ARTIST tag into individual names, in order, without empty or repeated entries.
 * @param {string | null | undefined} value
 * @returns {string[]}
 */
export function splitArtists(value) {
  if (typeof value !== 'string') return [];
  /** @type {string[]} */
  const artists = [];
  const seen = new Set();
  for (const part of value.split(ARTIST_SEPARATOR)) {
    const name = part.trim();
    if (!name || seen.has(name)) continue;
    seen.add(name);
    artists.push(name);
  }
  return artists;
}
