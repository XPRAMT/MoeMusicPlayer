import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  LYRICS_FADE_MS,
  LYRICS_STRIP_ALIGN,
  LYRICS_STRIP_TARGET,
  LYRICS_STRIP_SEPARATOR,
  buildLyricsStripLines,
  effectiveLyricsTextEffect,
  lyricsGraphemeCount,
  lyricsStripFinalArrivalMs,
  layoutLyricsStrip,
  lyricsLineWeight,
  lyricsStripFocus,
  lyricsStripAnchor,
  lyricsStripOffset,
  lyricsStripCubic,
  lyricsStripWindow,
  nextLyricsStripBreak,
  normalizeLyricsAnimation,
} from '../src/lib/lyrics-strip.js';

test('the animation setting falls back to none', () => {
  assert.equal(normalizeLyricsAnimation('continuous'), 'continuous');
  assert.equal(normalizeLyricsAnimation('none'), 'none');
  assert.equal(normalizeLyricsAnimation('bounce'), 'none');
  assert.equal(normalizeLyricsAnimation(undefined), 'none');
  assert.equal(LYRICS_STRIP_SEPARATOR, '\u3000');
  // Continuous always anchors right, whatever the text alignment setting.
  assert.equal(LYRICS_STRIP_ALIGN, 'left');
  assert.equal(LYRICS_STRIP_TARGET, 'center');
});

test('continuous motion replaces a selected shadow with a stroke, while other looks stay selected', () => {
  assert.equal(effectiveLyricsTextEffect('continuous', 'shadow'), 'stroke');
  assert.equal(effectiveLyricsTextEffect('continuous', 'stroke'), 'stroke');
  assert.equal(effectiveLyricsTextEffect('continuous', 'none'), 'none');
  assert.equal(effectiveLyricsTextEffect('none', 'shadow'), 'shadow');
});

test('strip lines skip empty lines and drop translations when hidden', () => {
  const timeline = [
    { startMs: 0, line: { text: ' a ', translation: 'A' } },
    { startMs: 1000, line: { text: '', translation: '' } },
    { startMs: 2000, line: { text: 'b', translation: null } },
  ];
  assert.deepEqual(buildLyricsStripLines(timeline, true), [
    { startMs: 0, text: 'a', translation: 'A' },
    { startMs: 2000, text: 'b', translation: '' },
  ]);
  assert.equal(buildLyricsStripLines(timeline, false)[0].translation, '');
  assert.deepEqual(buildLyricsStripLines(null, true), []);
});

test('cells are as wide as the wider text and one gap apart', () => {
  const layout = layoutLyricsStrip([100, 50, 80], [60, 90, 0], 20);
  assert.deepEqual(layout.widths, [100, 90, 80]);
  assert.deepEqual(layout.starts, [0, 120, 230]);
  assert.equal(lyricsStripAnchor(layout, 1, 'left'), 120);
  assert.equal(lyricsStripAnchor(layout, 1, 'center'), 165);
  assert.equal(lyricsStripAnchor(layout, 1, 'right'), 210);
});

test('the current line sits at the alignment point and glides to the next', () => {
  const layout = layoutLyricsStrip([100, 100, 100], [], 20);
  const starts = [1000, 3000, 5000];
  // Left: line 0 at x 0; half-way to line 1 at 2000 ms it has covered 3/8 (it starts from rest, C1 curve).
  assert.equal(lyricsStripOffset(starts, layout, 'left', 1000, 400), 0);
  assert.equal(lyricsStripOffset(starts, layout, 'left', 2000, 400), -45);
  assert.equal(lyricsStripOffset(starts, layout, 'left', 3000, 400), -120);
  // Centre: the cell's middle on the window's middle.
  assert.equal(lyricsStripOffset(starts, layout, 'center', 3000, 400), 200 - 170);
  // Right: the cell's right edge on the window's right edge.
  assert.equal(lyricsStripOffset(starts, layout, 'right', 1000, 400), 300);
  // Before the first line it rests on the first; after the last it rests on the last.
  assert.equal(lyricsStripOffset(starts, layout, 'left', 0, 400), 0);
  assert.equal(lyricsStripOffset(starts, layout, 'left', 9000, 400), -240);
  assert.equal(lyricsStripOffset([], layout, 'left', 0, 400), 0);
});

test('seeking lands on the interpolated place at once', () => {
  const layout = layoutLyricsStrip([100, 100], [], 0);
  assert.equal(lyricsStripOffset([0, 1000], layout, 'left', 250, 0), -15.625);
  assert.ok(Math.abs(lyricsStripOffset([0, 1000], layout, 'left', 750, 0) + 84.375) < 1e-9);
});

test('the next break is the next line start', () => {
  assert.equal(nextLyricsStripBreak([1000, 3000], 0), 1000);
  assert.equal(nextLyricsStripBreak([1000, 3000], 1500), 3000);
  assert.equal(nextLyricsStripBreak([1000, 3000], 3000), null);
  assert.equal(nextLyricsStripBreak([], 0), null);
  assert.equal(nextLyricsStripBreak([1000], 0), 1000);
});

test('the renderers anchor the continuous strip on the lyric area centre whatever the alignment setting', async () => {
  const { readFileSync } = await import('node:fs');
  // Windows draws natively (ADR 0034): the plan anchors on STRIP_ALIGN / STRIP_TARGET, never the alignment setting.
  const plan = readFileSync(new URL('../crates/player-lyrics-native/src/plan.rs', import.meta.url), 'utf8');
  assert.match(plan, /rules::STRIP_ALIGN,[\s\S]*rules::STRIP_TARGET,/);
  assert.doesNotMatch(plan, /align: Align|look\.align/);
  const kotlin = readFileSync(new URL('../plugins/tauri-plugin-media-index/android/src/main/java/com/moemusicplayer/mediaindex/LyricsFramePlanner.kt', import.meta.url), 'utf8');
  assert.match(kotlin, /FloatingLyricsRules\.STRIP_ALIGN, arrival,\s*strip\.mainWidths, strip\.lastGlyphCenters/);
  assert.match(kotlin, /stripTarget\(areaW, FloatingLyricsRules\.STRIP_TARGET\)/);
});

test('the strip holds only the lines near the lyric area, never the whole song', () => {
  const layout = { starts: Array.from({ length: 100 }, (_, i) => i * 110), widths: Array(100).fill(100) };
  // Gliding from cell 50 to cell 51 right-anchored in a 300 px area, with one area of margin per side.
  assert.deepEqual(lyricsStripWindow(layout, 300 - 5600, 300 - 5710, 300, 300), { first: 45, last: 54 });
  assert.deepEqual(lyricsStripWindow({ starts: [], widths: [] }, 0, 0, 300, 300), { first: 0, last: -1 });
});

test('the native renderer draws only the strip window', async () => {
  const { readFile } = await import('node:fs/promises');
  const source = await readFile(new URL('../crates/player-lyrics-native/src/plan.rs', import.meta.url), 'utf8');
  assert.match(source, /rules::strip_window\(/);
});

test('a line starts with its left edge on the lyric area centre and slides left to the next line', () => {
  // Cells 100 wide, 20 apart; the lyric area is 300 wide (any panel width), so the centre is 150.
  const layout = layoutLyricsStrip([100, 100, 100], [], 20);
  const starts = [1000, 3000, 5000];
  const at = (ms, view = 300) => lyricsStripOffset(starts, layout, LYRICS_STRIP_ALIGN, ms, view, LYRICS_STRIP_TARGET);
  assert.equal(at(1000), 150);
  assert.equal(at(3000), 150 - 120);
  assert.equal(at(2000), 150 - 45);
  assert.equal(at(5000), 150 - 240);
  // Independent of the panel width: only the centre moves.
  assert.equal(at(3000, 500), 250 - 120);
  // The left edge of the line at its start is exactly the centre.
  assert.equal(layout.starts[1] + at(3000), 150);
});

test('the final line keeps gliding until its last grapheme centre reaches the lyric area centre at track end', () => {
  const layout = layoutLyricsStrip([120, 80], [160, 0], 20);
  const starts = [1000, 5000];
  const finalLine = { endMs: 11000, lastGlyphCenter: 70 };
  const at = (ms) => lyricsStripOffset(starts, layout, 'left', ms, 300, 'center', finalLine);

  // The line still starts with its left edge on the centre.
  assert.equal(at(5000), 150 - 180);
  assert.ok(at(8000) < 150 - 180 && at(8000) > 150 - 250);
  // At the track end, the final glyph occupies [140, 160], centred at x=150.
  assert.equal(at(11000), 150 - (180 + 70));
  assert.equal(nextLyricsStripBreak(starts, 9000, 11000), 11000);
  assert.equal(nextLyricsStripBreak(starts, 11000, 11000), null);
  assert.equal(nextLyricsStripBreak(starts, 9000, null), null);
});

test('the final glide estimates 500 ms per grapheme and stops no later than track end', () => {
  assert.equal(lyricsGraphemeCount('Aか\u3099🙂'), 3);
  assert.equal(lyricsStripFinalArrivalMs(5000, '最後一句'), 7000);
  assert.equal(lyricsStripFinalArrivalMs(5000, '最後一句', 6200), 6200);
  assert.equal(lyricsStripFinalArrivalMs(5000, 'Aか\u3099🙂'), 6500);
});

// Line fades (ADR 0032): lines start at 1000, 5000 and 9000 ms.
const FADE_STARTS = [1000, 5000, 9000];
// A line's opacity factor from its weight, as a line-fading renderer would: `secondary` percent for a line that is not
// current, 100 for the current one, linear in between (the windows no longer fade lines; this pins the weights).
const weightFactor = (weight, secondary) => {
  const s = secondary / 100;
  return s + (1 - s) * Math.min(1, Math.max(0, weight));
};
const factorAt = (index, positionMs, secondary = 50) =>
  weightFactor(lyricsLineWeight(lyricsStripFocus(FADE_STARTS, positionMs), index), secondary);

test('the fade lasts 200 ms and ends exactly when the next line starts', () => {
  assert.equal(LYRICS_FADE_MS, 200);
  const before = lyricsStripFocus(FADE_STARTS, 4799);
  assert.equal(before.progress, 0);
  assert.equal(before.fadeStartMs, 4800);
  assert.equal(before.fadeMs, 200);
  assert.deepEqual([before.current, before.incoming], [0, 1]);
  assert.equal(lyricsStripFocus(FADE_STARTS, 4800).progress, 0);
  assert.equal(lyricsStripFocus(FADE_STARTS, 4900).progress, 0.5);
  assert.equal(lyricsStripFocus(FADE_STARTS, 4999).progress, 0.995);
  // At the start of the next line it has become the current one and nothing is half-way.
  const at = lyricsStripFocus(FADE_STARTS, 5000);
  assert.deepEqual([at.current, at.incoming, at.progress], [1, 2, 0]);
});

test('the current line fades to main x not-current and the next fades in to the main opacity', () => {
  // Not in a fade: the current line is 1 (the container's main opacity), every other line the not-current opacity.
  assert.equal(factorAt(0, 3000), 1);
  assert.equal(factorAt(1, 3000), 0.5);
  assert.equal(factorAt(2, 3000), 0.5);
  // Start, middle and end of the fade.
  assert.equal(factorAt(0, 4800), 1);
  assert.equal(factorAt(1, 4800), 0.5);
  assert.equal(factorAt(0, 4900), 0.75);
  assert.equal(factorAt(1, 4900), 0.75);
  assert.equal(factorAt(0, 5000), 0.5);
  assert.equal(factorAt(1, 5000), 1);
  // Lines further away never change.
  assert.equal(factorAt(2, 4900), 0.5);
});

test('80% main opacity x 50% not-current gives 40% for the lines around the current one', () => {
  const main = 0.8; // the window's own opacity setting, applied to the container
  assert.equal(main * factorAt(1, 3000), 0.4);
  assert.equal(main * factorAt(0, 3000), 0.8);
  assert.equal(main * factorAt(0, 5000), 0.4);
  assert.equal(main * factorAt(1, 5000), 0.8);
});

test('a not-current opacity of 100% does not dim anything', () => {
  for (const position of [0, 900, 950, 1000, 3000, 4900, 5000, 9000, 20000]) {
    for (const index of [0, 1, 2]) assert.equal(factorAt(index, position, 100), 1);
  }
  assert.equal(weightFactor(-3, 50), 0.5);
  assert.equal(weightFactor(7, 50), 1);
});

test('before the first line nothing is dimmed (like Now Playing)', () => {
  const early = lyricsStripFocus(FADE_STARTS, 100);
  assert.deepEqual([early.current, early.incoming, early.progress], [-1, 0, 0]);
  assert.equal(early.fadeStartMs, 800);
  for (const position of [-500, 0, 100, 799, 800]) {
    for (const index of [0, 1, 2]) assert.equal(factorAt(index, position), 1, `line ${index} at ${position}`);
  }
});

test('in the 200 ms before the first line starts, the other lines fade out and the first stays at 1', () => {
  for (const position of [800, 850, 900, 950, 999, 1000]) assert.equal(factorAt(0, position), 1);
  assert.equal(factorAt(1, 800), 1);
  assert.equal(factorAt(1, 900), 0.75);
  assert.equal(factorAt(2, 900), 0.75);
  assert.equal(factorAt(1, 1000), 0.5);
  assert.equal(factorAt(2, 1000), 0.5);
  // No jump at the first line's start: the value just before equals the value at the start.
  assert.ok(Math.abs(factorAt(1, 999.9) - factorAt(1, 1000)) < 1e-3);
  assert.equal(factorAt(0, 1000), 1);
  // The weights the view animates between: others 1 -> 0, the first 1 -> 1.
  const focus = lyricsStripFocus(FADE_STARTS, 900);
  assert.equal(lyricsLineWeight({ ...focus, progress: 0 }, 1), 1);
  assert.equal(lyricsLineWeight({ ...focus, progress: 1 }, 1), 0);
  assert.equal(lyricsLineWeight({ ...focus, progress: 0 }, 0), 1);
  assert.equal(lyricsLineWeight({ ...focus, progress: 1 }, 0), 1);
  // Same 200 ms rule afterwards: line 1 then fades in before it starts, the first one out.
  assert.equal(factorAt(1, 4900), 0.75);
  assert.equal(factorAt(0, 4900), 0.75);
});

test('a first line that starts within 200 ms of the song start fades over the time it has, from position 0', () => {
  const starts = [100, 5000];
  const focus = lyricsStripFocus(starts, 0);
  assert.equal(focus.fadeStartMs, 0);
  assert.equal(focus.fadeMs, 100);
  assert.equal(weightFactor(lyricsLineWeight(focus, 1), 50), 1);
  assert.equal(weightFactor(lyricsLineWeight(lyricsStripFocus(starts, 50), 1), 50), 0.75);
  assert.equal(weightFactor(lyricsLineWeight(lyricsStripFocus(starts, 100), 1), 50), 0.5);
});

test('seeking back before the first line restores full opacity for every line', () => {
  assert.equal(factorAt(1, 3000), 0.5);
  assert.equal(factorAt(1, 100), 1);
  assert.equal(factorAt(2, 100), 1);
  assert.equal(factorAt(1, 950), factorAt(1, 950));
  assert.equal(factorAt(1, 950), 0.625);
});

test('after the last line starts nothing fades and it stays the current line', () => {
  const last = lyricsStripFocus(FADE_STARTS, 9000);
  assert.deepEqual([last.current, last.incoming, last.progress, last.fadeStartMs], [2, -1, 0, null]);
  assert.equal(factorAt(2, 9000), 1);
  assert.equal(factorAt(2, 999999), 1);
  assert.equal(factorAt(1, 9000), 0.5);
  // The last line starts fading in 200 ms before it, like the others.
  assert.equal(factorAt(2, 8900), 0.75);
  assert.equal(factorAt(1, 8900), 0.75);
});

test('a seek lands on the same value as playing through (the function only looks at the position)', () => {
  assert.equal(factorAt(1, 4900), factorAt(1, 4900));
  assert.equal(factorAt(0, 7000), 0.5);
  assert.equal(factorAt(1, 7000), 1);
  assert.equal(factorAt(1, 1500), 0.5);
  assert.equal(factorAt(0, 1500), 1);
  assert.equal(factorAt(1, 500), 1);
});

test('a line shorter than 200 ms fades over its own length, so the previous fade ends where it starts', () => {
  const starts = [0, 100, 1000];
  const focus = lyricsStripFocus(starts, 50);
  assert.deepEqual([focus.current, focus.incoming], [0, 1]);
  assert.equal(focus.fadeStartMs, 0);
  assert.equal(focus.fadeMs, 100);
  assert.equal(focus.progress, 0.5);
  // At line 1's start, line 1 is fully current again.
  assert.equal(lyricsStripFocus(starts, 100).progress, 0);
  assert.equal(lyricsLineWeight(lyricsStripFocus(starts, 100), 1), 1);
  assert.equal(lyricsStripFocus([], 500).incoming, -1);
});

test('continuous mode dims by the static mask only, fed by each window own main and secondary opacity', () => {
  // ADR 0041 D10: no per-line fades; the mask takes main (plateau) and secondary (where the edge ramp starts).
  const plan = readFileSync(new URL('../crates/player-lyrics-native/src/plan.rs', import.meta.url), 'utf8');
  const bridge = readFileSync(new URL('../src-tauri/src/native_lyrics.rs', import.meta.url), 'utf8');
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  const win = readFileSync(new URL('../crates/player-lyrics-native/src/win.rs', import.meta.url), 'utf8');
  assert.doesNotMatch(plan, /rules::line_opacity_factor\(/);
  assert.match(win, /rules::centre_mask_stops\(\s*area\.width,\s*em,\s*f64::from\(main\) \/ 100\.0,\s*f64::from\(secondary\) \/ 100\.0,?\s*\)/);
  // No separate shared channel any more: each window's own settings carry both values.
  assert.doesNotMatch(bridge, /lyrics-inactive-opacity/);
  assert.doesNotMatch(app, /lyrics-inactive-opacity/);
  assert.doesNotMatch(app, /inactiveOpacityPercent/);
  assert.match(bridge, /main_opacity_percent: f64::from\(settings\.main_opacity_percent\)/);
  assert.match(bridge, /secondary_opacity_percent: f64::from\(settings\.secondary_opacity_percent\)/);
});

test('the glide is monotone, C1 and exact at every line start', () => {
  const layout = layoutLyricsStrip([100, 30, 200, 60, 90], [0, 0, 0, 0, 0], 20);
  // Uneven gaps: a 150 ms line and a long instrumental gap.
  const starts = [1000, 1150, 4000, 12000, 12500];
  const finalLine = { endMs: 14000, lastGlyphCenter: 40 };
  const at = (ms) => lyricsStripCubic(starts, layout, 'left', ms, 300, 'center', finalLine);
  starts.forEach((start, i) => assert.ok(Math.abs(at(start)[0] - (150 - layout.starts[i])) < 1e-9));
  assert.ok(Math.abs(at(14000)[0] - (150 - layout.starts[4] - 40)) < 1e-9);
  let previous = at(0)[0];
  for (let t = 0; t <= 15000; t += 10) {
    const value = at(t)[0];
    assert.ok(value <= previous + 1e-9, `backwards at ${t}`);
    previous = value;
  }
  for (const knot of [...starts, 14000]) {
    const before = at(knot - 1e-3);
    const speedBefore = before[1] + 2 * before[2] * 1e-3 + 3 * before[3] * 1e-6;
    assert.ok(Math.abs(speedBefore - at(knot)[1]) < 1e-4, `speed jump at ${knot}`);
  }
  // Rest before the first and after the terminal knot.
  assert.equal(at(1000)[1] + 0, 0);
  assert.equal(at(14000)[1] + 0, 0);
  // The short line does not push its neighbours' speed beyond 3x their own average slope.
  assert.ok(Math.abs(at(4000)[1]) <= 3 * (layout.starts[3] - layout.starts[2]) / 8000 + 1e-12);
});