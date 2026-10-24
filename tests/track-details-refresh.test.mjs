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
