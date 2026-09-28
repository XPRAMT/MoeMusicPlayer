import assert from 'node:assert/strict';
import test from 'node:test';
import {
  adaptPlaybackQueuePage,
  isCurrentPlaybackQueueEntry,
  playbackQueueCurrentEntryPosition,
} from '../src/lib/playback-queue-data.js';

function queueItem(traversalPosition, entryPosition, isCurrent = false) {
  return {
    traversalPosition,
    entryPosition,
    sourcePosition: entryPosition,
    trackId: 'duplicate-track',
    track: null,
    isCurrent,
  };
}

test('queue page adapter preserves traversal order and stable duplicate entry positions', () => {
  const page = {
    revision: 7,
    total: 100_000,
    offset: 40,
    cursor: 42,
    currentEntryPosition: 9,
    items: [queueItem(40, 2), queueItem(41, 9, true)],
  };
  const adapted = adaptPlaybackQueuePage(page, { offset: 40, limit: 40 });

  assert.equal(adapted.totalCount, 100_000);
  assert.equal(adapted.limit, 40);
  assert.deepEqual(adapted.items.map((item) => item.traversalPosition), [40, 41]);
  assert.deepEqual(adapted.items.map((item) => item.entryPosition), [2, 9]);
  assert.equal(adapted.items[0].trackId, adapted.items[1].trackId);
});

test('cursor metadata identifies the current duplicate by entry position rather than track id', () => {
  const firstDuplicate = queueItem(3, 17, false);
  const currentDuplicate = queueItem(4, 29, true);
  const page = {
    revision: 11,
    total: 50,
    offset: 0,
    cursor: 4,
    currentEntryPosition: 29,
    items: [firstDuplicate, currentDuplicate],
  };

  const currentPosition = playbackQueueCurrentEntryPosition(page);
  assert.equal(isCurrentPlaybackQueueEntry(firstDuplicate, currentPosition), false);
  assert.equal(isCurrentPlaybackQueueEntry(currentDuplicate, currentPosition), true);
  assert.equal(isCurrentPlaybackQueueEntry(currentDuplicate, null), true);
});

test('queue page adapter rejects invalid offsets, empty interior pages, and unsafe totals', () => {
  const valid = {
    revision: 1,
    total: 20,
    offset: 0,
    cursor: 0,
    currentEntryPosition: 0,
    items: [queueItem(0, 0, true)],
  };

  assert.throws(() => adaptPlaybackQueuePage({ ...valid, offset: 40 }, { offset: 0, limit: 40 }), /格式不符/);
  assert.throws(() => adaptPlaybackQueuePage({ ...valid, offset: 10, items: [] }, { offset: 10, limit: 40 }), /格式不符/);
  assert.throws(() => adaptPlaybackQueuePage({ ...valid, total: Number.MAX_SAFE_INTEGER + 1 }, { offset: 0, limit: 40 }), /超出支援範圍/);
  assert.throws(() => playbackQueueCurrentEntryPosition({ ...valid, currentEntryPosition: Number.MAX_SAFE_INTEGER + 1 }), /超出支援範圍/);
});
