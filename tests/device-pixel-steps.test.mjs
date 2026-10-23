import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { devicePixelEasing, devicePixelSteps, steppedPosition } from '../src/lib/device-pixel-steps.js';
import { playbackProgressSteps } from '../src/lib/playback-clock.ts';

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');

test('one step per device pixel of the distance, at least one', () => {
  assert.equal(devicePixelSteps(100, 2.5), 250);
  assert.equal(devicePixelSteps(-100, 2), 200);
  assert.equal(devicePixelSteps(0.8, 2.5), 2);
  assert.equal(devicePixelSteps(0, 2), 1);
  assert.equal(devicePixelSteps(Number.NaN, 2), 1);
  assert.equal(devicePixelSteps(10, 0), 10, 'a missing ratio counts as 1');
  assert.equal(devicePixelSteps(0.8, 2.5000000001), 2, 'float noise does not add a step');
  assert.equal(devicePixelEasing(40, 2), 'steps(80, end)');
});

test('the stepped position never lags more than one device pixel and lands on the target', () => {
  for (const [from, to, ratio] of [[0, -333.3, 2.5], [12.4, 812.9, 2], [5, 5.3, 2.5], [0, 1920, 1.25]]) {
    const steps = devicePixelSteps(to - from, ratio);
    for (let i = 0; i <= 1000; i += 1) {
      const p = i / 1000;
      const linear = from + (to - from) * p;
      const shown = steppedPosition(from, to, steps, p);
      assert.ok(Math.abs(linear - shown) * ratio <= 1 + 1e-9, `${from}->${to} at ${p}`);
    }
    assert.equal(steppedPosition(from, to, steps, 1), to);
    assert.equal(steppedPosition(from, to, steps, 0), from);
  }
});

test('slow moves change position on evenly spaced frames (no bunching)', () => {
  // 30 device px over 1 s at 120 Hz: every 4th frame moves exactly one pixel.
  const steps = devicePixelSteps(12, 2.5);
  const changes = [];
  let last = steppedPosition(0, 12, steps, 0);
  for (let frame = 1; frame <= 120; frame += 1) {
    const now = steppedPosition(0, 12, steps, frame / 120);
    if (now !== last) changes.push(frame);
    last = now;
  }
  const gaps = changes.slice(1).map((frame, i) => frame - changes[i]);
  assert.equal(changes.length, 30);
  assert.ok(gaps.every((gap) => gap === 4), String(gaps));
});

test('the progress bar steps per device pixel of the rest of the window width', () => {
  assert.equal(playbackProgressSteps(0, 1536, 2.5), 3840);
  assert.equal(playbackProgressSteps(0.25, 1536, 2.5), 2880);
  assert.equal(playbackProgressSteps(1, 1536, 2.5), 1);
  assert.equal(playbackProgressSteps(0.5, 1000, 2), 1000, 'a monitor at 200 % gets its own steps');
});

test('every continuous position animation uses device-pixel steps, recomputed with the ratio', () => {
  const progress = read('src/lib/PlaybackProgress.svelte');
  assert.match(progress, /steps\(\$\{run\.steps\}, end\) forwards/);
  assert.match(progress, /watchDevicePixelRatio/);
  // The taskbar / floating lyrics are native (ADR 0034): their glide and scroll sample whole device pixels.
  const lyrics = read('crates/player-lyrics-native/src/win.rs');
  assert.equal((lyrics.match(/DCOMPOSITION_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR/g) ?? []).length, 2, 'the glide and the long-line scroll');
  const marquee = read('src/lib/dock-marquee.ts');
  assert.match(marquee, /devicePixelEasing\(distance/);
  assert.match(marquee, /watchDevicePixelRatio\(update\)/);
});
