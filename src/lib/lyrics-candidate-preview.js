/** @typedef {import('./ipc').LyricsCandidate} LyricsCandidate */

/**
 * @param {LyricsCandidate} candidate
 * @returns {{ formatLabel: string, previewNotice: string | null }}
 */
export function getCandidatePresentation(candidate) {
  const qrcUnavailable = candidate.provider === 'qqmusic'
    && !candidate.hasSyncedLyrics
    && candidate.previewLines.length === 0;

  return {
    formatLabel: qrcUnavailable
      ? 'QRC 尚未解碼'
      : candidate.hasSyncedLyrics
        ? '同步歌詞'
        : '純歌詞',
    previewNotice: qrcUnavailable
      ? '原始 QRC 尚未解碼，目前無法預覽；仍可使用「使用這份」保存。'
      : null,
  };
}

/**
 * Filter provider candidates by a free-text query (title/artist/album/preview/reasons).
 * Empty query returns the original list order.
 * @param {LyricsCandidate[]} candidates
 * @param {string} query
 * @returns {LyricsCandidate[]}
 */
export function filterLyricsCandidates(candidates, query) {
  const needle = String(query ?? '').trim().toLowerCase();
  if (!needle) return Array.isArray(candidates) ? candidates : [];
  const list = Array.isArray(candidates) ? candidates : [];
  return list.filter((candidate) => {
    const haystack = [
      candidate.title,
      candidate.artist,
      candidate.album,
      ...(Array.isArray(candidate.previewLines) ? candidate.previewLines : []),
      ...(Array.isArray(candidate.reasons) ? candidate.reasons : []),
      candidate.provider,
    ]
      .filter((part) => part != null && String(part).trim() !== '')
      .join(' ')
      .toLowerCase();
    return haystack.includes(needle);
  });
}
