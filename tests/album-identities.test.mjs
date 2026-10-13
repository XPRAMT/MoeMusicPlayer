import test from 'node:test';
import assert from 'node:assert/strict';
import { buildAlbumEntries, findAlbumEntryByName } from '../src/lib/album-identities.js';

test('album labels merge simplified and traditional aliases but keep unmarked separate from literal 其它', async () => {
  const albums = await buildAlbumEntries([
    { name: '专辑 A', trackCount: 2 },
    { name: '專輯 A', trackCount: 3 },
    { name: '\u3000專輯 A\u3000', trackCount: 1 },
    { name: null, trackCount: 4 },
    { name: '其它', trackCount: 1 },
    { name: '  ', trackCount: 2 },
  ]);
  assert.equal(albums.length, 3);
  const merged = albums.find((album) => album.displayName === '專輯 A');
  assert.equal(merged?.aliases.includes('\u3000專輯 A\u3000'), true);
  assert.equal(merged?.aliases.includes('專輯 A'), true);
  assert.equal(merged?.trackCount, 6);
  const unmarked = albums.find((album) => album.unmarked);
  assert.equal(unmarked?.displayName, '其它');
  assert.ok(unmarked?.aliases.includes(''));
  assert.ok(unmarked?.aliases.includes('  '));
  assert.equal(unmarked?.trackCount, 6);
  const literal = albums.find((album) => !album.unmarked && album.displayName === '其它');
  assert.equal(literal?.trackCount, 1);
});

test('album identity ignores case and Japanese kana keeps its kanji spelling', async () => {
  const albums = await buildAlbumEntries([
    { name: 'Game Sound', trackCount: 1 },
    { name: 'game sound', trackCount: 1 },
    { name: '君の名は。', trackCount: 1 },
    { name: '君の名は。', trackCount: 2 },
  ]);
  assert.equal(albums.find((album) => album.aliases.length === 2)?.trackCount, 2);
  const japanese = await findAlbumEntryByName('君の名は。', albums);
  assert.equal(japanese?.displayName, '君の名は。');
  assert.equal(japanese?.trackCount, 3);
});

test('album display name prefers the more frequent original form, then Traditional form on a tie', async () => {
  const majority = await buildAlbumEntries([
    { name: '专辑 A', trackCount: 1 },
    { name: '專輯 A', trackCount: 2 },
  ]);
  assert.equal(majority[0]?.displayName, '專輯 A');

  const tie = await buildAlbumEntries([
    { name: '专辑 A', trackCount: 2 },
    { name: '專輯 A', trackCount: 2 },
  ]);
  assert.equal(tie[0]?.displayName, '專輯 A');
});
