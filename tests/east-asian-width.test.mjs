// The East Asian Width "wide" table (W or F) is generated once by scripts/generate-east-asian-width.mjs and written
// for Rust and Kotlin; the two copies must be the same ranges (ADR 0041 D10, terminal line estimate and line cap).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');
const numbers = (source) => Array.from(source.matchAll(/0x[0-9A-F]+/g), (m) => parseInt(m[0], 16));
const rust = read('crates/player-lyrics-rules/src/east_asian_width.rs');
const kotlin = read('plugins/tauri-plugin-media-index/android/src/main/java/com/moemusicplayer/mediaindex/EastAsianWidth.kt');
const rustTable = numbers(rust.slice(rust.indexOf('WIDE_RANGES'), rust.indexOf('];')));

test('the Rust and Kotlin wide tables are the same ranges', () => {
  const kotlinTable = kotlin.slice(kotlin.indexOf('intArrayOf('), kotlin.indexOf(')', kotlin.indexOf('intArrayOf(') + 11));
  assert.ok(rustTable.length > 100);
  assert.deepEqual(numbers(kotlinTable), rustTable);
  for (const source of [rust, kotlin]) {
    assert.match(source, /Unicode 17\.0\.0, https:\/\/www\.unicode\.org\/Public\/17\.0\.0\/ucd\/EastAsianWidth\.txt/);
  }
});

test('the wide table is sorted and disjoint', () => {
  const r = rustTable;
  assert.equal(r.length % 2, 0);
  for (let i = 0; i < r.length; i += 2) {
    assert.ok(r[i] <= r[i + 1]);
    if (i > 0) assert.ok(r[i] > r[i - 1] + 1, `ranges ${i / 2 - 1} and ${i / 2} touch or overlap`);
  }
});

test('no JS copy of the table is generated any more', () => {
  assert.doesNotMatch(read('scripts/generate-east-asian-width.mjs'), /east-asian-width\.js/);
});
