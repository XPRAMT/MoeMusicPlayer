import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';

test('chrome and playback controls keep fixed heights across width breakpoints', async () => {
  const css = await readFile(new URL('../src/app.css', import.meta.url), 'utf8');
  const chromeHeights = [...css.matchAll(/:root\s*\{[^}]*--chrome-bar-height:\s*(\d+)px/g)].map((match) => Number(match[1]));

  assert.deepEqual(chromeHeights, [74, 58], 'only the existing short-window-height rule may reduce the main header token');
  assert.match(css, /\.now-playing-overlay-topbar\s*\{[^}]*height:\s*var\(--chrome-bar-height\)[^}]*max-height:\s*var\(--chrome-bar-height\)/s);
  assert.doesNotMatch(css, /\.control-button\s*\{\s*width:\s*32px\s*;\s*height:\s*32px/s);
  assert.doesNotMatch(css, /\.play-button\s*\{\s*width:\s*36px\s*;\s*height:\s*36px/s);
  assert.doesNotMatch(css, /\.volume-toggle\s*\{\s*width:\s*32px\s*;\s*height:\s*32px/s);
  assert.doesNotMatch(css, /\.dock-controls\s*\{\s*height:\s*36px/s);
});
