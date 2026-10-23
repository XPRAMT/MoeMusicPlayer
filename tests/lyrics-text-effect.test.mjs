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

test('every lyrics settings page offers the same three text-effect choices', () => {
  const quick = readFileSync(path.join(root, 'src/lib/NowPlayingQuickSettingsControls.svelte'), 'utf8');
  const source = readFileSync(path.join(root, 'src/lib/LyricsLookSettings.svelte'), 'utf8');
  const css = readFileSync(path.join(root, 'src/app.css'), 'utf8');
  assert.match(quick, /<LyricsLookSettings/, 'the general page uses the shared lyrics settings');
  for (const value of ['shadow', 'stroke', 'none']) assert.ok(source.includes(`['${value}', tr(`), value);
  assert.match(source, /文字效果/);
  assert.match(source, /lyrics-text-effect-row/);
  assert.match(css, /\.layout-choice-row\.lyrics-text-effect-row\s*\{[^}]*grid-template-columns:\s*repeat\(3,/s);
  assert.doesNotMatch(source, /文字加上柔和陰影/);
  assert.doesNotMatch(source, /文字加上外框描邊/);
  assert.doesNotMatch(source, /不描邊也不加陰影/);
});

test('the three lyrics settings pages share one group order', () => {
  const source = readFileSync(path.join(root, 'src/lib/LyricsLookSettings.svelte'), 'utf8');
  const order = ['tr("顯示")', 'enable?.()', 'tr("顯示譯文")', 'tr("動畫")', 'tr("字體與大小")', 'tr("字體大小")', 'tr("譯文大小（相對原文）")', 'tr("外觀")', 'tr("文字效果")', 'tr("自訂顏色")', 'tr("透明度")', 'tr("對齊")', 'tr("位置與面板")'];
  const positions = order.map((needle) => source.indexOf(needle));
  for (const [i, at] of positions.entries()) assert.ok(at >= 0, order[i]);
  assert.deepEqual([...positions].sort((a, b) => a - b), positions);
});
