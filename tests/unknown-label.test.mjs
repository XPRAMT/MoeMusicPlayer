import assert from 'node:assert/strict';
import test from 'node:test';
import { displayPlaylistName, displayUnknown } from '../src/lib/unknown-label.js';
import { setLanguageForTesting } from '../src/lib/i18n.js';

test('only the backend 未知 placeholder is translated, real names are untouched', () => {
  setLanguageForTesting('zh-TW');
  assert.equal(displayUnknown('未知'), '未知');
  assert.equal(displayUnknown('漢字'), '漢字');
  assert.equal(displayUnknown('Daft Punk'), 'Daft Punk');
  setLanguageForTesting('zh-CN');
  assert.equal(displayUnknown('未知'), '未知', 'Simplified shows 未知 too');
  assert.equal(displayUnknown('漢字'), '漢字', 'the 漢字 group is only produced for Japanese collation and is not translated');
  setLanguageForTesting('ja');
  assert.equal(displayUnknown('未知'), '未知');
  setLanguageForTesting('en');
  assert.equal(displayUnknown('未知'), 'unknown');
  assert.equal(displayUnknown('漢字'), '漢字');
  setLanguageForTesting('zh-TW');
});

test('the favourites playlist name from the backend is translated, other playlist names are untouched', () => {
  setLanguageForTesting('zh-TW');
  assert.equal(displayPlaylistName('我的最愛'), '我的最愛');
  setLanguageForTesting('zh-CN');
  assert.equal(displayPlaylistName('我的最愛'), '我的最爱');
  setLanguageForTesting('ja');
  assert.equal(displayPlaylistName('我的最愛'), 'お気に入り');
  setLanguageForTesting('en');
  assert.equal(displayPlaylistName('我的最愛'), 'Favourites');
  assert.equal(displayPlaylistName(' Mix '), 'Mix');
  assert.equal(displayPlaylistName('  '), 'Untitled playlist');
  setLanguageForTesting('zh-TW');
});
