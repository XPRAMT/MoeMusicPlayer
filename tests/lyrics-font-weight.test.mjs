import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  DEFAULT_LYRICS_PREFERENCES,
  LYRICS_FONT_WEIGHT,
  lyricsAuxiliaryWeight,
  normalizeLyricsFontWeight,
  normalizeLyricsPreferences,
} from '../src/lib/lyrics-preferences.js';
import { DEFAULT_TASKBAR_LYRICS_SETTINGS, normalizeTaskbarLyricsSettings } from '../src/lib/taskbar-lyrics-settings.js';
import { DEFAULT_FLOATING_LYRICS_SETTINGS, normalizeFloatingLyricsSettings } from '../src/lib/floating-lyrics-settings.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const read = (file) => readFileSync(path.join(root, file), 'utf8');

test('the font weight defaults to 500 on every tab and is clamped to 100-1000 on load', () => {
  assert.deepEqual(LYRICS_FONT_WEIGHT, { min: 100, max: 1000, step: 100, default: 500 });
  assert.equal(DEFAULT_LYRICS_PREFERENCES.fontWeight, 500);
  assert.equal(DEFAULT_TASKBAR_LYRICS_SETTINGS.fontWeight, 500);
  assert.equal(DEFAULT_FLOATING_LYRICS_SETTINGS.fontWeight, 500);
  const tabs = [normalizeLyricsPreferences, normalizeTaskbarLyricsSettings, normalizeFloatingLyricsSettings];
  for (const normalize of tabs) {
    assert.equal(normalize({}).fontWeight, 500, 'an old file without the field');
    assert.equal(normalize({ fontWeight: 0 }).fontWeight, 100);
    assert.equal(normalize({ fontWeight: 99999 }).fontWeight, 1000);
    assert.equal(normalize({ fontWeight: 349.6 }).fontWeight, 350);
    assert.equal(normalize({ fontWeight: 'bold' }).fontWeight, 500);
    assert.equal(normalize({ fontWeight: 1000 }).fontWeight, 1000);
  }
  assert.equal(normalizeLyricsFontWeight(Number.NaN), 500);
});

test('each tab keeps its own weight', () => {
  assert.equal(normalizeLyricsPreferences({ fontWeight: 300 }).fontWeight, 300);
  assert.equal(normalizeTaskbarLyricsSettings({ fontWeight: 700 }).fontWeight, 700);
  assert.equal(normalizeFloatingLyricsSettings({ fontWeight: 900 }).fontWeight, 900);
});

test('translation and romanization rows are 50 lighter than the original, never below 1', () => {
  assert.equal(lyricsAuxiliaryWeight(550), 500);
  assert.equal(lyricsAuxiliaryWeight(100), 50);
  assert.equal(lyricsAuxiliaryWeight(1000), 950);
  assert.equal(lyricsAuxiliaryWeight(40), 1);
});

test('the Rust and Kotlin defaults and ranges match the JS ones', () => {
  const rust = read('src-tauri/src/settings.rs');
  assert.match(rust, /pub const LYRICS_FONT_WEIGHT_RANGE: \(u16, u16\) = \(100, 1000\);/);
  assert.match(rust, /pub fn default_lyrics_font_weight\(\) -> u16 \{\s*500\s*\}/);
  const kotlin = read('plugins/tauri-plugin-media-index/android/src/main/java/com/moemusicplayer/mediaindex/FloatingLyricsRules.kt');
  assert.match(kotlin, /DEFAULT_FONT_WEIGHT\s*=\s*500/);
  assert.match(kotlin, /MIN_FONT_WEIGHT\s*=\s*100/);
  assert.match(kotlin, /MAX_FONT_WEIGHT\s*=\s*1000/);
});

test('the 字體粗細 slider sits right after the translation size on every lyrics tab', () => {
  const source = read('src/lib/LyricsLookSettings.svelte');
  const slider = source.match(/<input[^>]*value=\{settings\.fontWeight\}[\s\S]*?\/>/)?.[0] ?? '';
  assert.match(slider, /type="range"/);
  assert.match(slider, /min=\{LYRICS_FONT_WEIGHT\.min\}/);
  assert.match(slider, /max=\{LYRICS_FONT_WEIGHT\.max\}/);
  assert.match(slider, /step=\{LYRICS_FONT_WEIGHT\.step\}/);
  assert.doesNotMatch(slider, /disabled|inert/);
  const at = (needle) => source.indexOf(needle);
  assert.ok(at('translationScalePercent: Number') < at('onChange({ fontWeight') && at('onChange({ fontWeight') < at('sizeExtra?.()'));
  // The playing-page drawer feeds and reads its own value.
  const quick = read('src/lib/NowPlayingQuickSettingsControls.svelte');
  assert.ok(quick.includes('fontWeight: lyrics.fontWeight'));
  assert.ok(quick.includes('patch.fontWeight'));
});

test('the playing page draws the original at the weight and the aux rows 50 lighter', () => {
  const view = read('src/lib/LyricsView.svelte');
  assert.match(view, /--lyric-primary-weight:\$\{preferences\.fontWeight\}/);
  assert.match(view, /--lyric-auxiliary-weight:\$\{lyricsAuxiliaryWeight\(preferences\.fontWeight\)\}/);
  assert.match(view, /\.lyric-primary \{[^}]*font-weight: var\(--lyric-primary-weight, 500\);/s);
  assert.match(view, /\.lyric-translation,\s*\.lyric-romanization \{[^}]*font-weight: var\(--lyric-auxiliary-weight, 500\);/s);
  assert.doesNotMatch(view, /`500 100px|`400 100px/, 'the ink-height measure uses the setting');
});

test('the label is translated', () => {
  for (const language of ['ja', 'en', 'zh-CN']) {
    const catalog = JSON.parse(read(`src/lib/i18n/${language}.json`));
    assert.ok(catalog['字體粗細'], language);
  }
});
