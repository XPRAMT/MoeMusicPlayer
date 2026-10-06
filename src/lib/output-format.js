/**
 * @typedef {{
 *   mode: 'windowsBuiltin' | 'highQuality',
 *   sourceRateHz: number | null,
 *   outputRateHz: number,
 *   deviceRateHz: number,
 *   conversion: 'none' | 'highQuality' | 'windows' | 'basic',
 *   fallbackReason: string | null,
 * }} OutputFormat
 */
/** @typedef {{ text: string, notice: string | null }} OutputFormatStatus */

/**
 * 44100 -> "44.1 kHz", 96000 -> "96 kHz", 22050 -> "22.05 kHz".
 * @param {number} hz
 * @returns {string}
 */
export function formatSampleRate(hz) {
  return `${Number((hz / 1000).toFixed(3))} kHz`;
}

/**
 * Describe the Windows output's sample-rate path for the playback settings.
 * @param {OutputFormat | null | undefined} format
 * @returns {OutputFormatStatus}
 */
export function describeOutputFormat(format) {
  if (!format) return { text: '音訊輸出尚未開啟。', notice: null };
  const output = formatSampleRate(format.outputRateHz);
  const device = formatSampleRate(format.deviceRateHz);
  if (format.sourceRateHz == null) {
    return { text: `尚未載入曲目（輸出 ${output}）`, notice: null };
  }
  const source = formatSampleRate(format.sourceRateHz);
  switch (format.conversion) {
    case 'highQuality': {
      const notice = format.mode === 'windowsBuiltin'
        ? format.fallbackReason
          ? `Windows 無法以 ${source} 開啟共享模式輸出，已改用 ${output} 並以高品質轉換。`
          : `輸出裝置未以 ${source} 開啟，暫以高品質轉換。`
        : null;
      return { text: `${source} → ${output}（高品質）`, notice };
    }
    case 'windows':
      return { text: `${source}（交由 Windows 轉換為 ${device}）`, notice: null };
    case 'basic':
      return {
        text: `${source} → ${output}（基本轉換）`,
        notice: '高品質轉換器無法處理此格式，已改用基本轉換。',
      };
    default:
      return { text: `原生 ${source}（未轉換）`, notice: null };
  }
}
