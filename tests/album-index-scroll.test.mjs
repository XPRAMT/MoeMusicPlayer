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

test('album index restores after active data and viewport are ready and saves while scrolling', () => {
  assert.match(indexSource, /if \(!active \|\| loading \|\| !element \|\| viewportWidth <= 0 \|\| scrollPositionRestored\) return/);
  assert.match(indexSource, /requestAnimationFrame\(\(\) =>/);
  assert.match(indexSource, /Math\.min\(getAlbumIndexScrollTop\(\), maxScrollTop\)/);
  assert.match(indexSource, /function handleScroll\(\): void \{\s*measure\(\);\s*if \(scrollPositionRestored && viewport\) setAlbumIndexScrollTop\(viewport\.scrollTop\);\s*\}/);
  assert.doesNotMatch(indexSource, /onDestroy\(\(\) => \{[\s\S]*?setAlbumIndexScrollTop\(viewport\.scrollTop\)/,
    'teardown can clamp scrollTop after removing grid content, so it must not overwrite the last scroll-event snapshot');
  assert.match(indexSource, /onscroll=\{handleScroll\}/);
});

test('album card index loads sorted SQLite snapshot pages instead of merging and sorting all facets in the renderer', () => {
  assert.match(indexSource, /library_create_album_index_snapshot/);
  assert.match(indexSource, /library_get_album_index_snapshot_page/);
  assert.match(indexSource, /new PagedListController<AlbumIndexEntry, string>/);
  assert.match(indexSource, /pageSnapshot\.revision/);
  assert.doesNotMatch(indexSource, /buildAlbumEntries|sortAlbumEntries|library_sort_text_indices/);
});
