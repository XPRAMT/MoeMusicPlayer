import assert from 'node:assert/strict';
import test from 'node:test';
import { lyricsForDisplay } from '../src/lib/lyrics-traditional.js';

const lyrics = {
  trackId: 'track-1',
  source: 'local',
  title: '头发',
  artist: null,
  album: null,
  durationMs: 3000,
  offsetMs: 0,
  synced: true,
  lines: [
    { startMs: 0, text: '头发与后面', translation: '软件界面', romanization: 'toufa', textTraditional: '頭髮與後面', translationTraditional: '軟件界面' },
    { startMs: 1000, text: '头发を切った後', translation: null, romanization: null },
    { startMs: 2000, text: '', translation: null, romanization: null },
  ],
};

test('the Traditional preference shows the backend-converted spelling of each line', () => {
  const shown = lyricsForDisplay(lyrics, true);
  assert.deepEqual(shown.lines.map((line) => line.text), ['頭髮與後面', '头发を切った後', '']);
  assert.equal(shown.lines[0].translation, '軟件界面');
  assert.equal(shown.lines[0].romanization, 'toufa');
  assert.equal(shown.title, '头发', 'only lines are converted');
  assert.equal(shown.trackId, 'track-1');
  assert.equal(lyrics.lines[0].text, '头发与后面', 'the stored lyrics are not changed');
});

test('without the preference, or with nothing to convert, the lyrics are returned as they are', () => {
  assert.equal(lyricsForDisplay(lyrics, false), lyrics);
  const plain = { ...lyrics, lines: [{ startMs: 0, text: '頭髮', translation: null, romanization: null }] };
  assert.equal(lyricsForDisplay(plain, true), plain);
});
