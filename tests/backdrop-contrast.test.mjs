import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveBackdropContrast } from '../src/lib/backdrop-contrast.js';
import { compositeBackgroundHex, relativeLuminance } from '../src/lib/theme.js';

test('resolveBackdropContrast skips sampling when opacity is zero or image is missing', async () => {
  assert.deepEqual(
    await resolveBackdropContrast({ pageHex: '#000000', imageUrl: null, opacityPercent: 40 }),
    { contrastHex: null, imageAverageHex: null, opacityPercent: 40 },
  );
  assert.deepEqual(
    await resolveBackdropContrast({
      pageHex: '#000000',
      imageUrl: 'https://example.invalid/cover.jpg',
      opacityPercent: 0,
    }),
    { contrastHex: null, imageAverageHex: null, opacityPercent: 0 },
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
  assert.equal(resolved.opacityPercent, 40);
});

test('automatic opacity keeps the brightest image contribution compatible with white text', async () => {
  const resolved = await resolveBackdropContrast({
    pageHex: '#000000',
    imageUrl: 'https://example.invalid/unused.jpg',
    opacityPercent: 100,
    automaticTextColor: 'white',
    cachedImageAverageHex: '#FFFFFF',
  });
  assert.ok(relativeLuminance(resolved.contrastHex) < 0.5);
  assert.ok(resolved.opacityPercent < 100);
  assert.ok(relativeLuminance(compositeBackgroundHex('#000000', '#FFFFFF', resolved.opacityPercent + 1)) >= 0.5);
});

test('automatic opacity keeps the brightest image contribution compatible with black text', async () => {
  const resolved = await resolveBackdropContrast({
    pageHex: '#FFFFFF',
    imageUrl: 'https://example.invalid/unused.jpg',
    opacityPercent: 0,
    automaticTextColor: 'black',
    cachedImageAverageHex: '#000000',
  });
  assert.ok(relativeLuminance(resolved.contrastHex) > 0.5);
  assert.ok(resolved.opacityPercent < 100);
  assert.ok(relativeLuminance(compositeBackgroundHex('#FFFFFF', '#000000', resolved.opacityPercent + 1)) <= 0.5);
});

test('automatic opacity preserves a full-strength image when it matches the fixed base color', async () => {
  const whiteText = await resolveBackdropContrast({
    pageHex: '#000000', imageUrl: 'image', opacityPercent: 100,
    automaticTextColor: 'white', cachedImageAverageHex: '#000000',
  });
  const blackText = await resolveBackdropContrast({
    pageHex: '#FFFFFF', imageUrl: 'image', opacityPercent: 100,
    automaticTextColor: 'black', cachedImageAverageHex: '#FFFFFF',
  });
  assert.equal(whiteText.opacityPercent, 100);
  assert.equal(blackText.opacityPercent, 100);
});
