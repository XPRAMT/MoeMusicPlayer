import test from 'node:test';
import assert from 'node:assert/strict';
import {
  acquireAlbumCardArtwork,
  hashDistance,
  loadAlbumCardThumbnails,
  perceptualHash32,
  rankAlbumArtwork,
  rememberAlbumCardArtwork,
  resolveAlbumArtworkDisplaySources,
} from '../src/lib/album-artwork.js';

function gray(fill) {
  return new Uint8Array(32 * 32).fill(fill);
}

test('album home and detail use one shared confirmed no-cover fallback rule', () => {
  const fallback = 'data:image/jpeg;base64,default';
  assert.deepEqual(resolveAlbumArtworkDisplaySources([], { fallbackUrl: fallback, resolved: false }), []);
  assert.deepEqual(resolveAlbumArtworkDisplaySources([], { fallbackUrl: fallback, resolved: true }), [fallback]);
  const selected = ['blob:cover-1', 'blob:cover-2'];
  assert.equal(resolveAlbumArtworkDisplaySources(selected, { fallbackUrl: fallback, resolved: true }), selected);
});

test('virtualized album cards reuse the same in-memory thumbnail after unmount and remount', () => {
  const key = `virtualized-${crypto.randomUUID()}`;
  const url = URL.createObjectURL(new Blob(['thumbnail-bytes']));
  const firstMount = rememberAlbumCardArtwork(key, url, 15);
  assert.equal(firstMount.value, url);
  firstMount.release();

  const remounted = acquireAlbumCardArtwork(key);
  assert.ok(remounted, 'the image remains cached when the virtual card is unmounted');
  assert.equal(remounted.value, url, 'remount must reuse the same object URL instead of requesting the disk cache again');
  remounted.release();
});

test('virtualized album cards remember tracks with no cover for the current list generation', () => {
  const key = `virtualized-no-cover-${crypto.randomUUID()}`;
  const firstMount = rememberAlbumCardArtwork(key, null);
  firstMount.release();
  const remounted = acquireAlbumCardArtwork(key);
  assert.ok(remounted, 'a known no-cover result is cached');
  assert.equal(remounted.value, null);
  remounted.release();
});

test('album home cards load the complete representative-cover mosaic before returning it', async () => {
  const loaded = [];
  const result = await loadAlbumCardThumbnails(['most-used', 'second', 'third', 'fourth', 'fifth'], async (trackId) => {
    loaded.push(trackId);
    return { url: `blob:${trackId}`, bytes: trackId.length };
  });

  assert.deepEqual(loaded, ['most-used', 'second', 'third', 'fourth']);
  assert.deepEqual(result.urls, ['blob:most-used', 'blob:second', 'blob:third', 'blob:fourth']);
  assert.equal(result.bytes, 'most-used'.length + 'second'.length + 'third'.length + 'fourth'.length);
});

test('album home artwork result waits for selected thumbnails and skips unavailable covers', async () => {
  let finishSecond;
  let completed = false;
  const loading = loadAlbumCardThumbnails(['first', 'second', 'missing'], async (trackId) => {
    if (trackId === 'second') return new Promise((resolve) => { finishSecond = resolve; });
    if (trackId === 'missing') return null;
    return { url: 'blob:first', bytes: 10 };
  }).then((result) => {
    completed = true;
    return result;
  });

  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(completed, false, 'do not publish a partial mosaic while another selected image is loading');
  finishSecond({ url: 'blob:second', bytes: 20 });
  const result = await loading;
  assert.deepEqual(result.urls, ['blob:first', 'blob:second']);
  assert.equal(result.bytes, 30);
});

test('32x32 pHash is stable for flat images and distinguishes structured luminance patterns', () => {
  const dark = perceptualHash32(gray(16));
  const bright = perceptualHash32(gray(240));
  assert.equal(dark, bright);

  const leftHalf = gray(0);
  for (let y = 0; y < 32; y += 1) {
    for (let x = 16; x < 32; x += 1) leftHalf[y * 32 + x] = 255;
  }
  const rightHalf = gray(0);
  for (let y = 0; y < 32; y += 1) {
    for (let x = 0; x < 16; x += 1) rightHalf[y * 32 + x] = 255;
  }
  assert.ok(hashDistance(perceptualHash32(leftHalf), perceptualHash32(rightHalf)) > 6);
});

test('separable 32x32 DCT pHash matches direct 2D coefficient calculation', () => {
  let state = 0x4a63d12f;
  const pixels = new Uint8Array(32 * 32);
  for (let index = 0; index < pixels.length; index += 1) {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    pixels[index] = state >>> 24;
  }
  const coefficients = [];
  for (let v = 0; v < 8; v += 1) {
    for (let u = 0; u < 8; u += 1) {
      if (u === 0 && v === 0) continue;
      let sum = 0;
      for (let y = 0; y < 32; y += 1) {
        for (let x = 0; x < 32; x += 1) {
          sum += pixels[y * 32 + x]
            * Math.cos(((2 * x + 1) * u * Math.PI) / 64)
            * Math.cos(((2 * y + 1) * v * Math.PI) / 64);
        }
      }
      coefficients.push(sum);
    }
  }
  const ordered = [...coefficients].sort((left, right) => left - right);
  const median = ordered[Math.floor(ordered.length / 2)];
  const expected = coefficients.reduce((hash, value, index) =>
    value > median ? hash | (1n << BigInt(index)) : hash, 0n);
  assert.equal(perceptualHash32(pixels), expected);
});

test('album artwork grouping counts repeated and near-identical covers, with stable frequency ties', () => {
  const ranked = rankAlbumArtwork([
    { trackId: 'cover-a', hash: 0n },
    { trackId: 'cover-b', hash: 0b11n },
    { trackId: 'cover-c', hash: (1n << 30n) | (1n << 31n) | (1n << 32n) },
    { trackId: 'cover-d', hash: (1n << 30n) | (1n << 31n) | (1n << 32n) },
  ], 2);
  assert.deepEqual(ranked.map(({ trackId, count }) => [trackId, count]), [
    ['cover-a', 2],
    ['cover-c', 2],
  ]);
});

test('album artwork grouping skips exact differences above the perceptual threshold', () => {
  const ranked = rankAlbumArtwork([
    { trackId: 'first', hash: 0n },
    { trackId: 'different', hash: 0b1111111n },
  ], 6);
  assert.deepEqual(ranked.map(({ trackId, count }) => [trackId, count]), [
    ['first', 1],
    ['different', 1],
  ]);
});

test('Hamming-distance tree keeps nearest-cover grouping consistent with the ordered greedy rule', () => {
  let state = 0x12345678;
  const candidates = Array.from({ length: 240 }, (_, index) => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    const low = BigInt(state);
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return { trackId: `track-${index}`, hash: low | (BigInt(state) << 32n) };
  });
  const expected = [];
  for (let index = 0; index < candidates.length; index += 1) {
    const candidate = candidates[index];
    let nearestGroup = null;
    let nearestDistance = 7;
    for (const group of expected) {
      const distance = hashDistance(candidate.hash, group.hash);
      if (distance < nearestDistance) {
        nearestGroup = group;
        nearestDistance = distance;
      }
    }
    if (nearestGroup && nearestDistance <= 6) nearestGroup.count += 1;
    else expected.push({ trackId: candidate.trackId, hash: candidate.hash, count: 1, firstIndex: index });
  }
  expected.sort((left, right) => right.count - left.count || left.firstIndex - right.firstIndex);
  assert.deepEqual(rankAlbumArtwork(candidates), expected);
});

