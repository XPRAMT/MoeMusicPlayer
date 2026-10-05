import assert from 'node:assert/strict';
import test from 'node:test';
import {
  clampLyricsTimingOffsetSec,
  formatLyricsTimingOffsetSec,
  loadLyricsTimingOffsetSec,
  lyricsTimingDelaySecToOffsetMs,
  saveLyricsTimingOffsetSec,
} from '../src/lib/lyrics-timing-offset.js';

test('clampLyricsTimingOffsetSec clamps, steps, and rejects non-finite', () => {
  assert.equal(clampLyricsTimingOffsetSec(0), 0);
  assert.equal(clampLyricsTimingOffsetSec(0.14), 0.1);
  assert.equal(clampLyricsTimingOffsetSec(0.15), 0.2);
  assert.equal(clampLyricsTimingOffsetSec(-5.04), -5);
  assert.equal(clampLyricsTimingOffsetSec(5.2), 5);
  assert.equal(clampLyricsTimingOffsetSec(Number.NaN), 0);
  assert.equal(clampLyricsTimingOffsetSec('1.3'), 1.3);
});

test('lyricsTimingDelaySecToOffsetMs maps positive delay to negative offsetMs', () => {
  assert.equal(lyricsTimingDelaySecToOffsetMs(1.5), -1500);
  assert.equal(lyricsTimingDelaySecToOffsetMs(-0.2), 200);
  assert.equal(lyricsTimingDelaySecToOffsetMs(0), 0);
});

test('formatLyricsTimingOffsetSec keeps one decimal and sign', () => {
  assert.equal(formatLyricsTimingOffsetSec(0), '0.0 秒');
  assert.equal(formatLyricsTimingOffsetSec(1.2), '+1.2 秒');
  assert.equal(formatLyricsTimingOffsetSec(-0.5), '-0.5 秒');
});

test('save/load persists per track in localStorage', () => {
  const store = new Map();
  globalThis.localStorage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => { store.set(key, String(value)); },
    removeItem: (key) => { store.delete(key); },
  };
  assert.equal(loadLyricsTimingOffsetSec('track-a'), 0);
  assert.equal(saveLyricsTimingOffsetSec('track-a', 1.25), 1.3);
  assert.equal(loadLyricsTimingOffsetSec('track-a'), 1.3);
  assert.equal(saveLyricsTimingOffsetSec('track-a', 0), 0);
  assert.equal(loadLyricsTimingOffsetSec('track-a'), 0);
  assert.equal(saveLyricsTimingOffsetSec('track-b', -2), -2);
  assert.equal(loadLyricsTimingOffsetSec('track-b'), -2);
  assert.equal(loadLyricsTimingOffsetSec('track-a'), 0);
});
