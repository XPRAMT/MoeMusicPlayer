/**
 * Word highlight on the Now Playing current line (ADR 0041), karaoke style. The word being sung
 * fills with the theme accent (`--accent`) from left to right with a soft edge; words already sung
 * stay accent until the line ends; words not yet sung keep the line's own colour (reduced motion:
 * a word turns wholly accent when it starts and stays so). Each word span gets one CSS animation
 * whose delay is the time until the word starts (negative once it has started) and whose duration
 * is the word's length; fill mode `forwards` holds the full fill after the word ends, so words
 * already past when the line is rebuilt are lit from the start. No style
 * is written per frame: the timings are set again only when the line, a seek, play/pause or the
 * offset changes. Word fills are not led: the 200 ms cue lead (ADR 0029) applies to line changes
 * only, so a word lights exactly when it is sung.
 */

/** The `@keyframes` name used by LyricsView.svelte. */
export const LYRIC_WORD_ANIMATION = 'lyric-word-wipe';

/** The `@keyframes` used instead under `prefers-reduced-motion`. */
export const LYRIC_WORD_REDUCED_ANIMATION = 'lyric-word-sing';

/**
 * Animation timings for the words of the current line.
 *
 * @param {{ s: number, e: number, t: string }[]} words word timing in lyric time (ms)
 * @param {string[] | null | undefined} texts the texts to draw (Traditional when chosen), same length
 * @param {number} positionMs the playback position now
 * @param {number} offsetMs the display offset (a line shows at `start - offset`)
 * @returns {{ text: string, delayMs: number, durationMs: number }[]}
 */
export function wordHighlightTimings(words, texts, positionMs, offsetMs) {
  return words.map((word, index) => {
    const start = word.s - offsetMs;
    return {
      text: texts && texts.length === words.length ? texts[index] : word.t,
      delayMs: Math.round(start - positionMs),
      durationMs: Math.max(0, Math.round(word.e - word.s)),
    };
  });
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
