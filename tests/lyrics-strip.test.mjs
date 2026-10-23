import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  LYRICS_FADE_MS,
  LYRICS_STRIP_ALIGN,
  LYRICS_STRIP_TARGET,
  LYRICS_STRIP_SEPARATOR,
  buildLyricsStripLines,
  layoutLyricsStrip,
  lyricsLineOpacityFactor,
  lyricsLineWeight,
  lyricsStripFocus,
  lyricsStripAnchor,
  lyricsStripOffset,
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
  // Left: line 0 at x 0, half-way to line 1 at 2000 ms.
  assert.equal(lyricsStripOffset(starts, layout, 'left', 1000, 400), 0);
  assert.equal(lyricsStripOffset(starts, layout, 'left', 2000, 400), -60);
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
  assert.equal(lyricsStripOffset([0, 1000], layout, 'left', 250, 0), -25);
  assert.equal(lyricsStripOffset([0, 1000], layout, 'left', 750, 0), -75);
});

test('the next break is the next line start', () => {
  assert.equal(nextLyricsStripBreak([1000, 3000], 0), 1000);
  assert.equal(nextLyricsStripBreak([1000, 3000], 1500), 3000);
  assert.equal(nextLyricsStripBreak([1000, 3000], 3000), null);
  assert.equal(nextLyricsStripBreak([], 0), null);
  assert.equal(nextLyricsStripBreak([1000], 0), 1000);
});

test('the views anchor the continuous strip on the lyric area centre whatever the alignment setting', async () => {
  const { readFileSync } = await import('node:fs');
  const view = readFileSync(new URL('../src/lib/TaskbarLyricsView.svelte', import.meta.url), 'utf8');
  assert.equal(view.match(/lyricsStripOffset\(stripStarts, stripLayout, LYRICS_STRIP_ALIGN,[^)]*LYRICS_STRIP_TARGET\)/g)?.length, 2);
  assert.doesNotMatch(view, /lyricsStripOffset\([^)]*settings\.align/);
  const kotlin = readFileSync(new URL('../plugins/tauri-plugin-media-index/android/src/main/java/com/moemusicplayer/mediaindex/FloatingLyricsOverlay.kt', import.meta.url), 'utf8');
  assert.match(kotlin, /FloatingLyricsRules\.STRIP_ALIGN, lyricsMs, [\s\S]{0,40}FloatingLyricsRules\.STRIP_TARGET\)/);
});

test('the strip holds only the lines near the lyric area, never the whole song', () => {
  const layout = { starts: Array.from({ length: 100 }, (_, i) => i * 110), widths: Array(100).fill(100) };
  // Gliding from cell 50 to cell 51 right-anchored in a 300 px area, with one area of margin per side.
  assert.deepEqual(lyricsStripWindow(layout, 300 - 5600, 300 - 5710, 300, 300), { first: 45, last: 54 });
  assert.deepEqual(lyricsStripWindow({ starts: [], widths: [] }, 0, 0, 300, 300), { first: 0, last: -1 });
});

test('the view draws the strip window at layout offsets, measured off-screen', async () => {
  const { readFile } = await import('node:fs/promises');
  const source = await readFile(new URL('../src/lib/TaskbarLyricsView.svelte', import.meta.url), 'utf8');
  assert.match(source, /lyricsStripWindow\(/);
  assert.match(source, /#each stripCells as cell/);
  assert.doesNotMatch(source, /#each stripLines as/);
  assert.match(source, /measureText/);
});

test('a line starts with its left edge on the lyric area centre and slides left to the next line', () => {
  // Cells 100 wide, 20 apart; the lyric area is 300 wide (any panel width), so the centre is 150.
  const layout = layoutLyricsStrip([100, 100, 100], [], 20);
  const starts = [1000, 3000, 5000];
  const at = (ms, view = 300) => lyricsStripOffset(starts, layout, LYRICS_STRIP_ALIGN, ms, view, LYRICS_STRIP_TARGET);
  assert.equal(at(1000), 150);
  assert.equal(at(3000), 150 - 120);
  assert.equal(at(2000), 150 - 60);
  assert.equal(at(5000), 150 - 240);
  // Independent of the panel width: only the centre moves.
  assert.equal(at(3000, 500), 250 - 120);
  // The left edge of the line at its start is exactly the centre.
  assert.equal(layout.starts[1] + at(3000), 150);
});

// Line fades (ADR 0032): lines start at 1000, 5000 and 9000 ms.
const FADE_STARTS = [1000, 5000, 9000];
const factorAt = (index, positionMs, inactive = 50, animation = 'continuous') =>
  lyricsLineOpacityFactor(lyricsLineWeight(lyricsStripFocus(FADE_STARTS, positionMs), index), inactive, animation);

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

test('a not-current opacity of 100% does not dim anything, and the setting is normalized', () => {
  for (const position of [0, 900, 950, 1000, 3000, 4900, 5000, 9000, 20000]) {
    for (const index of [0, 1, 2]) assert.equal(factorAt(index, position, 100), 1);
  }
  assert.equal(lyricsLineOpacityFactor(0, 5), 0.1);
  assert.equal(lyricsLineOpacityFactor(0, 250), 1);
  assert.equal(lyricsLineOpacityFactor(0, 'x'), 0.5);
  assert.equal(lyricsLineOpacityFactor(0, undefined), 0.5);
  assert.equal(lyricsLineOpacityFactor(-3, 50), 0.5);
  assert.equal(lyricsLineOpacityFactor(7, 50), 1);
});

test('the animation "none" applies no dimming', () => {
  for (const position of [0, 3000, 4900, 5000, 20000]) {
    for (const index of [0, 1, 2]) {
      assert.equal(factorAt(index, position, 50, 'none'), 1);
      assert.equal(factorAt(index, position, 50, 'bounce'), 1);
    }
  }
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
  assert.equal(lyricsLineOpacityFactor(lyricsLineWeight(focus, 1), 50), 1);
  assert.equal(lyricsLineOpacityFactor(lyricsLineWeight(lyricsStripFocus(starts, 50), 1), 50), 0.75);
  assert.equal(lyricsLineOpacityFactor(lyricsLineWeight(lyricsStripFocus(starts, 100), 1), 50), 0.5);
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

test('the views fade only the changing cells with an opacity-only animation, fed by the shared setting', () => {
  const view = readFileSync(new URL('../src/lib/TaskbarLyricsView.svelte', import.meta.url), 'utf8');
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  // Both strips (main and translation) use the same per-cell style, from the same pure functions.
  assert.equal((view.match(/style=\{cellFadeStyle\(cell\.index\)\}/g) ?? []).length, 2);
  assert.match(view, /lyricsLineOpacityFactor\(weight, inactiveOpacityPercent, settings\.animation\)/);
  assert.match(view, /@keyframes -global-taskbar-lyrics-fade-a \{\s*from \{ opacity: var\(--fade-from\); \}\s*to \{ opacity: var\(--fade-to\); \}/);
  // No will-change per cell (an own layer per line would cost more than the fade saves).
  assert.doesNotMatch(view, /strip-cell\s*\{[^}]*will-change/);
  // The shared setting reaches the windows by event (the preview too) and by one read at start-up.
  assert.match(view, /listen<unknown>\('lyrics-inactive-opacity'/);
  assert.match(view, /invokeCommand\('settings_get_lyrics_preferences'/);
  assert.match(app, /emit\('lyrics-inactive-opacity', inactiveOpacityPercent\)/);
  assert.match(app, /payload: \{ inactiveOpacityPercent \}/);
});
