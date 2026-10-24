import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { acceptsTrackChoice, createPlaybackCommandOrder } from '../src/lib/playback-command-order.js';

const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
const trackList = readFileSync(new URL('../src/lib/TrackList.svelte', import.meta.url), 'utf8');
const playlistList = readFileSync(new URL('../src/lib/PlaylistEntryList.svelte', import.meta.url), 'utf8');

function fn(source, name) {
  const start = source.indexOf(`function ${name}(`);
  assert.notEqual(start, -1, `${name} exists`);
  const end = source.indexOf('\n  }', start);
  return source.slice(start, end === -1 ? undefined : end);
}

test('a track choice is accepted while another command is in flight, but not before playback is ready or while seeking', () => {
  assert.equal(acceptsTrackChoice({ playbackReady: true, isSeeking: false }), true);
  assert.equal(acceptsTrackChoice({ playbackReady: false, isSeeking: false }), false);
  assert.equal(acceptsTrackChoice({ playbackReady: true, isSeeking: true }), false);
});

test('the newest command supersedes every older one', () => {
  const order = createPlaybackCommandOrder();
  const remote = order.begin();
  assert.equal(order.isLatest(remote), true);
  const local = order.begin();
  assert.equal(order.isLatest(remote), false, 'the slow remote choice is superseded');
  assert.equal(order.isLatest(local), true);
  const next = order.begin();
  assert.equal(order.isLatest(local), false);
  assert.equal(order.isLatest(next), true);
});

test('every list that chooses a track ignores the busy flag', () => {
  for (const name of ['playPlaylistEntry', 'playQueueEntry', 'playTrack', 'playArtistTrack', 'playAlbumTrack']) {
    const body = fn(app, name);
    assert.match(body, /acceptsTrackChoice\(\{ playbackReady, isSeeking \}\)/, `${name} uses the shared rule`);
    assert.doesNotMatch(body, /isSendingPlaybackCommand/, `${name} is not blocked by a command in flight`);
  }
  assert.doesNotMatch(fn(trackList, 'playRow'), /isSendingPlaybackCommand/);
  assert.doesNotMatch(fn(playlistList, 'playEntry'), /isSendingPlaybackCommand/);
});

test('a superseded command shows nothing and leaves the busy flag to the newest one', () => {
  const send = fn(app, 'sendPlaybackCommand');
  assert.match(send, /const token = options\.seek \? null : playbackCommandOrder\.begin\(\);/);
  assert.match(send, /if \(isLatest\(\)\) \{\s*applyPlaybackSnapshot/);
  assert.match(send, /\} catch \(error\) \{\s*if \(isLatest\(\)\) \{\s*const text = getErrorText\(error\);/);
  assert.match(send, /else if \(isLatest\(\)\) isSendingPlaybackCommand = false;/);
});

test('the controls keep their own duplicate-click guard', () => {
  for (const name of ['togglePlayback', 'controlPlayback', 'commitPlaybackSeek', 'setRepeatMode', 'toggleShuffle']) {
    assert.match(fn(app, name), /isSendingPlaybackCommand \|\| isSeeking/, `${name} still ignores clicks while busy`);
  }
});
