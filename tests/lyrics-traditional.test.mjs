import assert from 'node:assert/strict';
import test from 'node:test';
import { convertTrackLyrics } from '../src/lib/lyrics-traditional.js';

test('simplified lyrics convert to traditional glyphs without replacing wording', async () => {
  const converted = await convertTrackLyrics({
    trackId: 'track-1',
    source: 'local',
    title: '头发',
    artist: null,
    album: null,
    durationMs: 1000,
    offsetMs: 0,
    synced: true,
    lines: [
      { startMs: 0, text: '头发与后面', translation: '软件界面', romanization: 'toufa' },
      { startMs: 1000, text: '', translation: null, romanization: null },
    ],
  });

  assert.equal(converted.lines[0].text, '頭髮與後面');
  assert.equal(converted.lines[0].translation, '軟件界面');
  assert.equal(converted.lines[0].romanization, 'toufa');
  assert.equal(converted.lines[1].text, '');
  assert.equal(converted.title, '头发');
  assert.notEqual(converted.lines[0].translation, '軟體界面');
});
