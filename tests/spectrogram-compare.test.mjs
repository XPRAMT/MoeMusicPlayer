import test from 'node:test';
import assert from 'node:assert/strict';
import {
  FULL_VIEW, differenceImage, dropAssignment, layerHeightFraction, normalizeComparePrefs,
  panView, sampleLevel, worldStyle, zoomView,
} from '../src/lib/spectrogram-compare.js';

test('zoom keeps anchor and stays in bounds; pan clamps', () => {
  const v = zoomView(FULL_VIEW, 'x', 0.25, 0.5);
  assert.deepEqual([v.x0, v.x1], [0.125, 0.625]);
  assert.deepEqual(zoomView(v, 'x', 0.5, 10), FULL_VIEW);
  const p = panView(v, 10, 0);
  assert.ok(Math.abs(p.x1 - 1) < 1e-9);
  assert.deepEqual(worldStyle({ x0: 0.5, x1: 1, y0: 0, y1: 1 }), { left: -100, width: 200, bottom: -0, height: 100 });
});

test('layer height on shared axis', () => {
  assert.equal(layerHeightFraction(22050, 48000, false), 22050 / 48000);
  assert.equal(layerHeightFraction(48000, 48000, true), 1);
  assert.ok(layerHeightFraction(22050, 48000, true) > 22050 / 48000);
});

test('sampling and difference', () => {
  const a = { levels: new Uint8Array([0, 255, 0, 255]), columns: 2, height: 2, widthFraction: 1, heightFraction: 1 };
  const b = { levels: new Uint8Array([255, 255, 0, 0]), columns: 2, height: 2, widthFraction: 0.5, heightFraction: 1 };
  assert.equal(sampleLevel(a, 0.75, 0.9), 255); // top-right
  assert.equal(sampleLevel(b, 0.75, 0.9), null);
  const img = differenceImage(a, b, 2, 2);
  assert.deepEqual([...img.slice(0, 4)], [255, 0, 0, 255]); // top-left: B louder → red
});

test('drop assignment', () => {
  assert.deepEqual(dropAssignment(['a', 'b'], 'single', null, 0), { A: 'a', B: 'b' });
  assert.deepEqual(dropAssignment(['a'], 'single', 'B', 0.9), { A: 'a' });
  assert.deepEqual(dropAssignment(['a'], 'sideH', 'B', 0.1), { B: 'a' });
  assert.deepEqual(dropAssignment(['a'], 'wipe', null, 0.8), { B: 'a' });
  assert.equal(normalizeComparePrefs({ mode: 'x' }).mode, 'single');
});
