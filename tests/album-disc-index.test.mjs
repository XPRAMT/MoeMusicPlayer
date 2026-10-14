import test from 'node:test';
import assert from 'node:assert/strict';
import { buildAlbumDiscQuickIndex } from '../src/lib/album-disc-index.js';

test('album quick index groups only disc 2+ and places unmarked tracks last', () => {
  const result = buildAlbumDiscQuickIndex([
    { discNumber: 0, count: 4 },
    { discNumber: 1, count: 8 },
    { discNumber: 2, count: 3 },
    { discNumber: 4, count: 2 },
  ], 5);

  assert.deepEqual(result, {
    labels: ['DISC 2', 'DISC 4', '未知'],
    groups: [
      { label: 'DISC 2', beforeIndex: 0 },
      { label: 'DISC 4', beforeIndex: 3 },
      { label: '未知', beforeIndex: 5 },
    ],
  });
});

test('album quick index follows descending disc order but keeps unmarked last', () => {
  const result = buildAlbumDiscQuickIndex([
    { discNumber: 2, count: 3 },
    { discNumber: 4, count: 2 },
  ], 4, 'desc');

  assert.deepEqual(result, {
    labels: ['DISC 4', 'DISC 2', '未知'],
    groups: [
      { label: 'DISC 4', beforeIndex: 0 },
      { label: 'DISC 2', beforeIndex: 2 },
      { label: '未知', beforeIndex: 5 },
    ],
  });
});

test('album without disc 2+ has no disc quick index, even if disc 0 and 1 tags exist', () => {
  assert.deepEqual(buildAlbumDiscQuickIndex([
    { discNumber: 0, count: 3 },
    { discNumber: 1, count: 8 },
  ], 2), { labels: [], groups: [] });
});
