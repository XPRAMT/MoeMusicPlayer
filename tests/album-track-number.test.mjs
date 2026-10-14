import assert from 'node:assert/strict';
import { test } from 'node:test';
import { formatAlbumTrackNumber } from '../src/lib/format.ts';

test('album detail index shows track number without repeating its Disc group', () => {
  assert.equal(formatAlbumTrackNumber(1), '1');
  assert.equal(formatAlbumTrackNumber(12), '12');
  assert.equal(formatAlbumTrackNumber(null), '—');
});
