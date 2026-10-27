import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  BUFFERING_PULSE_PERIOD_MS,
  bufferingCueVisible,
  bufferingPulseStopMs,
  nextBufferingPulse,
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

test('the shared progress bar stops the pulse only on animationiteration, without timers', () => {
  assert.match(appSource, /<PlaybackProgress[\s\S]*?playbackState=\{playback\?\.state \?\? null\}/);
  assert.match(progressSource, /const buffering = \$derived\(bufferingCueVisible\(playbackState\)\)/);
  assert.match(progressSource, /<span class="progress-thumb" onanimationiteration=\{handlePulseCycleEnd\}><\/span>/);
  assert.match(progressSource, /event\.animationName !== 'progress-thumb-buffering'/);
  assert.match(progressSource, /type: 'cycle-end', buffering/);
  assert.doesNotMatch(progressSource, /setInterval|requestAnimationFrame|BUFFERING_CUE_DELAY_MS/);
  assert.match(progressSource, /data-buffering=\{pulsing \? 'true' : undefined\}/);
});
