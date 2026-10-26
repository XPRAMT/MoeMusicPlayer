// System-bar safe insets as numbers, for code that clamps floating panels to the window.
//
// The values are the same `--android-safe-*` custom properties that `.app-shell` pads by (App.svelte sets them
// from MainActivity's WindowInsets: systemBars | displayCutout | tappableElement, on every rotation and
// navigation-mode change). On Windows they stay at their `env(safe-area-inset-*, 0px)` default, i.e. 0.

/** @typedef {{ left: number, top: number, right: number, bottom: number }} SafeInsets */

/** @type {Readonly<SafeInsets>} */
export const NO_SAFE_INSETS = Object.freeze({ left: 0, top: 0, right: 0, bottom: 0 });

export const SAFE_INSET_PROPERTIES = Object.freeze({
  left: '--android-safe-left',
  top: '--android-safe-top',
  right: '--android-safe-right',
  bottom: '--android-safe-bottom',
});

/** @param {unknown} value */
export function parseSafeInsetPx(value) {
  const pixels = Number.parseFloat(String(value ?? '').trim());
  return Number.isFinite(pixels) && pixels > 0 ? pixels : 0;
}

/**
 * Current safe insets in CSS px (0 where unset or unresolved).
 * @param {Element | null} [element]
 * @returns {SafeInsets}
 */
export function readSafeInsets(element = typeof document === 'undefined' ? null : document.documentElement) {
  if (!element || typeof getComputedStyle !== 'function') return { ...NO_SAFE_INSETS };
  const style = getComputedStyle(element);
  return {
    left: parseSafeInsetPx(style.getPropertyValue(SAFE_INSET_PROPERTIES.left)),
    top: parseSafeInsetPx(style.getPropertyValue(SAFE_INSET_PROPERTIES.top)),
    right: parseSafeInsetPx(style.getPropertyValue(SAFE_INSET_PROPERTIES.right)),
    bottom: parseSafeInsetPx(style.getPropertyValue(SAFE_INSET_PROPERTIES.bottom)),
  };
}
