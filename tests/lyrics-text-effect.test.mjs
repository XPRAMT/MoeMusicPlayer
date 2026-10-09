import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  DEFAULT_LYRICS_PREFERENCES,
  normalizeLyricsPreferences,
} from '../src/lib/lyrics-preferences.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

test('lyrics textEffect defaults to shadow and accepts stroke/none', () => {
  assert.equal(DEFAULT_LYRICS_PREFERENCES.textEffect, 'shadow');
  assert.equal(normalizeLyricsPreferences({}).textEffect, 'shadow');
  assert.equal(normalizeLyricsPreferences({ textEffect: 'stroke' }).textEffect, 'stroke');
  assert.equal(normalizeLyricsPreferences({ textEffect: 'none' }).textEffect, 'none');
  assert.equal(normalizeLyricsPreferences({ textEffect: 'glow' }).textEffect, 'shadow');
});

test('lyrics view CSS exposes shadow, stroke, and none text effects', () => {
  const css = readFileSync(path.join(root, 'src/lib/LyricsView.svelte'), 'utf8');
  assert.match(css, /data-text-effect=\{preferences\.textEffect\}/);
  assert.match(css, /data-text-effect='shadow'/);
  assert.match(css, /data-text-effect='stroke'/);
  assert.match(css, /data-text-effect='none'/);
  assert.match(css, /-webkit-text-stroke/);
  assert.match(css, /--text-inverse-rgb/);
  assert.match(css, /移除歌詞/);
  assert.match(css, /data-testid="lyrics-remove"/);
  assert.match(css, /data-testid="lyrics-dismiss"/);
});

test('quick settings offers three text-effect choices', () => {
  const source = readFileSync(path.join(root, 'src/lib/NowPlayingQuickSettingsControls.svelte'), 'utf8');
  assert.match(source, /textEffect: 'shadow'/);
  assert.match(source, /textEffect: 'stroke'/);
  assert.match(source, /textEffect: 'none'/);
  assert.match(source, /歌詞文字效果/);
});
