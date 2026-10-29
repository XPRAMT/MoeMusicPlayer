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
  assert.match(dialog, /saveLyricsTimingOffsetSec\(id, 0\)\.then\(\(\) => emit\(LYRICS_TIMING_OFFSET_EVENT, \{ trackId: id, delaySec: 0 \}\)\)/);
  assert.match(dialog, /resetDelayBakedIntoSavedLyrics\(isDirty\('lyrics'\)\)/);
  assert.match(dialog, /function clearLyrics\(\) \{[^}]*nextBakedDelayState\(bakedDelayPending, \{ type: 'clear' \}\)/);
  assert.match(dialog, /nextBakedDelayState\(bakedDelayPending, \{ type: 'import', delaySec: timingDelaySec \}\)/);
  const view = await readFile(new URL('../src/lib/LyricsView.svelte', import.meta.url), 'utf8');
  assert.match(view, /listen<[^>]*>\(LYRICS_TIMING_OFFSET_EVENT[^]*?loadLyricsTimingOffsetSec\(shown\)\.then\(\(value\) => \{ if \(untrack\(\(\) => trackId\) === shown\) timingOffsetSec = value; \}\)/);
});

test('a delay baked in by an import is reset by the next Save that writes the lyrics, edited or not', async () => {
  const { nextBakedDelayState } = await import('../src/lib/lyrics-import.js');
  /** @param {any[]} events */
  const run = (events) => {
    let pending = false;
    const resets = [];
    for (const event of events) {
      const next = nextBakedDelayState(pending, event);
      pending = next.pending;
      if (event.type === 'save') resets.push(next.resetDelay);
    }
    return resets;
  };
  const save = { type: 'save', lyricsDirty: true };
  // Import + save resets; editing the imported lyrics in between (still dirty) does not matter.
  assert.deepEqual(run([{ type: 'import', delaySec: 1.5 }, save]), [true]);
  assert.deepEqual(run([{ type: 'import', delaySec: -0.5 }, /* edit */ save]), [true]);
  // Import with delay 0 bakes nothing.
  assert.deepEqual(run([{ type: 'import', delaySec: 0 }, save]), [false]);
  // 清空 after the import cancels the reset.
  assert.deepEqual(run([{ type: 'import', delaySec: 1.5 }, { type: 'clear' }, save]), [false]);
  // Re-import with delay 0 after a non-zero import cancels it.
  assert.deepEqual(run([{ type: 'import', delaySec: 1.5 }, { type: 'import', delaySec: 0 }, save]), [false]);
  // A second save after the reset does not reset again.
  assert.deepEqual(run([{ type: 'import', delaySec: 1.5 }, save, save]), [true, false]);
  // A save that does not write the lyrics field never resets, and still clears the pending state.
  assert.deepEqual(run([{ type: 'import', delaySec: 1.5 }, { type: 'save', lyricsDirty: false }, save]), [false, false]);
});

test('the import button is always enabled', async () => {
  const { readFile } = await import('node:fs/promises');
  const dialog = await readFile(new URL('../src/lib/TrackDetailsDialog.svelte', import.meta.url), 'utf8');
  const button = dialog.match(/<button[^>]*onclick={importOnlineLyrics}[^>]*>/);
  assert.ok(button, 'import button found');
  assert.doesNotMatch(button[0], /disabled/);
});

test('word-timed lines are written as enhanced LRC; line-only lyrics stay as before', () => {
  const lyrics = {
    synced: true,
    lines: [
      {
        startMs: 1000,
        text: 'Hello world',
        translation: '你好世界',
        words: [
          { s: 1000, e: 1500, t: 'Hello ' },
          { s: 1500, e: 2253, t: 'world' },
        ],
      },
      { startMs: 3000, text: 'a b', words: [{ s: 3000, e: 3200, t: 'a ' }, { s: 3500, e: 3900, t: 'b' }] },
      { startMs: 5000, text: 'plain line' },
    ],
  };
  assert.equal(
    lyricsToText(lyrics),
    [
      '[00:01.00]<00:01.00>Hello <00:01.50>world<00:02.25>',
      '[00:01.00]你好世界',
      '[00:03.00]<00:03.00>a <00:03.20><00:03.50>b<00:03.90>',
      '[00:05.00]plain line',
    ].join('\n'),
  );
  // The shift moves word stamps too.
  assert.match(lyricsToText(lyrics, 100), /^\[00:01\.10\]<00:01\.10>Hello <00:01\.60>world<00:02\.35>/);
  assert.equal(lyricsToText({ synced: true, lines: [{ startMs: 5000, text: 'plain line' }] }), '[00:05.00]plain line');
});

test('a line with its own end but no word timing keeps the end as one enhanced LRC word', () => {
  const lyrics = {
    synced: true,
    lines: [
      { startMs: 1000, endMs: 3250, text: 'Hello world', translation: '你好世界' },
      { startMs: 5000, text: 'no end' },
      { startMs: 6000, endMs: 6000, text: 'end not after start' },
      { startMs: 7000, endMs: 9000, text: 'ab', words: [{ s: 7000, e: 7500, t: 'a' }, { s: 7500, e: 8000, t: 'b' }] },
    ],
  };
  assert.equal(
    lyricsToText(lyrics),
    [
      '[00:01.00]<00:01.00>Hello world<00:03.25>',
      '[00:01.00]你好世界',
      '[00:05.00]no end',
      '[00:06.00]end not after start',
      // Word timing wins over the line's own end.
      '[00:07.00]<00:07.00>a<00:07.50>b<00:08.00>',
    ].join('\n'),
  );
  // The shift moves the end too.
  assert.match(lyricsToText(lyrics, 500), /^\[00:01\.50\]<00:01\.50>Hello world<00:03\.75>/);
});

test('translation rows stay line-level; romanization gets word stamps only with word timing', () => {
  const words = [{ s: 1000, e: 1500, t: 'ni ' }, { s: 1500, e: 2000, t: 'hao' }];
  const base = { startMs: 1000, text: 'ab', words: [{ s: 1000, e: 1500, t: 'a' }, { s: 1500, e: 2000, t: 'b' }], translation: 'AB', romanization: 'ni hao' };
  assert.equal(
    lyricsToText({ synced: true, lines: [{ ...base, romanizationWords: words }] }),
    '[00:01.00]<00:01.00>a<00:01.50>b<00:02.00>\n[00:01.00]AB\n[00:01.00]<00:01.00>ni <00:01.50>hao<00:02.00>',
  );
  assert.equal(lyricsToText({ synced: true, lines: [base] }).split('\n')[2], '[00:01.00]ni hao');
});
