import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DEFAULT_FLOATING_LYRICS_SETTINGS,
  buildFloatingLyricsNativeData,
  normalizeFloatingLyricsSettings,
  normalizeLyricsSettingsTab,
} from '../src/lib/floating-lyrics-settings.js';

test('floating lyrics settings repair field by field with their own defaults', () => {
  assert.deepEqual(normalizeFloatingLyricsSettings(undefined), { ...DEFAULT_FLOATING_LYRICS_SETTINGS });
  const repaired = normalizeFloatingLyricsSettings({
    enabled: true,
    fontSizePx: 'big',
    translationScalePercent: 250,
    customColor: '#ABCDEF',
    align: 'middle',
    textEffect: 'stroke',
    locked: true,
    position: { x: -1200, y: 30.5 },
  });
  assert.equal(repaired.enabled, true);
  assert.equal(repaired.fontSizePx, 28);
  assert.equal(repaired.translationScalePercent, 100);
  assert.equal(repaired.opacityPercent, 100, 'missing opacity is fully opaque');
  assert.equal(normalizeFloatingLyricsSettings({ opacityPercent: 40 }).opacityPercent, 40);
  assert.equal(normalizeFloatingLyricsSettings({ opacityPercent: 1 }).opacityPercent, 10);
  assert.equal(repaired.customColor, '#abcdef');
  assert.equal(repaired.align, 'center');
  assert.equal(repaired.textEffect, 'stroke');
  assert.equal(repaired.locked, true);
  assert.equal(repaired.position, null);
  assert.equal(repaired.showTranslation, true);
  assert.deepEqual(normalizeFloatingLyricsSettings({ position: { x: -1200, y: 300 } }).position, { x: -1200, y: 300 });
  assert.equal(normalizeFloatingLyricsSettings({ position: { x: 5_000_000, y: 0 } }).position, null);
  assert.equal(normalizeFloatingLyricsSettings({ fontSizePx: 0.2 }).fontSizePx, 1);
  assert.equal('displayMode' in normalizeFloatingLyricsSettings({}), false);
});

test('native overlay data keeps timed lines in order with one combined offset', () => {
  const data = {
    trackId: 't1',
    offsetMs: 100,
    lines: [
      { startMs: 2000, text: ' b ', translation: null },
      { startMs: null, text: 'untimed' },
      { startMs: 1000, text: 'a', translation: ' A ' },
    ],
  };
  assert.deepEqual(buildFloatingLyricsNativeData(data, 350.4, 180000), {
    trackId: 't1',
    offsetMs: 350,
    durationMs: 180000,
    lines: [
      { startMs: 1000, text: 'a', translation: 'A' },
      { startMs: 2000, text: 'b', translation: '' },
    ],
  });
  assert.deepEqual(buildFloatingLyricsNativeData(null, 0), { trackId: null, offsetMs: 0, durationMs: null, lines: [] });
});

test('the lyrics settings tab falls back to general where it does not exist', () => {
  assert.equal(normalizeLyricsSettingsTab('taskbar', true), 'taskbar');
  assert.equal(normalizeLyricsSettingsTab('taskbar', false), 'general');
  assert.equal(normalizeLyricsSettingsTab('floating', false), 'floating');
  assert.equal(normalizeLyricsSettingsTab('nope', true), 'general');
});