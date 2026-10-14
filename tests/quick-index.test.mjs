import test from 'node:test';
import assert from 'node:assert/strict';
import {
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

test('the initial viewport range is requested while grouping is still pending', () => {
  const requested = [];
  const didRequest = reportPagedRange({
    metricsReady: true,
    start: 0,
    end: 40,
    groups: [],
    totalCount: 40,
    onRange: (range) => requested.push(range),
  });

  assert.equal(didRequest, true);
  assert.deepEqual(requested, [{ start: 0, end: 40 }]);
  assert.equal(reportPagedRange({
    metricsReady: false,
    start: 0,
    end: 40,
    groups: [],
    totalCount: 40,
    onRange: (range) => requested.push(range),
  }), false);
  assert.equal(requested.length, 1, 'range waits for the viewport metrics while controller reset seeds the first page');
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
