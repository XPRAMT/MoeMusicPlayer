// Helpers for the song details panel's 「導入使用中歌詞」 button (ADR 0013).
// Any lyrics the player is currently using for the track qualify, whatever the source
// (online provider, .lrc sidecar, manually chosen file, embedded tag).

/** @param {{ lines?: unknown[] } | null | undefined} lyrics */
export function hasLyricsInUse(lyrics) {
  return !!lyrics && Array.isArray(lyrics.lines) && lyrics.lines.length > 0;
}

import { lyricsDisplayOffsetMs } from './lyrics-timing-offset.js';

/**
 * How far to move every timestamp when importing, so the written LRC plays exactly as the lyrics are shown now:
 * a line is shown once `position + lyricsDisplayOffsetMs(...) >= startMs`, i.e. at `startMs - offset`.
 * @param {number | null | undefined} lyricsOffsetMs the lyrics' own offsetMs @param {number} delaySec the per-track delay
 * @returns {number}
 */
export function importTimestampShiftMs(lyricsOffsetMs, delaySec) {
  const shift = -lyricsDisplayOffsetMs(lyricsOffsetMs, delaySec);
  return Object.is(shift, -0) ? 0 : shift;
}

/**
 * Whether the per-track lyrics delay has been baked into the lyrics field during this dialog session, and whether a
 * Save resets the delay to 0 so it is not applied twice (ADR 0013). An import with a non-zero delay marks it pending;
 * 清空, or an import with delay 0, cancels it; the next Save that writes the lyrics field (lyrics dirty) resets the
 * delay, even when the imported lyrics were edited before saving. Every Save clears the pending state.
 * @typedef {{ type: 'import', delaySec: number } | { type: 'clear' } | { type: 'save', lyricsDirty: boolean }} BakedDelayEvent
 * @param {boolean} pending @param {BakedDelayEvent} event
 * @returns {{ pending: boolean, resetDelay: boolean }}
 */
export function nextBakedDelayState(pending, event) {
  switch (event.type) {
    case 'import': return { pending: event.delaySec !== 0, resetDelay: false };
    case 'clear': return { pending: false, resetDelay: false };
    case 'save': return { pending: false, resetDelay: pending && event.lyricsDirty };
    default: return { pending, resetDelay: false };
  }
}

/** @param {number} ms */
export function formatLrcTimestamp(ms) {
  const total = Math.max(0, Math.round(ms / 10));
  const centis = total % 100;
  const seconds = Math.floor(total / 100) % 60;
  const minutes = Math.floor(total / 6000);
  const pad = (/** @type {number} */ value) => String(value).padStart(2, '0');
  return `[${pad(minutes)}:${pad(seconds)}.${pad(centis)}]`;
}

/**
 * Serialize parsed lyrics to text. Synced lines become LRC (offsets are already applied
 * by the backend parser). Translation and romanization, when present, follow the original
 * line as extra lines with the same timestamp (the bilingual LRC convention the parser reads).
 * `shiftMs` (see importTimestampShiftMs) is added to every timestamp; one that would go below 0 is written as 0.
 * Lines with word timing (ADR 0041) are written as enhanced LRC (A2 extension): the line stamp, then
 * `<mm:ss.xx>word` per word and a closing `<mm:ss.xx>` at the last word's end (also after a word followed by
 * a pause), with the same shift. Word texts are written as given, so the line text is their concatenation.
 * A line without word timing that has its own end (`endMs`, after its start) is written as one word,
 * `[start]<start>text<end>`, so the end survives; other line-only lyrics are written exactly as before.
 * @param {{ synced: boolean, lines: { startMs: number | null, endMs?: number | null, text: string, translation?: string | null, romanization?: string | null, words?: { s: number, e: number, t: string }[], romanizationWords?: { s: number, e: number, t: string }[] }[] }} lyrics
 * @param {number} [shiftMs]
 */
export function lyricsToText(lyrics, shiftMs = 0) {
  const out = [];
  const shift = Number.isFinite(shiftMs) ? shiftMs : 0;
  for (const line of lyrics.lines) {
    const stamp = lyrics.synced && line.startMs != null ? formatLrcTimestamp(Math.max(0, line.startMs + shift)) : '';
    const words = lyrics.synced && stamp && Array.isArray(line.words) ? line.words.filter((word) => word.t) : [];
    // A line with its own end but no word times (SRT/ASS cue, yrc/QRC line) keeps the end as `<start>text<end>`.
    const ownEnd = lyrics.synced && stamp && words.length === 0 && line.text && line.startMs != null
      && typeof line.endMs === 'number' && Number.isFinite(line.endMs) && line.endMs > line.startMs;
    const main = words.length > 0
      ? enhancedWords(words, shift)
      : ownEnd ? enhancedWords([{ s: /** @type {number} */ (line.startMs), e: /** @type {number} */ (line.endMs), t: line.text }], shift) : line.text;
    out.push(`${stamp}${main}`);
    if (line.translation) out.push(`${stamp}${line.translation}`);
    // Translations never get word stamps (they do not line up word for word); romanization does when it has word timing.
    const romanizationWords = lyrics.synced && stamp && Array.isArray(line.romanizationWords) ? line.romanizationWords.filter((word) => word.t) : [];
    if (line.romanization) out.push(`${stamp}${romanizationWords.length > 0 ? enhancedWords(romanizationWords, shift) : line.romanization}`);
  }
  return out.join('\n');
}

/** @param {{ s: number, e: number, t: string }[]} words @param {number} shift */
function enhancedWords(words, shift) {
  const stamp = (/** @type {number} */ ms) => `<${formatLrcTimestamp(Math.max(0, ms + shift)).slice(1, -1)}>`;
  let out = '';
  words.forEach((current, index) => {
    out += `${stamp(current.s)}${current.t}`;
    const next = words[index + 1];
    // Enhanced LRC words end at the next stamp; write the end only where it differs (a pause, or the last word).
    if (!next || Math.round((next.s - current.e) / 10) > 0) out += stamp(current.e);
  });
  return out;
}
