import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import {
  LYRIC_CUE_LEAD_MS,
  cueAnimationMs,
  displayCuePositions,
  easeOutCubic,
  findDisplayCueIndex,
  nextDisplayCuePosition,
} from '../src/lib/lyrics-cue-timing.ts';
import { correctPlaybackClock, estimatePlaybackPosition, msUntilPlaybackPosition } from '../src/lib/playback-clock.ts';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

test('lines are shown LYRIC_CUE_LEAD_MS early so the animation ends on the line start', () => {
  assert.equal(LYRIC_CUE_LEAD_MS, 200);
  const shown = displayCuePositions([1000, 5000, 9000], 0, true);
  assert.deepEqual(shown, [800, 4800, 8800]);
  assert.equal(findDisplayCueIndex(shown, 4799), 0);
  assert.equal(findDisplayCueIndex(shown, 4800), 1);
  assert.equal(cueAnimationMs([1000, 5000, 9000], 0, 1, 4800), 200);
  assert.equal(nextDisplayCuePosition(shown, 4800), 8800);
  assert.equal(nextDisplayCuePosition(shown, 9000), null);
});

test('a gap under 200 ms shortens the lead and the animation to the gap', () => {
  const starts = [1000, 1120, 3000];
  const shown = displayCuePositions(starts, 0, true);
  assert.deepEqual(shown, [800, 1000, 2800]);
  assert.equal(cueAnimationMs(starts, 0, 1, 1000), 120);
});

test('the first line is never shown before position 0', () => {
  assert.deepEqual(displayCuePositions([100, 2000], 0, true), [0, 1800]);
  assert.deepEqual(displayCuePositions([0, 2000], 0, true), [0, 1800]);
  assert.equal(findDisplayCueIndex(displayCuePositions([100], 0, true), 0), 0);
});

test('the user offset still applies on top of the lead', () => {
  assert.deepEqual(displayCuePositions([1000, 5000], 200, true), [600, 4600]);
  assert.equal(cueAnimationMs([1000, 5000], 200, 1, 4600), 200);
});

test('paused playback does not lead; resuming inside the lead animates for the time left', () => {
  const starts = [1000, 5000];
  const paused = displayCuePositions(starts, 0, false);
  assert.equal(findDisplayCueIndex(paused, 4850), 0);
  const playing = displayCuePositions(starts, 0, true);
  assert.equal(findDisplayCueIndex(playing, 4850), 1);
  assert.equal(cueAnimationMs(starts, 0, 1, 4850), 150);
  assert.equal(cueAnimationMs(starts, 0, 1, 5100), 0);
});

test('the cue timer fires on time while the playback clock is converging', () => {
  const sample = (positionMs) => ({ trackId: 'a', positionMs, playing: true, durationMs: 60_000 });
  const clock = correctPlaybackClock(correctPlaybackClock(null, sample(1000), 0), sample(1600), 500);
  const shown = displayCuePositions([3000, 8000], 0, true);
  const next = nextDisplayCuePosition(shown, estimatePlaybackPosition(clock, 500));
  assert.equal(next, 2800);
  const wait = msUntilPlaybackPosition(clock, 500, next);
  const position = estimatePlaybackPosition(clock, 500 + wait);
  assert.ok(Math.abs(position - 2800) < 1e-6);
  assert.equal(findDisplayCueIndex(shown, position), 0);
  assert.equal(cueAnimationMs([3000, 8000], 0, 0, position), 200);
});

test('easeOutCubic runs from 0 to 1', () => {
  assert.equal(easeOutCubic(0), 0);
  assert.equal(easeOutCubic(1), 1);
  assert.ok(easeOutCubic(0.5) > 0.5);
});

test('no hard-coded lyric transition times; the taskbar lyrics switch without a fade', () => {
  const view = readFileSync(path.join(root, 'src/lib/LyricsView.svelte'), 'utf8');
  assert.doesNotMatch(view, /\b140ms\b/);
  assert.match(view, /transition: color var\(--lyric-cue-ms\) ease/);
  assert.match(view, /--lyric-cue-ms:\$\{/);
  assert.doesNotMatch(view, /behavior: 'smooth'/);
  const taskbar = readFileSync(path.join(root, 'src/lib/TaskbarLyricsView.svelte'), 'utf8');
  assert.doesNotMatch(taskbar, /in:fade|svelte\/transition|\b120\b/);
});
