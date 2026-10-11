import { compositeBackgroundHex, isHexColor, relativeLuminance } from './theme.js';

const SAMPLE_SIZE = 32;

/** @param {string} imageUrl @returns {Promise<HTMLImageElement>} */
function loadImage(imageUrl) {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error('backdrop image failed to load'));
    image.src = imageUrl;
  });
}

/**
 * Downscale the image and return its alpha-weighted average color.
 * Blur is ignored; average color is nearly unchanged by a uniform blur.
 * @param {string | null | undefined} imageUrl
 * @returns {Promise<string | null>}
 */
export async function sampleImageAverageHex(imageUrl) {
  if (!imageUrl || typeof document === 'undefined') return null;
  try {
    const image = await loadImage(imageUrl);
    const canvas = document.createElement('canvas');
    canvas.width = SAMPLE_SIZE;
    canvas.height = SAMPLE_SIZE;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    if (!context) return null;
    context.drawImage(image, 0, 0, SAMPLE_SIZE, SAMPLE_SIZE);
    const { data } = context.getImageData(0, 0, SAMPLE_SIZE, SAMPLE_SIZE);
    let red = 0;
    let green = 0;
    let blue = 0;
    let weight = 0;
    for (let index = 0; index < data.length; index += 4) {
      const alpha = data[index + 3] / 255;
      if (alpha <= 0) continue;
      red += data[index] * alpha;
      green += data[index + 1] * alpha;
      blue += data[index + 2] * alpha;
      weight += alpha;
    }
    if (weight <= 0) return null;
    /** @param {number} value */
    const toByte = (value) => Math.max(0, Math.min(255, Math.round(value / weight)))
      .toString(16)
      .padStart(2, '0');
    return `#${toByte(red)}${toByte(green)}${toByte(blue)}`.toUpperCase();
  } catch {
    return null;
  }
}

/**
 * Resolve the contrast sampling color for a backdrop stack.
 * Returns null when there is no usable image contribution (caller should use page color).
 * @param {{
 *   pageHex: string,
 *   imageUrl?: string | null,
 *   opacityPercent: number,
 *   automaticTextColor?: 'white' | 'black' | null,
 *   cachedImageAverageHex?: string | null,
 * }} input
 * @returns {Promise<{ contrastHex: string | null, imageAverageHex: string | null, opacityPercent: number }>}
 */
export async function resolveBackdropContrast(input) {
  const pageHex = isHexColor(input.pageHex) ? input.pageHex.toUpperCase() : '#000000';
  const requestedOpacityPercent = typeof input.opacityPercent === 'number' && Number.isFinite(input.opacityPercent)
    ? Math.max(0, Math.min(100, Math.round(input.opacityPercent)))
    : 0;
  if (!input.imageUrl) {
    return { contrastHex: null, imageAverageHex: null, opacityPercent: input.automaticTextColor ? 0 : requestedOpacityPercent };
  }

  const imageAverageHex = isHexColor(input.cachedImageAverageHex)
    ? input.cachedImageAverageHex.toUpperCase()
    : await sampleImageAverageHex(input.imageUrl);
  if (!imageAverageHex) {
    return { contrastHex: null, imageAverageHex: null, opacityPercent: input.automaticTextColor ? 0 : requestedOpacityPercent };
  }

  let opacityPercent = requestedOpacityPercent;
  if (input.automaticTextColor) {
    const imageY = relativeLuminance(imageAverageHex);
    const targetY = 0.5;
    const strictnessMargin = 0.00001;
    if (input.automaticTextColor === 'white') {
      // On black, assume mixed luminance is opacity × image Y.
      opacityPercent = imageY < targetY
        ? 100
        : Math.min(100, ((targetY - strictnessMargin) / imageY) * 100);
    } else {
      // On white, assume mixed luminance is 1 - opacity × (1 - image Y).
      opacityPercent = imageY > targetY
        ? 100
        : Math.min(100, ((targetY - strictnessMargin) / (1 - imageY)) * 100);
    }
  }

  if (opacityPercent <= 0) {
    return {
      contrastHex: pageHex,
      imageAverageHex,
      opacityPercent,
    };
  }

  return {
    contrastHex: compositeBackgroundHex(pageHex, imageAverageHex, opacityPercent),
    imageAverageHex,
    opacityPercent,
  };
}
