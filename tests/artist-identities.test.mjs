import assert from 'node:assert/strict';
import test from 'node:test';
import { buildArtistIdentityMappings, findArtistEntryByName, preferredArtistName } from '../src/lib/artist-identities.js';

test('artist identity merges traditional and simplified Chinese names but preserves Japanese kana spelling', async () => {
  const entries = await buildArtistIdentityMappings(['苹果后台', '蘋果後臺', '花の歌', '花の歌']);
  assert.equal(entries[0].key, entries[1].key);
  assert.equal(entries[2].key, '花の歌');
  assert.equal(entries[2].traditional, '花の歌');
});

test('artist spelling uses the most frequent raw label and traditional spelling on ties', () => {
  const traditionalByName = new Map([
    ['后台', '後臺'],
    ['後臺', '後臺'],
  ]);
  assert.equal(preferredArtistName([
    { name: '后台', count: 2 },
    { name: '後臺', count: 2 },
  ], traditionalByName), '後臺');
  assert.equal(preferredArtistName([
    { name: '后台', count: 3 },
    { name: '後臺', count: 2 },
  ], traditionalByName), '后台');
});

test('now-playing artist navigation resolves any merged raw spelling to its artist detail entry', async () => {
  const [identity] = await buildArtistIdentityMappings(['Reol']);
  const entry = { key: identity.key, artistNames: ['REOL', 'Reol'], displayName: 'Reol' };
  assert.equal(await findArtistEntryByName('REOL', [entry]), entry);
  assert.equal(await findArtistEntryByName('Reol', [entry]), entry);
  assert.equal(await findArtistEntryByName('Unrelated', [entry]), null);
});
