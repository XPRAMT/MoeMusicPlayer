import test from 'node:test';
import assert from 'node:assert/strict';
import {
  buildQuickIndexGroups,
  createQuickIndexRowsScheduler,
  dataIndexToVisualIndex,
  getQuickIndexColumnCount,
  getQuickIndexDisplayLabel,
  visualIndexToAnchorDataIndex,
  visualIndexToDataIndex,
  visualRangeToDataRange,
} from '../src/lib/quick-index.js';

test('quick index columns are the rounded square root of label count', () => {
  assert.equal(getQuickIndexColumnCount(99), 10);
  assert.equal(getQuickIndexColumnCount(129), 11);
  assert.equal(getQuickIndexColumnCount(0), 1);
});

test('quick find displays the Unknown category as a full-width question mark', () => {
  assert.equal(getQuickIndexDisplayLabel('未知'), '？');
  assert.equal(getQuickIndexDisplayLabel('ㄅ'), 'ㄅ');
});

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
  assert.deepEqual(result.labels, ['A', 'ㄅ', 'Z'], 'unused categories are omitted');
});

test('group rows publish as each page is classified without waiting for the remaining scan', async () => {
  const published = [];
  let releaseSecondPage;
  let secondPageStarted;
  const secondPageReady = new Promise((resolve) => { secondPageStarted = resolve; });
  const secondPage = new Promise((resolve) => { releaseSecondPage = resolve; });
  const scan = buildQuickIndexGroups({
    totalCount: 2,
    pageSize: 1,
    fetchPage: async ({ offset }) => {
      if (offset === 0) return { items: [{ name: 'Apple' }] };
      secondPageStarted();
      return secondPage;
    },
    getName: (item) => item.name,
    getGroupings: async (names) => ({ labels: ['A', 'B'], groups: names.map((name) => name[0]) }),
    onProgress: (partial) => published.push(partial),
  });

  await secondPageReady;
  assert.deepEqual(published, [{ labels: ['A'], groups: [{ label: 'A', beforeIndex: 0 }] }]);
  releaseSecondPage({ items: [{ name: 'Banana' }] });
  assert.deepEqual(await scan, {
    labels: ['A', 'B'],
    groups: [{ label: 'A', beforeIndex: 0 }, { label: 'B', beforeIndex: 1 }],
  });
});

test('group rows arriving during scroll are coalesced and committed after the last scroll', () => {
  const commits = [];
  const timers = new Map();
  let nextTimer = 0;
  const scheduler = createQuickIndexRowsScheduler((rows) => commits.push(rows), {
    delayMs: 180,
    schedule(callback) {
      const id = ++nextTimer;
      timers.set(id, callback);
      return id;
    },
    cancel(id) {
      timers.delete(id);
    },
  });
  const firstPage = [{ label: 'A', beforeIndex: 0 }];
  const secondPage = [...firstPage, { label: 'B', beforeIndex: 100 }];

  scheduler.noteScroll(1000);
  scheduler.update(firstPage, 1050);
  scheduler.update(secondPage, 1100);
  assert.deepEqual(commits, [], 'group rows must not remap the visible list during a scroll gesture');

  const [timerId, callback] = [...timers.entries()][0];
  timers.delete(timerId);
  callback();
  assert.deepEqual(commits, [secondPage], 'only the newest complete row set is committed after scrolling settles');
  scheduler.dispose();
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
  assert.deepEqual(result.labels, ['未知', '#', 'H']);
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
        // The backend classifies the actual first character, so punctuation stays grouped.
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
