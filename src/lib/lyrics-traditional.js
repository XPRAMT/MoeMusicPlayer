/**
 * Simplified-to-Traditional lyric display (ADR 0030). The backend (crates/player-zh, OpenCC
 * 1.4.2 dictionaries) sends each line's Taiwan-Traditional spelling as `textTraditional` /
 * `translationTraditional` when it differs, so picking it is synchronous: no extra request, no
 * flash of the Simplified text, and the converted lines always belong to the lyrics they came with.
 * Romanization is never converted. The result is a display copy; callers must not store it.
 */

/**
 * @param {import('./ipc').TrackLyrics} lyrics
 * @param {boolean} traditional the Simplified-to-Traditional preference
 * @returns {import('./ipc').TrackLyrics}
 */
export function lyricsForDisplay(lyrics, traditional) {
  if (
    !traditional
    || !lyrics.lines.some(
      (line) => line.textTraditional != null || line.translationTraditional != null || line.wordsTraditional != null,
    )
  ) {
    return lyrics;
  }
  return {
    ...lyrics,
    lines: lyrics.lines.map((line) => ({
      ...line,
      text: line.textTraditional ?? line.text,
      translation: line.translationTraditional ?? line.translation,
      words:
        line.words && line.wordsTraditional?.length === line.words.length
          ? line.words.map((word, index) => ({ ...word, t: line.wordsTraditional?.[index] ?? word.t }))
          : line.words,
    })),
  };
}
