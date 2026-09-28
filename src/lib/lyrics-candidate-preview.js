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
