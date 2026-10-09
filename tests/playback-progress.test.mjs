import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';
import {
  clampPlaybackPosition,
  isPlaybackSeekableDuration,
  playbackSeekDisplayPosition,
  shouldReleasePlaybackSeekDraft,
} from '../src/lib/playback-scrubber.ts';
import { effectivePlaybackDurationMs } from '../src/lib/playback-duration.ts';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const appSource = readFileSync(path.join(root, 'src/App.svelte'), 'utf8');
const progressSource = readFileSync(path.join(root, 'src/lib/PlaybackProgress.svelte'), 'utf8');

test('the progress component displays the latest audio snapshot outside an active drag', () => {
  const initial = playbackSeekDisplayPosition(5_000, 60_000, null);
  const later = playbackSeekDisplayPosition(5_250, 60_000, null);
  assert.equal(initial, 5_000);
  assert.equal(later, 5_250);
  assert.ok(later > initial);
  assert.match(progressSource, /let draftPositionMs = \$state<number \| null>\(null\)/);
  assert.doesNotMatch(progressSource, /pendingPlaybackSeek|requestedAtMs|snapshotVersionAtRequest/);
});

test('seek draft stays on the clicked position until the worker snapshot catches up', () => {
  assert.match(progressSource, /oninput=\{updateDraft\}/);
  assert.match(progressSource, /onchange=\{commitDraft\}/);
  assert.match(progressSource, /<svelte:window onpointerup=\{finishPointerSeek\} onpointercancel=\{cancelDraft\}/);
  assert.match(progressSource, /seekSettled = true/);
  assert.match(progressSource, /accepted === false/);
  assert.equal(playbackSeekDisplayPosition(12_000, 60_000, 30_000), 30_000);
  assert.equal(shouldReleasePlaybackSeekDraft(12_000, 30_000, false, false, 12_000), false);
  assert.equal(shouldReleasePlaybackSeekDraft(12_250, 30_000, false, true, 12_000), false);
  assert.equal(shouldReleasePlaybackSeekDraft(30_040, 30_000, false, true, 12_000), true);
  assert.equal(shouldReleasePlaybackSeekDraft(30_040, 30_000, true, true, 12_000), false);
  assert.equal(clampPlaybackPosition(70_000, 60_000), 60_000);
  assert.equal(isPlaybackSeekableDuration(null), false);
  assert.equal(isPlaybackSeekableDuration(60_000), true);
});

test('App uses an audio snapshot fence so a pre-seek poll cannot overwrite the seek ACK', () => {
  assert.match(appSource, /let playbackSnapshotFence = 0/);
  assert.match(appSource, /snapshotVersion <= playbackSnapshotFence/);
  assert.match(appSource, /playbackSnapshotFence = \+\+playbackSnapshotRequestVersion/);
  assert.match(appSource, /durationMs=\{playbackDurationMs\}/);
  assert.match(appSource, /onSeek=\{commitPlaybackSeek\}/);
  assert.doesNotMatch(appSource, /pendingPlaybackSeek|shouldClearPendingPlaybackSeek/);
});

test('playlist track metadata supplies display duration when the decoder does not know it', () => {
  assert.equal(effectivePlaybackDurationMs(null, 184_000), 184_000);
  assert.equal(effectivePlaybackDurationMs(183_500, 184_000), 183_500);
  assert.equal(effectivePlaybackDurationMs(null, 0), null);
  assert.match(appSource, /effectivePlaybackDurationMs\(\s*playback\?\.durationMs,\s*playback\?\.currentTrack\?\.durationMs,/);
  assert.match(progressSource, /disabled=\{!seekEnabled\}/);
  assert.match(appSource, /playbackError = getErrorText\(error\)/);
});

test('progress slider stays enabled during non-seek playback busy to avoid timeline flash', () => {
  assert.match(progressSource, /disabled=\{\!seekEnabled\}/);
  assert.doesNotMatch(progressSource, /disabled=\{\!seekEnabled \|\| isSending\}/);
  assert.match(progressSource, /if \(\!seekEnabled \|\| isSending\) return;/);
  assert.match(appSource, /isSending=\{isSendingPlaybackCommand\}/);
});

test('progress slider remains visible and is not collapsed with volume-state hide', () => {
  const css = readFileSync(path.join(root, 'src/app.css'), 'utf8');
  assert.match(css, /\.volume-state\s*\{\s*display:\s*none;/);
  assert.match(css, /\.progress-slider,\s*\.volume-slider\s*\{/);
  assert.doesNotMatch(css, /\.progress-slider,\s*\.volume-state\s*\{\s*display:\s*none;/);
  assert.match(progressSource, /class="progress-slider"/);
  assert.match(progressSource, /<div class="progress-row" data-timeline-style=\{timelineStyle\}>/);
});

test('progress slider publishes playhead percent for the edge glow gradient', () => {
  const css = readFileSync(path.join(root, 'src/app.css'), 'utf8');
  assert.match(progressSource, /const progressPercent = \$derived\(/);
  assert.match(progressSource, /style=\{`--progress-pct: \$\{progressPercent\}%`\}/);
  assert.match(css, /--progress-pct/);
  assert.match(css, /::-webkit-slider-runnable-track/);
  assert.match(css, /margin-top:\s*-3\.5px/);
  assert.match(css, /row-gap:\s*8px/);
});

