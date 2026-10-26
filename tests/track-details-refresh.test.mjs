import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('saving track details refreshes that one track instead of syncing the library', async () => {
  const app = await readFile(new URL('../src/App.svelte', import.meta.url), 'utf8');
  const start = app.indexOf('async function reloadAfterTrackDetailsSaved(');
  const body = app.slice(start, start + app.slice(start).search(/\r?\n {2}\}\r?\n/));
  assert.ok(start > 0);
  assert.doesNotMatch(body, /syncLibrary\(|library_sync/);
  assert.match(body, /libraryListRevision \+= 1/);
  assert.match(body, /playback_refresh_current_track/);
  assert.match(body, /currentTrack\?\.id === savedTrackId\)[^]*artworkController\.reload\(\)[^]*lyricsController\.reload\(\)/);
  assert.match(app, /onSaved=\{\(\) => void reloadAfterTrackDetailsSaved\(trackDetailsTrackId\)\}/);
  const backend = await readFile(new URL('../src-tauri/src/track_details.rs', import.meta.url), 'utf8');
  assert.match(backend, /reread_saved_track\(&app_for_reread, &state, &track_id, id\)\.await\?/);
  assert.match(backend, /\.refresh_track_metadata\(id, &metadata, &locator\)/);
});

test('a track that starts is checked against its file in the background and a change refreshes the page', async () => {
  const app = await readFile(new URL('../src/App.svelte', import.meta.url), 'utf8');
  assert.match(app, /void invokeCommand\('playback_check_track_file', \{ trackId \}\)\.catch\(/);
  const start = app.indexOf("listen<string>('track-file-refreshed'");
  assert.ok(start > 0);
  const body = app.slice(start, start + 600);
  assert.match(body, /libraryListRevision \+= 1/);
  assert.match(body, /artworkController\.reload\(\)[^]*lyricsController\.reload\(\)/);
  const backend = await readFile(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8');
  const check = backend.slice(backend.indexOf('fn check_started_track_file('));
  const checkBody = check.slice(0, check.search(/\r?\n\}\r?\n/));
  // Nothing that reloads, reopens or seeks the playing source.
  assert.doesNotMatch(checkBody, /select_track|load_track|seek|play_track|request_play/i);
  assert.match(checkBody, /refresh_current_track\(app, &state\)/);
});
