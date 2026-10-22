import assert from 'node:assert/strict';
import test from 'node:test';
import {
  LYRICS_STRIP_SEPARATOR,
  buildLyricsStripLines,
  layoutLyricsStrip,
  lyricsStripAnchor,
  lyricsStripOffset,
  nextLyricsStripBreak,
  normalizeLyricsAnimation,
} from '../src/lib/lyrics-strip.js';

test('the animation setting falls back to none', () => {
  assert.equal(normalizeLyricsAnimation('continuous'), 'continuous');
  assert.equal(normalizeLyricsAnimation('none'), 'none');
  assert.equal(normalizeLyricsAnimation('bounce'), 'none');
  assert.equal(normalizeLyricsAnimation(undefined), 'none');
  assert.equal(LYRICS_STRIP_SEPARATOR, '\u3000');
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
