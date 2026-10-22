import assert from 'node:assert/strict';
import test from 'node:test';
import {
  CONVERGE_MS,
  correctPlaybackClock,
  estimatePlaybackPosition,
  msUntilPlaybackPosition,
  nextCuePosition,
  playbackClockPastEnd,
  playbackProgressRun,
} from '../src/lib/playback-clock.ts';
import { playbackSeekDisplayPosition } from '../src/lib/playback-scrubber.ts';

const sample = (positionMs, playing = true, trackId = 'a', durationMs = 10_000) => ({ trackId, positionMs, playing, durationMs });

test('a playing clock advances at rate 1 and a paused one stays put', () => {
  const playing = correctPlaybackClock(null, sample(1000), 0);
  assert.equal(estimatePlaybackPosition(playing, 500), 1500);
  const paused = correctPlaybackClock(null, sample(1000, false), 0);
  assert.equal(estimatePlaybackPosition(paused, 5000), 1000);
  assert.equal(estimatePlaybackPosition(null, 100), 0);
});

test('the estimate stays between 0 and the track length', () => {
  const clock = correctPlaybackClock(null, sample(9_900), 0);
  assert.equal(estimatePlaybackPosition(clock, 1000), 10_000);
  assert.equal(playbackClockPastEnd(clock, 1000), true);
  assert.equal(playbackClockPastEnd(clock, 50), false);
  assert.equal(estimatePlaybackPosition(correctPlaybackClock(null, sample(-50), 0), 0), 0);
  const unknown = correctPlaybackClock(null, sample(1000, true, 'a', null), 0);
  assert.equal(estimatePlaybackPosition(unknown, 1000), 2000);
  assert.equal(playbackClockPastEnd(unknown, 1e9), false);
});

test('a deviation under 30 ms is ignored', () => {
  const clock = correctPlaybackClock(null, sample(1000), 0);
  assert.equal(correctPlaybackClock(clock, sample(1520), 500), clock);
  assert.equal(correctPlaybackClock(clock, sample(1480), 500), clock);
});

test('a 30–250 ms deviation converges smoothly within CONVERGE_MS', () => {
  const clock = correctPlaybackClock(null, sample(1000), 0);
  const ahead = correctPlaybackClock(clock, sample(1600), 500); // 100 ms behind the shell
  assert.equal(estimatePlaybackPosition(ahead, 500), 1500, 'no jump at the correction');
  assert.ok(Math.abs(estimatePlaybackPosition(ahead, 500 + CONVERGE_MS) - (1600 + CONVERGE_MS)) < 1e-6);
  assert.ok(Math.abs(estimatePlaybackPosition(ahead, 2000 + CONVERGE_MS) - (3100 + CONVERGE_MS)) < 1e-6, 'rate 1 afterwards');
});

test('a small backwards deviation slows the clock but never moves it back', () => {
  const clock = correctPlaybackClock(null, sample(1000), 0);
  const behind = correctPlaybackClock(clock, sample(1300), 500); // estimate 1500, shell 1300
  let last = estimatePlaybackPosition(behind, 500);
  assert.equal(last, 1500);
  for (let t = 510; t <= 500 + CONVERGE_MS + 200; t += 10) {
    const next = estimatePlaybackPosition(behind, t);
    assert.ok(next >= last, `moved back at ${t}`);
    last = next;
  }
  assert.ok(Math.abs(estimatePlaybackPosition(behind, 500 + CONVERGE_MS) - (1300 + CONVERGE_MS)) < 1e-6);
});

test('a large deviation, a new track or a play/pause change jumps', () => {
  const clock = correctPlaybackClock(null, sample(1000), 0);
  assert.equal(estimatePlaybackPosition(correctPlaybackClock(clock, sample(5000), 500), 500), 5000);
  assert.equal(estimatePlaybackPosition(correctPlaybackClock(clock, sample(1300), 500), 500), 1500 - 0, 'within window converges');
  assert.equal(estimatePlaybackPosition(correctPlaybackClock(clock, sample(100, true, 'b'), 500), 500), 100);
  const paused = correctPlaybackClock(clock, sample(1450, false), 500);
  assert.equal(estimatePlaybackPosition(paused, 900), 1450);
  assert.equal(estimatePlaybackPosition(correctPlaybackClock(clock, sample(1510), 500, { force: true }), 500), 1510);
});

test('a drag draft still wins over the estimate', () => {
  const clock = correctPlaybackClock(null, sample(1000), 0);
  assert.equal(playbackSeekDisplayPosition(estimatePlaybackPosition(clock, 500), 10_000, 7000), 7000);
  assert.equal(playbackSeekDisplayPosition(estimatePlaybackPosition(clock, 500), 10_000, null), 1500);
});

test('only a jump bumps jumpId, so a smooth correction does not restart the CSS run', () => {
  const clock = correctPlaybackClock(null, sample(1000), 0);
  const smooth = correctPlaybackClock(clock, sample(1600), 500);
  assert.equal(smooth.jumpId, clock.jumpId);
  assert.equal(correctPlaybackClock(clock, sample(5000), 500).jumpId, clock.jumpId + 1);
  assert.equal(correctPlaybackClock(clock, sample(1500, false), 500).jumpId, clock.jumpId + 1);
});

test('msUntilPlaybackPosition wakes at the target, including during convergence', () => {
  const clock = correctPlaybackClock(null, sample(1000), 0);
  assert.equal(msUntilPlaybackPosition(clock, 0, 3000), 2000);
  assert.equal(msUntilPlaybackPosition(clock, 0, 500), 0);
  assert.equal(msUntilPlaybackPosition(correctPlaybackClock(null, sample(1000, false), 0), 0, 3000), null);
  const converging = correctPlaybackClock(clock, sample(1600), 500);
  for (const target of [1600, 2400, 4000]) {
    const wait = msUntilPlaybackPosition(converging, 500, target);
    assert.ok(Math.abs(estimatePlaybackPosition(converging, 500 + wait) - target) < 1e-6, `target ${target}`);
  }
});

test('nextCuePosition finds the next line start after the offset', () => {
  assert.equal(nextCuePosition([0, 1000, 2000], 0, 1000), 2000);
  assert.equal(nextCuePosition([0, 1000, 2000], 200, 500), 800);
  assert.equal(nextCuePosition([0, 1000], 0, 1500), null);
});

test('playbackProgressRun runs linearly to the end only while playing', () => {
  const clock = correctPlaybackClock(null, sample(2500), 0);
  assert.deepEqual(playbackProgressRun(clock, 0), { ratio: 0.25, remainingMs: 7500 });
  assert.deepEqual(playbackProgressRun(correctPlaybackClock(null, sample(2500, false), 0), 0), { ratio: 0.25, remainingMs: null });
  assert.deepEqual(playbackProgressRun(correctPlaybackClock(null, sample(2500, true, 'a', null), 0), 0), { ratio: 0, remainingMs: null });
});
