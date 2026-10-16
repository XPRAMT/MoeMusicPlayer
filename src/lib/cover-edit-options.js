// Cover edit settings for the song details panel (ADR 0013 D9): JPG quality and optional
// short-edge auto-resize (aspect ratio always kept). Persisted locally per device.

// v2: defaults changed to JPG 100 / short edge 2160. Older stored values (v1 key) are
// dropped once so everyone starts from the new defaults.
const KEY = 'moe.coverEditOptions.v2';
const LEGACY_KEYS = ['moe.coverEditOptions'];

/** @typedef {{ jpegQuality: number, autoResize: boolean, shortEdgePx: number }} CoverEditOptions */

/** @type {CoverEditOptions} */
export const DEFAULT_COVER_EDIT_OPTIONS = { jpegQuality: 100, autoResize: false, shortEdgePx: 2160 };

/** @param {unknown} value @returns {CoverEditOptions} */
export function normalizeCoverEditOptions(value) {
  const raw = /** @type {Record<string, unknown>} */ (value && typeof value === 'object' ? value : {});
  const quality = Number(raw.jpegQuality);
  const edge = Number(raw.shortEdgePx);
  return {
    jpegQuality: Number.isFinite(quality) ? Math.min(100, Math.max(1, Math.round(quality))) : DEFAULT_COVER_EDIT_OPTIONS.jpegQuality,
    autoResize: raw.autoResize === true,
    shortEdgePx: Number.isFinite(edge) ? Math.min(10000, Math.max(64, Math.round(edge))) : DEFAULT_COVER_EDIT_OPTIONS.shortEdgePx,
  };
}

/** @returns {CoverEditOptions} */
export function loadCoverEditOptions() {
  try {
    for (const legacy of LEGACY_KEYS) localStorage.removeItem(legacy);
    return normalizeCoverEditOptions(JSON.parse(localStorage.getItem(KEY) ?? 'null'));
  } catch {
    return { ...DEFAULT_COVER_EDIT_OPTIONS };
  }
}

/** @param {CoverEditOptions} options */
export function saveCoverEditOptions(options) {
  try {
    localStorage.setItem(KEY, JSON.stringify(normalizeCoverEditOptions(options)));
  } catch {
    // storage unavailable: keep in-memory only
  }
}
