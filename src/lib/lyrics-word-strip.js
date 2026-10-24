// Word-timed lyrics in the native windows (ADR 0041): reference implementation of the centre mask and the sung word
// in crates/player-lyrics-rules/src/words.rs and WordLyricsRules.kt, used by scripts/generate-lyrics-rules-vectors.mjs.
// The word glide and the karaoke edge are in lyrics-glide.js. The renderers themselves are native.

export const MASK_BAND_EM = 1;
export const MASK_SOFT_EM = 0.1;

/** Gradient stops `[offset 0..1, opacity]` of a row's static centre mask: an opaque em, straight ramps to 0 at the edges. */
export function centreMaskStops(/** @type {number} */ areaWidth, /** @type {number} */ emPx) {
  if (!(areaWidth > 0) || !(emPx > 0)) return [];
  const c = areaWidth / 2;
  const a = MASK_BAND_EM * emPx / 2;
  const s = MASK_SOFT_EM * emPx / 2;
  const q = c - a > 0 ? Math.max(0, (c - a - s) / (c - a)) : 1;
  return [[0, 0], [c - a - s, q], [c - a + s, 1], [c + a - s, 1], [c + a + s, q], [areaWidth, 0]]
    .map(([x, o]) => [Math.min(areaWidth, Math.max(0, x)) / areaWidth, o]);
}

export function centreMaskOpacity(/** @type {number} */ x, /** @type {number} */ areaWidth, /** @type {number} */ emPx) {
  const stops = centreMaskStops(areaWidth, emPx);
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
