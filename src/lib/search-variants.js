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

/**
 * Return the Traditional-script spelling used to build an identity key. Japanese names stay
 * untouched whenever they contain Hiragana or Katakana.
 * @param {string | null | undefined} value
 * @returns {Promise<string>}
 */
export async function getTraditionalIdentity(value) {
  const text = typeof value === 'string' ? value.trim() : '';
  if (!text || !HAN.test(text) || JAPANESE_KANA.test(text)) return text;
  const converters = await loadConverters();
  return converters.toTraditional(text).trim();
}

/**
 * Build an automatic title/artist query in Traditional-first, Simplified-second order.
 * Convert the fields independently so Japanese titles remain unchanged while a Chinese
 * artist field can still be searched in both scripts.
 * @param {string | null | undefined} title
 * @param {string | null | undefined} artist
 * @returns {Promise<string[]>}
 */
export async function getAutomaticSearchVariants(title, artist) {
  const fields = [title, artist]
    .map((value) => typeof value === 'string' ? value.trim() : '')
    .filter(Boolean);
  if (fields.length === 0) return [];

  /** @param {string} field */
  const shouldConvert = (field) => HAN.test(field) && !JAPANESE_KANA.test(field);
  if (!fields.some(shouldConvert)) return [fields.join(' ')];

  const converters = await loadConverters();
  const traditional = fields.map((field) => (
    shouldConvert(field) ? converters.toTraditional(field).trim() : field
  )).filter(Boolean).join(' ');
  const simplified = fields.map((field) => (
    shouldConvert(field) ? converters.toSimplified(field).trim() : field
  )).filter(Boolean).join(' ');
  return [...new Set([traditional, simplified].filter(Boolean))];
}
