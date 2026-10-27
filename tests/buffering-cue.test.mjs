import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  BUFFERING_CUE_DELAY_MS,
  bufferingCueVisible,
  msUntilBufferingCue,
} from '../src/lib/buffering-cue.js';

const css = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
const progressSource = readFileSync(new URL('../src/lib/PlaybackProgress.svelte', import.meta.url), 'utf8');
const appSource = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
const progressRunSource = readFileSync(new URL('../src/lib/progress-run.js', import.meta.url), 'utf8');

test('the cue shows only after 500 ms of buffering', () => {
  assert.equal(BUFFERING_CUE_DELAY_MS, 500);
  assert.equal(bufferingCueVisible('buffering', 0), false);
  assert.equal(bufferingCueVisible('buffering', 499), false);
  assert.equal(bufferingCueVisible('buffering', 500), true);
  assert.equal(bufferingCueVisible('buffering', 5000), true);
  for (const state of ['playing', 'paused', 'stopped', 'empty', null, undefined]) {
    assert.equal(bufferingCueVisible(state, 10_000), false, String(state));
  }
  assert.equal(bufferingCueVisible('buffering', Number.NaN), false);
});

test('one timer: the wait is the rest of the 500 ms, none outside buffering', () => {
  assert.equal(msUntilBufferingCue('buffering', 0), 500);
  assert.equal(msUntilBufferingCue('buffering', 200), 300);
  assert.equal(msUntilBufferingCue('buffering', 900), 0);
  assert.equal(msUntilBufferingCue('buffering', Number.NaN), 500);
  assert.equal(msUntilBufferingCue('playing', 0), null);
  assert.equal(msUntilBufferingCue('paused', 0), null);
  assert.equal(msUntilBufferingCue(null, 0), null);
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

test('the shared progress bar gets the playback state and uses one timer', () => {
  assert.match(appSource, /<PlaybackProgress[\s\S]*?playbackState=\{playback\?\.state \?\? null\}/);
  assert.match(progressSource, /msUntilBufferingCue\(playbackState, 0\)/);
  assert.match(progressSource, /clearTimeout\(timer\)/);
  assert.doesNotMatch(progressSource, /setInterval|requestAnimationFrame/);
  assert.match(progressSource, /data-buffering=\{bufferingCue \? 'true' : undefined\}/);
});
