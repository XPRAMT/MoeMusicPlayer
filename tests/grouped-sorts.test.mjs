import assert from 'node:assert/strict';
import test from 'node:test';
import { GROUPED_SORT_FIELDS, isGroupedSortField } from '../src/lib/grouped-sorts.js';

test('names, release and added time, and audio format are grouped; length and play count are not', () => {
  for (const field of ['title', 'artist', 'album', 'year', 'createdAt', 'audioFormat']) {
    assert.equal(isGroupedSortField(field), true, field);
  }
  for (const field of ['duration', 'playCount', 'position', '', null, undefined, 3]) {
    assert.equal(isGroupedSortField(field), false, String(field));
  }
  assert.equal(GROUPED_SORT_FIELDS.length, 6);
});
