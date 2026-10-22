// App-internal cover copy/paste keeps the original bytes (user decision 2026-10-22).
// The system clipboard only carries a PNG (the WebView re-encodes images), so 複製 remembers the
// cover's original bytes with a fingerprint of its decoded pixels; 貼上 uses those bytes when the
// clipboard image still has the same fingerprint, so an unscaled JPG is written as-is, not re-encoded.
// Anything else on the clipboard (a different image, one copied in another app) is used as read.

/** @type {{ fingerprint: string, original: Blob } | null} */
let copied = null;

/**
 * Remember the cover just put on the clipboard.
 * @param {string | null} fingerprint
 * @param {Blob | null} original
 */
export function rememberCopiedCover(fingerprint, original) {
  copied = fingerprint && original ? { fingerprint, original } : null;
}

/**
 * The remembered original bytes when the clipboard image matches the last copy, else null.
 * @param {string | null} fingerprint
 * @returns {Blob | null}
 */
export function originalForPastedCover(fingerprint) {
  return copied && fingerprint && copied.fingerprint === fingerprint ? copied.original : null;
}

/**
 * Fingerprint of decoded pixels: size plus SHA-256 of the RGBA data.
 * @param {number} width
 * @param {number} height
 * @param {Uint8Array<ArrayBuffer> | Uint8ClampedArray<ArrayBuffer>} rgba
 * @returns {Promise<string>}
 */
export async function pixelFingerprint(width, height, rgba) {
  const digest = await crypto.subtle.digest('SHA-256', rgba);
  const hex = Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('');
  return `${width}x${height}:${hex}`;
}
