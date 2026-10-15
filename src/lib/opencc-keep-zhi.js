/** Shared OpenCC override for the user's keep-只 rule (lyrics and search Simplified→Traditional). */

/**
 * User rule: Simplified 只 is never converted (Traditional text uses 只 too), so 只 must not
 * become 隻. OpenCC itself converts phrase by phrase (STPhrases) with greedy longest-match
 * segmentation; measure-word phrases such as 这只／一只／船只 map 只 to 隻 and can swallow the
 * 只 of a following 只有 (这只有 → 這隻有). Build an override for every STPhrases entry whose
 * value turns 只 into 隻, restoring 只 at those positions and keeping the rest of the phrase
 * conversion (一只猫 → 一只貓, 只字不提 → 只字不提).
 * @param {string} phrasesDict OpenCC "key value|key value" dictionary text
 * @returns {Array<[string, string]>}
 */
export function buildKeepZhiOverrides(phrasesDict) {
  /** @type {Array<[string, string]>} */
  const overrides = [];
  for (const entry of phrasesDict.split('|')) {
    const [key, value] = entry.split(' ');
    if (!key || !value || !key.includes('只') || !value.includes('隻')) continue;
    const keyChars = Array.from(key);
    const valueChars = Array.from(value);
    if (keyChars.length !== valueChars.length) continue;
    const kept = valueChars.map((ch, index) => (keyChars[index] === '只' && ch === '隻' ? '只' : ch)).join('');
    overrides.push([key, kept]);
  }
  return overrides;
}
