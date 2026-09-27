import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';
import {
  beginPlaybackSeekDraft,
  commitPlaybackSeekDraft,
  isPlaybackSeekableDuration,
  playbackSeekDisplayPosition,
  shouldClearPendingPlaybackSeek,
  updatePlaybackSeekDraft,
} from '../src/lib/playback-scrubber.ts';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const appSource = readFileSync(path.join(root, 'src/App.svelte'), 'utf8');

function progressSliderSource() {
  const start = appSource.indexOf('<input\n          class="progress-slider"');
  assert.notEqual(start, -1, 'missing playback progress slider');
  const end = appSource.indexOf('/>', start);
  assert.notEqual(end, -1, 'progress slider element must be self-closing');
  return appSource.slice(start, end + 2);
}

test('playback progress drag stays local until the user commits it', () => {
  const slider = progressSliderSource();
  assert.match(slider, /value=\{playbackSeekPositionMs\}/);
  assert.match(slider, /onpointerdown=\{beginPlaybackSeek\}/);
  assert.match(slider, /oninput=\{updatePlaybackSeek\}/);
  assert.match(slider, /onpointerup=\{\(\) => void commitPlaybackSeek\(\)\}/);
  assert.match(slider, /onchange=\{\(\) => void commitPlaybackSeek\(\)\}/);
  assert.match(slider, /onblur=\{\(\) => void commitPlaybackSeek\(\)\}/);
  assert.match(slider, /disabled=[^\n]*isPlaybackSeekableDuration/);

  const updateStart = appSource.indexOf('function updatePlaybackSeek(');
  const commitStart = appSource.indexOf('async function commitPlaybackSeek(');
  assert.notEqual(updateStart, -1);
  assert.notEqual(commitStart, -1);
  const updateBlock = appSource.slice(updateStart, commitStart);
  const commitBlock = appSource.slice(commitStart, appSource.indexOf('function applyPlaybackSnapshot(', commitStart));
  assert.doesNotMatch(updateBlock, /playback_seek/);
  assert.match(commitBlock, /invokeCommand\('playback_seek', \{ positionMs \}\)/);
});

test('drag draft survives playback ticks and commits only its final position', () => {
  const durationMs = 60_000;
  for (const state of ['playing', 'paused']) {
    let draft = beginPlaybackSeekDraft('track-1', 5_000, durationMs);
    assert.ok(draft);
    const seekRequests = [];
    const tickPosition = state === 'playing' ? 5_300 : 5_000;

    for (const inputPosition of [12_000, 18_000, 23_000]) {
      draft = updatePlaybackSeekDraft(draft, 'track-1', 5_000, durationMs, inputPosition);
      assert.equal(
        playbackSeekDisplayPosition(tickPosition, durationMs, 'track-1', draft, null),
        inputPosition,
        `${state} playback ticks must not move the thumb during drag`,
      );
      assert.deepEqual(seekRequests, [], 'input events must not send seek IPC');
    }

    const committedPosition = commitPlaybackSeekDraft(draft, 'track-1', durationMs);
    if (committedPosition !== null) seekRequests.push(committedPosition);
    assert.deepEqual(seekRequests, [23_000]);
  }
});

test('seek keeps its requested position until the audio snapshot catches up', () => {
  const pending = {
    trackId: 'track-1',
    positionMs: 23_000,
    requestedAtMs: 1_000,
    snapshotVersionAtRequest: 2,
  };
  assert.equal(playbackSeekDisplayPosition(5_000, 60_000, 'track-1', null, pending), 23_000);
  assert.equal(shouldClearPendingPlaybackSeek(pending, {
    trackId: 'track-1',
    positionMs: 5_300,
    durationMs: 60_000,
    isPlaying: true,
  }, 2, 1_250), false, 'a snapshot already in flight when seeking began is stale');
  assert.equal(shouldClearPendingPlaybackSeek(pending, {
    trackId: 'track-1',
    positionMs: 5_300,
    durationMs: 60_000,
    isPlaying: true,
  }, 3, 1_250), false, 'the command IPC may return a pre-seek position');
  assert.equal(shouldClearPendingPlaybackSeek(pending, {
    trackId: 'track-1',
    positionMs: 23_250,
    durationMs: 60_000,
    isPlaying: true,
  }, 4, 1_500), true, 'playing position is allowed to advance after the seek');
  assert.equal(shouldClearPendingPlaybackSeek(pending, {
    trackId: 'track-1',
    positionMs: 23_025,
    durationMs: 60_000,
    isPlaying: false,
  }, 5, 1_500), true, 'paused playback accepts a small clock rounding difference');
  assert.equal(shouldClearPendingPlaybackSeek(pending, {
    trackId: 'track-1',
    positionMs: 5_000,
    durationMs: 60_000,
    isPlaying: false,
  }, 6, 4_000), true, 'a seek that never reaches the backend eventually releases its draft');
});

test('unknown duration and track changes cannot produce a stale seek', () => {
  assert.equal(isPlaybackSeekableDuration(null), false);
  assert.equal(isPlaybackSeekableDuration(0), false);
  assert.equal(beginPlaybackSeekDraft('track-1', 0, null), null);

  const draft = beginPlaybackSeekDraft('track-1', 5_000, 60_000);
  assert.ok(draft);
  const changedTrackDraft = updatePlaybackSeekDraft(draft, 'track-2', 2_000, 60_000, 9_000);
  assert.equal(commitPlaybackSeekDraft(changedTrackDraft, 'track-2', 60_000), 9_000);
  assert.equal(commitPlaybackSeekDraft(draft, 'track-2', 60_000), null);
  assert.equal(commitPlaybackSeekDraft(draft, 'track-1', null), null);
});
