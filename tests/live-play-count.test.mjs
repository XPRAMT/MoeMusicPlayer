import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  applyLivePlayCountSample,
  createLivePlayCountState,
  getLivePlayedMs,
  livePlayCountValues,
  rememberLivePlayCount,
  toLivePlayCountView,
  withLivePlayedMs,
} from '../src/lib/live-play-count.js';

test('live play count credits natural position advances while playing only', () => {
  let state = createLivePlayCountState();
  state = applyLivePlayCountSample(state, {
    trackId: 't1',
    playedMs: 100_000,
    positionMs: 1_000,
    isPlaying: true,
  });
  assert.equal(getLivePlayedMs(state), 100_000);

  state = applyLivePlayCountSample(state, {
    trackId: 't1',
    playedMs: 100_000,
    positionMs: 2_000,
    isPlaying: true,
  });
  assert.equal(getLivePlayedMs(state), 101_000);

  state = applyLivePlayCountSample(state, {
    trackId: 't1',
    playedMs: 100_000,
    positionMs: 2_250,
    isPlaying: false,
  });
  assert.equal(getLivePlayedMs(state), 101_000);

  state = applyLivePlayCountSample(state, {
    trackId: 't1',
    playedMs: 100_000,
    positionMs: 3_000,
    isPlaying: true,
  });
  // resumed: no credit until a subsequent playing→playing advance
  assert.equal(getLivePlayedMs(state), 101_000);
  state = applyLivePlayCountSample(state, {
    trackId: 't1',
    playedMs: 100_000,
    positionMs: 3_500,
    isPlaying: true,
  });
  assert.equal(getLivePlayedMs(state), 101_500);
});

test('seek jumps and track changes do not leak credit onto other tracks', () => {
  let state = applyLivePlayCountSample(createLivePlayCountState(), {
    trackId: 't1',
    playedMs: 50_000,
    positionMs: 10_000,
    isPlaying: true,
  });
  state = applyLivePlayCountSample(state, {
    trackId: 't1',
    playedMs: 50_000,
    positionMs: 40_000,
    isPlaying: true,
  });
  assert.equal(getLivePlayedMs(state), 50_000, 'large seek jump is ignored');

  state = applyLivePlayCountSample(state, {
    trackId: 't2',
    playedMs: 10_000,
    positionMs: 100,
    isPlaying: true,
  });
  assert.equal(getLivePlayedMs(state), 10_000);
  assert.equal(toLivePlayCountView(state)?.trackId, 't2');

  const other = withLivePlayedMs({ id: 't1', playedMs: 50_000, durationMs: 100_000 }, toLivePlayCountView(state));
  assert.equal(other.playedMs, 50_000, 'other tracks keep their own playedMs');
  const current = withLivePlayedMs({ id: 't2', playedMs: 10_000, durationMs: 100_000 }, toLivePlayCountView(state));
  assert.equal(current.playedMs, 10_000);
});

test('persisted playedMs catch-up rebases the live session', () => {
  let state = applyLivePlayCountSample(createLivePlayCountState(), {
    trackId: 't1',
    playedMs: 100_000,
    positionMs: 0,
    isPlaying: true,
  });
  state = applyLivePlayCountSample(state, {
    trackId: 't1',
    playedMs: 100_000,
    positionMs: 1_000,
    isPlaying: true,
  });
  assert.equal(getLivePlayedMs(state), 101_000);
  state = applyLivePlayCountSample(state, {
    trackId: 't1',
    playedMs: 105_000,
    positionMs: 1_200,
    isPlaying: true,
  });
  assert.equal(getLivePlayedMs(state), 105_000);
});

test('withLivePlayedMs overlays playlist trackId rows', () => {
  const live = { trackId: 'abc', playedMs: 12_345 };
  const row = withLivePlayedMs({ trackId: 'abc', playedMs: 1_000, title: 'x' }, live);
  assert.equal(row.playedMs, 12_345);
  assert.equal(row.title, 'x');
  assert.equal(withLivePlayedMs({ trackId: 'zzz', playedMs: 1_000 }, live).playedMs, 1_000);
});

test('play-count rows keep completed in-session totals after the current track changes', () => {
  let state = applyLivePlayCountSample(createLivePlayCountState(), {
    trackId: 'temporary-a', playedMs: 90_000, positionMs: 1_000, isPlaying: true,
  });
  state = applyLivePlayCountSample(state, {
    trackId: 'temporary-a', playedMs: 90_000, positionMs: 2_000, isPlaying: true,
  });
  assert.equal(getLivePlayedMs(state), 91_000);
  const remembered = new Map();
  rememberLivePlayCount(remembered, toLivePlayCountView(state));
  state = applyLivePlayCountSample(state, {
    trackId: 'temporary-b', playedMs: 0, positionMs: 0, isPlaying: true,
  });

  assert.equal(
    withLivePlayedMs({ id: 'temporary-a', playedMs: 0, durationMs: 100_000 }, livePlayCountValues(remembered)).playedMs,
    91_000,
    'cached list rows must not revert when another song becomes current',
  );
});

test('play-count overlays never replace a newer SQLite value with an older live value', () => {
  assert.equal(
    withLivePlayedMs(
      { id: 'temporary-a', playedMs: 103_000 },
      { trackId: 'temporary-a', playedMs: 91_000 },
    ).playedMs,
    103_000,
  );
});
