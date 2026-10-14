import test from 'node:test';
import assert from 'node:assert/strict';
import {
  buildQuickIndexGroups,
  dataIndexToVisualIndex,
  visualIndexToAnchorDataIndex,
  visualIndexToDataIndex,
  visualRangeToDataRange,
} from '../src/lib/quick-index.js';

test('paged grouping keeps contiguous groups across page boundaries and trims names', async () => {
  const items = [
    { name: ' Apple' }, { name: 'A-2' }, { name: '中文' },
    { name: ' 中文2' }, { name: 'Zed' },
  ];
  const result = await buildQuickIndexGroups({
    totalCount: items.length,
    pageSize: 2,
    fetchPage: async ({ offset, limit }) => ({ items: items.slice(offset, offset + limit) }),
    getName: (item) => item.name,
    getGroupings: async (names) => ({
      labels: ['#', 'A', 'B', 'Z', 'ㄅ'],
      groups: names.map((name) => name.startsWith('A') ? 'A' : name.startsWith('Z') ? 'Z' : 'ㄅ'),
    }),
  });
  assert.deepEqual(result.groups, [
    { label: 'A', beforeIndex: 0 },
    { label: 'ㄅ', beforeIndex: 2 },
    { label: 'Z', beforeIndex: 4 },
  ]);
});

test('visual rows map to data rows without changing queue item indexes', () => {
  const groups = [
    { label: 'A', beforeIndex: 0 },
    { label: 'B', beforeIndex: 2 },
    { label: 'C', beforeIndex: 4 },
  ];
  assert.deepEqual([0, 1, 2, 3, 4, 5, 6, 7].map((index) => visualIndexToDataIndex(index, groups)), [null, 0, 1, null, 2, 3, null, 4]);
  assert.deepEqual([0, 1, 2, 3, 4].map((index) => dataIndexToVisualIndex(index, groups)), [1, 2, 4, 5, 7]);
  assert.deepEqual(visualRangeToDataRange(2, 7, groups, 5), { start: 1, end: 4 });
  assert.deepEqual(visualRangeToDataRange(0, 1, groups, 5), { start: 0, end: 0 });
  assert.equal(visualIndexToAnchorDataIndex(3, groups, 5), 2);
  assert.equal(visualIndexToAnchorDataIndex(5, groups, 5), 3);
});

test('stale grouping scan returns without publishing partial index data', async () => {
  let current = true;
  const result = await buildQuickIndexGroups({
    totalCount: 2,
    pageSize: 1,
    fetchPage: async ({ offset }) => {
      current = false;
      return { items: [{ name: String(offset) }] };
    },
    getName: (item) => item.name,
    getGroupings: async (names) => ({ labels: ['#'], groups: names.map(() => '#') }),
    isCurrent: () => current,
  });
  assert.equal(result, null);
});

test('empty artist and album values use the Unknown group while nonempty symbolic names keep locale grouping', async () => {
  const items = [{ name: '   ' }, { name: null }, { name: '—' }, { name: 'hanser' }];
  const result = await buildQuickIndexGroups({
    totalCount: items.length,
    fetchPage: async () => ({ items }),
    getName: (item) => item.name,
    getGroupings: async (names) => ({
      labels: ['#', 'H'],
      groups: names.map((name) => name.toLowerCase() === 'h' ? 'H' : '#'),
    }),
  });
  assert.deepEqual(result.groups, [
    { label: '未知', beforeIndex: 0 },
    { label: '#', beforeIndex: 2 },
    { label: 'H', beforeIndex: 3 },
  ]);
  assert.deepEqual(result.labels, ['#', 'H', '未知']);
});

test('leading punctuation uses its own locale grouping instead of the following letter', async () => {
  const items = [
    { name: '- EARTH DAY -' },
    { name: '-#000000-' },
    { name: '-BWW SCREAM-' },
    { name: '-Ending-' },
    { name: '-FINAL SIGMA-' },
    { name: 'Hanser' },
  ];
  const requestedNames = [];
  const result = await buildQuickIndexGroups({
    totalCount: items.length,
    fetchPage: async () => ({ items }),
    getName: (item) => item.name,
    getGroupings: async (names) => {
      requestedNames.push(...names);
      return {
        labels: ['&', 'B', 'E', 'F', 'H'],
        // Model the system grouping behavior: full strings ignore a leading hyphen,
        // while asking about the actual first character yields the punctuation bucket.
        groups: names.map((name) => name === '-'
          ? '&'
          : name.match(/[A-Za-z]/)?.[0]?.toUpperCase() ?? '&'),
      };
    },
  });

  assert.deepEqual(result.groups, [
    { label: '&', beforeIndex: 0 },
    { label: 'H', beforeIndex: 5 },
  ]);
  assert.deepEqual(requestedNames, ['-', '-', '-', '-', '-', 'H']);
});
