import assert from 'node:assert/strict';
import test from 'node:test';
import { splitArtists } from '../src/lib/artist-names.js';

test('artist tags split on the supported separators and keep each name once', () => {
  assert.deepEqual(splitArtists('hanser/yousa'), ['hanser', 'yousa']);
  assert.deepEqual(splitArtists('hanser | yousa'), ['hanser', 'yousa']);
  assert.deepEqual(splitArtists('hanser\\yousa'), ['hanser', 'yousa']);
  assert.deepEqual(splitArtists('hanser; yousa'), ['hanser', 'yousa']);
  assert.deepEqual(splitArtists('hanser,yousa'), ['hanser', 'yousa']);
  assert.deepEqual(splitArtists('John Lennon'), ['John Lennon']);
  assert.deepEqual(splitArtists('John Lennon, Paul McCartney'), ['John Lennon', 'Paul McCartney']);
  assert.deepEqual(splitArtists('hanser//yousa/hanser'), ['hanser', 'yousa']);
  assert.deepEqual(splitArtists('  hanser  '), ['hanser']);
  assert.deepEqual(splitArtists('///'), []);
  assert.deepEqual(splitArtists(null), []);
});
