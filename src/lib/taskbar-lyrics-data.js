// What the app shell sends the taskbar-lyrics window (ADR 0018): the lyrics of the current track exactly as the Now
// Playing page shows them (after the Simplified-to-Traditional conversion), reduced to what drawing one line needs.
// The window keeps no lyric state of its own, so it cannot disagree with the page.

/** @typedef {{ startMs: number, text: string, translation: string | null, romanization: null }} TaskbarLyricLine */
/** @typedef {{ trackId: string | null, offsetMs: number, lines: TaskbarLyricLine[] }} TaskbarLyricsData */

/**
 * Only synchronized lyrics have lines to show; plain lyrics, no lyrics and a track still loading all give no lines.
 * @param {string | null | undefined} trackId
 * @param {{ trackId?: string, synced?: boolean, offsetMs?: number, lines?: Array<{ startMs: number | null, text: string, translation: string | null }> } | null | undefined} lyrics
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
      .map((line) => ({ startMs: /** @type {number} */ (line.startMs), text: line.text, translation: line.translation ?? null, romanization: null })),
  };
}
