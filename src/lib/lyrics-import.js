// Helpers for the song details panel's 「導入使用中歌詞」 button (ADR 0013).
// Any lyrics the player is currently using for the track qualify, whatever the source
// (online provider, .lrc sidecar, manually chosen file, embedded tag).

/** @param {{ lines?: unknown[] } | null | undefined} lyrics */
export function hasLyricsInUse(lyrics) {
  return !!lyrics && Array.isArray(lyrics.lines) && lyrics.lines.length > 0;
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
 * @param {{ synced: boolean, lines: { startMs: number | null, text: string, translation?: string | null, romanization?: string | null }[] }} lyrics
 */
export function lyricsToText(lyrics) {
  const out = [];
  for (const line of lyrics.lines) {
    const stamp = lyrics.synced && line.startMs != null ? formatLrcTimestamp(line.startMs) : '';
    out.push(`${stamp}${line.text}`);
    if (line.translation) out.push(`${stamp}${line.translation}`);
    if (line.romanization) out.push(`${stamp}${line.romanization}`);
  }
  return out.join('\n');
}
