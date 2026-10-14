import test from 'node:test';
import assert from 'node:assert/strict';
import { buildAlbumEntries, findAlbumEntryByName, sortAlbumEntries } from '../src/lib/album-identities.js';

test('album labels merge simplified and traditional aliases but keep Unknown separate from literal 其它', async () => {
  const albums = await buildAlbumEntries([
    { name: '专辑 A', trackCount: 2, durationMs: 20_000, playedMs: 200 },
    { name: '專輯 A', trackCount: 3, durationMs: 30_000, playedMs: 300 },
    { name: '\u3000專輯 A\u3000', trackCount: 1, durationMs: 10_000, playedMs: 100 },
    { name: null, trackCount: 4, durationMs: 40_000, playedMs: 400 },
    { name: '其它', trackCount: 1, durationMs: 10_000, playedMs: 10 },
    { name: '  ', trackCount: 2, durationMs: 20_000, playedMs: 20 },
  ]);
  assert.equal(albums.length, 3);
  const merged = albums.find((album) => album.displayName === '專輯 A');
  assert.equal(merged?.aliases.includes('\u3000專輯 A\u3000'), true);
  assert.equal(merged?.aliases.includes('專輯 A'), true);
  assert.equal(merged?.trackCount, 6);
  assert.equal(merged?.durationMs, 60_000);
  assert.equal(merged?.playedMs, 600);
  const unmarked = albums.find((album) => album.unmarked);
  assert.equal(unmarked?.displayName, '未知');
  assert.ok(unmarked?.aliases.includes(''));
  assert.ok(unmarked?.aliases.includes('  '));
  assert.equal(unmarked?.trackCount, 6);
  assert.equal(unmarked?.durationMs, 60_000);
  assert.equal(unmarked?.playedMs, 420);
  const literal = albums.find((album) => !album.unmarked && album.displayName === '其它');
  assert.equal(literal?.trackCount, 1);
});

test('album index sorts each metric and keeps ICU album order for ties', async () => {
  const albums = await buildAlbumEntries([
    { name: 'Alpha', trackCount: 2, durationMs: 5000, playedMs: 1000 },
    { name: 'Beta', trackCount: 1, durationMs: 5000, playedMs: 3000 },
    { name: 'Gamma', trackCount: 3, durationMs: 9000, playedMs: 1000 },
  ]);
  const titleRanks = new Map([['album:alpha', 0], ['album:beta', 1], ['album:gamma', 2]]);

  assert.deepEqual(sortAlbumEntries(albums, 'title', 'asc', titleRanks).map((album) => album.displayName), ['Alpha', 'Beta', 'Gamma']);
  assert.deepEqual(sortAlbumEntries(albums, 'duration', 'desc', titleRanks).map((album) => album.displayName), ['Gamma', 'Alpha', 'Beta']);
  assert.deepEqual(sortAlbumEntries(albums, 'played', 'asc', titleRanks).map((album) => album.displayName), ['Alpha', 'Gamma', 'Beta']);
  assert.deepEqual(sortAlbumEntries(albums, 'trackCount', 'desc', titleRanks).map((album) => album.displayName), ['Gamma', 'Alpha', 'Beta']);
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
