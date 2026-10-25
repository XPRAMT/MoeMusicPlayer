// The East Asian Width "wide" table (W or F) is generated once by scripts/generate-east-asian-width.mjs and written
// for Rust, JS and Kotlin; the three copies must be the same ranges (ADR 0041 D10, terminal line estimate).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { EAST_ASIAN_WIDE_RANGES, isEastAsianWide } from '../src/lib/east-asian-width.js';

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');
const numbers = (source) => Array.from(source.matchAll(/0x[0-9A-F]+/g), (m) => parseInt(m[0], 16));

test('the Rust, JS and Kotlin wide tables are the same ranges', () => {
  const rust = read('crates/player-lyrics-rules/src/east_asian_width.rs');
  const rustTable = rust.slice(rust.indexOf('WIDE_RANGES'), rust.indexOf('];'));
  const kotlin = read('plugins/tauri-plugin-media-index/android/src/main/java/com/moemusicplayer/mediaindex/EastAsianWidth.kt');
  const kotlinTable = kotlin.slice(kotlin.indexOf('intArrayOf('), kotlin.indexOf(')', kotlin.indexOf('intArrayOf(') + 11));
  assert.deepEqual(numbers(rustTable), [...EAST_ASIAN_WIDE_RANGES]);
  assert.deepEqual(numbers(kotlinTable), [...EAST_ASIAN_WIDE_RANGES]);
  for (const source of [rust, kotlin, read('src/lib/east-asian-width.js')]) {
    assert.match(source, /Unicode 17\.0\.0, https:\/\/www\.unicode\.org\/Public\/17\.0\.0\/ucd\/EastAsianWidth\.txt/);
  }
});

test('the wide table is sorted and disjoint', () => {
  const r = EAST_ASIAN_WIDE_RANGES;
  assert.equal(r.length % 2, 0);
  for (let i = 0; i < r.length; i += 2) {
    assert.ok(r[i] <= r[i + 1]);
    if (i > 0) assert.ok(r[i] > r[i - 1] + 1, `ranges ${i / 2 - 1} and ${i / 2} touch or overlap`);
  }
});

test('wide and narrow code points', () => {
  for (const ch of ['啦', 'あ', 'ア', '한', 'Ａ', '！', '\u3000', '🙂', '\u{20000}']) {
    assert.equal(isEastAsianWide(ch.codePointAt(0)), true, ch);
  }
  for (const ch of ['A', ' ', 'é', 'ｱ', '…', '“', '\u2764', '\u{1F1EF}', '\u0301', 'Ω']) {
    assert.equal(isEastAsianWide(ch.codePointAt(0)), false, ch);
  }
});
