import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { albumCardArtworkMemoryKey, forgetAlbumCardArtwork, getAlbumArtworkSelectionRevision } from '../src/lib/album-artwork.js';

test('forgetting one album re-keys only that album\'s card cover cache', () => {
  const before = albumCardArtworkMemoryKey(7, 'album:edited', getAlbumArtworkSelectionRevision('album:edited'));
  const other = albumCardArtworkMemoryKey(7, 'album:other', getAlbumArtworkSelectionRevision('album:other'));
  forgetAlbumCardArtwork('album:edited');
  assert.notEqual(albumCardArtworkMemoryKey(7, 'album:edited', getAlbumArtworkSelectionRevision('album:edited')), before);
  assert.equal(albumCardArtworkMemoryKey(7, 'album:other', getAlbumArtworkSelectionRevision('album:other')), other);
});

test('a song details save keeps the album cover cache key and clears the edited albums only', async () => {
  const app = await readFile(new URL('../src/App.svelte', import.meta.url), 'utf8');
  assert.match(app, /albumArtworkEpoch = \$derived\(libraryListRevision - trackDetailsListRefreshes\)/);
  assert.match(app, /trackDetailsListRefreshes \+= 1;\s*libraryListRevision \+= 1;/);
  assert.match(app, /<AlbumIndex\s+refreshKey=\{libraryListRevision\}\s+artworkKey=\{albumArtworkEpoch\}/);
  assert.match(app, /listen<string\[\]>\('album-artwork-invalidated'[^]*?forgetAlbumCardArtwork\(albumKey\)/);
  const index = await readFile(new URL('../src/lib/AlbumIndex.svelte', import.meta.url), 'utf8');
  assert.match(index, /<AlbumCard [^>]*refreshKey=\{artworkKey \?\? refreshKey\}/);
  const backend = await readFile(new URL('../src-tauri/src/track_details.rs', import.meta.url), 'utf8');
  assert.match(backend, /forget_saved_album_covers\(&app_for_reread, &state, album_before, album_index_key\(&state, id\)\)/);
});
