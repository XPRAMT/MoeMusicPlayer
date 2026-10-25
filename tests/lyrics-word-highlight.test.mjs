import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { currentWordIndex, karaokeLineAnimation, karaokeWordsOf } from '../src/lib/lyrics-word-highlight.js';
import { lyricsForDisplay } from '../src/lib/lyrics-traditional.js';

const words = [
  { s: 1000, e: 1300, t: '早' },
  { s: 1300, e: 1300, t: '' },
  { s: 1500, e: 2000, t: '安 ' },
];

const widths = [30, 0, 50];

/** The edge share (0..1) the easing gives at input share `p` (piecewise linear between its points). */
function easingAt(/** @type {string} */ easing, /** @type {number} */ p) {
  const points = easing.slice(7, -1).split(', ').map((x) => x.split(' ').map(parseFloat)).map(([v, at]) => [v, at / 100]);
  let value = points[0][0];
  for (let i = 0; i + 1 < points.length; i += 1) {
    const [v0, p0] = points[i];
    const [v1, p1] = points[i + 1];
    if (p >= p0 && p <= p1) return p1 > p0 ? v0 + ((v1 - v0) * (p - p0)) / (p1 - p0) : v1;
    value = v1;
  }
  return value;
}

test('a line with its own end but no word times sweeps as one word from its start to its end', () => {
  const line = { startMs: 2000, endMs: 4000, text: 'Hello world', words: undefined };
  const sweep = karaokeWordsOf(line);
  assert.deepEqual(sweep, [{ s: 2000, e: 4000, t: 'Hello world' }]);
  const a = karaokeLineAnimation(sweep, [110], 2500, 0);
  assert.ok(a);
  assert.equal(a.delayMs, -500);
  assert.equal(a.durationMs, 2000);
  assert.equal(easingAt(a.easing, 0), 0);
  assert.equal(easingAt(a.easing, 1), 1);
  assert.ok(Math.abs(easingAt(a.easing, 0.5) - 0.5) < 1e-3);
  // The offset moves the start and the end alike.
  const shifted = karaokeLineAnimation(sweep, [110], 2500, 300);
  assert.equal(shifted.delayMs, -800);
  assert.equal(shifted.durationMs, 2000);
  // Reduced motion: the whole line lights at its start, like a one-word line.
  const reduced = karaokeLineAnimation(sweep, [110], 2500, 0, true);
  assert.equal(easingAt(reduced.easing, 0.001), 1);
});

test('a line without word times and without an end is not swept; word-timed lines keep their words', () => {
  assert.deepEqual(karaokeWordsOf({ startMs: 2000, text: 'plain' }), []);
  assert.deepEqual(karaokeWordsOf({ startMs: 2000, endMs: 2000, text: 'plain' }), []);
  assert.deepEqual(karaokeWordsOf({ startMs: 2000, endMs: 3000, text: '' }), []);
  assert.deepEqual(karaokeWordsOf({ startMs: null, endMs: 3000, text: 'x' }), []);
  assert.deepEqual(karaokeWordsOf(null), []);
  assert.equal(karaokeWordsOf({ startMs: 1000, endMs: 9000, text: '早安', words }), words);
});

test('one edge per line: from the first word start to the last word end, along the karaoke glide', () => {
  const a = karaokeLineAnimation(words, widths, 1100, 0);
  assert.ok(a);
  assert.equal(a.delayMs, -100);
  assert.equal(a.durationMs, 1000);
  assert.match(a.easing, /^linear\(0 0%, .*, 1 100%\)$/);
  // Monotone, never leaves 0..1, and the whole line is lit at the end.
  let last = -1;
  for (let p = 0; p <= 1; p += 0.01) {
    const v = easingAt(a.easing, p);
    assert.ok(v >= last - 1e-9 && v >= 0 && v <= 1, `${p}`);
    last = v;
  }
  assert.equal(easingAt(a.easing, 1), 1);
  assert.equal(karaokeLineAnimation([], [], 0, 0), null);
  assert.equal(karaokeLineAnimation(words, [0, 0, 0], 0, 0), null);
});

test('the lyric offset moves the edge like it moves the lines', () => {
  // A line shows at `start - offset`; a +200 ms offset starts the edge 200 ms earlier.
  assert.equal(karaokeLineAnimation(words, widths, 1000, 200)?.delayMs, -200);
  assert.equal(currentWordIndex(words, 900, 200), 0);
});

test('reduced motion: no sweep, each word lights wholly at its start', () => {
  const a = karaokeLineAnimation(words, widths, 0, 0, true);
  assert.ok(a);
  // Word 0 (30 of 80) lights at once at its start; word 2 at 1500 ms (input 50%).
  assert.equal(easingAt(a.easing, 0.001), 0.375);
  assert.equal(easingAt(a.easing, 0.49), 0.375);
  assert.equal(easingAt(a.easing, 0.5001), 1);
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

test('karaoke: one accent edge per line, sung text stays lit (forwards), no filter, hard edge under reduced motion', () => {
  const source = readFileSync(new URL('../src/lib/LyricsView.svelte', import.meta.url), 'utf8');
  const wipe = source.match(/@keyframes -global-lyric-line-wipe\s*\{[\s\S]*?\n {2}\}/)?.[0] ?? '';
  assert.match(wipe, /--lyric-line-fill:\s*0%/);
  assert.match(wipe, /--lyric-line-fill:\s*100%/);
  assert.match(wipe, /color:\s*transparent/);
  const rule = source.match(/\.lyric-karaoke\s*\{[^}]*\}/)?.[0] ?? '';
  assert.match(rule, /background-clip:\s*text/);
  assert.match(rule, /box-decoration-break:\s*slice/, 'one background strip over wrapped rows');
  assert.match(rule, /var\(--accent\)/);
  assert.match(rule, /animation-fill-mode:\s*forwards/, 'sung text keeps the fill until the line ends');
  assert.doesNotMatch(rule, /animation-fill-mode:\s*(both|backwards)/, 'text not yet sung stays the line colour');
  assert.doesNotMatch(rule, /filter|will-change/);
  assert.match(source, /<span class="lyric-karaoke" use:karaokeLine=/);
  assert.doesNotMatch(source, /class="lyric-word" style=/, 'words are not animated one by one');
  const reduced = source.match(/@media \(prefers-reduced-motion: reduce\)\s*\{\s*\.lyric-karaoke\s*\{[^}]*\}/)?.[0] ?? '';
  assert.match(reduced, /--lyric-line-edge:\s*0em/);
  const css = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.match(css, /@property --lyric-line-fill\s*\{[^}]*syntax:\s*'<percentage>'/);
});

test('romanization rows get word spans only with romanizationWords; translations never; romanization shows before translation', () => {
  const view = readFileSync(new URL('../src/lib/LyricsView.svelte', import.meta.url), 'utf8');
  assert.match(view, /line\?\.romanizationWords \?\? \[\]/);
  assert.match(view, /<p class="lyric-romanization has-words"/);
  assert.doesNotMatch(view, /<p class="lyric-translation has-words"/);
  const markup = view.slice(view.indexOf('</script>'));
  for (const block of markup.split('<div class="lyric-line').slice(1)) {
    const roma = block.indexOf('lyric-romanization');
    const trans = block.indexOf('lyric-translation');
    if (roma >= 0 && trans >= 0) assert.ok(roma < trans, 'romanization comes before translation');
  }
});

test('a rebuild after a seek resumes the edge mid-line and waits before the line', () => {
  const late = karaokeLineAnimation(words, widths, 1800, 0);
  assert.ok(late && late.delayMs < 0 && late.delayMs + late.durationMs > 0, 'mid-line: the edge continues from where it is');
  const done = karaokeLineAnimation(words, widths, 2500, 0);
  assert.ok(done && done.delayMs + done.durationMs <= 0, 'after the line the held fill shows it whole');
  const early = karaokeLineAnimation(words, widths, 900, 0);
  assert.ok(early && early.delayMs > 0, 'before the line nothing is lit');
});
