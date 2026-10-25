import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DEFAULT_TASKBAR_LYRICS_SETTINGS,
  fitTaskbarLyricsFontSize,
  getLyricScrollProgress,
  getTaskbarLyricsColor,
  getTaskbarLyricsEffectRgb,
  normalizeTaskbarLyricsSettings,
  toggleTaskbarLyricsMonitor,
} from '../src/lib/taskbar-lyrics-settings.js';

test('taskbar lyrics settings are repaired field by field', () => {
  assert.deepEqual(normalizeTaskbarLyricsSettings(null), DEFAULT_TASKBAR_LYRICS_SETTINGS);
  assert.deepEqual(normalizeTaskbarLyricsSettings({
    enabled: true, align: 'center', fontSizePx: 20.4, translationScalePercent: 60.4, useCustomColor: true, customColor: '#ABCDEF', showTranslation: true,
  }), { enabled: true, align: 'center', fontSizePx: 20, translationScalePercent: 60, useCustomColor: true, customColor: '#abcdef', showTranslation: true, textEffect: 'shadow', displayMode: 'all', customMonitors: [], mainOpacityPercent: 100, secondaryOpacityPercent: 50, fontWeight: 500, animation: 'none', hideWhenPaused: true });
  const repaired = normalizeTaskbarLyricsSettings({ enabled: 'yes', area: 'up', align: 3, fontSizePx: 99, customColor: 'red', showTranslation: 1 });
  assert.deepEqual(repaired, { ...DEFAULT_TASKBAR_LYRICS_SETTINGS, fontSizePx: 50 });
  assert.equal(normalizeTaskbarLyricsSettings({ fontSizePx: 1 }).fontSizePx, 1);
  assert.equal(normalizeTaskbarLyricsSettings({ fontSizePx: 0 }).fontSizePx, 1, 'the font starts at 1 px');
  assert.equal(normalizeTaskbarLyricsSettings({ fontSizePx: 50 }).fontSizePx, 50);
  assert.equal(normalizeTaskbarLyricsSettings({}).mainOpacityPercent, 100);
  assert.equal(normalizeTaskbarLyricsSettings({}).secondaryOpacityPercent, 50);
  assert.equal(normalizeTaskbarLyricsSettings({ mainOpacityPercent: 0 }).mainOpacityPercent, 1);
  assert.equal(normalizeTaskbarLyricsSettings({ mainOpacityPercent: 1 }).mainOpacityPercent, 1);
  assert.equal(normalizeTaskbarLyricsSettings({ mainOpacityPercent: 64.6 }).mainOpacityPercent, 65);
  assert.equal(normalizeTaskbarLyricsSettings({ mainOpacityPercent: 500 }).mainOpacityPercent, 100);
  assert.equal(normalizeTaskbarLyricsSettings({ mainOpacityPercent: 'x' }).mainOpacityPercent, 100);
  assert.equal(normalizeTaskbarLyricsSettings({ secondaryOpacityPercent: 0 }).secondaryOpacityPercent, 1);
  assert.equal(normalizeTaskbarLyricsSettings({ secondaryOpacityPercent: 500 }).secondaryOpacityPercent, 100);
  assert.equal(normalizeTaskbarLyricsSettings({ secondaryOpacityPercent: 'x' }).secondaryOpacityPercent, 50);
  // The former key is ignored, not converted; the secondary may exceed the main.
  assert.equal(normalizeTaskbarLyricsSettings({ opacityPercent: 30 }).mainOpacityPercent, 100);
  const crossed = normalizeTaskbarLyricsSettings({ mainOpacityPercent: 30, secondaryOpacityPercent: 90 });
  assert.deepEqual([crossed.mainOpacityPercent, crossed.secondaryOpacityPercent], [30, 90]);
  assert.equal(normalizeTaskbarLyricsSettings({ translationScalePercent: 0 }).translationScalePercent, 1);
  assert.equal(normalizeTaskbarLyricsSettings({ translationScalePercent: 100 }).translationScalePercent, 100);
  assert.equal(normalizeTaskbarLyricsSettings({ translationScalePercent: 400 }).translationScalePercent, 100);
  assert.equal(normalizeTaskbarLyricsSettings({ translationScalePercent: 'half' }).translationScalePercent, 100);
  assert.equal(normalizeTaskbarLyricsSettings({ fontSizePx: 'big' }).fontSizePx, 20);
});

test('old settings show lyrics on every taskbar; the monitor choice keeps offline ids', () => {
  assert.equal(DEFAULT_TASKBAR_LYRICS_SETTINGS.displayMode, 'all');
  assert.equal(normalizeTaskbarLyricsSettings({ enabled: true }).displayMode, 'all');
  assert.equal(normalizeTaskbarLyricsSettings({ displayMode: 'playerMonitor' }).displayMode, 'playerMonitor');
  assert.equal(normalizeTaskbarLyricsSettings({ displayMode: 'everywhere' }).displayMode, 'all');
  assert.deepEqual(normalizeTaskbarLyricsSettings({ displayMode: 'custom', customMonitors: [' A ', 'A', '', 3, 'B'] }).customMonitors, ['A', 'B']);
  assert.deepEqual(normalizeTaskbarLyricsSettings({ customMonitors: 'A' }).customMonitors, []);
  // 'offline' is a monitor that is not connected now: toggling another monitor leaves it chosen.
  assert.deepEqual(toggleTaskbarLyricsMonitor(['offline'], 'B'), ['offline', 'B']);
  assert.deepEqual(toggleTaskbarLyricsMonitor(['offline', 'B'], 'B'), ['offline']);
  assert.deepEqual(toggleTaskbarLyricsMonitor([], 'A'), ['A']);
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

test('the text effect is one of shadow, stroke and none (default shadow), and its colour is the opposite of the text', () => {
  assert.equal(DEFAULT_TASKBAR_LYRICS_SETTINGS.textEffect, 'shadow');
  for (const effect of ['shadow', 'stroke', 'none']) {
    assert.equal(normalizeTaskbarLyricsSettings({ textEffect: effect }).textEffect, effect);
  }
  assert.equal(normalizeTaskbarLyricsSettings({ textEffect: 'glow' }).textEffect, 'shadow');
  assert.equal(normalizeTaskbarLyricsSettings({ textEffect: 3 }).textEffect, 'shadow');
  // White text on a dark taskbar gets a dark effect; black text on a light taskbar gets a light one.
  assert.equal(getTaskbarLyricsEffectRgb(DEFAULT_TASKBAR_LYRICS_SETTINGS, false), '0, 0, 0');
  assert.equal(getTaskbarLyricsEffectRgb(DEFAULT_TASKBAR_LYRICS_SETTINGS, true), '255, 255, 255');
  const custom = (customColor) => ({ ...DEFAULT_TASKBAR_LYRICS_SETTINGS, useCustomColor: true, customColor });
  assert.equal(getTaskbarLyricsEffectRgb(custom('#ffee88'), true), '0, 0, 0', 'a light custom colour gets a dark effect');
  assert.equal(getTaskbarLyricsEffectRgb(custom('#223366'), false), '255, 255, 255', 'a dark custom colour gets a light effect');
});

test('taskbar animation is none unless continuous is chosen', () => {
  assert.equal(DEFAULT_TASKBAR_LYRICS_SETTINGS.animation, 'none');
  assert.equal(normalizeTaskbarLyricsSettings({ animation: 'continuous' }).animation, 'continuous');
  assert.equal(normalizeTaskbarLyricsSettings({ animation: 'wave' }).animation, 'none');
});
