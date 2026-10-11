import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveBackdropContrast } from '../src/lib/backdrop-contrast.js';
import { relativeLuminance } from '../src/lib/theme.js';

test('resolveBackdropContrast skips sampling when opacity is zero or image is missing', async () => {
  assert.deepEqual(
    await resolveBackdropContrast({ pageHex: '#000000', imageUrl: null, opacityPercent: 40 }),
    { contrastHex: null, imageAverageHex: null, opacityPercent: 40, calculation: null },
  );
  assert.deepEqual(
    await resolveBackdropContrast({
      pageHex: '#000000',
      imageUrl: 'https://example.invalid/cover.jpg',
      opacityPercent: 0,
    }),
    { contrastHex: null, imageAverageHex: null, opacityPercent: 0, calculation: null },
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
  assert.equal(resolved.calculation, null);
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

test('automatic opacity reports the white-text target, sampled brightness, and direct ratio', async () => {
  const resolved = await resolveBackdropContrast({
    pageHex: '#000000', imageUrl: 'image', opacityPercent: 100,
    automaticTextColor: 'white', clarityPercent: 60, cachedImageAverageHex: '#AAAAAA',
  });
  assert.equal(Math.round(relativeLuminance('#AAAAAA') * 100), 40);
  assert.equal(Math.round(resolved.opacityPercent), 50);
  assert.equal(resolved.calculation.displayText, '目標亮度（20%）÷圖片平均亮度（40%）＝透明度（50%）');
  assert.equal(resolved.calculation.kind, 'ratio');
});

test('automatic opacity reports when the sampled background already meets the white-text target', async () => {
  const resolved = await resolveBackdropContrast({
    pageHex: '#000000', imageUrl: 'image', opacityPercent: 100,
    automaticTextColor: 'white', clarityPercent: 60, cachedImageAverageHex: '#595959',
  });
  assert.equal(Math.round(relativeLuminance('#595959') * 100), 10);
  assert.equal(resolved.opacityPercent, 100);
  assert.equal(resolved.calculation.displayText, '目標亮度（20%）≥圖片平均亮度（10%），不處理');
  assert.equal(resolved.calculation.kind, 'noAdjustment');
});

test('automatic opacity reports the inverse white-base equation for black text', async () => {
  const resolved = await resolveBackdropContrast({
    pageHex: '#FFFFFF', imageUrl: 'image', opacityPercent: 100,
    automaticTextColor: 'black', clarityPercent: 60, cachedImageAverageHex: '#AAAAAA',
  });
  assert.equal(Math.round(resolved.opacityPercent), 33);
  assert.equal(resolved.calculation.displayText, '（100%−目標亮度（80%））÷（100%−圖片平均亮度（40%））＝透明度（33%）');
  assert.equal(resolved.calculation.kind, 'inverseRatio');
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

test('clarity sets the white-text target Y to 0.5, 0.25, and 0 at 0, 50, and 100 percent', async () => {
  const imageY = relativeLuminance('#DADADA');
  for (const [clarityPercent, expectedTargetY] of [[0, 0.5], [50, 0.25], [100, 0]]) {
    const resolved = await resolveBackdropContrast({
      pageHex: '#000000',
      imageUrl: 'image',
      opacityPercent: 100,
      automaticTextColor: 'white',
      clarityPercent,
      cachedImageAverageHex: '#DADADA',
    });
    assert.ok(Math.abs((resolved.opacityPercent / 100) * imageY - expectedTargetY) < 0.00002);
  }
});

test('clarity sets the black-text target Y to 0.5, 0.75, and 1 at 0, 50, and 100 percent', async () => {
  const imageY = relativeLuminance('#7C7C7C');
  for (const [clarityPercent, expectedTargetY] of [[0, 0.5], [50, 0.75], [100, 1]]) {
    const resolved = await resolveBackdropContrast({
      pageHex: '#FFFFFF',
      imageUrl: 'image',
      opacityPercent: 100,
      automaticTextColor: 'black',
      clarityPercent,
      cachedImageAverageHex: '#7C7C7C',
    });
    const compositedY = 1 - (resolved.opacityPercent / 100) * (1 - imageY);
    assert.ok(Math.abs(compositedY - expectedTargetY) < 0.00002);
  }
});
