import './helpers/untr-fs.mjs';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const css = read('src/app.css');
const cells = read('src/lib/TrackColumnCells.svelte');
const artistIndex = read('src/lib/ArtistIndex.svelte');


test('list header rows and group rows share one user-select none rule', () => {
  const found = [...css.matchAll(/((?:[^{}\n]+,\n)*\.list-group-row-surface)\s*\{([^}]*)\}/g)]
    .find((m) => /user-select:\s*none/.test(m[2]));
  assert.ok(found, 'shared rule exists');
  const block = found[1];
  for (const selector of ['.track-table-head', '.track-list-header', '.paged-virtual-header', '.list-group-row-surface']) {
    assert.ok(block.includes(selector), selector);
  }
  assert.match(found[2], /-webkit-user-select:\s*none/);
});

test('track data rows are not made unselectable by the header rule', () => {
  const found = [...css.matchAll(/((?:[^{}\n]+,\n)*\.list-group-row-surface)\s*\{([^}]*)\}/g)]
    .find((m) => /user-select:\s*none/.test(m[2]));
  assert.doesNotMatch(found[1], /track-row|paged-virtual-row|gridcell/);
});

test('column resize handle is 20px wide centred on the border, both mouse and touch', () => {
  const handle = cells.match(/\.column-resize-handle\s*\{([^}]*)\}/)[1];
  assert.match(handle, /right:\s*-10px/);
  assert.match(handle, /width:\s*20px/);
  assert.match(handle, /z-index:\s*2/);
  const line = cells.match(/\.column-resize-handle::after\s*\{([^}]*)\}/)[1];
  assert.match(line, /left:\s*9px/);
  assert.match(line, /width:\s*1px/);
  assert.doesNotMatch(cells, /pointer:\s*coarse/);
});

test('ArtistIndex reuses the shared handle without its own copy', () => {
  assert.doesNotMatch(artistIndex, /column-resize-handle\s*\{/);
  assert.match(artistIndex, /TrackColumnCells/);
});
