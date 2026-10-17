import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { LYRICS_ROW_VERTICAL_PADDING_PX, LYRICS_PRIMARY_LINE_HEIGHT } from '../src/lib/lyrics-window.js';

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
  assert.match(view, new RegExp('line-height: ' + LYRICS_PRIMARY_LINE_HEIGHT + ';'));
  assert.match(view, new RegExp('\\* ' + LYRICS_PRIMARY_LINE_HEIGHT + ' \\* 2\\)'));
});
