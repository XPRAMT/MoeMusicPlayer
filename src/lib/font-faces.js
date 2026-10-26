// Android WebView has no access to the system's font families by name, so a chosen font (ADR 0042) is loaded from
// its files through `@font-face` (the FontFace API) under `androidFontAlias(family)`. The backend allows exactly the
// listed font files in the asset protocol's scope. A face's file is fetched only when text uses it.
import { convertFileSrc } from '@tauri-apps/api/core';
import { androidFontFaceSources } from './font-picker.js';

const registered = new Set();

/** Whether this runs in the Android app's WebView. */
export function isAndroidWebView() {
  return typeof navigator !== 'undefined' && /Android/i.test(navigator.userAgent ?? '');
}

/**
 * Register `font`'s faces once (Android only; a no-op elsewhere, where CSS reaches installed families by name).
 * @param {import('./ipc').FontFamilyEntry | null | undefined} font
 */
export function ensureAndroidFontFaces(font) {
  if (!font || !isAndroidWebView() || registered.has(font.family)) return;
  if (typeof FontFace === 'undefined' || typeof document === 'undefined' || !document.fonts) return;
  registered.add(font.family);
  for (const face of androidFontFaceSources(font, convertFileSrc)) {
    try {
      document.fonts.add(new FontFace(face.family, face.source, face.descriptors));
    } catch {
      // A face the WebView cannot read draws in the next font of the stack.
    }
  }
}
