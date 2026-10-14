import test from 'node:test';
import assert from 'node:assert/strict';
import {
  buildQuickIndexGroups,
  buildPagedVirtualRows,
  getRestoredListScrollTop,
  reportPagedRange,
  shouldAdjustGroupAnchor,
  dataIndexToVisualIndex,
  getQuickIndexColumnCount,
  getQuickIndexDisplayLabel,
  getQuickIndexBuildKey,
  visualIndexToAnchorDataIndex,
  visualIndexToDataIndex,
  visualRangeToDataRange,
} from '../src/lib/quick-index.js';

test('page snapshot updates do not change the quick-index build identity', () => {
  const inputs = {
    listId: 'library',
    resetKey: 4,
    query: null,
    fieldFilter: null,
    sort: { field: 'title', direction: 'asc' },
    count: 5224,
    activeSort: { field: 'title', direction: 'asc' },
    groupingEnabled: true,
    sortable: true,
  };
  const initialSnapshot = { ...inputs, revision: 1, cachedPageCount: 1 };
  const laterSnapshot = { ...inputs, revision: 18, cachedPageCount: 4 };

  assert.equal(
    getQuickIndexBuildKey(initialSnapshot),
    getQuickIndexBuildKey(laterSnapshot),
    'fetching pages must not restart the index scan or blank the list',
  );
  assert.notEqual(
    getQuickIndexBuildKey(inputs),
    getQuickIndexBuildKey({ ...inputs, count: 5225 }),
    'a changed library count must rebuild the group index',
  );
});

test('quick index columns are the rounded square root of label count', () => {
  assert.equal(getQuickIndexColumnCount(99), 10);
  assert.equal(getQuickIndexColumnCount(129), 11);
  assert.equal(getQuickIndexColumnCount(0), 1);
});

test('quick find displays the Unknown category as a full-width question mark', () => {
  assert.equal(getQuickIndexDisplayLabel('未知'), '？');
  assert.equal(getQuickIndexDisplayLabel('ㄅ'), 'ㄅ');
});

test('complete grouping renders its group and track rows without waiting for a second state echo', () => {
  const groups = [{ label: 'A', beforeIndex: 0 }];
  const items = [{ id: 'track-a' }];
  const input = {
    groups,
    start: 0,
    end: 2,
    totalCount: 1,
    revision: 1,
    itemAt: (index) => items[index] ?? null,
    getKey: (item) => item.id,
  };
  assert.deepEqual(buildPagedVirtualRows({ ...input, ready: false }).rows, [], 'partial grouping must remain hidden');

  const rendered = buildPagedVirtualRows({
    ...input,
    ready: true,
    revision: 7,
  });

  assert.equal(rendered.revision, 7, 'page revisions must flow into the derived virtual rows');
  assert.deepEqual(rendered.rows, [
    { kind: 'group', visualIndex: 0, label: 'A', key: 'quick-index:0:A' },
    { kind: 'item', visualIndex: 1, index: 0, item: items[0], key: 'track-a' },
  ]);
});

test('a page revision rebuilds a stable non-grouped visible range', () => {
  const cachedTrack = { id: 'track-a' };
  const first = buildPagedVirtualRows({
    ready: true,
    groups: [],
    start: 0,
    end: 1,
    totalCount: 1,
    revision: 1,
    itemAt: () => null,
    getKey: (item) => item.id,
  });
  const afterPageLoad = buildPagedVirtualRows({
    ready: true,
    groups: [],
    start: 0,
    end: 1,
    totalCount: 1,
    revision: 2,
    itemAt: () => cachedTrack,
    getKey: (item) => item.id,
  });

  assert.deepEqual(first.rows.map(({ item }) => item), [null]);
  assert.equal(afterPageLoad.revision, 2);
  assert.deepEqual(afterPageLoad.rows.map(({ item }) => item), [cachedTrack]);
});

test('the initial estimated viewport range loads before measurement and refreshes after measurement', () => {
  const requested = [];
  const initialRequest = reportPagedRange({
    start: 0,
    end: 10,
    groups: [],
    totalCount: 40,
    onRange: (range) => requested.push(range),
  });

  assert.equal(initialRequest, true);
  assert.deepEqual(requested, [{ start: 0, end: 10 }]);
  assert.equal(reportPagedRange({
    start: 0,
    end: 13,
    groups: [],
    totalCount: 40,
    onRange: (range) => requested.push(range),
  }), true);
  assert.deepEqual(requested, [{ start: 0, end: 10 }, { start: 0, end: 13 }]);
});

test('saved visual scroll position waits for complete group rows and is restored only once', () => {
  const savedPosition = 1840;
  const base = {
    position: savedPosition,
    totalCount: 100,
    pending: false,
    rowHeight: 50,
    viewportHeight: 600,
  };

  assert.equal(getRestoredListScrollTop({ ...base, groupingReady: false, virtualTotalCount: 100 }), null);
  assert.equal(getRestoredListScrollTop({ ...base, groupingReady: true, virtualTotalCount: 114 }), savedPosition);
  assert.equal(getRestoredListScrollTop({ ...base, groupingReady: true, virtualTotalCount: 114, pending: true }), null);
  assert.equal(getRestoredListScrollTop({ ...base, groupingReady: true, virtualTotalCount: 20 }), 400);
});

test('group anchor adjustment is skipped for the first complete snapshot and at the list top', () => {
  assert.equal(shouldAdjustGroupAnchor({ groupsChanged: true, hasPreviousSnapshot: false, scrollTop: 1840 }), false);
  assert.equal(shouldAdjustGroupAnchor({ groupsChanged: true, hasPreviousSnapshot: true, scrollTop: 0 }), false);
  assert.equal(shouldAdjustGroupAnchor({ groupsChanged: true, hasPreviousSnapshot: true, scrollTop: 1840 }), true);
  assert.equal(shouldAdjustGroupAnchor({ groupsChanged: false, hasPreviousSnapshot: true, scrollTop: 1840 }), false);
});

test('re-entering a grouped list repeatedly does not accumulate heading-height offsets', () => {
  const savedVisualPosition = 1840;
  const groups = Array.from({ length: 18 }, (_, index) => ({ label: `group-${index}`, beforeIndex: index * 5 }));
  let savedPosition = savedVisualPosition;

  for (let entry = 0; entry < 32; entry += 1) {
    const beforeGrouping = getRestoredListScrollTop({
      position: savedPosition,
      groupingReady: false,
      totalCount: 100,
      pending: false,
      virtualTotalCount: 100,
      rowHeight: 50,
      viewportHeight: 600,
    });
    assert.equal(beforeGrouping, null, `entry ${entry + 1} must not restore in data-row coordinates`);

    const restored = getRestoredListScrollTop({
      position: savedPosition,
      groupingReady: true,
      totalCount: 100,
      pending: false,
      virtualTotalCount: 100 + groups.length,
      rowHeight: 50,
      viewportHeight: 600,
    });
    assert.equal(restored, savedVisualPosition, `entry ${entry + 1} must restore the same visual position`);
    assert.equal(shouldAdjustGroupAnchor({
      groupsChanged: true,
      hasPreviousSnapshot: false,
      scrollTop: restored,
    }), false, `entry ${entry + 1} must not re-anchor the first complete group snapshot`);
    savedPosition = restored;
  }
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

test('group boundaries are returned only after the complete scan finishes', async () => {
  let resolved = false;
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
  }).then((value) => {
    resolved = true;
    return value;
  });

  await secondPageReady;
  assert.equal(resolved, false, 'a partial group map must not be published');
  releaseSecondPage({ items: [{ name: 'Banana' }] });
  assert.deepEqual(await scan, {
    labels: ['A', 'B'],
    groups: [{ label: 'A', beforeIndex: 0 }, { label: 'B', beforeIndex: 1 }],
  });
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

test('leading punctuation uses its own locale grouping and sends each distinct initial once', async () => {
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
  assert.deepEqual(requestedNames, ['-', 'H']);
});

test('all page initials are classified in one backend call after every page is scanned', async () => {
  const items = [{ name: 'Apple' }, { name: 'Apricot' }, { name: 'Banana' }];
  const requested = [];
  let scanned = 0;
  const result = await buildQuickIndexGroups({
    totalCount: items.length,
    pageSize: 2,
    fetchPage: async ({ offset, limit }) => {
      scanned += 1;
      return { items: items.slice(offset, offset + limit) };
    },
    getName: (item) => item.name,
    getGroupings: async (initials) => {
      assert.equal(scanned, 2, 'grouping starts only after the complete list scan');
      requested.push(...initials);
      return { labels: ['A', 'B'], groups: initials.map((initial) => initial.toUpperCase()) };
    },
  });

  assert.deepEqual(requested, ['A', 'B'], 'repeat initials are sent once');
  assert.deepEqual(result.groups, [
    { label: 'A', beforeIndex: 0 },
    { label: 'B', beforeIndex: 2 },
  ]);
});
