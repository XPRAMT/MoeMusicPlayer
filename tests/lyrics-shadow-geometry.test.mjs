import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { LYRICS_ROW_VERTICAL_PADDING_PX, buildLyricsLayout, computeLyricsInkLineHeight } from '../src/lib/lyrics-window.js';
import { DEFAULT_LYRICS_PREFERENCES } from '../src/lib/lyrics-preferences.js';

const view = readFileSync(new URL('../src/lib/LyricsView.svelte', import.meta.url), 'utf8');

test('row vertical padding equals the first shadow blur radius (tight shadows touch at spacing 0)', () => {
  const shadow = Number(view.match(/--lyric-shadow-blur: (\d+)px;/)?.[1]);
  assert.equal(LYRICS_ROW_VERTICAL_PADDING_PX, shadow * 2);
  assert.match(view, /\.lyric-line \{[^}]*overflow: visible;[^}]*padding: var\(--lyric-shadow-blur\) 9px;/);
  assert.match(view, /0 0 var\(--lyric-glow-blur\) rgba\(var\(--text-inverse-rgb\), 0\.38\)/);
  assert.match(view, /0 0 var\(--lyric-shadow-blur\) rgba\(var\(--text-inverse-rgb\), 0\.6\)/);
  assert.doesNotMatch(view, /0 1px 3px|0\.82\)/);
});

test('clamped primary line draws its shadow as a filter so the clip does not cut the glow', () => {
  assert.match(view, /data-text-effect='shadow'\] \.lyric-primary \{\s*text-shadow: none;\s*filter:\s*drop-shadow/);
  assert.match(view, /line-height: var\(--lyric-primary-line-height, 1\.35\);/);
  assert.match(view, /max-height: calc\(var\(--lyric-primary-font-size, 14px\) \* var\(--lyric-primary-line-height, 1\.35\) \* 2\);/);
});

test('ink line-height fits ascenders and descenders around the centred content area', () => {
  // Android (Noto CJK): content 1.16/0.29, ink 0.85/0.25 -> descent side limits: 1.45 - 0.08.
  assert.equal(computeLyricsInkLineHeight({ fontAscent: 1.16, fontDescent: 0.29, inkAscent: 0.85, inkDescent: 0.25 }), 1.37);
  // Ink taller than the content area grows the line instead of clipping.
  assert.equal(computeLyricsInkLineHeight({ fontAscent: 0.9, fontDescent: 0.2, inkAscent: 1.0, inkDescent: 0.2 }), 1.3);
  assert.equal(computeLyricsInkLineHeight({ fontAscent: Number.NaN, fontDescent: 0, inkAscent: 0, inkDescent: 0 }, 1.35), 1.35);
});

test('layout estimate uses the measured ink line-height when provided', () => {
  const rows = [{ text: 'Querying destiny', translation: null, romanization: null }, { text: 'x', translation: null, romanization: null }];
  const layout = buildLyricsLayout(rows, { ...DEFAULT_LYRICS_PREFERENCES, primaryLineHeight: 1.37 }, 0);
  assert.ok(Math.abs(layout.heights[0] - (DEFAULT_LYRICS_PREFERENCES.primaryFontSizePx * 1.37 + LYRICS_ROW_VERTICAL_PADDING_PX)) < 1e-9);
});
