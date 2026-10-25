// What the app shell sends the taskbar-lyrics window (ADR 0018): the lyrics of the current track exactly as the Now
// Playing page shows them (after the Simplified-to-Traditional conversion), reduced to what drawing one line needs.
// The window keeps no lyric state of its own, so it cannot disagree with the page.

/** @typedef {{ s: number, e: number, t: string }} TaskbarLyricWord */
/** @typedef {{ startMs: number, endMs?: number, text: string, translation: string | null, romanization: null, words?: TaskbarLyricWord[] }} TaskbarLyricLine */
/** @typedef {{ trackId: string | null, offsetMs: number, lines: TaskbarLyricLine[] }} TaskbarLyricsData */

/**
 * Only synchronized lyrics have lines to show; plain lyrics, no lyrics and a track still loading all give no lines.
 * @param {string | null | undefined} trackId
 * @param {{ trackId?: string, synced?: boolean, offsetMs?: number, lines?: Array<{ startMs: number | null, endMs?: number | null, text: string, translation: string | null, words?: TaskbarLyricWord[] | null }> } | null | undefined} lyrics
 * @returns {TaskbarLyricsData}
 */
export function buildTaskbarLyricsData(trackId, lyrics) {
  const id = trackId ?? null;
  if (!lyrics || !lyrics.synced || (lyrics.trackId !== undefined && lyrics.trackId !== id)) {
    return { trackId: id, offsetMs: 0, lines: [] };
  }
  return {
    trackId: id,
    offsetMs: Number.isFinite(lyrics.offsetMs) ? /** @type {number} */ (lyrics.offsetMs) : 0,
    lines: (lyrics.lines ?? [])
      .filter((line) => typeof line.startMs === 'number' && Number.isFinite(line.startMs))
      .map((line) => {
        /** @type {TaskbarLyricLine} */
        const out = { startMs: /** @type {number} */ (line.startMs), text: line.text, translation: line.translation ?? null, romanization: null };
        // The line's own end (SRT/ASS cue, yrc/QRC line, enhanced LRC closing stamp), in the same time as `startMs`:
        // the continuous strip ends a line without word times there instead of at its duration cap (ADR 0041 D10).
        if (typeof line.endMs === 'number' && Number.isFinite(line.endMs)) out.endMs = line.endMs;
        // Word timing (ADR 0041) only when there is some: the native windows glide by words and light the sung word.
        if (Array.isArray(line.words) && line.words.length > 0) out.words = line.words.map((word) => ({ s: word.s, e: word.e, t: word.t }));
        return out;
      }),
  };
}
