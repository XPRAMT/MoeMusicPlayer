import test from 'node:test';
import assert from 'node:assert/strict';
import { formatLrcTimestamp, hasLyricsInUse, lyricsToText } from '../src/lib/lyrics-import.js';

test('any lyrics in use qualify regardless of source', () => {
  const lines = [{ startMs: 0, text: 'a' }];
  for (const source of ['netease', 'qqmusic', 'manual', 'sidecar', 'embedded']) {
    assert.equal(hasLyricsInUse({ source, lines }), true);
  }
  assert.equal(hasLyricsInUse({ source: 'netease', lines: [] }), false);
  assert.equal(hasLyricsInUse(null), false);
});

test('lrc timestamps and serialization keep translation and romanization lines', () => {
  assert.equal(formatLrcTimestamp(61234), '[01:01.23]');
  assert.equal(
    lyricsToText({ synced: true, lines: [{ startMs: 1000, text: '一', translation: 'one', romanization: 'ichi' }, { startMs: 2500, text: '二' }] }),
    '[00:01.00]一\n[00:01.00]one\n[00:01.00]ichi\n[00:02.50]二',
  );
  assert.equal(lyricsToText({ synced: false, lines: [{ startMs: null, text: 'x' }] }), 'x');
});
