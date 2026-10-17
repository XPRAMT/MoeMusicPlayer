import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DEFAULT_TASKBAR_LYRICS_SETTINGS,
  fitTaskbarLyricsFontSize,
  getLyricScrollProgress,
  getTaskbarLyricsColor,
  normalizeTaskbarLyricsSettings,
} from '../src/lib/taskbar-lyrics-settings.js';

test('taskbar lyrics settings are repaired field by field', () => {
  assert.deepEqual(normalizeTaskbarLyricsSettings(null), DEFAULT_TASKBAR_LYRICS_SETTINGS);
  assert.deepEqual(normalizeTaskbarLyricsSettings({
    enabled: true, align: 'center', fontSizePx: 20.4, translationScalePercent: 60.4, useCustomColor: true, customColor: '#ABCDEF', showTranslation: true,
  }), { enabled: true, align: 'center', fontSizePx: 20, translationScalePercent: 60, useCustomColor: true, customColor: '#abcdef', showTranslation: true });
  const repaired = normalizeTaskbarLyricsSettings({ enabled: 'yes', area: 'up', align: 3, fontSizePx: 99, customColor: 'red', showTranslation: 1 });
  assert.deepEqual(repaired, { ...DEFAULT_TASKBAR_LYRICS_SETTINGS, fontSizePx: 50 });
  assert.equal(normalizeTaskbarLyricsSettings({ fontSizePx: 1 }).fontSizePx, 1);
  assert.equal(normalizeTaskbarLyricsSettings({ fontSizePx: 0 }).fontSizePx, 1, 'the font starts at 1 px');
  assert.equal(normalizeTaskbarLyricsSettings({ fontSizePx: 50 }).fontSizePx, 50);
  assert.equal(normalizeTaskbarLyricsSettings({ translationScalePercent: 0 }).translationScalePercent, 1);
  assert.equal(normalizeTaskbarLyricsSettings({ translationScalePercent: 100 }).translationScalePercent, 100);
  assert.equal(normalizeTaskbarLyricsSettings({ translationScalePercent: 400 }).translationScalePercent, 100);
  assert.equal(normalizeTaskbarLyricsSettings({ translationScalePercent: 'half' }).translationScalePercent, 75);
  assert.equal(normalizeTaskbarLyricsSettings({ fontSizePx: 'big' }).fontSizePx, 16);
});

test('colour follows the taskbar theme unless a custom colour is chosen', () => {
  assert.equal(getTaskbarLyricsColor(DEFAULT_TASKBAR_LYRICS_SETTINGS, true), '#000000');
  assert.equal(getTaskbarLyricsColor(DEFAULT_TASKBAR_LYRICS_SETTINGS, false), '#ffffff');
  assert.equal(getTaskbarLyricsColor({ ...DEFAULT_TASKBAR_LYRICS_SETTINGS, useCustomColor: true, customColor: '#ff8800' }, true), '#ff8800');
});

test('the font shrinks to fit the window height, more with a translation line', () => {
  assert.equal(fitTaskbarLyricsFontSize(16, 32, 0), 16);
  assert.equal(fitTaskbarLyricsFontSize(32, 32, 0), 22);
  assert.equal(fitTaskbarLyricsFontSize(16, 32, 75), 13);
  assert.equal(fitTaskbarLyricsFontSize(16, 32, 100), 11, 'a translation as large as the original takes two full lines');
  assert.equal(fitTaskbarLyricsFontSize(16, 32, 1), 16, 'a tiny translation barely takes room');
  assert.equal(fitTaskbarLyricsFontSize(16, 0, 75), 16);
  assert.equal(fitTaskbarLyricsFontSize(16, 1, 0), 1, 'the font can go down to 1 px');
});

test('a line wider than the window scrolls over the time it is shown and rests at the end', () => {
  assert.equal(getLyricScrollProgress(0, 4000), 0);
  assert.equal(getLyricScrollProgress(1800, 4000), 0.5);
  assert.equal(getLyricScrollProgress(3600, 4000), 1);
  assert.equal(getLyricScrollProgress(3900, 4000), 1, 'the last tenth rests');
  assert.equal(getLyricScrollProgress(-50, 4000), 0);
  assert.equal(getLyricScrollProgress(500, 0), 0, 'no duration, no scroll');
  assert.equal(getLyricScrollProgress(Number.NaN, 4000), 0);
});
