import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { LYRIC_LINE_ANIMATION } from '../src/lib/lyrics-word-highlight.js';

const read = (p) => readFileSync(new URL(p, import.meta.url), 'utf8');

test('animation names written inline are declared as global keyframes', () => {
  const view = read('../src/lib/LyricsView.svelte');
  const appCss = read('../src/app.css');
  for (const name of [LYRIC_LINE_ANIMATION]) {
    const global = view.includes(`@keyframes -global-${name} `) || view.includes(`@keyframes -global-${name}{`)
      || new RegExp(`@keyframes\\s+${name}\\s*\\{`).test(appCss);
    assert.ok(global, `@keyframes ${name} must be global (inline animation-name)`);
    assert.doesNotMatch(view, new RegExp(`@keyframes\\s+${name}\\s*\\{`), `scoped @keyframes ${name} in LyricsView.svelte`);
  }
});
