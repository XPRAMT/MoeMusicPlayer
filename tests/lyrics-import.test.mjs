import test from 'node:test';
import assert from 'node:assert/strict';
import { formatLrcTimestamp, isOnlineLyrics, lyricsToText } from '../src/lib/lyrics-import.js';

test('only online providers qualify, including manual picks of online candidates', () => {
  const lines = [{ startMs: 0, text: 'a' }];
  assert.equal(isOnlineLyrics({ source: 'netease', provider: 'netease', lines }), true);
  assert.equal(isOnlineLyrics({ source: 'manual', provider: 'qqmusic', lines }), true);
  assert.equal(isOnlineLyrics({ source: 'manual', provider: 'local', lines }), false);
  assert.equal(isOnlineLyrics({ source: 'embedded', provider: 'embedded', lines }), false);
  assert.equal(isOnlineLyrics({ source: 'netease', provider: 'netease', lines: [] }), false);
  assert.equal(isOnlineLyrics(null), false);
});

test('lrc timestamps and serialization', () => {
  assert.equal(formatLrcTimestamp(61234), '[01:01.23]');
  assert.equal(
    lyricsToText({ synced: true, lines: [{ startMs: 1000, text: '一', translation: 'one' }, { startMs: 2500, text: '二' }] }),
    '[00:01.00]一\n[00:01.00]one\n[00:02.50]二',
  );
  assert.equal(lyricsToText({ synced: false, lines: [{ startMs: null, text: 'x' }] }), 'x');
});
