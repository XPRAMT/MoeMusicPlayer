// Regenerates tests/fixtures/lyrics-rules-vectors.json from the JS rules (the reference implementation).
// The same file is checked by tests/lyrics-rules-vectors.test.mjs (JS), crates/player-lyrics-rules (Rust) and
// FloatingLyricsRulesTest.kt (Android), so the three implementations cannot drift (ADR 0034).
// Usage: node scripts/generate-lyrics-rules-vectors.mjs
import { writeFileSync } from 'node:fs';
import {
  layoutLyricsStrip, lyricsStripOffset, lyricsStripWindow, lyricsStripFocus, lyricsLineWeight,
  lyricsLineOpacityFactor, lyricsStripFinalArrivalMs, nextLyricsStripBreak, lyricsEdgeFadeWidth, lyricsEdgeFadeOpacity,
} from '../src/lib/lyrics-strip.js';
import { glideOffset, sungWord, wordGlideKnots, wordMaskOpacity } from '../src/lib/lyrics-word-strip.js';
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

const opacity = [];
for (const animation of ['none', 'continuous']) {
  for (const [weight, inactive] of [[0, 50], [1, 50], [0.5, 50], [0, 10], [0.25, 80], [2, 50], [-1, 100]]) {
    opacity.push({ weight, inactive, animation, expected: lyricsLineOpacityFactor(weight, inactive, animation) });
  }
}

const edgeFade = [];
for (const [areaWidth, areaHeight] of [[400, 40], [60, 40], [80, 40], [0, 40], [400, 0], [301, 33.5]]) {
  for (const x of [-5, 0, 10, 20, 39.9, 40, 200, 360, 380, areaWidth, areaWidth + 3]) {
    edgeFade.push({ areaWidth, areaHeight, x, width: lyricsEdgeFadeWidth(areaWidth, areaHeight), expected: lyricsEdgeFadeOpacity(x, areaWidth, areaHeight) });
  }
}

// Word-timed lyrics (ADR 0041 option C): knots, speed cap, glide, centre mask, sung word.
const wordLines = [
  { startMs: 1000, cellStart: 0, wordStarts: [1000, 1300, 1320, 1900], wordCentres: [12, 40, 70, 110] },
  { startMs: 3000, cellStart: 160, wordStarts: [], wordCentres: [] },
  { startMs: 5000, cellStart: 300, wordStarts: [5000, 5000, 5400, 5450, 6000], wordCentres: [10, 30, 55, 90, 120] },
];
const wordKnotsVector = wordGlideKnots(wordLines);
const wordGlide = { lines: wordLines, target: 200, knots: wordKnotsVector, samples: [] };
for (const positionMs of [0, 1000, 1150, 1299, 1300, 1310, 1600, 2500, 3000, 4000, 4990, 5000, 5200, 5420, 5800, 6000, 9000]) {
  wordGlide.samples.push({ positionMs, expected: glideOffset(wordKnotsVector, positionMs, 200) });
}
const wordMask = [];
for (const [areaWidth, emPx, inactive] of [[1000, 40, 50], [80, 40, 30], [300, 24, 100], [0, 40, 50]]) {
  for (const x of [0, 10, 39, 40, 41, 120, 130, 150, 160, 170, 178, 180, 182, 200, 470, 480, 500, 520, 600, areaWidth]) {
    wordMask.push({ areaWidth, emPx, inactive, x, expected: wordMaskOpacity(x, areaWidth, emPx, inactive) });
  }
}
const sungWords = [[1000, 1200], [1200, 1200], [1300, 1500]];
const sung = [999, 1000, 1199, 1200, 1250, 1400, 1500].map((t) => ({ words: sungWords, t, expected: sungWord(sungWords, t) }));

const vectors = { note: 'Generated by scripts/generate-lyrics-rules-vectors.mjs; do not edit by hand.', active, offsets, scroll, finalArrival, strip, windows, breaks, weights, opacity, edgeFade, wordGlide, wordMask, sung };
writeFileSync(new URL('../tests/fixtures/lyrics-rules-vectors.json', import.meta.url), `${JSON.stringify(vectors, null, 1)}\n`);
