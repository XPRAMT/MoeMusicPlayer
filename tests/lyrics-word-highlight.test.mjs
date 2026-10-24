import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { currentWordIndex, wordHighlightTimings } from '../src/lib/lyrics-word-highlight.js';
import { lyricsForDisplay } from '../src/lib/lyrics-traditional.js';

const words = [
  { s: 1000, e: 1300, t: '早' },
  { s: 1300, e: 1300, t: '' },
  { s: 1500, e: 2000, t: '安 ' },
];

test('each word animates from its own start for its own length, never led', () => {
  const timings = wordHighlightTimings(words, null, 1100, 0);
  assert.deepEqual(timings.map((t) => [t.text, t.delayMs, t.durationMs]), [
    ['早', -100, 300],
    ['', 200, 0],
    ['安 ', 400, 500],
  ]);
});

test('the lyric offset moves the words like it moves the lines', () => {
  // A line shows at `start - offset`; a +200 ms offset lights each word 200 ms earlier.
  assert.equal(wordHighlightTimings(words, null, 1000, 200)[0].delayMs, -200);
  assert.equal(currentWordIndex(words, 900, 200), 0);
});

test('only the word being sung is current; gaps, zero-length words and the line ends have none', () => {
  assert.equal(currentWordIndex(words, 999, 0), -1);
  assert.equal(currentWordIndex(words, 1000, 0), 0);
  assert.equal(currentWordIndex(words, 1300, 0), -1, 'the zero-length word and the gap are not lit');
  assert.equal(currentWordIndex(words, 1499, 0), -1);
  assert.equal(currentWordIndex(words, 1500, 0), 2);
  assert.equal(currentWordIndex(words, 2000, 0), -1, 'a word ends at its end');
});

test('the Traditional choice swaps the word texts and keeps their timing', () => {
  const lyrics = {
    lines: [{ startMs: 1000, text: '头发', textTraditional: '頭髮', translation: null, romanization: null,
      words: [{ s: 1000, e: 1200, t: '头' }, { s: 1200, e: 1400, t: '发' }], wordsTraditional: ['頭', '髮'] }],
  };
  const shown = lyricsForDisplay(lyrics, true).lines[0];
  assert.equal(shown.text, '頭髮');
  assert.deepEqual(shown.words.map((w) => [w.s, w.t]), [[1000, '頭'], [1200, '髮']]);
  assert.equal(lyricsForDisplay(lyrics, false).lines[0].words[0].t, '头');
});

test('the lit word uses the theme accent with no fill and no filter', () => {
  const source = readFileSync(new URL('../src/lib/LyricsView.svelte', import.meta.url), 'utf8');
  const keyframes = source.match(/@keyframes lyric-word-sing\s*\{[\s\S]*?\n {2}\}/)?.[0] ?? '';
  assert.match(keyframes, /color:\s*var\(--accent\)/);
  const rule = source.match(/\.lyric-word\s*\{[^}]*\}/)?.[0] ?? '';
  assert.match(rule, /animation-name:\s*lyric-word-sing/);
  assert.match(rule, /animation-fill-mode:\s*none/);
  assert.doesNotMatch(rule, /filter|will-change/);
});
