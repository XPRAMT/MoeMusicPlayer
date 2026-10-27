import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  BUFFERING_POLL_MS,
  BUFFERING_PULSE_DELAY_MS,
  BUFFERING_PULSE_PERIOD_MS,
  bufferingCueVisible,
  bufferingPulseStartMs,
  bufferingPulseStopMs,
  msUntilBufferingPulse,
  nextBufferingPulse,
  playbackPollIntervalMs,
} from '../src/lib/buffering-cue.js';

const css = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
const progressSource = readFileSync(new URL('../src/lib/PlaybackProgress.svelte', import.meta.url), 'utf8');
const appSource = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
const progressRunSource = readFileSync(new URL('../src/lib/progress-run.js', import.meta.url), 'utf8');

test('the cue shows exactly while the state is buffering', () => {
  assert.equal(bufferingCueVisible('buffering'), true);
  for (const state of ['playing', 'paused', 'stopped', 'empty', 'Buffering', '', null, undefined]) {
    assert.equal(bufferingCueVisible(state), false, String(state));
  }
});

test('the pulse is a 1000 ms brightness 1 -> 0.8 animation on the thumb only', () => {
  const rule = css.match(/\.progress-row\[data-buffering\] \.progress-thumb \{([^}]*)\}/);
  assert.ok(rule, 'thumb buffering rule');
  assert.equal(BUFFERING_PULSE_PERIOD_MS, 1000);
  assert.match(rule[1], /animation:\s*progress-thumb-buffering\s+1000ms\s+ease-in-out\s+infinite/);
  const keyframes = css.match(/@keyframes progress-thumb-buffering \{([\s\S]*?)\n\}/);
  assert.ok(keyframes, 'keyframes');
  const props = [...keyframes[1].matchAll(/([a-z-]+)\s*:/g)].map((match) => match[1]);
  assert.deepEqual([...new Set(props)], ['filter']);
  assert.match(keyframes[1], /brightness\(1\)/);
  assert.match(keyframes[1], /brightness\(0\.8\)/);
  // Only the thumb uses the keyframes.
  const users = [...css.matchAll(/([^{}]+)\{[^{}]*progress-thumb-buffering[^{}]*\}/g)].map((match) => match[1].trim());
  assert.deepEqual(users, ['.progress-row[data-buffering] .progress-thumb']);
  assert.doesNotMatch(progressRunSource, /buffering/);
});

test('reduced motion turns the pulse off', () => {
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{\s*\.progress-row\[data-buffering\] \.progress-thumb \{\s*animation: none;/);
});

test('a started cycle always plays to its end', () => {
  // Buffering 0.3 s still pulses the full first second.
  assert.equal(bufferingPulseStopMs(0, 300), 1000);
  // Buffering 2.4 s stops at the end of the third cycle.
  assert.equal(bufferingPulseStopMs(0, 2400), 3000);
  // Ending exactly on a boundary stops there; ending at once still plays one cycle.
  assert.equal(bufferingPulseStopMs(0, 2000), 2000);
  assert.equal(bufferingPulseStopMs(500, 500), 1500);
});

test('the pulsing flag only stops at a cycle boundary and carries on when buffering resumes', () => {
  let pulsing = false;
  pulsing = nextBufferingPulse(pulsing, { type: 'state', buffering: true });
  assert.equal(pulsing, true, 'starts at once');
  pulsing = nextBufferingPulse(pulsing, { type: 'cycle-end', buffering: true });
  assert.equal(pulsing, true, 'keeps going while buffering');
  pulsing = nextBufferingPulse(pulsing, { type: 'state', buffering: false });
  assert.equal(pulsing, true, 'buffering ended mid-cycle: the cycle runs on');
  pulsing = nextBufferingPulse(pulsing, { type: 'state', buffering: true });
  assert.equal(pulsing, true, 'buffering again in the last cycle: no restart, no break');
  pulsing = nextBufferingPulse(pulsing, { type: 'state', buffering: false });
  pulsing = nextBufferingPulse(pulsing, { type: 'cycle-end', buffering: false });
  assert.equal(pulsing, false, 'stops at the cycle end');
  assert.equal(nextBufferingPulse(false, { type: 'state', buffering: false }), false);
  // Reduced motion: no animation, hence no boundary, so it stops at once.
  assert.equal(nextBufferingPulse(true, { type: 'state', buffering: false, reducedMotion: true }), false);
});

test('the pulse waits for 1 s of continuous buffering', () => {
  assert.equal(BUFFERING_PULSE_DELAY_MS, 1000);
  // Buffering 0.8 s: never pulses.
  assert.equal(bufferingPulseStartMs(0, 800), null);
  assert.equal(bufferingPulseStartMs(1000, 1999), null);
  // Buffering 1.3 s: starts at 1.0 s and still plays its full cycle, stopping at 2.0 s.
  const start = bufferingPulseStartMs(0, 1300);
  assert.equal(start, 1000);
  assert.equal(bufferingPulseStopMs(start, 1300), 2000);
  // Exactly 1 s counts.
  assert.equal(bufferingPulseStartMs(0, 1000), 1000);
  // One timer from the first sight of buffering.
  assert.equal(msUntilBufferingPulse('buffering', 1000, 1000), 1000);
  assert.equal(msUntilBufferingPulse('buffering', 1000, 1300), 700);
  assert.equal(msUntilBufferingPulse('buffering', 1000, 2400), 0);
  assert.equal(msUntilBufferingPulse('playing', 1000, 1000), null);
  assert.equal(msUntilBufferingPulse(null, 1000, 1000), null);
});

test('the snapshot poll runs every 100 ms only while buffering', () => {
  assert.equal(BUFFERING_POLL_MS, 100);
  assert.equal(playbackPollIntervalMs('buffering', 500), 100);
  for (const state of ['playing', 'paused', 'stopped', 'empty', null, undefined]) {
    assert.equal(playbackPollIntervalMs(state, 500), 500, String(state));
  }
});

test('the app keeps one poll and switches its interval with the state', () => {
  assert.match(appSource, /const desiredPlaybackPollMs = \$derived\(playbackPollIntervalMs\(playback\?\.state, PLAYBACK_POLL_MS\)\)/);
  assert.match(appSource, /function startPlaybackPoll\(intervalMs: number\): void \{\s*if \(playbackPollTimer !== undefined\) clearInterval\(playbackPollTimer\);/);
  assert.equal((appSource.match(/playbackPollTimer = setInterval\(/g) ?? []).length, 1);
});

test('the shared progress bar gates the pulse with one timer and stops it only on animationiteration', () => {
  assert.match(appSource, /<PlaybackProgress[\s\S]*?playbackState=\{playback\?\.state \?\? null\}/);
  assert.match(progressSource, /const buffering = \$derived\(bufferingCueVisible\(playbackState\)\)/);
  assert.match(progressSource, /msUntilBufferingPulse\('buffering', since, since\)/);
  assert.match(progressSource, /return \(\) => clearTimeout\(timer\);\s*\}\);\s*let pulsing/);
  assert.match(progressSource, /type: 'state', buffering: start/);
  assert.match(progressSource, /<span class="progress-thumb" onanimationiteration=\{handlePulseCycleEnd\}><\/span>/);
  assert.match(progressSource, /event\.animationName !== 'progress-thumb-buffering'/);
  assert.match(progressSource, /type: 'cycle-end', buffering \}/);
  assert.doesNotMatch(progressSource, /setInterval|requestAnimationFrame/);
  assert.match(progressSource, /data-buffering=\{pulsing \? 'true' : undefined\}/);
});
