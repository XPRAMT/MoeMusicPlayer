import test from 'node:test';
import assert from 'node:assert/strict';
import { ALBUM_COVER_LOOKUP_LIMIT, withAlbumCoverLookupSlot } from '../src/lib/album-artwork.js';

test('only a few card cover lookups run at once and the rest wait in order', async () => {
  let running = 0;
  let peak = 0;
  const order = [];
  const task = (id) => async () => {
    running += 1; peak = Math.max(peak, running); order.push(id);
    await new Promise((resolve) => setTimeout(resolve, 5));
    running -= 1; return id;
  };
  const results = await Promise.all([1, 2, 3, 4, 5, 6].map((id) => withAlbumCoverLookupSlot(task(id))));
  assert.deepEqual(results, [1, 2, 3, 4, 5, 6]);
  assert.equal(peak, ALBUM_COVER_LOOKUP_LIMIT);
  assert.deepEqual(order, [1, 2, 3, 4, 5, 6]);
});

test('a lookup that waited and is no longer wanted is not run', async () => {
  let wanted = true;
  let ran = 0;
  const slow = () => new Promise((resolve) => setTimeout(() => resolve('slow'), 10));
  const blockers = [withAlbumCoverLookupSlot(slow), withAlbumCoverLookupSlot(slow)];
  const waiting = withAlbumCoverLookupSlot(async () => { ran += 1; return 'late'; }, () => wanted);
  wanted = false;
  await Promise.all(blockers);
  assert.equal(await waiting, null);
  assert.equal(ran, 0);
  // the places are free again afterwards
  assert.equal(await withAlbumCoverLookupSlot(async () => 'next'), 'next');
});
