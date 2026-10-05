import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';
import {
  clampPlaybackPosition,
  isPlaybackSeekableDuration,
  playbackSeekDisplayPosition,
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

test('seek draft is local, commits once on change or pointerup, and cancel restores snapshot', () => {
  assert.match(progressSource, /oninput=\{updateDraft\}/);
  assert.match(progressSource, /onchange=\{commitDraft\}/);
  assert.match(progressSource, /<svelte:window onpointerup=\{finishPointerSeek\} onpointercancel=\{cancelDraft\}/);
  assert.match(progressSource, /function commitDraft\(\): void \{/);
  assert.match(progressSource, /draftPositionMs = null;\s*draftTrackId = null;\s*pointerActive = false;\s*if \(!seekEnabled \|\| isSending \|\| requestedPositionMs === positionMs\) return;\s*void onSeek\(requestedPositionMs\);/);
  assert.match(progressSource, /function cancelDraft\(\): void \{/);
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
