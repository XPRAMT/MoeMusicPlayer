import { tr } from './i18n.js';
/** @typedef {import('./ipc').LyricsCandidate} LyricsCandidate */

/**
 * @param {LyricsCandidate} candidate
 * @returns {{ formatLabel: string, previewNotice: string | null }}
 */
export function getCandidatePresentation(candidate) {
  const qrcUnavailable = candidate.provider === 'qqmusic'
    && candidate.hasWordSyncedLyrics
    && candidate.previewLines.length === 0;
  const formatLabel = candidate.hasWordSyncedLyrics
    ? tr('逐字歌詞')
    : candidate.hasSyncedLyrics
      ? tr('逐行歌詞')
      : tr('純歌詞');
  const auxiliaryLabels = [
    candidate.hasTranslation ? tr('譯') : null,
    candidate.hasRomanization ? tr('羅') : null,
  ].filter(Boolean);

  return {
    formatLabel: [formatLabel, ...auxiliaryLabels].join(' · '),
    previewNotice: qrcUnavailable
      ? tr('原始 QRC 尚未解碼，目前無法預覽；仍可使用「選擇」保存。')
      : null,
  };
}

/**
 * The text that should filter the candidates on screen. What was typed filters the current results, but once it has
 * been submitted as a search the results are the answer to exactly that text: filtering them by it again would hide
 * every candidate whose title merely spells it differently ("1,2, Fanclub", "12fanclub", the kana title) and report
 * "no match" until a later batch happened to contain the literal text.
 * @param {string} typed  what is in the search box
 * @param {string} submitted  the text of the search that produced the candidates ('' when none was typed)
 * @returns {string}
 */
export function effectiveCandidateFilter(typed, submitted) {
  const text = String(typed ?? '').trim();
  return text === String(submitted ?? '').trim() ? '' : text;
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
