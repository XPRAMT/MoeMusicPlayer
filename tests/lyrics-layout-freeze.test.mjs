import assert from 'node:assert/strict';
import test from 'node:test';
import { frozenRowWidth, lyricsLayoutSize } from '../src/lib/lyrics-layout-freeze.js';

test('the lyrics keep the settled layout size while frozen and follow the measured size otherwise', () => {
  const settled = { width: 400, height: 500 };
  const measured = { width: 250, height: 480 };
  assert.equal(lyricsLayoutSize(true, settled, measured), settled);
  assert.equal(lyricsLayoutSize(false, settled, measured), measured);
});

test('rows get a fixed width only while frozen and measured', () => {
  assert.equal(frozenRowWidth(true, 400), '400px');
  assert.equal(frozenRowWidth(false, 400), undefined);
  assert.equal(frozenRowWidth(true, 0), undefined);
});

test('the zoom wiring: gesture-only will-change, frozen lyrics rows, App passes the zooming flag', async () => {
  await import('./helpers/untr-fs.mjs');
  const { readFileSync } = await import('node:fs');
  const css = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.match(css, /\.cover-stage\[data-cover-gesture\]\s*\{\s*will-change: transform;/);
  assert.doesNotMatch(css, /\.cover-stage\[data-cover-zoomed\]\s*\{[^}]*will-change/);
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  assert.match(app, /layoutFrozen=\{coverZooming\}/);
  assert.match(app, /onZoomingChange: \(zooming\) => \{ coverZooming = zooming; \}/);
  assert.doesNotMatch(app, /--np-cover-zoom/);
  const lyrics = readFileSync(new URL('../src/lib/LyricsView.svelte', import.meta.url), 'utf8');
  assert.match(lyrics, /--lyrics-frozen-width/);
  assert.match(lyrics, /if \(layoutFrozen\) return;/);
  assert.match(lyrics, /contain: layout paint;/);
});
