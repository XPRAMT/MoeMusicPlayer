import assert from 'node:assert/strict';
import test from 'node:test';
import {
  CONVERGE_MS,
  correctPlaybackClock,
  estimatePlaybackPosition,
  msUntilPlaybackPosition,
  nextCuePosition,
  playbackClockPastEnd,
  playbackClockRunning,
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

test('a stalled stream freezes the clock instead of a 0.5 s sawtooth, then it catches up', () => {
  // The backend says "playing" while the position is stuck at 1000 (a remote stream waiting for data).
  const samples = [[0, 1000], [500, 1000], [1000, 1000], [1500, 1000], [2000, 1200], [2500, 1700], [3000, 2200], [3500, 2700]];
  let clock = null;
  let last = -Infinity;
  const jumpIds = new Set();
  let runKeys = 0;
  for (const [at, position] of samples) {
    const previousRunKey = clock?.runKey;
    clock = correctPlaybackClock(clock, sample(position), at);
    if (previousRunKey !== undefined && clock.runKey !== previousRunKey) runKeys += 1;
    jumpIds.add(clock.jumpId);
    // Sample densely between polls: the shown position never goes back.
    for (let t = at; t < at + 500; t += 10) {
      const shown = estimatePlaybackPosition(clock, t);
      assert.ok(shown >= last - 1e-9, `moved back at ${t}: ${shown} < ${last}`);
      last = shown;
    }
  }
  assert.equal(jumpIds.size, 1, 'a stall never bumps jumpId');
  assert.equal(runKeys, 2, 'one freeze and one resume');
  assert.equal(clock.frozen, false);
  // Caught up: from the last sample it runs with the real position again.
  assert.ok(Math.abs(estimatePlaybackPosition(clock, 3500 + CONVERGE_MS) - (2700 + CONVERGE_MS)) < 1);
});

test('a frozen clock reports the frozen position and is not running', () => {
  const clock = correctPlaybackClock(correctPlaybackClock(null, sample(1000), 0), sample(1000), 500);
  assert.equal(clock.frozen, true);
  assert.equal(clock.playing, true);
  assert.equal(playbackClockRunning(clock), false);
  assert.equal(estimatePlaybackPosition(clock, 500), 1500, 'held where it was shown, not 1000');
  assert.equal(estimatePlaybackPosition(clock, 5000), 1500);
  assert.equal(msUntilPlaybackPosition(clock, 600, 2000), null);
  assert.equal(playbackClockPastEnd(clock, 1e9), false);
  assert.deepEqual(playbackProgressRun(clock, 600), { ratio: 0.15, remainingMs: null });
  // Still stuck: the same object, so nothing reruns.
  assert.equal(correctPlaybackClock(clock, sample(1000), 1000), clock);
  // Still behind but moving again within JUMP_DEVIATION_MS: resumes from 1500 slower than 1, never back.
  const resumed = correctPlaybackClock(clock, sample(1300), 1500);
  assert.equal(resumed.frozen, false);
  assert.equal(resumed.jumpId, clock.jumpId);
  assert.equal(resumed.runKey, clock.runKey + 1);
  assert.equal(estimatePlaybackPosition(resumed, 1500), 1500);
  assert.ok(resumed.rate > 0 && resumed.rate < 1);
  assert.equal(playbackProgressRun(resumed, 1500).remainingMs, 8500);
});

test('frozen and stuck while far behind waits; a real catch-up resumes at rate 1', () => {
  let clock = correctPlaybackClock(correctPlaybackClock(null, sample(1000), 0), sample(1000), 500); // frozen at 1500
  clock = correctPlaybackClock(clock, sample(1100), 1000); // creeping (100 in 500 ms): still waiting
  assert.equal(clock.frozen, true);
  clock = correctPlaybackClock(clock, sample(1510), 1100); // short wall time but caught up
  assert.equal(clock.frozen, false);
  assert.equal(clock.rate, 1);
  assert.equal(estimatePlaybackPosition(clock, 1600), 2000);
});

test('genuine jumps still jump while playing, frozen or not', () => {
  const clock = correctPlaybackClock(null, sample(1000), 0);
  const frozen = correctPlaybackClock(clock, sample(1000), 500);
  for (const base of [clock, frozen]) {
    const ahead = correctPlaybackClock(base, sample(3000), 500);
    assert.equal(ahead.jumpId, base.jumpId + 1, 'ahead by more than JUMP_DEVIATION_MS');
    assert.equal(estimatePlaybackPosition(ahead, 500), 3000);
    assert.equal(correctPlaybackClock(base, sample(100, true, 'b'), 1000).jumpId, base.jumpId + 1, 'new track');
    assert.equal(correctPlaybackClock(base, sample(1000, false), 1000).jumpId, base.jumpId + 1, 'pause');
    assert.equal(correctPlaybackClock(base, sample(1000, true, 'a', 20_000), 1000).jumpId, base.jumpId + 1, 'duration');
    assert.equal(correctPlaybackClock(base, sample(1000), 1000, { force: true }).jumpId, base.jumpId + 1, 'force');
  }
  // Moved back far (an external seek), not a stall: jumps back at once.
  const back = correctPlaybackClock(clock, sample(200), 500);
  assert.equal(back.jumpId, clock.jumpId + 1);
  assert.equal(estimatePlaybackPosition(back, 500), 200);
  const backWhileFrozen = correctPlaybackClock(frozen, sample(200), 1000);
  assert.equal(backWhileFrozen.jumpId, frozen.jumpId + 1);
  assert.equal(backWhileFrozen.frozen, false);
  // Stuck but the estimate is far more than STALL_HOLD_MAX_MS ahead (a long gap between samples): jumps.
  assert.equal(correctPlaybackClock(clock, sample(1000), 3000).jumpId, clock.jumpId + 1);
  // A short gap between samples is never judged a stall.
  assert.equal(correctPlaybackClock(clock, sample(1000), 200).frozen, false);
});

test('ignored samples keep the same clock but still record the sample for stall detection', () => {
  let clock = correctPlaybackClock(null, sample(1000), 0);
  const same = correctPlaybackClock(clock, sample(1500), 500);
  assert.equal(same, clock);
  // Stuck from here: the stall is judged against the 1500 sample at 500, not the 1000 one at 0.
  clock = correctPlaybackClock(same, sample(1500), 1000);
  assert.equal(clock.frozen, true);
  assert.equal(estimatePlaybackPosition(clock, 1000), 2000);
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
