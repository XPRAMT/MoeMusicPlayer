/**
 * Word highlight on the Now Playing current line (ADR 0041 D10), karaoke style: ONE accent edge per line moves along
 * the smoothed karaoke glide (`karaokeGlide` in lyrics-glide.js, the same curve as the Windows and Android 「無」
 * mode): at the first word's start the edge is at the first word's left, at the last word's end the whole line is
 * lit; in between the words guide it with its speed kept within 0.5x-1.5x of the line's average. Sung text stays
 * accent until the line changes (fill mode `forwards`); unsung text keeps the line's own colour.
 *
 * The line's text is one inline span whose background (accent up to `--lyric-line-fill`, the line colour after it)
 * is clipped to the glyphs. Its fragments on wrapped rows share one background strip (`box-decoration-break: slice`),
 * so the edge runs across the rows in reading order, and the edge's coordinate is the share of the line's total
 * width. One CSS animation drives it, with the curve as its `linear()` timing function; no style is written per frame:
 * the timing is set again only when the line, a seek, play/pause or the offset changes (and the word widths are
 * measured once then). Word timing is not led: the 200 ms cue lead (ADR 0029) applies to line changes only.
 * Reduced motion: no sweep; each word turns wholly accent when it starts (steps in the timing function).
 */

import { glideAt, karaokeGlide } from './lyrics-glide.js';

/** The `@keyframes` name used by LyricsView.svelte. */
export const LYRIC_LINE_ANIMATION = 'lyric-line-wipe';

/** Quadratic glide pieces are sampled this many times inside for the piecewise-linear `linear()` easing. */
export const KARAOKE_PIECE_SAMPLES = 4;

/**
 * The texts of the current line's words (Traditional when chosen).
 * @param {{ t: string }[]} words @param {string[] | null | undefined} texts same length, or none
 * @returns {string[]}
 */
export function wordTexts(words, texts) {
  return words.map((word, index) => (texts && texts.length === words.length ? texts[index] : word.t));
}

/** @param {number} x */
const round = (x) => Math.round(x * 1e4) / 1e4;

/**
 * The single-edge animation of a word-timed line.
 * @param {{ s: number, e: number }[]} words word timing in lyric time (ms)
 * @param {number[]} widths each word's drawn width (px, any unit: only the shares count)
 * @param {number} positionMs the playback position now
 * @param {number} offsetMs the display offset (a line shows at `start - offset`)
 * @param {boolean} [reduced] prefers-reduced-motion: steps at the word starts instead of the sweep
 * @returns {{ delayMs: number, durationMs: number, easing: string } | null} null when there is nothing to light
 */
export function karaokeLineAnimation(words, widths, positionMs, offsetMs, reduced = false) {
  const n = Math.min(words.length, widths.length);
  const total = widths.slice(0, n).reduce((sum, w) => sum + Math.max(0, w || 0), 0);
  if (n === 0 || !(total > 0)) return null;
  /** @type {[number, number][]} */
  const spans = [];
  let x = 0;
  for (let i = 0; i < n; i += 1) {
    const w = Math.max(0, widths[i] || 0) / total;
    spans.push([x, x + w]);
    x += w;
  }
  /** @type {[number, number][]} */
  const times = words.slice(0, n).map((w) => [w.s - offsetMs, w.e - offsetMs]);
  const glide = karaokeGlide(spans, times);
  const start = glide[0][0];
  const end = glide[glide.length - 1][0];
  const duration = Math.max(1, end - start);
  /** @type {[number, number][]} `[output 0..1, input 0..1]` */
  const points = [];
  if (reduced) {
    let lit = spans[0][0];
    points.push([lit, 0]);
    for (let i = 0; i < n; i += 1) {
      const at = Math.min(1, Math.max(0, (times[i][0] - start) / duration));
      points.push([lit, at]);
      lit = Math.max(lit, spans[i][1]);
      points.push([lit, at]);
    }
    points.push([1, 1]);
  } else {
    for (let i = 0; i + 1 < glide.length; i += 1) {
      const [t, , , c] = glide[i];
      const next = glide[i + 1][0];
      const steps = c !== 0 ? KARAOKE_PIECE_SAMPLES : 1;
      for (let k = 0; k < steps; k += 1) {
        const time = t + ((next - t) * k) / steps;
        points.push([glideAt(glide, time)[0], (time - start) / duration]);
      }
    }
    points.push([1, 1]);
  }
  const easing = `linear(${points.map(([v, p]) => `${round(v)} ${round(p * 100)}%`).join(', ')})`;
  return { delayMs: Math.round(start - positionMs), durationMs: Math.round(duration), easing };
}

/**
 * The index of the word sung at `positionMs`, or -1 (before, after, or in a gap between words).
 * Zero-length words are never current.
 *
 * @param {{ s: number, e: number }[]} words
 * @param {number} positionMs
 * @param {number} offsetMs
 */
export function currentWordIndex(words, positionMs, offsetMs) {
  const t = positionMs + offsetMs;
  for (let index = words.length - 1; index >= 0; index -= 1) {
    const word = words[index];
    if (word.e > word.s && t >= word.s && t < word.e) return index;
  }
  return -1;
}
