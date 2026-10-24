// Word-timed lyrics in the native windows (ADR 0041): reference implementation of the centre mask and the sung word
// in crates/player-lyrics-rules/src/words.rs and WordLyricsRules.kt, used by scripts/generate-lyrics-rules-vectors.mjs.
// The word glide and the karaoke edge are in lyrics-glide.js. The renderers themselves are native.

export const MASK_BAND_EM = 1;
export const MASK_EDGE_EM = 1;

/**
 * Gradient stops `[offset 0..1, opacity]` of the static continuous-mode mask (experimental): 0 → inactive over the
 * edge em, inactive → 1 up to an opaque em centred on the area, mirrored. Below four em the em shrinks to a quarter.
 */
export function centreMaskStops(/** @type {number} */ areaWidth, /** @type {number} */ emPx, /** @type {number} */ inactive) {
  if (!(areaWidth > 0) || !(emPx > 0)) return [];
  const a = Number.isNaN(inactive) ? 1 : Math.min(1, Math.max(0, inactive));
  const e = Math.min(emPx, areaWidth / 4);
  const edge = MASK_EDGE_EM * e;
  const half = MASK_BAND_EM * e / 2;
  const c = areaWidth / 2;
  return [[0, 0], [edge, a], [c - half, 1], [c + half, 1], [areaWidth - edge, a], [areaWidth, 0]]
    .map(([x, o]) => [x / areaWidth, o]);
}

export function centreMaskOpacity(/** @type {number} */ x, /** @type {number} */ areaWidth, /** @type {number} */ emPx, /** @type {number} */ inactive) {
  const stops = centreMaskStops(areaWidth, emPx, inactive);
  if (stops.length === 0) return 0;
  const p = Math.min(1, Math.max(0, x / areaWidth));
  let value = stops[0][1];
  for (let i = 0; i + 1 < stops.length; i += 1) {
    const [a, va] = stops[i];
    const [b, vb] = stops[i + 1];
    if (p >= a && p <= b) {
      value = b > a ? va + (vb - va) * (p - a) / (b - a) : vb;
      if (p < b) break;
    }
  }
  return value;
}

/** Index of the word sung at `t` (`start <= t < end`, zero-length never), or -1. */
export function sungWord(/** @type {[number, number][]} */ words, /** @type {number} */ t) {
  let i = 0;
  while (i < words.length && words[i][0] <= t) i += 1;
  for (let j = i - 1; j >= 0; j -= 1) if (t < words[j][1] && words[j][1] > words[j][0]) return j;
  return -1;
}
