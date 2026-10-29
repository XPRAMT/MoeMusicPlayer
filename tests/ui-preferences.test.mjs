import test from 'node:test';
import assert from 'node:assert/strict';
import {
  legacyPatchToCarry, normalizeArtistTitleWidth, readLegacyUiPreferences, removeLegacyUiPreferences,
} from '../src/lib/ui-preferences.js';

function memoryStorage(entries) {
  const map = new Map(Object.entries(entries));
  return {
    getItem: (key) => (map.has(key) ? map.get(key) : null),
    removeItem: (key) => map.delete(key),
    keys: () => [...map.keys()],
  };
}

test('legacy localStorage values become a uiPreferences patch', () => {
  const storage = memoryStorage({
    'moe.spectrogramSettings.v2': JSON.stringify({ fftSize: 8192, colormap: 'magma' }),
    'moe.spectrogramCompare': JSON.stringify({ mode: 'overlay', opacityB: 0.3 }),
    'moe.coverEditOptions.v2': JSON.stringify({ jpegQuality: 90 }),
    'moe.artistIndex.titleWidthPx': '5000',
    'moe.settings.lyricsTab': 'floating',
  });
  const patch = readLegacyUiPreferences(storage);
  assert.equal(patch.spectrogram.fftSize, 8192);
  assert.equal(patch.spectrogramCompare.mode, 'overlay');
  assert.equal(patch.coverEdit.jpegQuality, 90);
  assert.equal(patch.artistIndexTitleWidthPx, 1200);
  assert.equal(patch.lyricsSettingsTab, 'floating');
});

test('v1 spectrogram key keeps its look but not FFT size; bad values are skipped', () => {
  const patch = readLegacyUiPreferences(memoryStorage({
    'moe.spectrogramSettings': JSON.stringify({ fftSize: 1024, colormap: 'magma' }),
    'moe.settings.lyricsTab': 'nope',
    'moe.artistIndex.titleWidthPx': 'x',
  }));
  assert.equal(patch.spectrogram.fftSize, 4096);
  assert.equal(patch.spectrogram.colormap, 'magma');
  assert.equal(patch.lyricsSettingsTab, undefined);
  assert.equal(patch.artistIndexTitleWidthPx, undefined);
  assert.deepEqual(readLegacyUiPreferences(memoryStorage({})), {});
});

test('only values settings.json does not have are carried, and old keys are removed', () => {
  const carry = legacyPatchToCarry({ lyricsSettingsTab: 'general', artistIndexTitleWidthPx: null }, { lyricsSettingsTab: 'floating', artistIndexTitleWidthPx: 300 });
  assert.deepEqual(carry, { artistIndexTitleWidthPx: 300 });
  const storage = memoryStorage({ 'moe.coverEditOptions': '{}', 'moe.settings.lyricsTab': 'general', other: '1' });
  removeLegacyUiPreferences(storage);
  assert.deepEqual(storage.keys(), ['other']);
  assert.equal(normalizeArtistTitleWidth(null), null);
  assert.equal(normalizeArtistTitleWidth(50), 120);
});