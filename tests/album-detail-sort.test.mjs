import test from 'node:test';
import assert from 'node:assert/strict';
import {
  DEFAULT_ALBUM_DETAIL_SORT,
  normalizeAlbumDetailSort,
  toggleAlbumDetailSort,
} from '../src/lib/album-detail-sort.js';

test('album details default to ascending track number and normalize saved settings', () => {
  assert.deepEqual(DEFAULT_ALBUM_DETAIL_SORT, { field: 'position', direction: 'asc' });
  assert.deepEqual(normalizeAlbumDetailSort(undefined), { field: 'position', direction: 'asc' });
  assert.deepEqual(normalizeAlbumDetailSort({ field: 'duration', direction: 'desc' }), {
    field: 'duration', direction: 'desc',
  });
  assert.deepEqual(normalizeAlbumDetailSort({ field: 'unknown', direction: 'sideways' }), {
    field: 'position', direction: 'asc',
  });
});

test('album detail sort toggles direction and starts a new field ascending', () => {
  assert.deepEqual(toggleAlbumDetailSort({ field: 'position', direction: 'asc' }, 'position'), {
    field: 'position', direction: 'desc',
  });
  assert.deepEqual(toggleAlbumDetailSort({ field: 'duration', direction: 'desc' }, 'position'), {
    field: 'position', direction: 'asc',
  });
  assert.deepEqual(toggleAlbumDetailSort({ field: 'position', direction: 'asc' }, 'title'), {
    field: 'title', direction: 'asc',
  });
});
