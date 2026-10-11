import assert from 'node:assert/strict';
import test from 'node:test';
import { getBackdropTargetPercent, resolveBackdropContrast, rgbAverageBrightness, sampleImageAverage } from '../src/lib/backdrop-contrast.js';

test('resolveBackdropContrast skips sampling when opacity is zero or image is missing', async () => {
  assert.deepEqual(
    await resolveBackdropContrast({ pageHex: '#000000', imageUrl: null, opacityPercent: 40 }),
    { contrastHex: null, imageAverageHex: null, imageAlpha: null, opacityPercent: 40, calculation: null },
  );
  assert.deepEqual(
    await resolveBackdropContrast({
      pageHex: '#000000',
      imageUrl: 'https://example.invalid/cover.jpg',
      opacityPercent: 0,
    }),
    { contrastHex: null, imageAverageHex: null, imageAlpha: null, opacityPercent: 0, calculation: null },
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
  assert.equal(resolved.imageAlpha, 1);
  assert.equal(resolved.contrastHex, '#666666');
  assert.equal(resolved.opacityPercent, 40);
  assert.equal(resolved.calculation, null);
});

test('backdrop target text follows selected text color and clarity', () => {
  assert.deepEqual(
    [0, 50, 100].map((clarity) => getBackdropTargetPercent('white', clarity)),
    [100, 50, 0],
  );
  assert.deepEqual(
    [0, 50, 100].map((clarity) => getBackdropTargetPercent('black', clarity)),
    [0, 50, 100],
  );
});

test('image Y is the unweighted arithmetic mean of RGB channels without linearization', () => {
  assert.equal(rgbAverageBrightness('#808080'), 128 / 255);
  assert.equal(rgbAverageBrightness('#FF0000'), 1 / 3);
  assert.equal(rgbAverageBrightness('#00FF00'), 1 / 3);
});

test('image RGB average alpha-weights visible pixels and skips fully transparent pixels', async () => {
  const previousImage = Object.getOwnPropertyDescriptor(globalThis, 'Image');
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  const data = new Uint8ClampedArray(32 * 32 * 4);
  data.set([0, 0, 0, 255], 0);
  data.set([255, 255, 255, 255], 4);
  data.set([255, 0, 0, 128], 8);
  data.set([255, 255, 255, 0], 12);
  globalThis.Image = class {
    set src(_value) { this.onload(); }
  };
  globalThis.document = {
    createElement: () => ({
      getContext: () => ({
        drawImage() {},
        getImageData: () => ({ data }),
      }),
    }),
  };
  try {
    assert.deepEqual(await sampleImageAverage('fixture'), {
      averageHex: '#996666',
      alpha: (2 + 128 / 255) / (32 * 32),
    });
  } finally {
    if (previousImage) Object.defineProperty(globalThis, 'Image', previousImage);
    else delete globalThis.Image;
    if (previousDocument) Object.defineProperty(globalThis, 'document', previousDocument);
    else delete globalThis.document;
  }
});

test('automatic opacity composites source alpha against the selected black or white base', async () => {
  const whiteTextOnBlack = await resolveBackdropContrast({
    pageHex: '#000000', imageUrl: 'image', opacityPercent: 100,
    automaticTextColor: 'white', cachedImageAverageHex: '#000000', cachedImageAlpha: 0.5,
  });
  const blackTextOnWhite = await resolveBackdropContrast({
    pageHex: '#FFFFFF', imageUrl: 'image', opacityPercent: 100,
    automaticTextColor: 'black', clarityPercent: 50,
    cachedImageAverageHex: '#000000', cachedImageAlpha: 0.5,
  });
  assert.equal(whiteTextOnBlack.calculation.imagePercent, 0);
  assert.equal(whiteTextOnBlack.calculation.kind, 'noAdjustment');
  assert.equal(blackTextOnWhite.calculation.imagePercent, 50);
  assert.ok(blackTextOnWhite.opacityPercent > 99.99);
  assert.equal(blackTextOnWhite.contrastHex, '#808080');
});

test('automatic opacity uses the direct Y ratio for white text on a black base', async () => {
  const imageY = rgbAverageBrightness('#DADADA');
  const resolved = await resolveBackdropContrast({
    pageHex: '#000000',
    imageUrl: 'https://example.invalid/unused.jpg',
    opacityPercent: 100,
    automaticTextColor: 'white',
    clarityPercent: 50,
    cachedImageAverageHex: '#DADADA',
  });
  assert.ok(Math.abs(resolved.opacityPercent - ((0.5 - 0.00001) / imageY) * 100) < 0.000001);
  assert.equal(Math.round(resolved.opacityPercent), 58);
  assert.ok(resolved.opacityPercent * imageY / 100 < 0.5);
});

test('automatic opacity uses the direct Y ratio for black text on a white base', async () => {
  const imageY = rgbAverageBrightness('#7C7C7C');
  const resolved = await resolveBackdropContrast({
    pageHex: '#FFFFFF',
    imageUrl: 'https://example.invalid/unused.jpg',
    opacityPercent: 0,
    automaticTextColor: 'black',
    clarityPercent: 50,
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
  assert.equal(Math.round(rgbAverageBrightness('#AAAAAA') * 100), 67);
  assert.equal(Math.round(resolved.opacityPercent), 60);
  assert.equal(resolved.calculation.displayText, '目標亮度（40%）÷圖片平均亮度（67%）= 透明度（60%）');
  assert.equal(resolved.calculation.kind, 'ratio');
});

test('automatic opacity reports when the sampled background already meets the white-text target', async () => {
  const resolved = await resolveBackdropContrast({
    pageHex: '#000000', imageUrl: 'image', opacityPercent: 100,
    automaticTextColor: 'white', clarityPercent: 60, cachedImageAverageHex: '#2E2E2E',
  });
  assert.equal(Math.round(rgbAverageBrightness('#2E2E2E') * 100), 18);
  assert.equal(resolved.opacityPercent, 100);
  assert.equal(resolved.calculation.displayText, '目標亮度（40%）≥圖片平均亮度（18%），不處理');
  assert.equal(resolved.calculation.kind, 'noAdjustment');
});

test('automatic opacity reports the inverse white-base equation for black text', async () => {
  const resolved = await resolveBackdropContrast({
    pageHex: '#FFFFFF', imageUrl: 'image', opacityPercent: 100,
    automaticTextColor: 'black', clarityPercent: 80, cachedImageAverageHex: '#AAAAAA',
  });
  assert.equal(Math.round(resolved.opacityPercent), 60);
  assert.equal(resolved.calculation.displayText, '（100%−目標亮度（80%））÷（100%−圖片平均亮度（67%））= 透明度（60%）');
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

test('clarity sets the white-text target Y to 1, 0.5, and 0 at 0, 50, and 100 percent', async () => {
  const imageY = rgbAverageBrightness('#DADADA');
  for (const [clarityPercent, expectedTargetY] of [[0, 1], [50, 0.5], [100, 0]]) {
    const resolved = await resolveBackdropContrast({
      pageHex: '#000000',
      imageUrl: 'image',
      opacityPercent: 100,
      automaticTextColor: 'white',
      clarityPercent,
      cachedImageAverageHex: '#DADADA',
    });
    const compositedY = (resolved.opacityPercent / 100) * imageY;
    if (clarityPercent === 0) assert.ok(compositedY < expectedTargetY);
    else if (clarityPercent === 50) assert.ok(compositedY < expectedTargetY);
    else assert.equal(compositedY, expectedTargetY);
  }
});

test('clarity sets the black-text target Y to 0, 0.5, and 1 at 0, 50, and 100 percent', async () => {
  const imageY = rgbAverageBrightness('#7C7C7C');
  for (const [clarityPercent, expectedTargetY] of [[0, 0], [50, 0.5], [100, 1]]) {
    const resolved = await resolveBackdropContrast({
      pageHex: '#FFFFFF',
      imageUrl: 'image',
      opacityPercent: 100,
      automaticTextColor: 'black',
      clarityPercent,
      cachedImageAverageHex: '#7C7C7C',
    });
    const compositedY = 1 - (resolved.opacityPercent / 100) * (1 - imageY);
    if (clarityPercent === 0) assert.ok(compositedY > expectedTargetY);
    else if (clarityPercent === 50) assert.ok(compositedY > expectedTargetY);
    else assert.equal(compositedY, expectedTargetY);
  }
});

test('clarity strict bounds adjust exact boundary images except at explicit zero and one endpoints', async () => {
  const whiteLow = await resolveBackdropContrast({
    pageHex: '#000000', imageUrl: 'image', opacityPercent: 100,
    automaticTextColor: 'white', clarityPercent: 0, cachedImageAverageHex: '#FFFFFF',
  });
  const whiteHigh = await resolveBackdropContrast({
    pageHex: '#000000', imageUrl: 'image', opacityPercent: 100,
    automaticTextColor: 'white', clarityPercent: 100, cachedImageAverageHex: '#000000',
  });
  const blackLow = await resolveBackdropContrast({
    pageHex: '#FFFFFF', imageUrl: 'image', opacityPercent: 100,
    automaticTextColor: 'black', clarityPercent: 0, cachedImageAverageHex: '#000000',
  });
  const blackHigh = await resolveBackdropContrast({
    pageHex: '#FFFFFF', imageUrl: 'image', opacityPercent: 100,
    automaticTextColor: 'black', clarityPercent: 100, cachedImageAverageHex: '#FFFFFF',
  });
  assert.ok(whiteLow.opacityPercent < 100);
  assert.equal(whiteHigh.opacityPercent, 100);
  assert.ok(blackLow.opacityPercent < 100);
  assert.equal(blackHigh.opacityPercent, 100);
});
