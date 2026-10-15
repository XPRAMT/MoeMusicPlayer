/**
 * In-memory Simplified-to-Traditional conversion for the lyric lines of one song.
 * Dictionaries: OpenCC STPhrases, STCharacters, and TWVariants (Taiwan glyphs).
 * TWPhrases is not loaded, so regional wording such as 软件→軟體 is not rewritten.
 *
 * opencc-js is MIT. The dictionaries come from OpenCC data and are Apache-2.0.
 * License texts: licenses/opencc-js-MIT.txt and licenses/opencc-data-Apache-2.0.txt.
 */

/** @type {Promise<(text: string) => string> | null} */
let converterPromise = null;
const JAPANESE_KANA = /[\p{Script=Hiragana}\p{Script=Katakana}]/u;

function loadConverter() {
  if (!converterPromise) {
    converterPromise = Promise.all([
      import('opencc-js/core'),
      import('opencc-js/dict/STPhrases'),
      import('opencc-js/dict/STCharacters'),
      import('opencc-js/dict/TWVariants'),
    ]).then(([core, phrases, characters, variants]) => core.ConverterFactory(
      // opencc-js loads a group in reverse, so earlier dictionaries win on equal keys: the 只
      // overrides go first and replace only the matching STPhrases entries.
      [buildKeepZhiOverrides(phrases.default), phrases.default, characters.default],
      [variants.default],
    ));
  }
  return converterPromise;
}

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

/** @param {(text: string) => string} convert @param {string | null | undefined} value */
function convertLyricLine(convert, value) {
  if (typeof value !== 'string' || value.length === 0) return value ?? null;
  // OpenCC's Chinese dictionaries can rewrite kanji that are also used in
  // Japanese. Keep the whole line intact whenever it contains Japanese kana.
  if (JAPANESE_KANA.test(value)) return value;
  return convert(value);
}

/**
 * Convert every lyric line of the current song. Romanization is left unchanged.
 * The result is a display copy; callers must not write it back to storage.
 * @param {import('./ipc').TrackLyrics} lyrics
 * @returns {Promise<import('./ipc').TrackLyrics>}
 */
export async function convertTrackLyrics(lyrics) {
  const convert = await loadConverter();
  return {
    ...lyrics,
    lines: lyrics.lines.map((line) => ({
      ...line,
      text: /** @type {string} */ (convertLyricLine(convert, line.text) ?? ''),
      translation: /** @type {string | null} */ (convertLyricLine(convert, line.translation)),
    })),
  };
}
