import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const read = (relativePath) => readFileSync(path.join(root, relativePath), 'utf8').replace(/\r\n/g, '\n');

const sources = {
  track: read('src/lib/TrackList.svelte'),
  playlist: read('src/lib/PlaylistEntryList.svelte'),
  queue: read('src/lib/PlaybackQueueList.svelte'),
  artist: read('src/lib/ArtistIndex.svelte'),
  album: read('src/lib/AlbumIndex.svelte'),
  albumDetail: read('src/lib/AlbumDetail.svelte'),
  paging: read('src/lib/track-list-data.js'),
  ipc: read('src/lib/ipc.ts'),
  tauri: read('src-tauri/src/lib.rs'),
};

test('all list UIs request bounded pages through the shared 1000-row controller', () => {
  assert.match(sources.paging, /TRACK_PAGE_SIZE = 1000/);
  assert.match(sources.paging, /this\.pageSize = TRACK_PAGE_SIZE;/);
  assert.doesNotMatch(sources.paging, /pageSize\?:\s*number|options\.pageSize/);
  assert.doesNotMatch(sources.albumDetail, /PAGE_SIZE\s*=\s*\d+/);
  for (const [name, source] of Object.entries(sources).filter(([name]) =>
    ['track', 'playlist', 'queue', 'artist', 'album'].includes(name))) {
    assert.match(source, /new PagedListController</, `${name} should use shared paging`);
    assert.doesNotMatch(source, /pageSize\s*:\s*\d+/, `${name} must use the fixed page size`);
  }
  assert.match(sources.albumDetail, /<TrackList[\s\S]*?fetchPage=\{fetchPage\}/);
});

test('search, sorting, aggregation, and grouping are delegated to SQLite list commands', () => {
  assert.match(sources.track, /invokeCommand\('library_get_page'/);
  assert.match(sources.track, /library_get_quick_index_for_tracks/);
  assert.match(sources.playlist, /playlist_get_page/);
  assert.match(sources.playlist, /playlist_get_quick_index_groupings/);
  assert.match(sources.queue, /playback_get_queue_page/);
  assert.match(sources.queue, /playback_get_queue_quick_index_groupings/);
  assert.match(sources.artist, /library_create_artist_index_snapshot/);
  assert.match(sources.artist, /library_get_artist_index_snapshot_page/);
  assert.match(sources.album, /library_create_album_index_snapshot/);
  assert.match(sources.album, /library_get_album_index_snapshot_page/);
  assert.match(sources.albumDetail, /library_get_album_detail_facets/);
  assert.match(sources.albumDetail, /library_get_album_page/);

  assert.doesNotMatch(sources.ipc, /library_sort_text_indices/);
  assert.doesNotMatch(sources.tauri, /library_sort_text_indices/);
  for (const [name, source] of Object.entries(sources).filter(([name]) =>
    ['track', 'playlist', 'queue', 'artist', 'album', 'albumDetail'].includes(name))) {
    assert.doesNotMatch(source, /buildQuickIndexGroups|sortAlbumEntries|sortArtistEntries/,
      `${name} must not reconstruct full-list sorting or groups in the renderer`);
  }
});
