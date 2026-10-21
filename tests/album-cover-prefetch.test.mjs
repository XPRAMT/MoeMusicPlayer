import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { ALBUM_PREFETCH_MAX_CARDS, getAlbumPrefetchIndices } from '../src/lib/album-cover-prefetch.js';
import { ALBUM_COVER_LOOKUP_LIMIT, withAlbumArtworkSlot, withAlbumCoverLookupSlot } from '../src/lib/album-artwork.js';

const range = (from, to) => Array.from({ length: to - from }, (_, i) => from + i);

test('one page before and one page after the visible cards, nearest first, toward the scroll direction first', () => {
  // 10 visible cards (20..29): prefetch 10 after and 10 before, 20 in total.
  const down = getAlbumPrefetchIndices({ start: 20, end: 30, total: 500, direction: 1 });
  assert.deepEqual(down, [...range(30, 40), ...range(10, 20).reverse()]);
  const up = getAlbumPrefetchIndices({ start: 20, end: 30, total: 500, direction: -1 });
  assert.deepEqual(up, [...range(10, 20).reverse(), ...range(30, 40)]);
});

test('the window is clamped to the list and follows the viewport size', () => {
  assert.deepEqual(getAlbumPrefetchIndices({ start: 0, end: 6, total: 10 }), [6, 7, 8, 9]);
  assert.deepEqual(getAlbumPrefetchIndices({ start: 4, end: 10, total: 10 }), [3, 2, 1, 0]);
  assert.deepEqual(getAlbumPrefetchIndices({ start: 0, end: 3, total: 100 }), [3, 4, 5]);
  assert.equal(getAlbumPrefetchIndices({ start: 40, end: 60, total: 1000 }).length, 40);
});

test('the total window and each side are capped however large the viewport is', () => {
  const huge = getAlbumPrefetchIndices({ start: 500, end: 800, total: 5000 });
  assert.equal(huge.length, ALBUM_PREFETCH_MAX_CARDS);
  assert.deepEqual(huge.slice(0, 60), range(800, 860));
  assert.deepEqual(huge.slice(60), range(440, 500).reverse());
  assert.equal(getAlbumPrefetchIndices({ start: 5, end: 10, total: 100, maxCards: 4 }).length, 4);
});

test('invalid input prefetches nothing', () => {
  assert.deepEqual(getAlbumPrefetchIndices({ start: -1, end: -1, total: 10 }), []);
  assert.deepEqual(getAlbumPrefetchIndices({ start: 5, end: 5, total: 10 }), []);
  assert.deepEqual(getAlbumPrefetchIndices({ start: 0, end: 4, total: 0 }), []);
  assert.deepEqual(getAlbumPrefetchIndices({ start: Number.NaN, end: 4, total: 10 }), []);
});

test('prefetch lookups wait for visible cards and never take every place', async () => {
  const order = [];
  let running = 0;
  let peakLow = 0;
  let low = 0;
  const task = (id, isLow) => async () => {
    running += 1; if (isLow) { low += 1; peakLow = Math.max(peakLow, low); }
    order.push(id);
    await new Promise((resolve) => setTimeout(resolve, 5));
    running -= 1; if (isLow) low -= 1;
    return id;
  };
  const jobs = [
    withAlbumCoverLookupSlot(task('low1', true), () => true, 'low'),
    withAlbumCoverLookupSlot(task('low2', true), () => true, 'low'),
    withAlbumCoverLookupSlot(task('low3', true), () => true, 'low'),
    withAlbumCoverLookupSlot(task('high1', false)),
    withAlbumCoverLookupSlot(task('high2', false)),
    withAlbumCoverLookupSlot(task('high3', false)),
  ];
  await Promise.all(jobs);
  assert.equal(peakLow, 1, 'prefetch uses a single place');
  assert.ok(ALBUM_COVER_LOOKUP_LIMIT >= 2);
  // low1 started first (nothing else waited), the waiting visible cards then go before the other prefetch jobs.
  assert.deepEqual(order.slice(0, 1), ['low1']);
  assert.ok(order.indexOf('high1') < order.indexOf('low2'));
  assert.ok(order.indexOf('high3') < order.indexOf('low2') || order.indexOf('high3') < order.indexOf('low3'));
});

test('a stale prefetch job that waited is skipped in the lookup and the thumbnail limiters', async () => {
  let wanted = true;
  let ran = 0;
  const slow = () => new Promise((resolve) => setTimeout(() => resolve('slow'), 10));
  const blockers = [withAlbumCoverLookupSlot(slow), withAlbumCoverLookupSlot(slow)];
  const waiting = withAlbumCoverLookupSlot(async () => { ran += 1; }, () => wanted, 'low');
  const artBlockers = [withAlbumArtworkSlot(slow), withAlbumArtworkSlot(slow)];
  const artWaiting = withAlbumArtworkSlot(async () => { ran += 1; }, () => wanted, 'low');
  wanted = false;
  await Promise.all([...blockers, ...artBlockers]);
  assert.equal(await waiting, null);
  assert.equal(await artWaiting, null);
  assert.equal(ran, 0);
});

test('album index schedules the prefetch only for an active, restored list and cancels it otherwise', () => {
  const source = readFileSync(new URL('../src/lib/AlbumIndex.svelte', import.meta.url), 'utf8').replace(/\r\n/g, '\n');
  assert.match(source, /const ready = active && !loading && scrollPositionRestored/);
  assert.match(source, /coverPrefetcher\.cancel\(\)/);
  assert.match(source, /coverPrefetcher\.schedule\(items\)/);
});
