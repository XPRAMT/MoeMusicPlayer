// The shared lyric-rule vectors (ADR 0034) still match the JS rules; Rust and Kotlin check the same file.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  layoutLyricsStrip, lyricsStripOffset, lyricsStripWindow, lyricsStripFocus, lyricsLineWeight,
  lyricsLineOpacityFactor, lyricsStripFinalArrivalMs, nextLyricsStripBreak, lyricsEdgeFadeWidth, lyricsEdgeFadeOpacity,
} from '../src/lib/lyrics-strip.js';
import { findActiveLyricIndex } from '../src/lib/lyrics-window.js';
import { lyricsDisplayOffsetMs } from '../src/lib/lyrics-timing-offset.js';
import { getLyricScrollProgress } from '../src/lib/taskbar-lyrics-settings.js';

const v = JSON.parse(readFileSync(new URL('./fixtures/lyrics-rules-vectors.json', import.meta.url), 'utf8'));

test('shared lyric-rule vectors match the JS rules', () => {
  for (const c of v.active) assert.equal(findActiveLyricIndex(c.starts.map((startMs) => ({ startMs })), c.positionMs, c.offsetMs), c.expected);
  for (const c of v.offsets) assert.equal(lyricsDisplayOffsetMs(c.lyricsOffsetMs, c.delaySec), c.expected);
  for (const c of v.scroll) assert.equal(getLyricScrollProgress(c.elapsedMs, c.durationMs), c.expected);
  for (const c of v.finalArrival) assert.equal(lyricsStripFinalArrivalMs(c.lineStartMs, c.text, c.trackEndMs), c.expected);
  for (const c of v.strip) {
    const layout = layoutLyricsStrip(c.main, c.translation, c.gap, c.anchors);
    assert.equal(lyricsStripOffset(c.starts, layout, c.align, c.positionMs, c.viewWidth, c.target, c.finalLine), c.expected);
  }
  for (const c of v.windows) {
    const range = lyricsStripWindow(layoutLyricsStrip(c.main, c.translation, c.gap), c.fromOffset, c.toOffset, c.viewWidth, c.margin);
    assert.deepEqual([range.first, range.last], [c.first, c.last]);
  }
  for (const c of v.breaks) assert.equal(nextLyricsStripBreak(c.starts, c.positionMs, c.finalArrivalMs), c.expected);
  for (const c of v.weights) {
    const focus = lyricsStripFocus(c.starts, c.positionMs);
    assert.deepEqual(c.starts.map((_, i) => lyricsLineWeight(focus, i)), c.expected);
  }
  for (const c of v.edgeFade) {
    assert.equal(lyricsEdgeFadeWidth(c.areaWidth, c.areaHeight), c.width);
    assert.equal(lyricsEdgeFadeOpacity(c.x, c.areaWidth, c.areaHeight), c.expected);
  }
  for (const c of v.opacity) assert.equal(lyricsLineOpacityFactor(c.weight, c.inactive, c.animation), c.expected);
});

const close = (/** @type {number} */ a, /** @type {number} */ b, /** @type {string} */ what) => assert.ok(Math.abs(a - b) < 1e-9, `${what}: ${a} != ${b}`);
const samePieces = (/** @type {number[][]} */ actual, /** @type {number[][]} */ expected, /** @type {string} */ what) => {
  assert.equal(actual.length, expected.length, what);
  actual.forEach((p, i) => p.forEach((x, k) => close(x, expected[i][k], `${what} piece ${i}`)));
};

test('the smoothed glide (ADR 0041 D10) matches the shared vectors', async () => {
  const { smoothGlide, glideAt, karaokeGlide, wordStripGlide } = await import('../src/lib/lyrics-glide.js');
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
  const strip = wordStripGlide(v.stripGlide.lines);
  samePieces(strip, v.stripGlide.pieces, 'strip glide');
});

test('the smoothed glide is monotone, C1, exact on hard knots and inside the speed band', async () => {
  const { smoothGlide, glideAt, GLIDE_MIN_SPEED, GLIDE_MAX_SPEED } = await import('../src/lib/lyrics-glide.js');
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
