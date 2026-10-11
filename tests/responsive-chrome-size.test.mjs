import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';

test('chrome and playback controls keep fixed heights across width breakpoints', async () => {
  const css = await readFile(new URL('../src/app.css', import.meta.url), 'utf8');
  const chromeHeights = [...css.matchAll(/:root\s*\{[^}]*--chrome-bar-height:\s*(\d+)px/g)].map((match) => Number(match[1]));

  assert.deepEqual(chromeHeights, [74, 58], 'only the existing short-window-height rule may reduce the main header token');
  assert.match(css, /@media \(max-width: 560px\)\s*\{\s*\.now-playing-overlay-topbar\s*\{[^}]*min-height:\s*88px/s);
  assert.match(css, /@media \(max-width: 390px\)\s*\{\s*\.now-playing-overlay-topbar\s*\{\s*min-height:\s*82px/s);
  assert.doesNotMatch(css, /\.control-button\s*\{\s*width:\s*32px\s*;\s*height:\s*32px/s);
  assert.doesNotMatch(css, /\.play-button\s*\{\s*width:\s*36px\s*;\s*height:\s*36px/s);
  assert.doesNotMatch(css, /\.volume-toggle\s*\{\s*width:\s*32px\s*;\s*height:\s*32px/s);
  assert.doesNotMatch(css, /\.dock-controls\s*\{\s*height:\s*36px/s);
});
