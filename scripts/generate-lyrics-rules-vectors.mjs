// Regenerates tests/fixtures/lyrics-rules-vectors.json from the JS rules (the reference implementation).
// The same file is checked by tests/lyrics-rules-vectors.test.mjs (JS), crates/player-lyrics-rules (Rust) and
// FloatingLyricsRulesTest.kt (Android), so the three implementations cannot drift (ADR 0034).
// Usage: node scripts/generate-lyrics-rules-vectors.mjs
import { writeFileSync } from 'node:fs';
import {
  layoutLyricsStrip, lyricsStripOffset, lyricsStripWindow, lyricsStripFocus, lyricsLineWeight,
  lyricsStripFinalArrivalMs, nextLyricsStripBreak, lyricsEdgeFadeWidth, lyricsEdgeFadeOpacity,
} from '../src/lib/lyrics-strip.js';
import { sungWord, centreMaskOpacity } from '../src/lib/lyrics-word-strip.js';
import { smoothGlide, glideAt, karaokeGlide, wordStripKnots, knotCubic } from '../src/lib/lyrics-glide.js';
import { findActiveLyricIndex } from '../src/lib/lyrics-window.js';
import { lyricsDisplayOffsetMs } from '../src/lib/lyrics-timing-offset.js';
import { getLyricScrollProgress } from '../src/lib/taskbar-lyrics-settings.js';

const starts = [1000, 3000, 5000, 9000];
const active = [];
for (const [positionMs, offsetMs] of [[0, 0], [999, 0], [1000, 0], [2700, 300], [4999, 0], [5000, 0], [20000, 0], [1500, -600]]) {
  active.push({ starts, positionMs, offsetMs, expected: findActiveLyricIndex(starts.map((startMs) => ({ startMs })), positionMs, offsetMs) });
}
active.push({ starts: [], positionMs: 500, offsetMs: 0, expected: -1 });

const offsets = [[null, 0], [250, 0], [0, 0.15], [100, -1.25], [0, 25], [-40, -0]].map(([lyricsOffsetMs, delaySec]) => ({
  lyricsOffsetMs, delaySec, expected: lyricsDisplayOffsetMs(lyricsOffsetMs, delaySec),
}));

const scroll = [[0, 4000], [1800, 4000], [3600, 4000], [5000, 4000], [-10, 4000], [100, 0]].map(([elapsedMs, durationMs]) => ({
  elapsedMs, durationMs, expected: getLyricScrollProgress(elapsedMs, durationMs),
}));

const finalArrival = [[1000, 'abc', null], [1000, '', null], [1000, '夜に駆ける', 2500], [0, 'あ👨‍👩‍👧', null]].map(([lineStartMs, text, trackEndMs]) => ({
  lineStartMs, text, trackEndMs, expected: lyricsStripFinalArrivalMs(lineStartMs, text, trackEndMs),
}));

const main = [100, 50, 80, 120];
const translation = [60, 90, 0, 40];
const gap = 20;
const layout = layoutLyricsStrip(main, translation, gap);
const strip = [];
// The second list has a very short line (150 ms) and a long gap, so the curve's tangent choice is pinned too.
for (const [list, positions] of [
  [starts, [0, 1000, 1500, 2000, 2500, 3000, 4500, 6000, 7500, 9000, 9500, 10000, 12000]],
  [[1000, 1150, 5000, 5200], [1000, 1075, 1150, 2000, 4000, 5000, 5100, 5200, 6000, 7000]],
]) {
  for (const align of ['left', 'center', 'right']) {
    for (const target of ['left', 'center']) {
      for (const positionMs of positions) {
        for (const finalLine of [null, { endMs: 11000, lastGlyphCenter: 110 }]) {
          strip.push({
            starts: list, main, translation, gap, align, target, positionMs, viewWidth: 400, finalLine,
            expected: lyricsStripOffset(list, layout, align, positionMs, 400, target, finalLine),
          });
        }
        // The first glyph's centre as the left anchor (ADR 0041 D10); 200 is clamped into its cell.
        if (align === 'left') {
          const anchors = [8, 12.5, 200, 0];
          const anchored = layoutLyricsStrip(main, translation, gap, anchors);
          strip.push({
            starts: list, main, translation, gap, anchors, align, target, positionMs, viewWidth: 400, finalLine: null,
            expected: lyricsStripOffset(list, anchored, align, positionMs, 400, target, null),
          });
        }
      }
    }
  }
}

const windows = [[0, 0, 100, 0], [-200, -330, 300, 300], [-500, -500, 50, 0], [5000, 5000, 10, 0]].map(([from, to, view, margin]) => {
  const range = lyricsStripWindow(layout, from, to, view, margin);
  return { main, translation, gap, fromOffset: from, toOffset: to, viewWidth: view, margin, first: range.first, last: range.last };
});

const breaks = [[0, null], [1000, null], [9000, null], [9000, 11000], [12000, 11000]].map(([positionMs, finalArrivalMs]) => ({
  starts, positionMs, finalArrivalMs, expected: nextLyricsStripBreak(starts, positionMs, finalArrivalMs),
}));

// `android` marks cases the Kotlin `cellWeight` must match too: Android's lead-in is a function of the position and
// does not shorten a fade to a line shorter than 200 ms (AGENTS.md, 浮動歌詞), so those cases are Windows/JS only.
const weights = [];
for (const [list, android] of [[starts, true], [[100, 250, 400], false], [[1000, 1100, 3000], false]]) {
  for (const positionMs of [0, 500, 799, 850, 900, 1000, 1050, 1100, 2800, 2900, 3000, 4900, 5000, 8850, 9000, 15000]) {
    const focus = lyricsStripFocus(list, positionMs);
    weights.push({ starts: list, positionMs, android, expected: list.map((_, index) => lyricsLineWeight(focus, index)) });
  }
}

const edgeFade = [];
for (const [areaWidth, areaHeight] of [[400, 40], [60, 40], [80, 40], [0, 40], [400, 0], [301, 33.5]]) {
  for (const x of [-5, 0, 10, 20, 39.9, 40, 200, 360, 380, areaWidth, areaWidth + 3]) {
    edgeFade.push({ areaWidth, areaHeight, x, width: lyricsEdgeFadeWidth(areaWidth, areaHeight), expected: lyricsEdgeFadeOpacity(x, areaWidth, areaHeight) });
  }
}

// The smoothed glide (ADR 0041 D10): pieces and samples of the core curve, the karaoke edge and the strip glide.
const glideSamples = (/** @type {any[]} */ pieces, /** @type {number[]} */ times) => times.map((t) => ({ t, expected: glideAt(pieces, t) }));
// 61 sample times from `from` to `to` (rounded to 0.25 ms, so every language reads the same numbers).
const sampleTimes = (/** @type {number} */ from, /** @type {number} */ to) => {
  const out = [];
  for (let i = 0; i <= 60; i += 1) out.push(Math.round((from + ((to - from) * i) / 60) * 4) / 4);
  return out;
};
const glide = [
  // Dense words: many guides close together, one sharing a time, one off the band.
  { hard: [[1000, 0], [3000, 200]], guides: [[[1100, 5], [1150, 40], [1150, 41], [1200, 42], [1300, 45], [2000, 190], [2900, 199]]], easeEnds: false },
  // Long gap between lines (more than 3x apart: the knot's tangent is the slower stretch's upper bound).
  { hard: [[1000, 0], [2000, 100], [30000, 200], [31000, 300]], guides: [[[1500, 70]], [[10000, 110]], []], easeEnds: true },
  // Knots sharing a time are dropped, backward values raised.
  { hard: [[0, 0], [0, 50], [100, 40], [200, 10]], guides: [[], [[50, 90]], []], easeEnds: true },
  { hard: [[500, 20]], guides: [], easeEnds: true },
].map((c) => {
  const pieces = smoothGlide(/** @type {any} */ (c.hard), /** @type {any} */ (c.guides), c.easeEnds);
  const end = c.hard[c.hard.length - 1][0];
  return { ...c, pieces, samples: glideSamples(pieces, sampleTimes(c.hard[0][0] - 100, end + 100)) };
});
const karaokeCases = [
  { spans: [[0, 20], [20, 30], [34, 60]], words: [[1000, 1200], [1200, 1200], [1300, 1500]] },
  { spans: [[0, 20], [20, 40], [40, 60], [64, 80], [80, 120]], words: [[1000, 1100], [1100, 1150], [1150, 1600], [1700, 1720], [1720, 2400]] },
  { spans: [[5, 9]], words: [[700, 700]] },
  { spans: [], words: [] },
].map((c) => {
  const pieces = karaokeGlide(/** @type {any} */ (c.spans), /** @type {any} */ (c.words));
  return { ...c, pieces, samples: glideSamples(pieces, sampleTimes(600, 2500)) };
});
// The strip glide ignores word times: only line starts and last-word ends. Cases: ordinary lines, a line without word
// times, a one-glyph line, an end not after its start, an overlap, and a long gap.
const stripGlideCases = [
  [
    { startMs: 1000, cellStart: 0, anchor: 10, endMs: 2400, lastCentre: 110 },
    { startMs: 3000, cellStart: 160, anchor: 9, endMs: null, lastCentre: 0 },
    { startMs: 5000, cellStart: 300, anchor: 8, endMs: 6000, lastCentre: 120 },
  ],
  [
    { startMs: 1000, cellStart: 0, anchor: 10, endMs: 1800, lastCentre: 60 },
    { startMs: 2000, cellStart: 90, anchor: 5, endMs: 2300, lastCentre: 5 },
    { startMs: 2500, cellStart: 150, anchor: 6, endMs: 2500, lastCentre: 70 },
    { startMs: 9000, cellStart: 260, anchor: 6, endMs: 12000, lastCentre: 200 },
    { startMs: 11000, cellStart: 520, anchor: 7, endMs: 13000, lastCentre: 90 },
    { startMs: 40000, cellStart: 640, anchor: 7, endMs: 41000, lastCentre: 100 },
  ],
  [{ startMs: 500, cellStart: 0, anchor: 4, endMs: 900, lastCentre: 4 }],
];
const stripGlide = stripGlideCases.map((lines) => {
  const knots = wordStripKnots(lines);
  const from = knots.length > 0 ? knots[0][0] - 100 : 0;
  const to = knots.length > 0 ? knots[knots.length - 1][0] + 100 : 100;
  return { lines, knots, samples: sampleTimes(from, to).map((t) => ({ t, expected: knotCubic(knots, t) })) };
});
const centreMask = [];
for (const [areaWidth, emPx, main, secondary] of [
  [1000, 40, 1, 0.5], [80, 40, 1, 0.5], [300, 24, 1, 0.05], [1000, 40, 1, 1], [400, 30, 1, 0], [0, 40, 1, 0.5],
  // secondary above main, a small main, absolute values, a narrow area
  [1000, 40, 0.3, 0.8], [1000, 40, 0.6, 0.3], [1000, 40, 0.01, 0.1], [80, 40, 0.4, 0.9], [60, 40, 0.7, 0.2], [1000, 40, 0, 0],
]) {
  for (const x of [0, 10, 20, 39, 40, 41, 120, 130, 150, 160, 170, 178, 180, 182, 200, 470, 480, 500, 520, 600, 960, areaWidth]) {
    centreMask.push({ areaWidth, emPx, main, secondary, x, expected: centreMaskOpacity(x, areaWidth, emPx, main, secondary) });
  }
}
const sungWords = [[1000, 1200], [1200, 1200], [1300, 1500]];
const sung = [999, 1000, 1199, 1200, 1250, 1400, 1500].map((t) => ({ words: sungWords, t, expected: sungWord(sungWords, t) }));

const vectors = { note: 'Generated by scripts/generate-lyrics-rules-vectors.mjs; do not edit by hand.', active, offsets, scroll, finalArrival, strip, windows, breaks, weights, edgeFade, glide, karaokeGlide: karaokeCases, stripGlide, centreMask, sung };
writeFileSync(new URL('../tests/fixtures/lyrics-rules-vectors.json', import.meta.url), `${JSON.stringify(vectors, null, 1)}\n`);
