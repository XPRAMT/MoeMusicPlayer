import assert from 'node:assert/strict';
import test from 'node:test';
import {
  buildVirtualRows,
  getNextActiveIndex,
  getVirtualRange,
  PagedListController,
  TRACK_LIST_MAX_CACHED_PAGES,
  TRACK_PAGE_SIZE,
} from '../src/lib/track-list-data.js';

test('shared keyboard navigation covers arrows, home/end, pages, and leaves unrelated keys alone', () => {
  assert.equal(getNextActiveIndex('ArrowDown', 3, 100, 196, 49), 4);
  assert.equal(getNextActiveIndex('ArrowUp', 3, 100, 196, 49), 2);
  assert.equal(getNextActiveIndex('Home', 30, 100, 196, 49), 0);
  assert.equal(getNextActiveIndex('End', 30, 100, 196, 49), 99);
  assert.equal(getNextActiveIndex('PageDown', 30, 100, 196, 49), 34);
  assert.equal(getNextActiveIndex('PageUp', 30, 100, 196, 49), 26);
  assert.equal(getNextActiveIndex('Escape', 3, 100, 196, 49), null);
  assert.equal(getNextActiveIndex('ArrowDown', 0, 0, 196, 49), null);
});

function playlistPage(playlistId, offset, limit, totalCount) {
  const items = Array.from(
    { length: Math.min(limit, Math.max(0, totalCount - offset)) },
    (_, index) => ({
      position: offset + index,
      trackId: offset + index < 2 ? 'same-track' : `track-${offset + index}`,
      title: `Entry ${offset + index}`,
      hasEnabledMapping: offset + index !== 2,
      playlistId,
    }),
  );
  if (offset === 0 && items.length > 2) {
    items[2] = { ...items[2], trackId: null, hasEnabledMapping: false, title: 'Unmatched entry' };
  }
  return { items, offset, limit, totalCount };
}

test('generic paged list keeps duplicate playlist entries distinct by position and shows unmatched entries', async () => {
  const data = new PagedListController(({ scope, offset, limit }) =>
    Promise.resolve(playlistPage(scope, offset, limit, 100)));
  data.reset('playlist-a');
  await data.ensureRange(0, 8);

  const rows = buildVirtualRows(data, { start: 0, end: 4 }, data.snapshot().revision, (entry) => entry.position);
  assert.deepEqual(rows.rows.slice(0, 3).map((row) => row.key), [0, 1, 2]);
  assert.equal(rows.rows[0].item.trackId, rows.rows[1].item.trackId);
  assert.equal(rows.rows[0].key === rows.rows[1].key, false);
  assert.equal(rows.rows[2].item.trackId, null);
  assert.equal(rows.rows[2].item.title, 'Unmatched entry');
  assert.equal(rows.rows[2].item.hasEnabledMapping, false);
});

test('switching playlists and refreshing the same playlist discard stale page responses', async () => {
  const pending = [];
  const data = new PagedListController((request) => new Promise((resolve) => {
    pending.push({ request, resolve });
  }));

  data.reset('playlist-old');
  const oldRequest = data.ensureRange(0, 10);
  assert.equal(pending[0].request.scope, 'playlist-old');

  data.reset('playlist-new');
  const newRequest = data.ensureRange(0, 10);
  assert.equal(pending[1].request.scope, 'playlist-new');
  pending[1].resolve(playlistPage('playlist-new', 0, TRACK_PAGE_SIZE, 20));
  await newRequest;
  assert.equal(data.itemAt(0).playlistId, 'playlist-new');

  data.reset('playlist-new', 1);
  const refreshRequest = data.ensureRange(0, 10);
  assert.equal(pending[2].request.scope, 'playlist-new');
  pending[0].resolve(playlistPage('playlist-old', 0, TRACK_PAGE_SIZE, 20));
  pending[2].resolve(playlistPage('playlist-new', 0, TRACK_PAGE_SIZE, 20));
  await Promise.all([oldRequest, refreshRequest]);
  assert.equal(data.snapshot().generation, 3);
  assert.equal(data.itemAt(0).playlistId, 'playlist-new');
});

test('100k playlist paging shares the bounded page and viewport budgets', async () => {
  const totalCount = 100_000;
  let maxCachedPages = 0;
  let maxRenderedRows = 0;
  let calls = 0;
  const data = new PagedListController(async ({ scope, offset, limit }) => {
    calls += 1;
    return playlistPage(scope, offset, limit, totalCount);
  });
  data.reset('large-playlist');

  const maxScrollTop = totalCount * 49 - 420;
  for (let scrollTop = 0; scrollTop < maxScrollTop; scrollTop += 49 * 36) {
    const range = getVirtualRange(scrollTop, 420, totalCount, 49, 8, 33);
    await data.ensureRange(range.start, range.end);
    const snapshot = data.snapshot();
    const rows = buildVirtualRows(data, range, snapshot.revision, (entry) => entry.position);
    maxCachedPages = Math.max(maxCachedPages, snapshot.cachedPageCount);
    maxRenderedRows = Math.max(maxRenderedRows, rows.rows.length);
    assert.ok(rows.rows.every((row) => row.key !== undefined));
  }

  assert.ok(maxCachedPages <= TRACK_LIST_MAX_CACHED_PAGES);
  assert.ok(data.snapshot().cachedItemCount <= TRACK_PAGE_SIZE * TRACK_LIST_MAX_CACHED_PAGES);
  assert.ok(maxRenderedRows <= 30);
  assert.ok(calls < totalCount / 10);
  assert.equal(data.snapshot().totalCount, totalCount);
});
