import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';
import {
  getAlbumIndexScrollTop,
  setAlbumIndexScrollTop,
} from '../src/lib/album-index-scroll.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const indexSource = readFileSync(path.join(root, 'src/lib/AlbumIndex.svelte'), 'utf8').replace(/\r\n/g, '\n');

test('album index scroll position is retained as a bounded in-memory value', () => {
  setAlbumIndexScrollTop(1280.5);
  assert.equal(getAlbumIndexScrollTop(), 1280.5);
  setAlbumIndexScrollTop(-20);
  assert.equal(getAlbumIndexScrollTop(), 0);
  setAlbumIndexScrollTop(Number.NaN);
  assert.equal(getAlbumIndexScrollTop(), 0);
});

test('album index restores only after active data and viewport are ready, then saves scroll changes', () => {
  assert.match(indexSource, /if \(!active \|\| loading \|\| !element \|\| viewportWidth <= 0 \|\| scrollPositionRestored\) return/);
  assert.match(indexSource, /requestAnimationFrame\(\(\) =>/);
  assert.match(indexSource, /Math\.min\(getAlbumIndexScrollTop\(\), maxScrollTop\)/);
  assert.match(indexSource, /if \(scrollPositionRestored && viewport\) setAlbumIndexScrollTop\(viewport\.scrollTop\)/);
  assert.match(indexSource, /onscroll=\{handleScroll\}/);
});
