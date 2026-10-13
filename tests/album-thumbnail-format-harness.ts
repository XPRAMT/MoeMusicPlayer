import {
  ALBUM_ART_THUMBNAIL_SIZE,
  createAlbumThumbnail,
  createAlbumThumbnailInWorker,
} from '../src/lib/album-artwork.js';

declare global {
  interface Window {
    __albumThumbnailFormatResult?: {
      size: number;
      mainDimensions: [number, number];
      workerDimensions: [number, number];
      mainMatchesQuality090: boolean;
      workerMatchesQuality090: boolean;
      mainDiffersFromQuality100: boolean;
      workerDiffersFromQuality100: boolean;
      mainDiffersFromQuality088: boolean;
      workerDiffersFromQuality088: boolean;
      workerBase64MatchesBytes: boolean;
    };
  }
}

function sameBytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

async function encodeBaseline(bytes: Uint8Array, quality: number): Promise<Blob> {
  const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/jpeg' }));
  try {
    const cropSize = Math.min(bitmap.width, bitmap.height);
    const sourceX = Math.floor((bitmap.width - cropSize) / 2);
    const sourceY = Math.floor((bitmap.height - cropSize) / 2);
    const canvas = document.createElement('canvas');
    canvas.width = ALBUM_ART_THUMBNAIL_SIZE;
    canvas.height = ALBUM_ART_THUMBNAIL_SIZE;
    const context = canvas.getContext('2d', { alpha: false });
    if (!context) throw new Error('baseline 2D context unavailable');
    context.fillStyle = '#000';
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.drawImage(bitmap, sourceX, sourceY, cropSize, cropSize, 0, 0, canvas.width, canvas.height);
    return await new Promise((resolve, reject) => {
      canvas.toBlob((blob) => blob ? resolve(blob) : reject(new Error('baseline JPEG encode failed')), 'image/jpeg', quality);
    });
  } finally {
    bitmap.close();
  }
}

async function dimensions(blob: Blob): Promise<[number, number]> {
  const bitmap = await createImageBitmap(blob);
  try {
    return [bitmap.width, bitmap.height];
  } finally {
    bitmap.close();
  }
}

async function run(): Promise<void> {
  const source = document.createElement('canvas');
  source.width = 384;
  source.height = 256;
  const context = source.getContext('2d', { alpha: false });
  if (!context) throw new Error('fixture 2D context unavailable');
  const image = context.createImageData(source.width, source.height);
  for (let y = 0; y < source.height; y += 1) {
    for (let x = 0; x < source.width; x += 1) {
      const offset = (y * source.width + x) * 4;
      image.data[offset] = (x * 37 + y * 13 + ((x * y) % 251)) & 0xff;
      image.data[offset + 1] = (x * 11 + y * 53 + ((x + 2 * y) % 239)) & 0xff;
      image.data[offset + 2] = (x * 29 + y * 7 + ((3 * x + y) % 227)) & 0xff;
      image.data[offset + 3] = 255;
    }
  }
  context.putImageData(image, 0, 0);
  const sourceBlob = await new Promise<Blob>((resolve, reject) => {
    source.toBlob((blob) => blob ? resolve(blob) : reject(new Error('fixture source encode failed')), 'image/jpeg', 1.0);
  });
  const sourceBytes = new Uint8Array(await sourceBlob.arrayBuffer());
  const [quality090, quality100, quality088] = await Promise.all([
    encodeBaseline(sourceBytes, 0.90),
    encodeBaseline(sourceBytes, 1.0),
    encodeBaseline(sourceBytes, 0.88),
  ]);
  const mainBlob = await createAlbumThumbnail(sourceBytes, 'image/jpeg');
  const workerResult = await createAlbumThumbnailInWorker(sourceBytes.slice().buffer, 'image/jpeg');
  const workerBlob = new Blob([workerResult.bytes], { type: 'image/jpeg' });
  const quality090Bytes = new Uint8Array(await quality090.arrayBuffer());
  const quality100Bytes = new Uint8Array(await quality100.arrayBuffer());
  const quality088Bytes = new Uint8Array(await quality088.arrayBuffer());
  const mainBytes = new Uint8Array(await mainBlob.arrayBuffer());
  const workerBytes = workerResult.bytes;
  const decodedWorkerBase64 = Uint8Array.from(atob(workerResult.dataBase64), (character) => character.charCodeAt(0));

  window.__albumThumbnailFormatResult = {
    size: ALBUM_ART_THUMBNAIL_SIZE,
    mainDimensions: await dimensions(mainBlob),
    workerDimensions: await dimensions(workerBlob),
    mainMatchesQuality090: sameBytes(mainBytes, quality090Bytes),
    workerMatchesQuality090: sameBytes(workerBytes, quality090Bytes),
    mainDiffersFromQuality100: !sameBytes(mainBytes, quality100Bytes),
    workerDiffersFromQuality100: !sameBytes(workerBytes, quality100Bytes),
    mainDiffersFromQuality088: !sameBytes(mainBytes, quality088Bytes),
    workerDiffersFromQuality088: !sameBytes(workerBytes, quality088Bytes),
    workerBase64MatchesBytes: sameBytes(decodedWorkerBase64, workerBytes),
  };
  document.querySelector('#result')!.textContent = JSON.stringify(window.__albumThumbnailFormatResult);
}

void run().catch((error) => {
  document.querySelector('#result')!.textContent = `ERROR: ${error instanceof Error ? error.message : String(error)}`;
});
