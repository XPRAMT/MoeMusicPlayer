// Word-timed lyrics in the native windows (ADR 0041 option C): reference implementation of
// crates/player-lyrics-rules/src/words.rs and FloatingLyricsRules.kt (word glide, speed cap, centre mask, sung word),
// used by scripts/generate-lyrics-rules-vectors.mjs. The renderers themselves are native.

export const WORD_SPEED_FACTOR = 2;
export const WORD_MAX_LEAD_MS = 600;
export const MASK_BAND_EM = 1;
export const MASK_SOFT_EM = 0.1;

/**
 * @typedef {{ startMs: number, cellStart: number, wordStarts: number[], wordCentres: number[] }} KnotLine
 * @typedef {{ t: number, v: number, line: number }} Knot
 */

/** @param {KnotLine[]} lines @returns {Knot[]} */
export function wordKnots(lines) {
  /** @type {Knot[]} */
  const out = [];
  lines.forEach((l, line) => {
    const n = Math.min(l.wordStarts.length, l.wordCentres.length);
    if (n === 0) {
      out.push({ t: l.startMs, v: l.cellStart, line });
      return;
    }
    for (let i = 0; i < n; i += 1) out.push({ t: l.wordStarts[i], v: l.cellStart + Math.max(0, l.wordCentres[i]), line });
  });
  return out;
}

/** @param {Knot[]} knots @param {number} lineCount @returns {(number | null)[]} */
export function lineSpeedCaps(knots, lineCount) {
  const caps = [];
  for (let line = 0; line < lineCount; line += 1) {
    const mine = knots.filter((k) => k.line === line);
    if (mine.length < 2) { caps.push(null); continue; }
    const dt = mine[mine.length - 1].t - mine[0].t;
    const dv = mine[mine.length - 1].v - mine[0].v;
    caps.push(dt > 0 && dv > 0 ? WORD_SPEED_FACTOR * dv / dt : null);
  }
  return caps;
}

/** @param {Knot[]} knots @param {number} lineCount @returns {Knot[]} */
export function limitKnotSpeed(knots, lineCount) {
  const caps = lineSpeedCaps(knots, lineCount);
  const k = knots.map((x) => ({ ...x }));
  for (let i = 1; i < k.length; i += 1) if (k[i].v < k[i - 1].v) k[i].v = k[i - 1].v;
  const original = k.map((x) => x.t);
  for (let i = k.length - 1; i >= 1; i -= 1) {
    const cap = caps[k[i].line];
    if (cap == null) continue;
    const need = (k[i].v - k[i - 1].v) / cap;
    if (k[i].t - k[i - 1].t >= need) continue;
    let t = Math.max(k[i].t - need, original[i - 1] - WORD_MAX_LEAD_MS);
    if (i >= 2) t = Math.max(t, k[i - 2].t);
    k[i - 1].t = Math.min(k[i - 1].t, t);
  }
  /** @type {Knot[]} */
  const out = [];
  for (const knot of k) {
    while (out.length > 0 && out[out.length - 1].t >= knot.t) out.pop();
    out.push(knot);
  }
  return out;
}

/** @param {KnotLine[]} lines @returns {[number, number][]} */
export function wordGlideKnots(lines) {
  return limitKnotSpeed(wordKnots(lines), lines.length).map((k) => [k.t, k.v]);
}

/** Offset (`target - v`) of the monotone C1 glide through the knots at `positionMs`. */
export function glideOffset(/** @type {[number, number][]} */ knots, /** @type {number} */ positionMs, /** @type {number} */ target) {
  const n = knots.length;
  if (n === 0) return 0;
  const tangent = (/** @type {number} */ k) => {
    if (k === 0 || k >= n - 1) return 0;
    const h0 = knots[k][0] - knots[k - 1][0];
    const h1 = knots[k + 1][0] - knots[k][0];
    if (h0 <= 0 || h1 <= 0) return 0;
    const d0 = (knots[k][1] - knots[k - 1][1]) / h0;
    const d1 = (knots[k + 1][1] - knots[k][1]) / h1;
    if (d0 <= 0 || d1 <= 0) return 0;
    const w0 = 2 * h1 + h0;
    const w1 = h1 + 2 * h0;
    return (w0 + w1) / (w0 / d0 + w1 / d1);
  };
  let i = -1;
  while (i + 1 < n && knots[i + 1][0] <= positionMs) i += 1;
  if (i < 0) return target - knots[0][1];
  if (i >= n - 1) return target - knots[n - 1][1];
  const [t0, v0] = knots[i];
  const h = knots[i + 1][0] - t0;
  const m0 = tangent(i);
  const m1 = tangent(i + 1);
  const d = (knots[i + 1][1] - v0) / h;
  const c2 = (3 * d - 2 * m0 - m1) / h;
  const c3 = (m0 + m1 - 2 * d) / (h * h);
  const u = positionMs - t0;
  return target - (v0 + u * (m0 + u * (c2 + u * c3)));
}

/** Gradient stops `[offset 0..1, opacity]` of a row's static centre mask. */
export function wordMaskStops(/** @type {number} */ areaWidth, /** @type {number} */ emPx, /** @type {number} */ inactivePercent) {
  if (!(areaWidth > 0) || !(emPx > 0)) return [];
  const inactive = Math.min(100, Math.max(10, inactivePercent)) / 100;
  const c = areaWidth / 2;
  const a = MASK_BAND_EM * emPx / 2;
  const b = a + MASK_BAND_EM * emPx;
  const s = MASK_SOFT_EM * emPx / 2;
  return [[0, 0], [c - b, inactive], [c - a - s, inactive], [c - a + s, 1], [c + a - s, 1], [c + a + s, inactive], [c + b, inactive], [areaWidth, 0]]
    .map(([x, o]) => [Math.min(areaWidth, Math.max(0, x)) / areaWidth, o]);
}

export function wordMaskOpacity(/** @type {number} */ x, /** @type {number} */ areaWidth, /** @type {number} */ emPx, /** @type {number} */ inactivePercent) {
  const stops = wordMaskStops(areaWidth, emPx, inactivePercent);
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

/**
 * Karaoke clip edge (ADR 0041, 「無」 mode): the x (in the row's own px) up to which the line is lit. Words already
 * sung and the word being sung are lit; inside the sung word the edge wipes linearly with the time. Before the first
 * word the edge is the first word's left; after a word ends it rests at that word's right. Monotone when the spans are.
 */
export function karaokeClipX(/** @type {[number, number][]} */ spans, /** @type {[number, number][]} */ words, /** @type {number} */ t) {
  const n = Math.min(spans.length, words.length);
  if (n === 0) return 0;
  let x = spans[0][0];
  for (let i = 0; i < n; i += 1) {
    const [s, e] = words[i];
    const [l, r] = spans[i];
    if (t < s) break;
    if (t >= e || !(e > s)) { x = Math.max(x, r); continue; }
    x = Math.max(x, l + (r - l) * (t - s) / (e - s));
    break;
  }
  return x;
}
