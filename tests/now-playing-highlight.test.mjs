import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
const trackList = readFileSync(new URL('../src/lib/TrackList.svelte', import.meta.url), 'utf8');
const albumDetail = readFileSync(new URL('../src/lib/AlbumDetail.svelte', import.meta.url), 'utf8');

test('the library, artist and album lists highlight the track the player is on, like the playlist page', () => {
  assert.match(app, /let currentTrackId = \$derived\(playback\?\.currentTrack\?\.id \?\? null\);/);
  // Library and artist TrackList plus AlbumDetail: all three take the player's current track.
  assert.equal(app.match(/selectedTrackId=\{currentTrackId\}/g)?.length, 3);
  // No click-time copy that would go stale when the track changes by itself (next, previous, auto-advance).
  assert.doesNotMatch(app, /selectedTrackId\s*=\s*(?!\{)/);
  assert.doesNotMatch(app, /let selectedTrackId/);
  // The playlist page uses the same source.
  assert.match(app, /currentTrackId=\{playback\?\.currentTrack\?\.id \?\? null\}/);
  // The rows compare by track id, recomputed per render.
  assert.match(trackList, /isSelected=\{\(track\) => selectedTrackId === track\.id\}/);
  assert.match(albumDetail, /\{selectedTrackId\}/);
});
