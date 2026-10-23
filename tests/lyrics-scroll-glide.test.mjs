import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import {
  glidePosition,
  planTimedScroll,
  glideVelocity,
  restingGlide,
  retargetGlide,
  snapToDevicePixel,
  timedNavigationKind,
} from '../src/lib/lyrics-scroll-glide.ts';
import { LYRIC_OFFSET_GLIDE_MS, cueAnimationMs, displayCuePositions, easeOutCubic, findDisplayCueIndex } from '../src/lib/lyrics-cue-timing.ts';

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

test('the delay glide uses the one 200 ms lyric animation length', () => {
  assert.equal(LYRIC_OFFSET_GLIDE_MS, 200);
});

test('a glide that already ended restarts from the real scroll position at rest', () => {
  const old = retargetGlide(null, 0, 100, 0, LYRIC_OFFSET_GLIDE_MS);
  const next = retargetGlide(old, 100, 400, 1000, LYRIC_OFFSET_GLIDE_MS);
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
  assert.match(view, /snapToDevicePixel\(/);
});

// Simulates the Now Playing list: lines every 1000 ms, 50 px each; the target of line i is i * 50 px. Delay steps every
// `stepEveryMs` while playback crosses natural cue boundaries (and their 200 ms lead windows). Frames are 60 Hz.
function simulate({ steps, stepEveryMs, startPositionMs, durationMs }) {
  const starts = Array.from({ length: 20 }, (_, i) => 1000 * (i + 1));
  const target = (index) => Math.max(0, index) * 50;
  let offset = 0;
  let stepIndex = 0;
  let glide = null;
  let scrollTop = target(findDisplayCueIndex(displayCuePositions(starts, offset, true), startPositionMs));
  let previous = { index: findDisplayCueIndex(displayCuePositions(starts, offset, true), startPositionMs), offset };
  const frames = [];
  for (let now = 0; now <= durationMs; now += 1000 / 60) {
    if (stepIndex < steps.length && now >= (stepIndex + 1) * stepEveryMs) offset += steps[stepIndex++];
    // Fresh position for every decision (the fix: the index never uses a stale estimate after a delay step).
    const position = startPositionMs + now;
    const shown = displayCuePositions(starts, offset, true);
    const index = findDisplayCueIndex(shown, position);
    if (index !== previous.index) {
      const kind = timedNavigationKind({
        sameTrackAndLayout: true, seeked: false, offsetChanged: previous.offset !== offset, playing: true,
        previousIndex: previous.index, index, glideRunning: glide !== null && now < glide.startedAt + glide.durationMs,
      });
      const animationMs = cueAnimationMs(starts, offset, index, position);
      const running = glide !== null && now < glide.startedAt + glide.durationMs;
      const plan = planTimedScroll({ kind, glideRunning: running, animationMs, reducedMotion: false, distancePx: target(index) - scrollTop });
      if (plan === 'retarget') glide = retargetGlide(running ? glide : null, scrollTop, target(index), now, LYRIC_OFFSET_GLIDE_MS);
      else if (plan === 'rest') glide = restingGlide(scrollTop, target(index), now, animationMs);
      else if (plan === 'instant') { glide = null; scrollTop = target(index); }
    }
    previous = { index, offset };
    if (glide) scrollTop = snapToDevicePixel(glidePosition(glide, now), 1);
    frames.push({ now, scrollTop, index });
  }
  const settled = glide ? glidePosition(glide, Number.POSITIVE_INFINITY) : scrollTop;
  return { frames, settled, finalTarget: target(previous.index) };
}

test('rapid delay steps across a natural cue change glide without a jump and end on the right line', () => {
  for (const stepEveryMs of [50, 70, 100]) {
    for (const startPositionMs of [700, 820, 900, 980]) { // before, at the start of, inside and at the end of the lead window
      for (const pattern of [[100, 100, 100, 100], [-100, -100, -100], [100, -100, 100, -100, 100], [300, -300, 300]]) {
        const { frames, settled, finalTarget } = simulate({ steps: pattern, stepEveryMs, startPositionMs, durationMs: 1500 });
        for (let i = 1; i < frames.length; i += 1) {
          const delta = Math.abs(frames[i].scrollTop - frames[i - 1].scrollTop);
          assert.ok(delta <= 16, `jump of ${delta} px at ${frames[i].now.toFixed(0)} ms (every ${stepEveryMs} ms from ${startPositionMs}, ${pattern})`);
        }
        assert.equal(settled, finalTarget);
      }
    }
  }
});

test('a cue advance during a running glide retargets it; a standalone advance is the ADR 0029 ease-out', () => {
  assert.equal(planTimedScroll({ kind: 'cue', glideRunning: true, animationMs: 0, reducedMotion: false, distancePx: 50 }), 'retarget');
  assert.equal(planTimedScroll({ kind: 'cue', glideRunning: false, animationMs: 200, reducedMotion: false, distancePx: 50 }), 'rest');
  assert.equal(planTimedScroll({ kind: 'cue', glideRunning: false, animationMs: 5, reducedMotion: false, distancePx: 50 }), 'instant');
  assert.equal(planTimedScroll({ kind: 'glide', glideRunning: true, animationMs: 0, reducedMotion: true, distancePx: 50 }), 'instant');
  assert.equal(planTimedScroll({ kind: 'instant', glideRunning: true, animationMs: 200, reducedMotion: false, distancePx: 50 }), 'instant');
  assert.equal(timedNavigationKind({ sameTrackAndLayout: true, seeked: false, offsetChanged: false, playing: true, previousIndex: 2, index: 5, glideRunning: true }), 'glide');
  assert.equal(timedNavigationKind({ sameTrackAndLayout: true, seeked: true, offsetChanged: false, playing: true, previousIndex: 2, index: 3, glideRunning: true }), 'instant');
});

test('the shown index is computed from a fresh position after every delay step', () => {
  const view = readFileSync(path.join(root, 'src/lib/LyricsView.svelte'), 'utf8');
  assert.match(view, /let cuePositionMs = \$derived\.by\(\(\) => \{\s*cueTick;\s*timedOffsetMs;/);
  assert.match(view, /planTimedScroll\(\{ kind, glideRunning: cueGlide !== null/);
});
