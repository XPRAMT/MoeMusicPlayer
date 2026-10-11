import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveBackdropContrast } from '../src/lib/backdrop-contrast.js';
import { relativeLuminance } from '../src/lib/theme.js';

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

test('automatic opacity uses the direct Y ratio for white text on a black base', async () => {
  const imageY = relativeLuminance('#DADADA');
  const resolved = await resolveBackdropContrast({
    pageHex: '#000000',
    imageUrl: 'https://example.invalid/unused.jpg',
    opacityPercent: 100,
    automaticTextColor: 'white',
    cachedImageAverageHex: '#DADADA',
  });
  assert.ok(Math.abs(resolved.opacityPercent - ((0.5 - 0.00001) / imageY) * 100) < 0.000001);
  assert.equal(Math.round(resolved.opacityPercent), 71);
  assert.ok(resolved.opacityPercent * imageY / 100 < 0.5);
});

test('automatic opacity uses the direct Y ratio for black text on a white base', async () => {
  const imageY = relativeLuminance('#7C7C7C');
  const resolved = await resolveBackdropContrast({
    pageHex: '#FFFFFF',
    imageUrl: 'https://example.invalid/unused.jpg',
    opacityPercent: 0,
    automaticTextColor: 'black',
    cachedImageAverageHex: '#7C7C7C',
  });
  assert.ok(Math.abs(resolved.opacityPercent - ((0.5 - 0.00001) / (1 - imageY)) * 100) < 0.000001);
  assert.ok(1 - (resolved.opacityPercent / 100) * (1 - imageY) > 0.5);
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
