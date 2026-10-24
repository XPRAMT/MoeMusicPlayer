// The smoothed glide of word-timed lyrics (ADR 0041 D10): one pure curve used for the continuous strip's word glide
// (Windows native, Android floating lyrics) and for the karaoke colour edge (Now Playing, 「無」 mode on Windows and
// Android). Reference implementation of `crates/player-lyrics-rules/src/glide.rs` and `LyricsGlide` in
// `WordLyricsRules.kt`; checked by the shared vectors (scripts/generate-lyrics-rules-vectors.mjs).
//
// Hard knots (line starts, and the line end of a karaoke line) are hit exactly. Between two hard knots the curve
// follows soft guides (the words) but its speed stays within GLIDE_MIN_SPEED..GLIDE_MAX_SPEED x the stretch's average
// speed. Construction: a polyline through the hard knots and the guides, each guide clamped into what the speed band
// can reach from the previous point and still reach the end; each hard knot gets a tangent (a straight run of
// GLIDE_KNOT_WINDOW of the shorter neighbouring stretch on each side); every other corner is rounded by a symmetric
// quadratic blend over half of each neighbouring run. The speed is then piecewise linear between polyline slopes, so
// it never leaves the band, never goes negative (monotone) and is continuous (C1).
//
// A glide is a list of pieces `[t, v, s, c]`: from lyrics time `t` up to the next piece the value is
// `v + s u + c u^2` (`u = time - t`); the last piece is the rest value (s = c = 0). Before the first piece the value is
// the first piece's `v`.

export const GLIDE_MIN_SPEED = 0.5;
export const GLIDE_MAX_SPEED = 1.5;
/** Each hard knot's straight run on each side, as a fraction of the shorter neighbouring stretch. */
export const GLIDE_KNOT_WINDOW = 0.125;

/** @typedef {[number, number, number, number]} GlidePiece */

/** @param {number} x @param {number} lo @param {number} hi */
const clamp = (x, lo, hi) => Math.min(hi, Math.max(lo, x));

/**
 * The smoothed glide through hard knots `[t, v]` (strictly increasing times; later knots with an earlier or equal time
 * are dropped, values are made non-decreasing), with soft guides `guides[j]` (`[t, v]`, in time order) between knot j
 * and knot j + 1. `easeEnds`: the curve starts and stops with speed 0 (the strip rests outside it); otherwise it starts
 * and ends at the first and last stretch's average speed (the karaoke edge).
 * @param {[number, number][]} hard @param {[number, number][][]} guides @param {boolean} easeEnds
 * @returns {GlidePiece[]}
 */
export function smoothGlide(hard, guides, easeEnds) {
  /** @type {[number, number][]} */
  const knots = [];
  /** @type {[number, number][][]} */
  const between = [];
  let dropped = false;
  hard.forEach(([t, v], j) => {
    if (!Number.isFinite(t) || !Number.isFinite(v)) { dropped = true; return; }
    if (knots.length > 0 && t <= knots[knots.length - 1][0]) { dropped = true; return; }
    if (knots.length > 0) between.push(dropped ? [] : (guides[j - 1] ?? []));
    knots.push([t, knots.length > 0 ? Math.max(v, knots[knots.length - 1][1]) : v]);
    dropped = false;
  });
  const n = knots.length;
  if (n === 0) return [];
  if (n === 1) return [[knots[0][0], knots[0][1], 0, 0]];
  const avg = [];
  for (let j = 0; j + 1 < n; j += 1) avg.push((knots[j + 1][1] - knots[j][1]) / (knots[j + 1][0] - knots[j][0]));
  const span = (/** @type {number} */ j) => knots[j + 1][0] - knots[j][0];
  const tangent = [];
  const window = [];
  for (let j = 0; j < n; j += 1) {
    if (j === 0 || j === n - 1) {
      const a = avg[j === 0 ? 0 : n - 2];
      tangent.push(easeEnds ? 0 : a);
      window.push(GLIDE_KNOT_WINDOW * span(j === 0 ? 0 : n - 2));
      continue;
    }
    const aL = avg[j - 1];
    const aR = avg[j];
    const lo = GLIDE_MIN_SPEED * Math.max(aL, aR);
    const hi = GLIDE_MAX_SPEED * Math.min(aL, aR);
    const base = aL + aR > 0 ? (2 * aL * aR) / (aL + aR) : 0;
    tangent.push(lo <= hi ? clamp(base, lo, hi) : hi);
    window.push(GLIDE_KNOT_WINDOW * Math.min(span(j - 1), span(j)));
  }
  // The polyline: [t, v, hard].
  /** @type {[number, number, boolean][]} */
  const points = [[knots[0][0], knots[0][1], true]];
  for (let j = 0; j + 1 < n; j += 1) {
    const lo = GLIDE_MIN_SPEED * avg[j];
    const hi = GLIDE_MAX_SPEED * avg[j];
    const a = [knots[j][0] + window[j], knots[j][1] + tangent[j] * window[j]];
    const b = [knots[j + 1][0] - window[j + 1], knots[j + 1][1] - tangent[j + 1] * window[j + 1]];
    points.push([a[0], a[1], false]);
    let prev = a;
    for (const [t, v] of between[j] ?? []) {
      if (!(t > prev[0]) || !(t < b[0]) || !Number.isFinite(v)) continue;
      const low = Math.max(prev[1] + lo * (t - prev[0]), b[1] - hi * (b[0] - t));
      const high = Math.min(prev[1] + hi * (t - prev[0]), b[1] - lo * (b[0] - t));
      const value = low <= high ? clamp(v, low, high) : (low + high) / 2;
      prev = [t, value];
      points.push([t, value, false]);
    }
    points.push([b[0], b[1], false]);
    points.push([knots[j + 1][0], knots[j + 1][1], true]);
  }
  const slope = (/** @type {number} */ k) => (points[k + 1][1] - points[k][1]) / (points[k + 1][0] - points[k][0]);
  /** @type {GlidePiece[]} */
  const pieces = [];
  let cursor = [points[0][0], points[0][1]];
  for (let k = 1; k < points.length; k += 1) {
    const [t, v, isHard] = points[k];
    const sIn = slope(k - 1);
    const sOut = k + 1 < points.length ? slope(k) : 0;
    let r = 0;
    if (!isHard && k + 1 < points.length && Math.abs(sOut - sIn) > 1e-12) {
      r = Math.min(t - points[k - 1][0], points[k + 1][0] - t) / 2;
    }
    if (t - r > cursor[0]) pieces.push([cursor[0], cursor[1], sIn, 0]);
    if (r > 0) {
      pieces.push([t - r, v - sIn * r, sIn, (sOut - sIn) / (4 * r)]);
      cursor = [t + r, v + sOut * r];
    } else {
      cursor = [t, v];
    }
  }
  pieces.push([cursor[0], cursor[1], 0, 0]);
  return pieces;
}

/** Index of the piece in effect at `time` (-1 before the first). @param {GlidePiece[]} pieces @param {number} time */
function pieceIndex(pieces, time) {
  let lo = 0;
  let hi = pieces.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (pieces[mid][0] <= time) lo = mid + 1; else hi = mid;
  }
  return lo - 1;
}

/**
 * The glide at lyrics time `time` as `[value, speed, halfAcceleration]`: the value at `time + u` is
 * `value + speed u + halfAcceleration u^2` up to the next piece. `[0, 0, 0]` for an empty glide.
 * @param {GlidePiece[]} pieces @param {number} time @returns {[number, number, number]}
 */
export function glideAt(pieces, time) {
  if (pieces.length === 0) return [0, 0, 0];
  const i = pieceIndex(pieces, time);
  if (i < 0) return [pieces[0][1], 0, 0];
  const [t, v, s, c] = pieces[i];
  const u = time - t;
  return [v + u * (s + c * u), s + 2 * c * u, c];
}

/** The glide's value at `time`. @param {GlidePiece[]} pieces @param {number} time */
export function glideValue(pieces, time) {
  return glideAt(pieces, time)[0];
}

/** The next lyrics time after `time` at which the glide's piece changes, or null. @param {GlidePiece[]} pieces @param {number} time */
export function nextGlideBreak(pieces, time) {
  const i = pieceIndex(pieces, time) + 1;
  return i < pieces.length ? pieces[i][0] : null;
}

/**
 * The first lyrics time at which the glide reaches `value` (it is monotone): the first piece's time when it starts
 * there or beyond, null when it never gets there.
 * @param {GlidePiece[]} pieces @param {number} value
 */
export function glideTimeAt(pieces, value) {
  if (pieces.length === 0) return null;
  if (pieces[0][1] >= value) return pieces[0][0];
  for (let i = 0; i + 1 < pieces.length; i += 1) {
    const [t, v, s, c] = pieces[i];
    const h = pieces[i + 1][0] - t;
    const end = v + h * (s + c * h);
    if (end < value) continue;
    const d = value - v;
    let u;
    if (Math.abs(c) < 1e-15) u = s > 0 ? d / s : 0;
    else {
      const disc = Math.max(0, s * s + 4 * c * d);
      u = (2 * d) / (s + Math.sqrt(disc));
      if (!Number.isFinite(u)) u = 0;
    }
    return t + clamp(u, 0, h);
  }
  return pieces[pieces.length - 1][1] >= value ? pieces[pieces.length - 1][0] : null;
}

/**
 * The karaoke edge of one line (「無」 mode, Now Playing): the x (in the row's own px, or any word-width coordinate)
 * up to which the line is lit. Hard at the first word's start (its left edge) and the last word's end (its right edge,
 * the whole line lit); each word's start (left edge) and end (right edge) guide it in between.
 * @param {[number, number][]} spans `[left, right]` per word @param {[number, number][]} words `[start, end]` per word
 * @returns {GlidePiece[]}
 */
export function karaokeGlide(spans, words) {
  const n = Math.min(spans.length, words.length);
  if (n === 0) return [];
  const start = words[0][0];
  let end = words[n - 1][1];
  if (!(end > start)) end = start + 1;
  /** @type {[number, number][]} */
  const guides = [];
  for (let i = 0; i < n; i += 1) {
    if (i > 0) guides.push([words[i][0], spans[i][0]]);
    if (i < n - 1 && words[i][1] < words[i + 1][0]) guides.push([words[i][1], spans[i][1]]);
  }
  return smoothGlide([[start, spans[0][0]], [end, spans[n - 1][1]]], [guides], false);
}

/**
 * The continuous strip's word glide (strip px of the point on the lyric area's centre): one hard knot per line (its
 * start, its first glyph centre `cellStart + anchor`) and, for a word-timed line, its word starts (their centres) as
 * guides; the last line with words also ends on its last word. Rests before the first and after the last knot.
 * @param {{ startMs: number, cellStart: number, anchor?: number, wordStarts: number[], wordCentres: number[] }[]} lines
 * @returns {GlidePiece[]}
 */
export function wordStripGlide(lines) {
  /** @type {[number, number][]} */
  const hard = [];
  /** @type {[number, number][][]} */
  const guides = [];
  lines.forEach((line, index) => {
    const n = Math.min(line.wordStarts.length, line.wordCentres.length);
    /** @type {[number, number][]} */
    const mine = [];
    for (let i = 0; i < n; i += 1) {
      if (line.wordStarts[i] > line.startMs) mine.push([line.wordStarts[i], line.cellStart + Math.max(0, line.wordCentres[i])]);
    }
    hard.push([line.startMs, line.cellStart + Math.max(0, line.anchor ?? 0)]);
    if (index === lines.length - 1 && mine.length > 0) {
      const last = mine.pop();
      guides.push(mine);
      hard.push(/** @type {[number, number]} */ (last));
    } else {
      guides.push(mine);
    }
  });
  return smoothGlide(hard, guides, true);
}
