/**
 * Regenerate tests/fixtures/zh-convert-opencc-js.json, the golden table recorded from the
 * opencc-js 1.4.2 conversion that the app used before ADR 0030 moved conversion to Rust.
 * The old frontend rules are inlined here (keep-只 overrides, Japanese-kana guard) so the
 * table stays reproducible after the runtime files were removed. opencc-js is a devDependency.
 *
 * Usage: node scripts/generate-zh-convert-fixture.mjs
 */
import { readFileSync, writeFileSync } from 'node:fs';
import * as core from 'opencc-js/core';
import stPhrases from 'opencc-js/dict/STPhrases';
import stCharacters from 'opencc-js/dict/STCharacters';
import twVariants from 'opencc-js/dict/TWVariants';
import tsPhrases from 'opencc-js/dict/TSPhrases';
import tsCharacters from 'opencc-js/dict/TSCharacters';

const fixtureUrl = new URL('../tests/fixtures/zh-convert-opencc-js.json', import.meta.url);
const KANA = /[\p{Script=Hiragana}\p{Script=Katakana}]/u;
const HAN = /\p{Script=Han}/u;

function buildKeepZhiOverrides(phrasesDict) {
  const overrides = [];
  for (const entry of phrasesDict.split('|')) {
    const [key, value] = entry.split(' ');
    if (!key || !value || !key.includes('只') || !value.includes('隻')) continue;
    const keyChars = Array.from(key);
    const valueChars = Array.from(value);
    if (keyChars.length !== valueChars.length) continue;
    overrides.push([key, valueChars.map((ch, i) => (keyChars[i] === '只' && ch === '隻' ? '只' : ch)).join('')]);
  }
  return overrides;
}

const keepZhi = buildKeepZhiOverrides(stPhrases);
const lyricsS2t = core.ConverterFactory([keepZhi, stPhrases, stCharacters], [twVariants]);
const searchS2t = core.ConverterFactory([keepZhi, stPhrases, stCharacters], []);
const searchT2s = core.ConverterFactory([tsPhrases, tsCharacters], []);

/** Old lyric display: whole line unchanged when it contains kana. */
const lyricLine = (text) => (KANA.test(text) ? text : lyricsS2t(text));
/** Old search helpers: unchanged without Han or with kana. */
const convertible = (text) => HAN.test(text) && !KANA.test(text);
const searchTraditional = (text) => (convertible(text) ? searchS2t(text) : text);
const searchSimplified = (text) => (convertible(text) ? searchT2s(text) : text);

const previous = JSON.parse(readFileSync(fixtureUrl, 'utf8'));
const cases = previous.cases.map(({ category, input }) => ({
  category,
  input,
  lyricsTraditional: lyricLine(input),
  searchTraditional: searchTraditional(input),
  searchSimplified: searchSimplified(input),
}));

writeFileSync(fixtureUrl, `${JSON.stringify({ ...previous, cases }, null, 2)}\n`);
console.log(`wrote ${cases.length} cases`);
