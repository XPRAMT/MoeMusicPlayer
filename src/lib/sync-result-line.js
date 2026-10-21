import { tr } from './i18n.js';

/**
 * One source's result line in the sync banner: "同步完成；新增 n 個項目，移除 n 個項目。", with ", n 個項目錯誤" only when
 * there are errors. Without the counts (they come with the finished event) only the outcome shows.
 * @param {string} outcome @param {number | null} added @param {number | null} removed @param {number} errorCount
 */
export function formatSyncResultLine(outcome, added, removed, errorCount) {
  if (added === null || removed === null) {
    return errorCount > 0 ? tr('{0}；{1}個項目錯誤。', [outcome, errorCount.toLocaleString()]) : tr('{0}。', [outcome]);
  }
  const counts = [added.toLocaleString(), removed.toLocaleString()];
  return errorCount > 0
    ? tr('{0}；新增{1}個項目，移除{2}個項目，{3}個項目錯誤。', [outcome, ...counts, errorCount.toLocaleString()])
    : tr('{0}；新增{1}個項目，移除{2}個項目。', [outcome, ...counts]);
}
