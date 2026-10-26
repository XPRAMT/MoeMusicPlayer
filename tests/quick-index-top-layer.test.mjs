import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { topLayerPopover } from '../src/lib/top-layer-popover.js';

const read = (path) => readFile(new URL(path, import.meta.url), 'utf8');

const popovers = [
  { file: '../src/lib/PagedVirtualList.svelte', layer: 'quick-index-layer', popover: 'quick-index-popover', backdrop: 'quick-index-backdrop', label: 'quick-index-label' },
  { file: '../src/lib/AlbumIndex.svelte', layer: 'album-quick-index-layer', popover: 'album-quick-index-popover', backdrop: 'album-quick-index-backdrop', label: 'album-quick-index-label' },
];

for (const { file, layer, popover, backdrop, label } of popovers) {
  test(`${file}: the quick index is a manual popover in the top layer around the backdrop and the panel`, async () => {
    const source = await read(file);
    assert.match(source, /import \{ topLayerPopover \} from '\.\/top-layer-popover\.js';/);
    const open = source.indexOf(`<div class="${layer}" popover="manual" use:topLayerPopover={{ onShown: focusFirstQuickIndexLabel }}>`);
    assert.ok(open >= 0, 'top-layer wrapper missing');
    assert.ok(source.indexOf(`class="${backdrop}"`) > open, 'backdrop inside the layer');
    assert.ok(source.indexOf(`class="${popover} floating-surface"`) > open, 'panel inside the layer');
    // The first label is focused after showPopover (a hidden [popover] cannot take focus), without scrolling ancestors.
    assert.ok(source.includes(`querySelector<HTMLButtonElement>('.${label}:not(:disabled)')?.focus({ preventScroll: true })`));
  });

  test(`${file}: the layer resets the UA [popover] box and the panel keeps the plain floating-surface look`, async () => {
    const source = await read(file);
    const rule = source.match(new RegExp(`\\.${layer} \\{([^}]*)\\}`))?.[1] ?? '';
    for (const declaration of [/position: ?fixed/, /inset: ?0/, /margin: ?0/, /padding: ?0/, /border: ?0/, /overflow: ?visible/, /background: ?transparent/, /max-width: ?none/, /max-height: ?none/]) {
      assert.match(rule, declaration);
    }
    assert.doesNotMatch(source, /:global\(\.floating-surface\) \.[\w-]*quick-index-popover/, 'no nested-surface override');
    const panel = source.match(new RegExp(`\\.${popover} \\{([^}]*)\\}`))?.[1] ?? '';
    assert.doesNotMatch(panel, /background|backdrop-filter|opacity/, 'look comes only from .floating-surface');
  });
}

test('the quick index uses viewport coordinates: no containing-block conversion and no backdrop repositioning', async () => {
  const list = await read('../src/lib/PagedVirtualList.svelte');
  assert.doesNotMatch(list, /fixed-containing-block|fixedPlacement|quickIndexBackdropStyle/);
  const rule = list.match(/\n  \.quick-index-backdrop \{([^}]*)\}/)?.[1] ?? '';
  assert.match(rule, /background: transparent;/);
  assert.match(rule, /(^|[^-])backdrop-filter: none;/);
  assert.match(rule, /box-shadow: none;/);
});

test('topLayerPopover shows the node, then calls onShown, and hides it when destroyed', () => {
  const calls = [];
  let open = false;
  const node = {
    matches: (selector) => selector === ':popover-open' && open,
    showPopover: () => { calls.push('show'); open = true; },
    hidePopover: () => { calls.push('hide'); open = false; },
  };
  const action = topLayerPopover(node, { onShown: (shown) => calls.push(shown === node ? 'shown' : 'wrong') });
  assert.deepEqual(calls, ['show', 'shown']);
  action.destroy();
  assert.deepEqual(calls, ['show', 'shown', 'hide']);
  action.destroy();
  assert.deepEqual(calls, ['show', 'shown', 'hide'], 'hiding twice is a no-op');
});

test('topLayerPopover leaves the element in place without Popover API support', () => {
  let shown = 0;
  const action = topLayerPopover({ matches: () => false }, { onShown: () => { shown += 1; } });
  assert.equal(shown, 1);
  assert.doesNotThrow(() => action.destroy());
});

test('the font picker rows are the library track rows', async () => {
  const picker = await read('../src/lib/FontPicker.svelte');
  assert.match(picker, /rowClassName="track-row virtual-track-row font-picker-row"/);
  assert.doesNotMatch(picker, /:global\(\.paged-virtual-row\.font-picker-row/, 'no own row, hover or selected styles');
  const name = picker.match(/\.font-picker-row-name \{([^}]*)\}/)?.[1] ?? '';
  assert.doesNotMatch(name, /font-size|padding|font-weight/, 'size and padding come from .track-row');
  // Group rows are PagedVirtualList's own (no picker override).
  assert.doesNotMatch(picker, /paged-virtual-group/);
});
