import assert from 'node:assert/strict';
import test from 'node:test';

import { pruneScales, syncVolumes } from '../src/lib/volume-sync.js';

const volumes = (entries) => new Map(Object.entries(entries));

test('other devices keep their ratio to the one that moved', () => {
  const scales = new Map();
  const before = volumes({ a: 50, b: 25, c: 100 });
  const updates = syncVolumes({ changedId: 'a', next: 80, volumes: before, ids: ['a', 'b', 'c'], scales });
  assert.deepEqual([...updates], [['b', 40], ['c', 100]], 'b stays half of a, c is clamped to 100');
});

test('ratios within 10% snap to equal and are remembered, so going to 0 and back restores the proportions', () => {
  const scales = new Map();
  let current = volumes({ a: 50, b: 53 });
  let updates = syncVolumes({ changedId: 'a', next: 30, volumes: current, ids: ['a', 'b'], scales });
  assert.equal(updates.get('b'), 30, 'about equal snaps to the same level');
  current = volumes({ a: 30, b: 30 });
  updates = syncVolumes({ changedId: 'a', next: 0, volumes: current, ids: ['a', 'b'], scales });
  assert.equal(updates.get('b'), 0);
  current = volumes({ a: 0, b: 0 });
  updates = syncVolumes({ changedId: 'a', next: 60, volumes: current, ids: ['a', 'b'], scales });
  assert.equal(updates.get('b'), 60, 'the remembered ratio is used even though both were at 0');
});

test('moving the other device uses the same pair ratio seen from its side', () => {
  const scales = new Map();
  syncVolumes({ changedId: 'a', next: 80, volumes: volumes({ a: 40, b: 20 }), ids: ['a', 'b'], scales });
  const updates = syncVolumes({ changedId: 'b', next: 10, volumes: volumes({ a: 80, b: 40 }), ids: ['a', 'b'], scales });
  assert.equal(updates.get('a'), 20, 'b is half of a, so a is twice b');
});

test('devices outside the sync or without a known volume are left alone, and old ratios are forgotten', () => {
  const scales = new Map();
  const updates = syncVolumes({ changedId: 'a', next: 50, volumes: volumes({ a: 100, b: 100 }), ids: ['a', 'b', 'c'], scales });
  assert.deepEqual([...updates.keys()], ['b']);
  assert.ok(scales.size > 0);
  pruneScales(scales, ['a']);
  assert.equal(scales.size, 0);
});
