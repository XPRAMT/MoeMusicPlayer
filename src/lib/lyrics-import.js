// Helpers for the song details panel's 「導入」 button (ADR 0013).
// Only lyrics coming from an online provider qualify; `provider` survives manual selection.

const ONLINE_PROVIDERS = new Set(['netease', 'qqmusic']);

/** @param {{ provider?: string, source?: string, lines?: unknown[] } | null | undefined} lyrics */
export function isOnlineLyrics(lyrics) {
  if (!lyrics || !Array.isArray(lyrics.lines) || lyrics.lines.length === 0) return false;
  const provider = lyrics.provider ?? lyrics.source;
  return ONLINE_PROVIDERS.has(provider ?? '');
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
 * by the backend parser); a translation, when present, is emitted as a second line with
 * the same timestamp (the common NetEase-style bilingual LRC convention).
 * @param {{ synced: boolean, lines: { startMs: number | null, text: string, translation?: string | null }[] }} lyrics
 */
export function lyricsToText(lyrics) {
  const out = [];
  for (const line of lyrics.lines) {
    const stamp = lyrics.synced && line.startMs != null ? formatLrcTimestamp(line.startMs) : '';
    out.push(`${stamp}${line.text}`);
    if (line.translation) out.push(`${stamp}${line.translation}`);
  }
  return out.join('\n');
}
