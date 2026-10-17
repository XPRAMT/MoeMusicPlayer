import { tr } from './i18n.js';
/**
 * @typedef {{
 *   mode: 'windowsBuiltin' | 'highQuality',
 *   sourceRateHz: number | null,
 *   outputRateHz: number,
 *   deviceRateHz: number,
 *   conversion: 'none' | 'highQuality' | 'windows' | 'basic',
 *   fallbackReason: string | null,
 *   dseeHx: 'off' | 'active' | 'bypassed' | 'unavailable',
 *   dseeNotice: string | null,
 *   dseeOutputRateHz: number | null,
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
  if (!format) return { text: tr('音訊輸出尚未開啟。'), notice: null };
  const output = formatSampleRate(format.outputRateHz);
  const device = formatSampleRate(format.deviceRateHz);
  if (format.sourceRateHz == null) {
    return { text: tr("尚未載入曲目（輸出 {0}）", [output]), notice: null };
  }
  const source = formatSampleRate(format.sourceRateHz);
  if (format.dseeHx === 'active') {
    const enhancedRate = format.dseeOutputRateHz ?? 96_000;
    const enhanced = `${source} → ${formatSampleRate(enhancedRate)}（DSEE HX）`;
    if (format.conversion === 'windows') {
      return { text: tr("{0}，再交由 Windows 轉換為 {1}", [enhanced, device]), notice: format.dseeNotice };
    }
    if ((format.conversion === 'highQuality' || format.conversion === 'basic') && format.outputRateHz !== enhancedRate) {
      return { text: tr("{0} → {1}（高品質）", [enhanced, output]), notice: format.dseeNotice };
    }
    return { text: enhanced, notice: format.dseeNotice };
  }
  const status = describeSampleRatePath(format, source, output, device);
  if (format.dseeHx === 'bypassed' || format.dseeHx === 'unavailable') {
    return { text: status.text, notice: format.dseeNotice ?? status.notice };
  }
  return status;
}

/**
 * @param {OutputFormat} format
 * @param {string} source
 * @param {string} output
 * @param {string} device
 * @returns {OutputFormatStatus}
 */
function describeSampleRatePath(format, source, output, device) {
  switch (format.conversion) {
    case 'highQuality': {
      const notice = format.mode === 'windowsBuiltin'
        ? format.fallbackReason
          ? tr("Windows 無法以 {0} 開啟共享模式輸出，已改用 {1} 並以高品質轉換。", [source, output])
          : tr("輸出裝置未以 {0} 開啟，暫以高品質轉換。", [source])
        : null;
      return { text: tr("{0} → {1}（高品質）", [source, output]), notice };
    }
    case 'windows':
      return { text: tr("{0}（交由 Windows 轉換為 {1}）", [source, device]), notice: null };
    case 'basic':
      return {
        text: tr("{0} → {1}（基本轉換）", [source, output]),
        notice: tr('高品質轉換器無法處理此格式，已改用基本轉換。'),
      };
    default:
      return { text: tr("原生 {0}（未轉換）", [source]), notice: null };
  }
}
