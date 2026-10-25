// The shared lyric-rule vectors (ADR 0034) still match the JS rules; Rust and Kotlin check the same file.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  layoutLyricsStrip, lyricsStripOffset, lyricsStripWindow, lyricsStripFocus, lyricsLineWeight,
  lyricsStripFinalArrivalMs, nextLyricsStripBreak, lyricsEdgeFadeWidth, lyricsEdgeFadeOpacity,
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
});

const close = (/** @type {number} */ a, /** @type {number} */ b, /** @type {string} */ what) => assert.ok(Math.abs(a - b) < 1e-9, `${what}: ${a} != ${b}`);
const samePieces = (/** @type {number[][]} */ actual, /** @type {number[][]} */ expected, /** @type {string} */ what) => {
  assert.equal(actual.length, expected.length, what);
  actual.forEach((p, i) => p.forEach((x, k) => close(x, expected[i][k], `${what} piece ${i}`)));
};

test('the smoothed glide (ADR 0041 D10) matches the shared vectors', async () => {
  const { smoothGlide, glideAt, karaokeGlide, wordStripKnots, knotCubic } = await import('../src/lib/lyrics-glide.js');
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
  for (const c of v.stripGlide) {
    const knots = wordStripKnots(c.lines);
    samePieces(knots, c.knots, 'strip knots');
    for (const s of c.samples) knotCubic(knots, s.t).forEach((x, k) => close(x, s.expected[k], `strip glide ${s.t}`));
  }
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

test('the strip word glide only knows line starts and last-word ends: exact, monotone, C1, overlap-safe', async () => {
  const { wordStripKnots, knotCubic, knotValue, nextKnotBreak } = await import('../src/lib/lyrics-glide.js');
  const lines = [
    { startMs: 1000, cellStart: 0, anchor: 10, endMs: 2000, lastCentre: 90 },
    { startMs: 3000, cellStart: 120, anchor: 8, endMs: null, lastCentre: 0 },
    { startMs: 4000, cellStart: 200, anchor: 6, endMs: 4500, lastCentre: 6 },
    { startMs: 5000, cellStart: 260, anchor: 5, endMs: 7000, lastCentre: 80 },
    { startMs: 6500, cellStart: 400, anchor: 5, endMs: 9000, lastCentre: 60 },
  ];
  const knots = wordStripKnots(lines);
  assert.deepEqual(knots, [[1000, 10], [2000, 90], [3000, 128], [4000, 206], [5000, 265], [6500, 405], [9000, 460]]);
  for (const [t, value] of knots) close(knotValue(knots, t), value, `knot ${t}`);
  let last = -Infinity;
  for (let t = 900; t < 9100; t += 1000 / 120) {
    const [value, speed] = knotCubic(knots, t);
    assert.ok(value >= last - 1e-9 && speed >= -1e-12, `monotone at ${t}`);
    last = value;
  }
  for (const [t] of knots) close(knotCubic(knots, t - 1e-6)[1], knotCubic(knots, t + 1e-6)[1], `C1 at ${t}`);
  assert.equal(nextKnotBreak(knots, 0), 1000);
  assert.equal(nextKnotBreak(knots, 9000), null);
  assert.deepEqual(wordStripKnots([]), []);
});

test('a word-timed line ending just before the next starts glides straight on instead of jumping', async () => {
  const { wordStripKnots, knotValue } = await import('../src/lib/lyrics-glide.js');
  // 拿出下一颗软糖 ends 10 ms before the next line (20 px glyphs, 40 px gap): its end knot is skipped.
  const knots = wordStripKnots([
    { startMs: 207810, cellStart: 0, anchor: 10, endMs: null },
    { startMs: 212700, cellStart: 280, anchor: 10, endMs: 215180, lastCentre: 130 },
    { startMs: 215190, cellStart: 460, anchor: 10, endMs: null },
    { startMs: 231300, cellStart: 680, anchor: 10, endMs: null },
  ]);
  assert.deepEqual(knots, [[207810, 10], [212700, 290], [215190, 470], [231300, 690]]);
  let largest = 0;
  let prev = knotValue(knots, 214000);
  for (let t = 214000 + 1000 / 120; t < 216500; t += 1000 / 120) {
    const now = knotValue(knots, t);
    largest = Math.max(largest, now - prev);
    prev = now;
  }
  assert.ok(largest < 1.5, `largest frame step ${largest} px`);
  // A long pause, and a gap exactly as fast as the line, keep the end knot; so does the last line.
  assert.deepEqual(wordStripKnots([
    { startMs: 1000, cellStart: 0, anchor: 10, endMs: 2000, lastCentre: 110 },
    { startMs: 30000, cellStart: 140, anchor: 10, endMs: 31000, lastCentre: 110 },
    { startMs: 32000, cellStart: 340, anchor: 10, endMs: 33000, lastCentre: 60 },
  ]), [[1000, 10], [2000, 110], [30000, 150], [31000, 250], [32000, 350], [33000, 400]]);
});
