import assert from 'node:assert/strict';
import test from 'node:test';
import { buildTaskbarLyricsData } from '../src/lib/taskbar-lyrics-data.js';

const lyrics = (overrides = {}) => ({
  trackId: 'a',
  synced: true,
  offsetMs: 250,
  lines: [
    { startMs: 1000, text: '第一行', translation: 'first', romanization: 'x' },
    { startMs: null, text: '無時間', translation: null, romanization: null },
    { startMs: 2000, text: '第二行', translation: null, romanization: null },
  ],
  ...overrides,
});

test('synced lyrics become timed lines with only what the taskbar draws', () => {
  assert.deepEqual(buildTaskbarLyricsData('a', lyrics()), {
    trackId: 'a',
    offsetMs: 250,
    lines: [
      { startMs: 1000, text: '第一行', translation: 'first', romanization: null },
      { startMs: 2000, text: '第二行', translation: null, romanization: null },
    ],
  });
});

test('a line\'s own end (SRT/ASS, yrc/QRC) is carried to the native windows', () => {
  const data = buildTaskbarLyricsData('a', lyrics({
    lines: [
      { startMs: 1000, endMs: 3500, text: '第一行', translation: null, romanization: null },
      { startMs: 4000, endMs: null, text: '第二行', translation: null, romanization: null },
    ],
  }));
  assert.deepEqual(data.lines, [
    { startMs: 1000, endMs: 3500, text: '第一行', translation: null, romanization: null },
    { startMs: 4000, text: '第二行', translation: null, romanization: null },
  ]);
});

test('plain lyrics, no lyrics and lyrics of another track give no lines', () => {
  const none = { trackId: 'a', offsetMs: 0, lines: [] };
  assert.deepEqual(buildTaskbarLyricsData('a', lyrics({ synced: false })), none);
  assert.deepEqual(buildTaskbarLyricsData('a', null), none);
  assert.deepEqual(buildTaskbarLyricsData('a', undefined), none);
  assert.deepEqual(buildTaskbarLyricsData('b', lyrics()), { trackId: 'b', offsetMs: 0, lines: [] });
  assert.deepEqual(buildTaskbarLyricsData(null, null), { trackId: null, offsetMs: 0, lines: [] });
});
