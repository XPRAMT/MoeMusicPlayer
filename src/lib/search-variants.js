/**
 * Build deduplicated Simplified/Traditional variants for text search.
 * Japanese queries are returned unchanged so the Chinese dictionaries cannot rewrite kanji.
 */
/** @type {Promise<{toTraditional:(text:string)=>string,toSimplified:(text:string)=>string}> | null} */
let convertersPromise = null;

function loadConverters() {
  if (!convertersPromise) {
    convertersPromise = Promise.all([
      import('opencc-js/core'),
      import('opencc-js/dict/STPhrases'),
      import('opencc-js/dict/STCharacters'),
      import('opencc-js/dict/TSPhrases'),
      import('opencc-js/dict/TSCharacters'),
    ]).then(([core, stPhrases, stCharacters, tsPhrases, tsCharacters]) => ({
      toTraditional: core.ConverterFactory(
        [stPhrases.default, stCharacters.default],
        [],
      ),
      toSimplified: core.ConverterFactory(
        [tsPhrases.default, tsCharacters.default],
        [],
      ),
    }));
  }
  return convertersPromise;
}

const HAN = /\p{Script=Han}/u;
const JAPANESE_KANA = /[\p{Script=Hiragana}\p{Script=Katakana}]/u;

/**
 * Return the original query followed by any distinct Chinese-script variants.
 * @param {string | null | undefined} value
 * @returns {Promise<string[]>}
 */
export async function getSearchVariants(value) {
  const query = typeof value === 'string' ? value.trim() : '';
  if (!query) return [];
  if (!HAN.test(query) || JAPANESE_KANA.test(query)) return [query];

  const converters = await loadConverters();
  const simplified = converters.toSimplified(query).trim();
  const alternate = simplified === query
    ? converters.toTraditional(query).trim()
    : simplified;
  return [...new Set([
    query,
    alternate,
  ].filter(Boolean))];
}
