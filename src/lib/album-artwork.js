export const ALBUM_ART_THUMBNAIL_SIZE = 512;
export const ALBUM_ART_HASH_SIZE = 32;
export const ALBUM_ART_SIMILARITY_DISTANCE = 6;
const DCT_LOW_FREQUENCY_COUNT = 8;
const DCT_BASIS_32 = Array.from({ length: DCT_LOW_FREQUENCY_COUNT }, (_, frequency) =>
  Array.from({ length: ALBUM_ART_HASH_SIZE }, (_, sample) =>
    Math.cos(((2 * sample + 1) * frequency * Math.PI) / (2 * ALBUM_ART_HASH_SIZE))),
);
const ARTWORK_WORKER_LIMIT = 2;
let activeArtworkWorkers = 0;
/** @type {(() => void)[]} */
const artworkWorkerQueue = [];

/** @typedef {{ worker: Worker, busy: boolean, job: { bytes: ArrayBuffer, mimeType: string, size: number, resolve: (value: { sha256: string, bytes: Uint8Array, dataBase64: string }) => void, reject: (reason: unknown) => void } | null }} ThumbnailWorkerSlot */
/** @type {ThumbnailWorkerSlot[]} */
const thumbnailWorkerSlots = [];
/** @type {{ bytes: ArrayBuffer, mimeType: string, size: number, resolve: (value: { sha256: string, bytes: Uint8Array, dataBase64: string }) => void, reject: (reason: unknown) => void }[]} */
const thumbnailWorkerQueue = [];

// Album cards are virtualized and are destroyed as the user scrolls. Keep the
// small JPEGs in memory across those unmounts so returning to a row does not
// reopen every thumbnail from UserData on C:.
const ALBUM_CARD_MEMORY_CACHE_MAX_ENTRIES = 1200;
const ALBUM_CARD_MEMORY_CACHE_MAX_BYTES = 128 * 1024 * 1024;
/** @type {Map<string, { value: string | string[] | null, bytes: number, leases: number }>} */
const albumCardMemoryCache = new Map();
let albumCardMemoryCacheBytes = 0;

/** Keep album cards and detail headers on the same confirmed no-cover fallback. */
/** @param {string[]} sources @param {{ fallbackUrl: string, resolved: boolean }} options */
export function resolveAlbumArtworkDisplaySources(sources, { fallbackUrl, resolved }) {
  if (sources.length > 0) return sources;
  return resolved ? [fallbackUrl] : [];
}

function trimAlbumCardMemoryCache() {
  while (albumCardMemoryCache.size > ALBUM_CARD_MEMORY_CACHE_MAX_ENTRIES
    || albumCardMemoryCacheBytes > ALBUM_CARD_MEMORY_CACHE_MAX_BYTES) {
    let evictKey;
    for (const [key, entry] of albumCardMemoryCache) {
      if (entry.leases === 0) {
        evictKey = key;
        break;
      }
    }
    if (evictKey === undefined) return;
    const entry = albumCardMemoryCache.get(evictKey);
    if (!entry) return;
    albumCardMemoryCache.delete(evictKey);
    albumCardMemoryCacheBytes -= entry.bytes;
    if (Array.isArray(entry.value)) entry.value.forEach((url) => URL.revokeObjectURL(url));
    else if (entry.value) URL.revokeObjectURL(entry.value);
  }
}

/** Acquire an in-memory card thumbnail. A null value is a cached no-cover result. */
/** @param {string} key */
export function acquireAlbumCardArtwork(key) {
  const entry = albumCardMemoryCache.get(key);
  if (!entry) return null;
  albumCardMemoryCache.delete(key);
  albumCardMemoryCache.set(key, entry);
  entry.leases += 1;
  let released = false;
  return {
    value: entry.value,
    release() {
      if (released) return;
      released = true;
      entry.leases = Math.max(0, entry.leases - 1);
      trimAlbumCardMemoryCache();
    },
  };
}

/** Store resolved cover images (or a no-cover result) and return a lease for the card. */
/** @param {string} key @param {string | string[] | null} value @param {number} [bytes] */
export function rememberAlbumCardArtwork(key, value, bytes = 0) {
  const existing = albumCardMemoryCache.get(key);
  if (existing) {
    if (Array.isArray(value)) value.forEach((url) => URL.revokeObjectURL(url));
    else if (value && value !== existing.value) URL.revokeObjectURL(value);
    return acquireAlbumCardArtwork(key);
  }
  const entry = { value, bytes: value ? Math.max(0, bytes) : 0, leases: 1 };
  albumCardMemoryCache.set(key, entry);
  albumCardMemoryCacheBytes += entry.bytes;
  trimAlbumCardMemoryCache();
  let released = false;
  return {
    value: entry.value,
    release() {
      if (released) return;
      released = true;
      entry.leases = Math.max(0, entry.leases - 1);
      trimAlbumCardMemoryCache();
    },
  };
}

/** @template T @param {() => Promise<T>} task @returns {Promise<T>} */
export function withAlbumArtworkSlot(task) {
  return new Promise((resolve, reject) => {
    const run = async () => {
      activeArtworkWorkers += 1;
      try { resolve(await task()); } catch (error) { reject(error); }
      finally {
        activeArtworkWorkers -= 1;
        artworkWorkerQueue.shift()?.();
      }
    };
    if (activeArtworkWorkers < ARTWORK_WORKER_LIMIT) void run();
    else artworkWorkerQueue.push(() => { void run(); });
  });
}

/** @param {Uint8Array} bytes @returns {ArrayBuffer} */
export function copyToArrayBuffer(bytes) {
  const buffer = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(buffer).set(bytes);
  return buffer;
}

/** @param {ArrayBuffer} bytes @returns {Promise<string>} */
export async function sha256Hex(bytes) {
  const digest = await globalThis.crypto.subtle.digest('SHA-256', bytes);
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

/**
 * Hash, decode, crop and JPEG-encode a card thumbnail away from the renderer thread.
 * The original buffer is transferred to a bounded worker pool so the WebView stays
 * responsive while cold artwork is read and resized.
 * @param {ArrayBuffer} bytes
 * @param {string} mimeType
 * @param {number} [size]
 * @returns {Promise<{ sha256: string, bytes: Uint8Array, dataBase64: string }>}
 */
export function createAlbumThumbnailInWorker(bytes, mimeType, size = ALBUM_ART_THUMBNAIL_SIZE) {
  return new Promise((resolve, reject) => {
    thumbnailWorkerQueue.push({ bytes, mimeType, size, resolve, reject });
    pumpThumbnailWorkers();
  });
}

function pumpThumbnailWorkers() {
  if (typeof Worker === 'undefined' || typeof OffscreenCanvas === 'undefined') {
    while (thumbnailWorkerQueue.length > 0) {
      thumbnailWorkerQueue.shift()?.reject(new Error('此 WebView 不支援背景封面縮圖處理。'));
    }
    return;
  }
  while (thumbnailWorkerQueue.length > 0) {
    let slot = thumbnailWorkerSlots.find((candidate) => !candidate.busy);
    if (!slot && thumbnailWorkerSlots.length < ARTWORK_WORKER_LIMIT) {
      try {
        const worker = new Worker(new URL('./album-thumbnail.worker.ts', import.meta.url), { type: 'module' });
        /** @type {ThumbnailWorkerSlot} */
        const createdSlot = { worker, busy: false, job: null };
        slot = createdSlot;
        worker.addEventListener('message', (event) => {
          const current = createdSlot.job;
          if (!current) return;
          createdSlot.job = null;
          createdSlot.busy = false;
          if (event.data?.ok && event.data.thumbnail instanceof ArrayBuffer) {
            current.resolve({
              sha256: event.data.sha256,
              bytes: new Uint8Array(event.data.thumbnail),
              dataBase64: event.data.dataBase64,
            });
          } else {
            current.reject(new Error(event.data?.error || '背景封面縮圖處理失敗。'));
          }
          pumpThumbnailWorkers();
        });
        worker.addEventListener('error', (event) => {
          const current = createdSlot.job;
          createdSlot.job = null;
          createdSlot.busy = false;
          if (current) current.reject(new Error(event.message || '背景封面縮圖工作者停止運作。'));
          worker.terminate();
          const index = thumbnailWorkerSlots.indexOf(createdSlot);
          if (index >= 0) thumbnailWorkerSlots.splice(index, 1);
          pumpThumbnailWorkers();
        });
        thumbnailWorkerSlots.push(createdSlot);
      } catch (error) {
        const current = thumbnailWorkerQueue.shift();
        current?.reject(error);
        continue;
      }
    }
    if (!slot) return;
    const job = thumbnailWorkerQueue.shift();
    if (!job) return;
    slot.busy = true;
    slot.job = job;
    try {
      slot.worker.postMessage({ bytes: job.bytes, mimeType: job.mimeType, size: job.size }, [job.bytes]);
    } catch (error) {
      slot.job = null;
      slot.busy = false;
      job.reject(error);
    }
  }
}

/**
 * Produce a square crop-fitted thumbnail using the same center-crop framing as album cards.
 * The input is decoded only for the duration of this call.
 * @param {ArrayBuffer} bytes
 * @param {string} mimeType
 * @param {number} [size]
 * @returns {Promise<Blob>}
 */
export async function createAlbumThumbnail(bytes, mimeType, size = ALBUM_ART_THUMBNAIL_SIZE) {
  const bitmap = await createImageBitmap(new Blob([bytes], { type: mimeType }));
  try {
    const edge = Math.max(1, Math.min(1024, Math.floor(size)));
    const cropSize = Math.min(bitmap.width, bitmap.height);
    const sx = Math.floor((bitmap.width - cropSize) / 2);
    const sy = Math.floor((bitmap.height - cropSize) / 2);
    const canvas = document.createElement('canvas');
    canvas.width = edge;
    canvas.height = edge;
    const context = canvas.getContext('2d', { alpha: false });
    if (!context) throw new Error('無法建立專輯封面縮圖。');
    context.fillStyle = '#000';
    context.fillRect(0, 0, edge, edge);
    context.drawImage(bitmap, sx, sy, cropSize, cropSize, 0, 0, edge, edge);
    return await new Promise((resolve, reject) => {
      canvas.toBlob((blob) => blob ? resolve(blob) : reject(new Error('無法編碼專輯封面縮圖。')), 'image/jpeg', 0.90);
    });
  } finally {
    bitmap.close();
  }
}

/**
 * Make a 64-bit pHash from a row-major 32x32 grayscale thumbnail.
 * @param {ArrayLike<number>} grayPixels
 * @returns {bigint}
 */
export function perceptualHash32(grayPixels) {
  if (!grayPixels || grayPixels.length !== ALBUM_ART_HASH_SIZE * ALBUM_ART_HASH_SIZE) {
    throw new RangeError('感知雜湊輸入必須是 32×32 灰階像素。');
  }
  const size = ALBUM_ART_HASH_SIZE;
  let mean = 0;
  for (let index = 0; index < grayPixels.length; index += 1) mean += Number(grayPixels[index]);
  mean /= grayPixels.length;
  let variance = 0;
  for (let index = 0; index < grayPixels.length; index += 1) {
    const delta = Number(grayPixels[index]) - mean;
    variance += delta * delta;
  }
  if (variance / grayPixels.length < 0.01) return 0n;
  // DCT is separable: transform rows first, then columns. This reduces each
  // artwork from 63 full 32×32 transforms to two small 8×32 passes.
  const horizontal = Array.from({ length: size }, () => new Float64Array(DCT_LOW_FREQUENCY_COUNT));
  for (let y = 0; y < size; y += 1) {
    for (let u = 0; u < DCT_LOW_FREQUENCY_COUNT; u += 1) {
      let sum = 0;
      for (let x = 0; x < size; x += 1) {
        sum += Number(grayPixels[y * size + x]) * DCT_BASIS_32[u][x];
      }
      horizontal[y][u] = sum;
    }
  }
  const coefficients = [];
  for (let v = 0; v < DCT_LOW_FREQUENCY_COUNT; v += 1) {
    for (let u = 0; u < DCT_LOW_FREQUENCY_COUNT; u += 1) {
      if (u === 0 && v === 0) continue;
      let sum = 0;
      for (let y = 0; y < size; y += 1) {
        sum += horizontal[y][u] * DCT_BASIS_32[v][y];
      }
      coefficients.push(sum);
    }
  }
  const sorted = [...coefficients].sort((left, right) => left - right);
  const median = sorted[Math.floor(sorted.length / 2)];
  return coefficients.reduce((hash, coefficient, index) =>
    coefficient > median ? hash | (1n << BigInt(index)) : hash, 0n);
}

/**
 * Decode a cover, center-crop it to a 32x32 canvas and return its pHash.
 * @param {ArrayBuffer} bytes
 * @param {string} mimeType
 * @returns {Promise<bigint>}
 */
export async function perceptualHashArtwork(bytes, mimeType) {
  const bitmap = await createImageBitmap(new Blob([bytes], { type: mimeType }));
  try {
    const edge = ALBUM_ART_HASH_SIZE;
    const cropSize = Math.min(bitmap.width, bitmap.height);
    const sx = Math.floor((bitmap.width - cropSize) / 2);
    const sy = Math.floor((bitmap.height - cropSize) / 2);
    const canvas = document.createElement('canvas');
    canvas.width = edge;
    canvas.height = edge;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    if (!context) throw new Error('無法計算專輯封面相似度。');
    context.fillStyle = '#000';
    context.fillRect(0, 0, edge, edge);
    context.drawImage(bitmap, sx, sy, cropSize, cropSize, 0, 0, edge, edge);
    const { data } = context.getImageData(0, 0, edge, edge);
    const gray = new Uint8Array(edge * edge);
    for (let source = 0, target = 0; target < gray.length; source += 4, target += 1) {
      gray[target] = Math.round((data[source] + data[source + 1] + data[source + 2]) / 3);
    }
    return perceptualHash32(gray);
  } finally {
    bitmap.close();
  }
}

/** @param {bigint} left @param {bigint} right */
export function hashDistance(left, right) {
  let value = left ^ right;
  let count = 0;
  while (value !== 0n) {
    value &= value - 1n;
    count += 1;
  }
  return count;
}

/** @typedef {{ group: { trackId: string, hash: bigint, count: number, firstIndex: number }, children: Map<number, ArtworkHashNode> }} ArtworkHashNode */

/**
 * Group artwork candidates in their supplied disc/track order. Each candidate counts once;
 * near-identical covers share a group, and ties keep the earliest track's cover first.
 * @template {{trackId:string, hash:bigint}} T
 * @param {T[]} candidates
 * @param {number} [maxDistance]
 * @returns {{trackId:string, hash:bigint, count:number, firstIndex:number}[]}
 */
export function rankAlbumArtwork(candidates, maxDistance = ALBUM_ART_SIMILARITY_DISTANCE) {
  /** @type {{trackId:string,hash:bigint,count:number,firstIndex:number}[]} */
  const groups = [];
  /** @type {ArtworkHashNode | null} */
  let root = null;
  /** @type {Map<bigint, (typeof groups)[number]>} */
  const exact = new Map();
  candidates.forEach((candidate, firstIndex) => {
    const sameHash = exact.get(candidate.hash);
    if (sameHash) {
      sameHash.count += 1;
      return;
    }

    /** @type {ArtworkHashNode[]} */
    const pending = root ? [root] : [];
    let match = null;
    let nearest = maxDistance + 1;
    while (pending.length > 0) {
      const node = pending.pop();
      if (!node) continue;
      const distance = hashDistance(candidate.hash, node.group.hash);
      if (distance <= maxDistance && (distance < nearest || (distance === nearest && node.group.firstIndex < (match?.firstIndex ?? Infinity)))) {
        nearest = distance;
        match = node.group;
      }
      for (const [edge, child] of node.children) {
        if (edge >= distance - maxDistance && edge <= distance + maxDistance) pending.push(child);
      }
    }

    if (match) {
      match.count += 1;
      return;
    }

    const group = { trackId: candidate.trackId, hash: candidate.hash, count: 1, firstIndex };
    groups.push(group);
    exact.set(candidate.hash, group);
    /** @type {ArtworkHashNode} */
    const inserted = { group, children: new Map() };
    if (!root) {
      root = inserted;
      return;
    }
    let node = root;
    while (true) {
      const edge = hashDistance(candidate.hash, node.group.hash);
      const child = node.children.get(edge);
      if (!child) {
        node.children.set(edge, inserted);
        return;
      }
      node = child;
    }
  });
  return groups.sort((left, right) => right.count - left.count || left.firstIndex - right.firstIndex);
}
