import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveBackdropContrast } from '../src/lib/backdrop-contrast.js';

test('resolveBackdropContrast skips sampling when opacity is zero or image is missing', async () => {
  assert.deepEqual(
    await resolveBackdropContrast({ pageHex: '#000000', imageUrl: null, opacityPercent: 40 }),
    { contrastHex: null, imageAverageHex: null },
  );
  assert.deepEqual(
    await resolveBackdropContrast({
      pageHex: '#000000',
      imageUrl: 'https://example.invalid/cover.jpg',
      opacityPercent: 0,
    }),
    { contrastHex: null, imageAverageHex: null },
  );
});

test('resolveBackdropContrast reuses a cached image average without loading', async () => {
  const resolved = await resolveBackdropContrast({
    pageHex: '#000000',
    imageUrl: 'https://example.invalid/unused.jpg',
    opacityPercent: 40,
    cachedImageAverageHex: '#FFFFFF',
  });
  assert.equal(resolved.imageAverageHex, '#FFFFFF');
  assert.equal(resolved.contrastHex, '#666666');
});
