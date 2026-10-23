import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import {
  LYRIC_OFFSET_GLIDE_MS,
  glidePosition,
  glideVelocity,
  restingGlide,
  retargetGlide,
  snapToDevicePixel,
  timedNavigationKind,
} from '../src/lib/lyrics-scroll-glide.ts';
import { easeOutCubic } from '../src/lib/lyrics-cue-timing.ts';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

test('a resting glide is the ADR 0029 ease-out cubic and lands exactly', () => {
  const glide = restingGlide(100, 300, 0, 200);
  for (const t of [0, 0.1, 0.25, 0.5, 0.75, 0.9]) {
    assert.ok(Math.abs(glidePosition(glide, t * 200) - (100 + 200 * easeOutCubic(t))) < 1e-9);
  }
  assert.equal(glidePosition(glide, 200), 300);
  assert.equal(glidePosition(glide, 999), 300);
  assert.equal(glideVelocity(glide, 200), 0);
});

test('retargeting keeps position and speed continuous', () => {
  let glide = retargetGlide(null, 0, 500, 0, LYRIC_OFFSET_GLIDE_MS);
  assert.equal(glide.velocity, 0);
  // Ten delay steps 30 ms apart, each moving the target: never a jump, never a speed step.
  let target = 500;
  for (let step = 1; step <= 10; step += 1) {
    const now = step * 30;
    const before = glidePosition(glide, now);
    const speed = glideVelocity(glide, now);
    target += step % 3 === 0 ? -120 : 80;
    glide = retargetGlide(glide, -1, target, now, LYRIC_OFFSET_GLIDE_MS);
    assert.ok(Math.abs(glide.from - before) < 1e-9, 'position continuous');
    assert.ok(Math.abs(glideVelocity(glide, now) - speed) < 1e-9, 'speed continuous');
    // One frame later it moved only a little (no restart from the old scrollTop).
    assert.ok(Math.abs(glidePosition(glide, now + 16) - before) < 60);
  }
  assert.equal(glidePosition(glide, 300 + LYRIC_OFFSET_GLIDE_MS), target);
});

test('a glide that already ended restarts from the real scroll position at rest', () => {
  const old = retargetGlide(null, 0, 100, 0, 240);
  const next = retargetGlide(old, 100, 400, 1000, 240);
  assert.equal(next.from, 100);
  assert.equal(next.velocity, 0);
});

test('scroll positions are whole device pixels', () => {
  assert.equal(snapToDevicePixel(10.3, 1), 10);
  assert.equal(snapToDevicePixel(10.3, 2), 10.5);
  assert.equal(snapToDevicePixel(10.3, 1.5), 10);
  assert.equal(snapToDevicePixel(7, Number.NaN), 7);
});

test('a delay change glides; seeks, track and layout changes stay instant; the cue advance is unchanged', () => {
  const base = { sameTrackAndLayout: true, seeked: false, offsetChanged: false, playing: true, previousIndex: 3, index: 4 };
  assert.equal(timedNavigationKind(base), 'cue');
  assert.equal(timedNavigationKind({ ...base, index: 6 }), 'instant');
  assert.equal(timedNavigationKind({ ...base, offsetChanged: true, index: 2 }), 'glide');
  assert.equal(timedNavigationKind({ ...base, offsetChanged: true, index: 7, playing: false }), 'glide');
  assert.equal(timedNavigationKind({ ...base, offsetChanged: true, index: -1 }), 'instant');
  assert.equal(timedNavigationKind({ ...base, offsetChanged: true, seeked: true }), 'instant');
  assert.equal(timedNavigationKind({ ...base, offsetChanged: true, sameTrackAndLayout: false }), 'instant');
  assert.equal(timedNavigationKind({ ...base, playing: false }), 'instant');
});

test('LyricsView glides on delay changes and keeps reduced motion instant', () => {
  const view = readFileSync(path.join(root, 'src/lib/LyricsView.svelte'), 'utf8');
  assert.match(view, /timedNavigationKind\(/);
  assert.match(view, /offsetChanged: previous !== null && previous\.offsetMs !== offsetMs/);
  assert.match(view, /!reducedMotion && kind === 'glide'/);
  assert.match(view, /snapToDevicePixel\(/);
});
