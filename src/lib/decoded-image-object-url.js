/**
 * Creates an object URL only after the browser has decoded its image.
 * Callers can keep rendering the previous URL until this promise resolves.
 * @param {ArrayBuffer} bytes
 * @param {string} mimeType
 * @param {{ createObjectURL?: (blob: Blob) => string, revokeObjectURL?: (url: string) => void, createImage?: () => HTMLImageElement }} [options]
 * @returns {Promise<string>}
 */
export async function createDecodedImageObjectUrl(bytes, mimeType, options = {}) {
  const createObjectURL = options.createObjectURL ?? ((blob) => URL.createObjectURL(blob));
  const revokeObjectURL = options.revokeObjectURL ?? ((url) => URL.revokeObjectURL(url));
  const createImage = options.createImage ?? (() => new Image());
  const url = createObjectURL(new Blob([bytes], { type: mimeType }));

  try {
    const image = createImage();
    if (typeof image.decode === 'function') {
      image.src = url;
      await image.decode();
    } else {
      await new Promise((resolve, reject) => {
        image.onload = resolve;
        image.onerror = () => reject(new Error('Image failed to load'));
        image.src = url;
      });
    }
    if (image.naturalWidth === 0 || image.naturalHeight === 0) {
      throw new Error('Decoded image has no dimensions');
    }
    return url;
  } catch (error) {
    revokeObjectURL(url);
    throw error;
  }
}
