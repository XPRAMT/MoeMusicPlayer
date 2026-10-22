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

test('import bakes the lyrics delay into every timestamp, same sign as lyricsDisplayOffsetMs', async () => {
  const { importTimestampShiftMs } = await import('../src/lib/lyrics-import.js');
  const { findActiveLyricIndex } = await import('../src/lib/lyrics-window.js');
  const { lyricsDisplayOffsetMs } = await import('../src/lib/lyrics-timing-offset.js');
  const lyrics = { synced: true, lines: [{ startMs: 300, text: '一', translation: 'one' }, { startMs: 5000, text: '二' }] };
  // +1.5 s delay: lyrics come later, so the written timestamps are 1.5 s later.
  assert.equal(importTimestampShiftMs(0, 1.5), 1500);
  assert.equal(lyricsToText(lyrics, importTimestampShiftMs(0, 1.5)), '[00:01.80]一\n[00:01.80]one\n[00:06.50]二');
  // −1 s: earlier; a timestamp that would be negative is written as 0.
  assert.equal(lyricsToText(lyrics, importTimestampShiftMs(0, -1)), '[00:00.00]一\n[00:00.00]one\n[00:04.00]二');
  assert.equal(importTimestampShiftMs(null, 0), 0);
  assert.equal(lyricsToText(lyrics, 0), lyricsToText(lyrics));
  // Shown now with the delay == shown after import with the delay reset to 0.
  const timeline = lyrics.lines;
  const shifted = lyrics.lines.map((line) => ({ ...line, startMs: line.startMs + importTimestampShiftMs(250, 1.5) }));
  for (const position of [0, 1000, 1549, 1550, 1551, 6000, 6249, 6250]) {
    assert.equal(findActiveLyricIndex(shifted, position, 0), findActiveLyricIndex(timeline, position, lyricsDisplayOffsetMs(250, 1.5)), `at ${position} ms`);
  }
});

test('the details page resets the delay after saving imported lyrics and the playback page follows the event', async () => {
  const { readFile } = await import('node:fs/promises');
  const dialog = await readFile(new URL('../src/lib/TrackDetailsDialog.svelte', import.meta.url), 'utf8');
  assert.match(dialog, /lyricsToText\(lyricsInUse, importTimestampShiftMs\(lyricsInUse\.offsetMs, timingDelaySec\)\)/);
  assert.match(dialog, /saveLyricsTimingOffsetSec\(trackId, 0\)[^]*?emit\(LYRICS_TIMING_OFFSET_EVENT, \{ trackId, delaySec: 0 \}\)/);
  assert.match(dialog, /if \(isDirty\('lyrics'\)\) resetDelayBakedIntoSavedLyrics\(/);
  const view = await readFile(new URL('../src/lib/LyricsView.svelte', import.meta.url), 'utf8');
  assert.match(view, /listen<[^>]*>\(LYRICS_TIMING_OFFSET_EVENT[^]*?timingOffsetSec = loadLyricsTimingOffsetSec\(shown\)/);
});

test('the import button is always enabled', async () => {
  const { readFile } = await import('node:fs/promises');
  const dialog = await readFile(new URL('../src/lib/TrackDetailsDialog.svelte', import.meta.url), 'utf8');
  const button = dialog.match(/<button[^>]*onclick={importOnlineLyrics}[^>]*>/);
  assert.ok(button, 'import button found');
  assert.doesNotMatch(button[0], /disabled/);
});
