interface Request {
  bytes: ArrayBuffer;
  mimeType: string;
  size: number;
}

const scope = self as unknown as {
  addEventListener(type: 'message', listener: (event: MessageEvent<Request>) => void): void;
  postMessage(message: unknown, transfer?: Transferable[]): void;
};

scope.addEventListener('message', async (event: MessageEvent<Request>) => {
  const { bytes, mimeType, size } = event.data;
  let bitmap: ImageBitmap | undefined;
  try {
    const sha256 = await crypto.subtle.digest('SHA-256', bytes);
    const hash = [...new Uint8Array(sha256)].map((byte) => byte.toString(16).padStart(2, '0')).join('');
    bitmap = await createImageBitmap(new Blob([bytes], { type: mimeType }));
    const edge = Math.max(1, Math.min(1024, Math.floor(size)));
    const cropSize = Math.min(bitmap.width, bitmap.height);
    const sx = Math.floor((bitmap.width - cropSize) / 2);
    const sy = Math.floor((bitmap.height - cropSize) / 2);
    const canvas = new OffscreenCanvas(edge, edge);
    const context = canvas.getContext('2d', { alpha: false });
    if (!context) throw new Error('無法建立專輯封面縮圖。');
    context.fillStyle = '#000';
    context.fillRect(0, 0, edge, edge);
    context.drawImage(bitmap, sx, sy, cropSize, cropSize, 0, 0, edge, edge);
    const blob = await canvas.convertToBlob({ type: 'image/jpeg', quality: 1.0 });
    const thumbnail = await blob.arrayBuffer();
    const thumbnailBytes = new Uint8Array(thumbnail);
    let binaryText = '';
    for (let offset = 0; offset < thumbnailBytes.length; offset += 0x8000) {
      binaryText += String.fromCharCode(...thumbnailBytes.subarray(offset, offset + 0x8000));
    }
    scope.postMessage({ ok: true, sha256: hash, thumbnail, dataBase64: btoa(binaryText) }, [thumbnail]);
  } catch (error) {
    scope.postMessage({ ok: false, error: error instanceof Error ? error.message : String(error) });
  } finally {
    bitmap?.close();
  }
});
