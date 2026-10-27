import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { bufferingCueVisible } from '../src/lib/buffering-cue.js';

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

test('the pulse is a 1000 ms brightness 1 -> 0.9 animation on the thumb only', () => {
  const rule = css.match(/\.progress-row\[data-buffering\] \.progress-thumb \{([^}]*)\}/);
  assert.ok(rule, 'thumb buffering rule');
  assert.match(rule[1], /animation:\s*progress-thumb-buffering\s+1000ms\s+ease-in-out\s+infinite/);
  const keyframes = css.match(/@keyframes progress-thumb-buffering \{([\s\S]*?)\n\}/);
  assert.ok(keyframes, 'keyframes');
  const props = [...keyframes[1].matchAll(/([a-z-]+)\s*:/g)].map((match) => match[1]);
  assert.deepEqual([...new Set(props)], ['filter']);
  assert.match(keyframes[1], /brightness\(1\)/);
  assert.match(keyframes[1], /brightness\(0\.9\)/);
  // Only the thumb uses the keyframes.
  const users = [...css.matchAll(/([^{}]+)\{[^{}]*progress-thumb-buffering[^{}]*\}/g)].map((match) => match[1].trim());
  assert.deepEqual(users, ['.progress-row[data-buffering] .progress-thumb']);
  assert.doesNotMatch(progressRunSource, /buffering/);
});

test('reduced motion turns the pulse off', () => {
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{\s*\.progress-row\[data-buffering\] \.progress-thumb \{\s*animation: none;/);
});

test('the shared progress bar follows the playback state directly, without timers', () => {
  assert.match(appSource, /<PlaybackProgress[\s\S]*?playbackState=\{playback\?\.state \?\? null\}/);
  assert.match(progressSource, /const bufferingCue = \$derived\(bufferingCueVisible\(playbackState\)\)/);
  assert.doesNotMatch(progressSource, /BUFFERING_CUE_DELAY_MS|msUntilBufferingCue|setInterval|requestAnimationFrame/);
  assert.match(progressSource, /data-buffering=\{bufferingCue \? 'true' : undefined\}/);
});
