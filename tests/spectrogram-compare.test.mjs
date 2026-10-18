import test from 'node:test';
import assert from 'node:assert/strict';
import {
  FULL_VIEW, differenceImage, dropAssignment, layerHeightFraction, normalizeComparePrefs,
  panView, sampleLevel, timeTickStep, timeTickValues, worldStyle, zoomView,
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

test('the time axis ticks every 10 seconds, coarser for a long track and finer when zoomed in', () => {
  // A 3:45 track over 1700 px: 10 seconds are 75 px apart.
  assert.equal(timeTickStep(225, 1700), 10);
  assert.equal(timeTickStep(120, 1200), 10);
  // An hour over 1700 px: 10 seconds would be 4.7 px apart, so the step grows until the labels fit.
  assert.equal(timeTickStep(3600, 1700), 120);
  // Zoomed far in: 10 seconds would be 1000 px apart.
  assert.equal(timeTickStep(17, 1700), 2);
  assert.equal(timeTickStep(1.7, 1700), 0.2);
  assert.equal(timeTickStep(0, 1700), 10);
  assert.equal(timeTickStep(60, 0), 10);
});

test('time ticks are the multiples of the step inside the view', () => {
  assert.deepEqual(timeTickValues(0, 45, 10), [0, 10, 20, 30, 40]);
  assert.deepEqual(timeTickValues(15, 45, 10), [20, 30, 40]);
  assert.deepEqual(timeTickValues(0, 1, 0.5), [0, 0.5, 1]);
  assert.deepEqual(timeTickValues(5, 4, 10), []);
  assert.deepEqual(timeTickValues(0, 10, 0), []);
});

test('zooming both axes around one pointer position keeps that position fixed on each axis', () => {
  const first = zoomView(FULL_VIEW, 'x', 0.3, 0.5);
  const both = zoomView(first, 'y', 0.7, 0.5);
  assert.ok(both.x1 - both.x0 < 1 && both.y1 - both.y0 < 1);
  // The world point under the pointer is the same before and after, on each axis.
  assert.ok(Math.abs((both.x0 + (both.x1 - both.x0) * 0.3) - 0.3) < 1e-9);
  assert.ok(Math.abs((both.y0 + (both.y1 - both.y0) * 0.7) - 0.7) < 1e-9);
});
