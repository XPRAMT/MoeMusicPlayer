import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { albumCoverSourcesHaveSameContent } from '../src/lib/album-artwork.js';

const bytes = (...values) => new Uint8Array(values).buffer;
const store = new Map([['blob:a', bytes(1, 2, 3)], ['blob:b', bytes(1, 2, 3)], ['blob:c', bytes(9, 9, 9)], ['blob:d', bytes(1, 2)]]);
const read = async (url) => { if (!store.has(url)) throw new Error('revoked'); return store.get(url); };

test('identical image bytes behind different URLs count as the same cover', async () => {
  assert.equal(await albumCoverSourcesHaveSameContent(['blob:a'], ['blob:b'], read), true);
});

test('different bytes, sizes, counts, unreadable or empty sources are replaced', async () => {
  assert.equal(await albumCoverSourcesHaveSameContent(['blob:a'], ['blob:c'], read), false);
  assert.equal(await albumCoverSourcesHaveSameContent(['blob:a'], ['blob:d'], read), false);
  assert.equal(await albumCoverSourcesHaveSameContent(['blob:a'], ['blob:b', 'blob:c'], read), false);
  assert.equal(await albumCoverSourcesHaveSameContent(['blob:gone'], ['blob:b'], read), false);
  assert.equal(await albumCoverSourcesHaveSameContent([], [], read), false);
});

test('album detail skips the cover swap and revision bump when the scan result has the same content', () => {
  const source = readFileSync(new URL('../src/lib/AlbumDetail.svelte', import.meta.url), 'utf8');
  const skip = source.indexOf('albumCoverSourcesHaveSameContent(initialCoverSources, urls)');
  const swap = source.indexOf('coverScanResolved = true;');
  assert.ok(skip > 0 && swap > skip);
  assert.match(source.slice(skip, swap), /URL\.revokeObjectURL\(url\)\);\s*return;/);
});
