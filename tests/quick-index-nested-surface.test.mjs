import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';

const read = (path) => readFile(new URL(path, import.meta.url), 'utf8');

test('quick index popover inside another floating surface has no nested backdrop-filter', async () => {
  const list = await read('../src/lib/PagedVirtualList.svelte');
  const nested = list.match(/:global\(\.floating-surface\) \.quick-index-popover\.floating-surface \{([^}]*)\}/);
  assert.ok(nested, 'nested popover rule missing');
  assert.match(nested[1], /(^|[^-])backdrop-filter: none;/);
  assert.match(nested[1], /-webkit-backdrop-filter: none;/);
  assert.match(nested[1], /background: color-mix\(in srgb, var\(--text-inverse\) \d+%, transparent\);/);
});

test('quick index backdrop is an invisible click-catcher', async () => {
  const list = await read('../src/lib/PagedVirtualList.svelte');
  const rule = list.match(/\n  \.quick-index-backdrop \{([^}]*)\}/)?.[1] ?? '';
  assert.match(rule, /background: transparent;/);
  assert.match(rule, /(^|[^-])backdrop-filter: none;/);
  assert.match(rule, /box-shadow: none;/);
  // The inline style only positions it.
  const assign = list.match(/quickIndexBackdropStyle = `([^`]*)`/)?.[1] ?? '';
  assert.doesNotMatch(assign, /background|filter|shadow|opacity/);
});

test('opening the quick index focuses its first label without scrolling ancestors', async () => {
  const list = await read('../src/lib/PagedVirtualList.svelte');
  assert.match(list, /\.quick-index-label:not\(:disabled\)'\)\?\.focus\(\{ preventScroll: true \}\)/);
  assert.doesNotMatch(list, /\.quick-index-label:not\(:disabled\)'\)\?\.focus\(\)/);
});
