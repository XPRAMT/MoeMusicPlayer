import { compositeBackgroundHex, isHexColor } from './theme.js';

/** @typedef {import('./ipc').BackdropOpacityCalculation} BackdropOpacityCalculation */

const SAMPLE_SIZE = 32;

/** @param {'white' | 'black'} textColor @param {number} clarityPercent @returns {number} */
export function getBackdropTargetPercent(textColor, clarityPercent) {
  const safeClarityPercent = typeof clarityPercent === 'number' && Number.isFinite(clarityPercent)
    ? Math.max(0, Math.min(100, clarityPercent))
    : 0;
  const clarity = safeClarityPercent / 100;
  const targetY = textColor === 'white'
    ? 0.5 * (1 - clarity)
    : 0.5 + 0.5 * clarity;
  return Math.round(targetY * 100);
}

/** @param {string} hex @returns {number} */
export function rgbAverageBrightness(hex) {
  if (!isHexColor(hex)) return 0;
  const red = Number.parseInt(hex.slice(1, 3), 16);
  const green = Number.parseInt(hex.slice(3, 5), 16);
  const blue = Number.parseInt(hex.slice(5, 7), 16);
  return (red + green + blue) / (3 * 255);
}

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
 * Downscale the image and return alpha-weighted RGB averages and mean alpha.
 * Blur is ignored; average color is nearly unchanged by a uniform blur.
 * @param {string | null | undefined} imageUrl
 * @returns {Promise<{ averageHex: string, alpha: number } | null>}
 */
export async function sampleImageAverage(imageUrl) {
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
    return {
      averageHex: `#${toByte(red)}${toByte(green)}${toByte(blue)}`.toUpperCase(),
      alpha: weight / (SAMPLE_SIZE * SAMPLE_SIZE),
    };
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
 *   clarityPercent?: number,
 *   cachedImageAverageHex?: string | null,
 *   cachedImageAlpha?: number | null,
 * }} input
 * @returns {Promise<{ contrastHex: string | null, imageAverageHex: string | null, imageAlpha: number | null, opacityPercent: number, calculation: BackdropOpacityCalculation | null }>}
 */
export async function resolveBackdropContrast(input) {
  const pageHex = isHexColor(input.pageHex) ? input.pageHex.toUpperCase() : '#000000';
  const requestedOpacityPercent = typeof input.opacityPercent === 'number' && Number.isFinite(input.opacityPercent)
    ? Math.max(0, Math.min(100, Math.round(input.opacityPercent)))
    : 0;
  if (!input.imageUrl) {
    return { contrastHex: null, imageAverageHex: null, imageAlpha: null, opacityPercent: input.automaticTextColor ? 0 : requestedOpacityPercent, calculation: null };
  }

  const cachedImageSample = isHexColor(input.cachedImageAverageHex)
    ? {
      averageHex: input.cachedImageAverageHex.toUpperCase(),
      alpha: typeof input.cachedImageAlpha === 'number' && Number.isFinite(input.cachedImageAlpha)
        ? Math.max(0, Math.min(1, input.cachedImageAlpha))
        : 1,
    }
    : await sampleImageAverage(input.imageUrl);
  if (!cachedImageSample) {
    return { contrastHex: null, imageAverageHex: null, imageAlpha: null, opacityPercent: input.automaticTextColor ? 0 : requestedOpacityPercent, calculation: null };
  }
  const { averageHex: imageAverageHex, alpha: imageAlpha } = cachedImageSample;

  let opacityPercent = requestedOpacityPercent;
  /** @type {BackdropOpacityCalculation | null} */
  let calculation = null;
  if (input.automaticTextColor) {
    const pageY = rgbAverageBrightness(pageHex);
    const imageColorY = rgbAverageBrightness(imageAverageHex);
    const imageY = pageY + (imageColorY - pageY) * imageAlpha;
    const clarityPercent = typeof input.clarityPercent === 'number' && Number.isFinite(input.clarityPercent)
      ? Math.max(0, Math.min(100, input.clarityPercent))
      : 0;
    const clarity = clarityPercent / 100;
    const strictnessMargin = 0.00001;
    if (input.automaticTextColor === 'white') {
      // Higher clarity asks for a darker result: Y < 0.5 × (1 - clarity).
      const nominalTargetY = 0.5 * (1 - clarity);
      const targetY = Math.max(0, nominalTargetY - strictnessMargin);
      // On black, assume mixed luminance is opacity × image Y.
      const needsAdjustment = imageY > nominalTargetY;
      opacityPercent = !needsAdjustment
        ? 100
        : Math.min(100, (targetY / imageY) * 100);
      const targetPercent = getBackdropTargetPercent('white', clarityPercent);
      const imagePercent = Math.round(imageY * 100);
      calculation = {
        kind: needsAdjustment ? 'ratio' : 'noAdjustment',
        textColor: 'white',
        targetPercent,
        imagePercent,
        opacityPercent: Math.round(opacityPercent),
        displayText: needsAdjustment
          ? `目標亮度（${targetPercent}%）÷圖片平均亮度（${imagePercent}%）= 透明度（${Math.round(opacityPercent)}%）`
          : `目標亮度（${targetPercent}%）≥圖片平均亮度（${imagePercent}%），不處理`,
      };
    } else {
      // Higher clarity asks for a brighter result: Y > 0.5 + 0.5 × clarity.
      const nominalTargetY = 0.5 + 0.5 * clarity;
      const targetY = Math.min(1, nominalTargetY + strictnessMargin);
      // On white, assume mixed luminance is 1 - opacity × (1 - image Y).
      const needsAdjustment = imageY < nominalTargetY;
      opacityPercent = !needsAdjustment
        ? 100
        : Math.min(100, ((1 - targetY) / (1 - imageY)) * 100);
      const targetPercent = getBackdropTargetPercent('black', clarityPercent);
      const imagePercent = Math.round(imageY * 100);
      calculation = {
        kind: needsAdjustment ? 'inverseRatio' : 'noAdjustment',
        textColor: 'black',
        targetPercent,
        imagePercent,
        opacityPercent: Math.round(opacityPercent),
        displayText: needsAdjustment
          ? `（100%−目標亮度（${targetPercent}%））÷（100%−圖片平均亮度（${imagePercent}%））= 透明度（${Math.round(opacityPercent)}%）`
          : `目標亮度（${targetPercent}%）≤圖片平均亮度（${imagePercent}%），不處理`,
      };
    }
  }

  if (opacityPercent <= 0) {
    return {
      contrastHex: pageHex,
      imageAverageHex,
      imageAlpha,
      opacityPercent,
      calculation,
    };
  }

  return {
    contrastHex: compositeBackgroundHex(pageHex, imageAverageHex, opacityPercent * imageAlpha),
    imageAverageHex,
    imageAlpha,
    opacityPercent,
    calculation,
  };
}
