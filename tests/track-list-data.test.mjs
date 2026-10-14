import assert from 'node:assert/strict';
import { performance } from 'node:perf_hooks';
import test from 'node:test';
import {
  buildVirtualRows,
  getVirtualRange,
  PagedTrackList,
  TRACK_LIST_MAX_CACHED_PAGES,
  TRACK_PAGE_SIZE,
  TRACK_PAGE_SIZE_MAX,
} from '../src/lib/track-list-data.js';

const TOTAL_TRACKS = 100_000;

test('all song pages use a fixed 1000 items and cannot be resized with the viewport', () => {
  assert.equal(TRACK_PAGE_SIZE, 1000);
  assert.equal(TRACK_PAGE_SIZE_MAX, 1000);
  assert.equal(typeof PagedTrackList.prototype.setPageSize, 'undefined');
});

function makeTrack(id) {
  return {
    id,
    title: `Track ${id}`,
    artist: 'Synthetic artist',
    album: 'Synthetic album',
    albumArtist: null,
    trackNumber: null,
    discNumber: null,
    durationMs: 180_000,
    codec: 'test',
    bitrateBps: null,
    sampleRateHz: null,
  };
}

function makePage(offset, limit, totalCount, prefix = 'track') {
  const itemCount = Math.min(limit, Math.max(0, totalCount - offset));
  return {
    items: Array.from({ length: itemCount }, (_, index) => makeTrack(`${prefix}-${offset + index}`)),
    offset,
    limit,
    totalCount,
  };
}

test('virtual range remains bounded near the beginning, middle, and end of 100k rows', () => {
  const viewportHeight = 680;
  const rowHeight = 40;
  const starts = [0, 500_000, TOTAL_TRACKS * rowHeight - viewportHeight];
  let maxRows = 0;

  for (const scrollTop of starts) {
    const range = getVirtualRange(scrollTop, viewportHeight, TOTAL_TRACKS, rowHeight, 8, 37);
    assert.ok(range.start >= 0);
    assert.ok(range.end <= TOTAL_TRACKS);
    assert.ok(range.end >= range.start);
    maxRows = Math.max(maxRows, range.end - range.start);
  }

  assert.ok(maxRows <= 36, `expected at most 36 rendered rows, got ${maxRows}`);
});

test('reset loads the first page without waiting for a viewport scroll event', async () => {
  const requests = [];
  const data = new PagedTrackList(async (request) => {
    requests.push({ offset: request.offset, limit: request.limit });
    return makePage(request.offset, request.limit, 23);
  });

  data.reset('', 'initial-list');
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(requests, [{ offset: 0, limit: TRACK_PAGE_SIZE }]);
  assert.equal(data.snapshot().totalCount, 23);
  assert.equal(data.itemAt(0)?.id, 'track-0');
});

test('concurrent requests for the same page are deduplicated', async () => {
  let resolvePage;
  let requestCount = 0;
  const data = new PagedTrackList(() => {
    requestCount += 1;
    return new Promise((resolve) => { resolvePage = resolve; });
  });
  data.reset('', 'dedupe');

  const first = data.ensureRange(0, 12);
  const second = data.ensureRange(4, 20);
  assert.equal(requestCount, 1);

  resolvePage(makePage(0, TRACK_PAGE_SIZE, 32));
  await Promise.all([first, second]);
  assert.equal(data.trackAt(0)?.id, 'track-0');
  assert.equal(requestCount, 1);
});

test('late response from an older query cannot replace the current page', async () => {
  const pending = [];
  const data = new PagedTrackList((request) => new Promise((resolve) => {
    pending.push({ request, resolve });
  }));

  data.reset('old', 'old-query');
  const oldRequest = data.ensureRange(0, 10);
  assert.equal(pending.length, 1);

  data.reset('new', 'new-query');
  const newRequest = data.ensureRange(0, 10);
  assert.equal(pending.length, 2);

  pending[1].resolve(makePage(0, TRACK_PAGE_SIZE, 20, 'new'));
  await newRequest;
  assert.equal(data.trackAt(0)?.id, 'new-0');

  pending[0].resolve(makePage(0, TRACK_PAGE_SIZE, 20, 'old'));
  await oldRequest;
  assert.equal(data.trackAt(0)?.id, 'new-0');
  assert.equal(data.snapshot().query, 'new');
});

test('failed page can be retried without losing loaded data', async () => {
  let requestCount = 0;
  const data = new PagedTrackList(async ({ offset, limit }) => {
    requestCount += 1;
    if (requestCount === 1) throw new Error('temporary read failure');
    return makePage(offset, limit, 18);
  });
  data.reset('', 'retry');

  await data.ensureRange(0, 12);
  assert.equal(data.snapshot().errors[0]?.message, 'temporary read failure');
  assert.equal(data.trackAt(0), null);

  await data.retry(0);
  assert.equal(data.snapshot().errors.length, 0);
  assert.equal(data.trackAt(0)?.id, 'track-0');
  assert.equal(requestCount, 2);
});

test('page count changes during scrolling are reported instead of mixing snapshots', async () => {
  const data = new PagedTrackList(async ({ offset, limit }) =>
    makePage(offset, limit, offset === 0 ? 80 : 81), { pageSize: 40 });
  data.reset('', 'changing-count');
  await data.ensureRange(0, 12);
  await data.ensureRange(40, 50);

  assert.equal(data.snapshot().totalCount, 80);
  assert.match(data.snapshot().errors[0]?.message ?? '', /有變更/);
  assert.equal(data.trackAt(0)?.id, 'track-0');
  assert.equal(data.trackAt(40), null);
});

test('page size is fixed for the lifetime of a list even when the requested viewport range changes', async () => {
  const requests = [];
  const data = new PagedTrackList(async (request) => {
    requests.push({ offset: request.offset, limit: request.limit });
    return makePage(request.offset, request.limit, 2500);
  });
  data.reset('', 'fixed-page-size');
  await data.ensureRange(0, 20);
  await data.ensureRange(850, 900);
  await data.ensureRange(1850, 1900);

  assert.deepEqual(requests, [
    { offset: 0, limit: 1000 },
    { offset: 1000, limit: 1000 },
  ]);
  assert.equal(data.snapshot().totalCount, 2500);
  assert.equal(typeof data.setPageSize, 'undefined');
});

test('a visible page load refreshes the same virtual range without scrolling', async () => {
  let resolveNextPage;
  const data = new PagedTrackList(({ offset, limit }) => {
    if (offset === 40) return new Promise((resolve) => { resolveNextPage = resolve; });
    return Promise.resolve(makePage(offset, limit, 100));
  }, { pageSize: 40 });
  data.reset('', 'same-range-refresh');
  await data.ensureRange(0, 20);

  const visibleRange = { start: 40, end: 50 };
  const beforeSnapshot = data.snapshot();
  const before = buildVirtualRows(data, visibleRange, beforeSnapshot.revision);
  assert.ok(before.rows.every((row) => row.track === null));
  assert.equal(beforeSnapshot.totalCount, 100);

  const loadingVisiblePage = data.ensureRange(visibleRange.start, visibleRange.end);
  resolveNextPage(makePage(40, 40, 100));
  await loadingVisiblePage;

  const afterSnapshot = data.snapshot();
  const after = buildVirtualRows(data, visibleRange, afterSnapshot.revision);
  assert.deepEqual(visibleRange, { start: 40, end: 50 });
  assert.ok(after.revision > before.revision);
  assert.ok(after.rows.every((row, index) => row.track?.id === `track-${40 + index}`));
});

test('synthetic 100k scrolling keeps fetched cache and rendered rows bounded', async (t) => {
  let fetchedRecords = 0;
  let requestCount = 0;
  const data = new PagedTrackList(async ({ offset, limit }) => {
    requestCount += 1;
    const page = makePage(offset, limit, TOTAL_TRACKS);
    fetchedRecords += page.items.length;
    return page;
  });
  data.reset('', '100k-benchmark');

  const rowHeight = 40;
  const headerHeight = 37;
  const viewportHeight = 680;
  const maxScrollTop = TOTAL_TRACKS * rowHeight - viewportHeight;
  const startedAt = performance.now();
  let maxRenderedRows = 0;
  let maxCachedPages = 0;
  let maxCachedItems = 0;
  let scrollSteps = 0;

  for (let scrollTop = 0; scrollTop <= maxScrollTop; scrollTop += rowHeight * 36) {
    const range = getVirtualRange(scrollTop, viewportHeight, TOTAL_TRACKS, rowHeight, 8, headerHeight);
    await data.ensureRange(range.start, range.end);
    const snapshot = data.snapshot();
    maxRenderedRows = Math.max(maxRenderedRows, range.end - range.start);
    maxCachedPages = Math.max(maxCachedPages, snapshot.cachedPageCount);
    maxCachedItems = Math.max(maxCachedItems, snapshot.cachedItemCount);
    scrollSteps += 1;
  }

  const elapsedMs = performance.now() - startedAt;
  assert.ok(maxRenderedRows <= 36, `render window grew to ${maxRenderedRows} rows`);
  assert.ok(maxCachedPages <= TRACK_LIST_MAX_CACHED_PAGES);
  assert.ok(maxCachedItems <= TRACK_PAGE_SIZE * TRACK_LIST_MAX_CACHED_PAGES);
  assert.equal(data.snapshot().totalCount, TOTAL_TRACKS);
  assert.ok(requestCount < TOTAL_TRACKS / 10, `unexpected request count ${requestCount}`);

  t.diagnostic(JSON.stringify({
    syntheticTracks: TOTAL_TRACKS,
    scrollSteps,
    requestCount,
    fetchedRecords,
    maxRenderedRows,
    maxCachedPages,
    maxCachedItems,
    elapsedMs: Number(elapsedMs.toFixed(2)),
    note: 'Node data-window simulation; this does not measure WebView frame time.',
  }));
});
