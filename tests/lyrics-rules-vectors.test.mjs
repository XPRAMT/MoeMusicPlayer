// The JS rules the app still runs match the shared lyric-rule vectors, which the Rust rules generate
// (`cargo run -p player-lyrics-rules --example generate_vectors`, ADR 0034): the active line, the display offset and the
// long-line scroll timing of the main window, and the Now Playing karaoke glide (the same curve as the Windows and
// Android 「無」 mode). Rust and Kotlin check the whole file.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { findActiveLyricIndex } from '../src/lib/lyrics-window.js';
import { lyricsDisplayOffsetMs } from '../src/lib/lyrics-timing-offset.js';
import { getLyricScrollProgress } from '../src/lib/taskbar-lyrics-settings.js';
import { smoothGlide, glideAt, karaokeGlide, GLIDE_MIN_SPEED, GLIDE_MAX_SPEED } from '../src/lib/lyrics-glide.js';

const v = JSON.parse(readFileSync(new URL('./fixtures/lyrics-rules-vectors.json', import.meta.url), 'utf8'));

test('the vectors come from the Rust generator', () => {
  assert.match(v.note, /cargo run -p player-lyrics-rules --example generate_vectors/);
});

test('shared lyric-rule vectors match the JS rules', () => {
  for (const c of v.active) assert.equal(findActiveLyricIndex(c.starts.map((startMs) => ({ startMs })), c.positionMs, c.offsetMs), c.expected);
  for (const c of v.offsets) assert.equal(lyricsDisplayOffsetMs(c.lyricsOffsetMs, c.delaySec), c.expected);
  for (const c of v.scroll) assert.equal(getLyricScrollProgress(c.elapsedMs, c.durationMs), c.expected);
});

const close = (/** @type {number} */ a, /** @type {number} */ b, /** @type {string} */ what) => assert.ok(Math.abs(a - b) < 1e-9, `${what}: ${a} != ${b}`);
const samePieces = (/** @type {number[][]} */ actual, /** @type {number[][]} */ expected, /** @type {string} */ what) => {
  assert.equal(actual.length, expected.length, what);
  actual.forEach((p, i) => p.forEach((x, k) => close(x, expected[i][k], `${what} piece ${i}`)));
};

test('the smoothed glide (ADR 0041 D10) matches the shared vectors', () => {
  for (const c of v.glide) {
    const pieces = smoothGlide(c.hard, c.guides, c.easeEnds);
    samePieces(pieces, c.pieces, JSON.stringify(c.hard));
    for (const s of c.samples) glideAt(pieces, s.t).forEach((x, k) => close(x, s.expected[k], `glide ${s.t}`));
  }
  for (const c of v.karaokeGlide) {
    const pieces = karaokeGlide(c.spans, c.words);
    samePieces(pieces, c.pieces, 'karaoke');
    for (const s of c.samples) close(glideAt(pieces, s.t)[0], s.expected[0], `karaoke ${s.t}`);
  }
});

test('the smoothed glide is monotone, C1, exact on hard knots and inside the speed band', () => {
  /** @type {[number, number][]} */
  const hard = [[1000, 0], [3000, 200], [3400, 260], [6000, 500]];
  /** @type {[number, number][][]} */
  const guides = [[[1100, 5], [1150, 40], [1200, 42], [2000, 190]], [[3200, 255]], [[4000, 270], [4100, 400], [5000, 410]]];
  for (const easeEnds of [false, true]) {
    const pieces = smoothGlide(hard, guides, easeEnds);
    for (const [t, value] of hard) close(glideAt(pieces, t)[0], value, `hard ${t}`);
    for (let i = 0; i + 2 < pieces.length; i += 1) {
      const [t, value, s, c] = pieces[i];
      const h = pieces[i + 1][0] - t;
      close(value + h * (s + c * h), pieces[i + 1][1], 'C0');
      close(s + 2 * c * h, pieces[i + 1][2], 'C1');
    }
    let last = -Infinity;
    for (let t = 900; t < 6100; t += 1000 / 120) {
      const [value, speed] = glideAt(pieces, t);
      assert.ok(value >= last - 1e-9, `monotone at ${t}`);
      last = value;
      const j = hard.findIndex((k, i) => i + 1 < hard.length && t >= k[0] && t < hard[i + 1][0]);
      if (j < 0) continue;
      const a = (hard[j + 1][1] - hard[j][1]) / (hard[j + 1][0] - hard[j][0]);
      const eased = easeEnds && ((j === 0 && t < 1000 + 0.1875 * 2000) || (j === 2 && t > 6000 - 0.1875 * 2600));
      if (!eased) assert.ok(speed >= GLIDE_MIN_SPEED * a - 1e-9 && speed <= GLIDE_MAX_SPEED * a + 1e-9, `band at ${t}: ${speed} vs ${a}`);
    }
  }
});
