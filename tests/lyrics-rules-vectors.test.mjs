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
    const layout = layoutLyricsStrip(c.main, c.translation, c.gap);
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
