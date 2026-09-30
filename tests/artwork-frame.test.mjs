import assert from 'node:assert/strict';
import test from 'node:test';
import { calculateArtworkFrame } from '../src/lib/artwork-frame.js';

test('artwork frame fits square, portrait and landscape images without changing source ratio', () => {
  const cases = [
    { sourceWidth: 800, sourceHeight: 800 },
    { sourceWidth: 600, sourceHeight: 900 },
    { sourceWidth: 1200, sourceHeight: 800 },
  ];
  for (const image of cases) {
    const frame = calculateArtworkFrame({
      ...image,
      availableWidth: 700,
      availableHeight: 500,
      maxWidth: 650,
      maxHeight: 480,
      border: 2,
    });
    assert.ok(frame);
    assert.ok(Math.abs(frame.imageWidth / frame.imageHeight - image.sourceWidth / image.sourceHeight) < 1e-9);
    assert.ok(frame.width <= 650 && frame.height <= 480);
    assert.ok(frame.width <= 700 && frame.height <= 500);
  }
});

test('artwork frame rejects unavailable or invalid geometry', () => {
  assert.equal(calculateArtworkFrame({ sourceWidth: 800, sourceHeight: 800, availableWidth: 0, availableHeight: 500, maxWidth: 650, maxHeight: 480 }), null);
  assert.equal(calculateArtworkFrame({ sourceWidth: 0, sourceHeight: 800, availableWidth: 700, availableHeight: 500, maxWidth: 650, maxHeight: 480 }), null);
});
