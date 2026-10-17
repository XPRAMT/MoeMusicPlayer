import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DEFAULT_TASKBAR_LYRICS_SETTINGS,
  fitTaskbarLyricsFontSize,
  getTaskbarLyricsColor,
  normalizeTaskbarLyricsSettings,
} from '../src/lib/taskbar-lyrics-settings.js';

test('taskbar lyrics settings are repaired field by field', () => {
  assert.deepEqual(normalizeTaskbarLyricsSettings(null), DEFAULT_TASKBAR_LYRICS_SETTINGS);
  assert.deepEqual(normalizeTaskbarLyricsSettings({
    enabled: true, align: 'center', fontSizePx: 20.4, useCustomColor: true, customColor: '#ABCDEF', showTranslation: true,
  }), { enabled: true, align: 'center', fontSizePx: 20, useCustomColor: true, customColor: '#abcdef', showTranslation: true });
  const repaired = normalizeTaskbarLyricsSettings({ enabled: 'yes', area: 'up', align: 3, fontSizePx: 99, customColor: 'red', showTranslation: 1 });
  assert.deepEqual(repaired, { ...DEFAULT_TASKBAR_LYRICS_SETTINGS, fontSizePx: 32 });
  assert.equal(normalizeTaskbarLyricsSettings({ fontSizePx: 1 }).fontSizePx, 10);
  assert.equal(normalizeTaskbarLyricsSettings({ fontSizePx: 'big' }).fontSizePx, 16);
});

test('colour follows the taskbar theme unless a custom colour is chosen', () => {
  assert.equal(getTaskbarLyricsColor(DEFAULT_TASKBAR_LYRICS_SETTINGS, true), '#000000');
  assert.equal(getTaskbarLyricsColor(DEFAULT_TASKBAR_LYRICS_SETTINGS, false), '#ffffff');
  assert.equal(getTaskbarLyricsColor({ ...DEFAULT_TASKBAR_LYRICS_SETTINGS, useCustomColor: true, customColor: '#ff8800' }, true), '#ff8800');
});

test('the font shrinks to fit the window height, more with a translation line', () => {
  assert.equal(fitTaskbarLyricsFontSize(16, 32, false), 16);
  assert.equal(fitTaskbarLyricsFontSize(32, 32, false), 25);
  assert.equal(fitTaskbarLyricsFontSize(16, 32, true), 14);
  assert.equal(fitTaskbarLyricsFontSize(16, 0, true), 16);
  assert.equal(fitTaskbarLyricsFontSize(16, 4, false), 8);
});
