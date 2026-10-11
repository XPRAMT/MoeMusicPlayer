import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';

test('main dock keeps its cover full size and hides controls in two ordered stages', async () => {
  const css = await readFile(new URL('../src/app.css', import.meta.url), 'utf8');
  const baseArt = css.match(/\.dock-art\s*\{([^}]*)\}/)?.[1] ?? '';

  assert.match(baseArt, /width:\s*64px/);
  assert.match(baseArt, /height:\s*64px/);
  assert.match(baseArt, /min-width:\s*64px/);
  assert.match(baseArt, /min-height:\s*64px/);
  assert.match(baseArt, /max-width:\s*64px/);
  assert.match(baseArt, /max-height:\s*64px/);
  assert.match(baseArt, /flex:\s*0 0 64px/);

  const stageOne = css.match(/\/\* Main dock stage 1:[\s\S]*?@media \(max-width: 720px\)\s*\{([^]*?)\n\}/)?.[1] ?? '';
  const stageTwo = css.match(/\/\* Main dock stage 2:[\s\S]*?@media \(max-width: 520px\)\s*\{([^]*?)\n\}/)?.[1] ?? '';

  assert.match(stageOne, /\.dock-favorite/);
  assert.match(stageOne, /\.dock-volume/);
  assert.doesNotMatch(stageOne, /\.secondary-control/);
  assert.match(stageTwo, /\.dock-controls \.secondary-control/);
  assert.match(stageTwo, /grid-template-columns:\s*minmax\(0, 1fr\) auto/);
  assert.match(stageTwo, /justify-self:\s*end/);
  assert.match(stageTwo, /width:\s*var\(--dock-hide-height\)/);
  assert.match(stageTwo, /min-width:\s*var\(--dock-hide-height\)/);
  assert.match(stageTwo, /flex:\s*0 0 var\(--dock-hide-height\)/);

  const compactViewportRules = css.match(/@media \(max-width: 820px\)\s*\{([^]*?)\n\}/)?.[1] ?? '';
  assert.doesNotMatch(compactViewportRules, /\.dock-art\s*\{[^}]*?(?:width|height):\s*(?:5\d|[1-6]\d)px/s);
});
