import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { DEFAULT_TASKBAR_LYRICS_SETTINGS, normalizeTaskbarLyricsSettings } from '../src/lib/taskbar-lyrics-settings.js';
import { DEFAULT_FLOATING_LYRICS_SETTINGS, normalizeFloatingLyricsSettings } from '../src/lib/floating-lyrics-settings.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const read = (file) => readFileSync(path.join(root, file), 'utf8');

test('暫停時隱藏 is on by default for the taskbar and the floating lyrics, each saved apart', () => {
  assert.equal(DEFAULT_TASKBAR_LYRICS_SETTINGS.hideWhenPaused, true);
  assert.equal(DEFAULT_FLOATING_LYRICS_SETTINGS.hideWhenPaused, true);
  for (const normalize of [normalizeTaskbarLyricsSettings, normalizeFloatingLyricsSettings]) {
    assert.equal(normalize({}).hideWhenPaused, true, 'an old file without the field');
    assert.equal(normalize({ hideWhenPaused: false }).hideWhenPaused, false);
    assert.equal(normalize({ hideWhenPaused: 'no' }).hideWhenPaused, true, 'a non-boolean takes the default');
    assert.equal(normalize(null).hideWhenPaused, true);
  }
  // Separate fields: one tab's choice is not the other's.
  assert.equal(normalizeTaskbarLyricsSettings({ hideWhenPaused: false }).hideWhenPaused, false);
  assert.equal(normalizeFloatingLyricsSettings({}).hideWhenPaused, true);
});

test('the Rust and Kotlin defaults are on too', () => {
  const rust = read('src-tauri/src/settings.rs');
  assert.match(rust, /fn default_hide_when_paused\(\) -> bool \{\s*true\s*\}/);
  assert.equal((rust.match(/#\[serde\(default = "default_hide_when_paused"\)\]\s*pub hide_when_paused: bool,/g) ?? []).length, 2,
    'both settings structs read old files without the field as on');
  const kotlin = read('plugins/tauri-plugin-media-index/android/src/main/java/com/moemusicplayer/mediaindex/FloatingLyricsOverlay.kt');
  assert.match(kotlin, /optBoolean\("hideWhenPaused", true\)/);
});

test('the toggle sits in 顯示 after the tab\'s own toggles and before 動畫, on the taskbar and floating tabs only', () => {
  const look = read('src/lib/LyricsLookSettings.svelte');
  const extra = look.indexOf('{@render displayExtra?.()}');
  const toggle = look.indexOf('tr("暫停時隱藏")');
  const animation = look.indexOf('{#if withAnimation}');
  assert.ok(extra > 0 && toggle > extra && animation > toggle, 'displayExtra, then 暫停時隱藏, then 動畫');
  assert.match(look, /\{#if withHideWhenPaused\}/);
  assert.match(look, /onChange\(\{ hideWhenPaused: event\.currentTarget\.checked \}\)/);
  const app = read('src/App.svelte');
  const taskbar = app.slice(app.indexOf('idPrefix="{scope}taskbar-lyrics"'), app.indexOf('{#snippet enable()}', app.indexOf('idPrefix="{scope}taskbar-lyrics"')));
  const floating = app.slice(app.indexOf('idPrefix="{scope}floating-lyrics"'), app.indexOf('{#snippet enable()}', app.indexOf('idPrefix="{scope}floating-lyrics"')));
  assert.match(taskbar, /withHideWhenPaused/);
  assert.match(floating, /withHideWhenPaused/);
  assert.doesNotMatch(read('src/lib/NowPlayingQuickSettingsControls.svelte'), /withHideWhenPaused/, 'not on the Now Playing tab');
});
