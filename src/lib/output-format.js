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
 *   exclusive?: { bits: number, channels: number, sourceChannels: number, bitPerfect: boolean, volumeApplied: boolean, underruns: number } | null,
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

/**
 * The sample-rate path of a synced (non-main) device: the track's path up to the rate the stream carries, then the
 * rate conversion for this device when its rate differs.
 * @param {OutputFormat | null | undefined} format  the main stream's format (the track's path)
 * @param {number | null} deviceRateHz  the rate this device runs at
 * @param {boolean} converted  the device's rate differs from the stream's
 * @returns {string}
 */
export function describeMirrorPath(format, deviceRateHz, converted) {
  const device = deviceRateHz == null ? null : formatSampleRate(deviceRateHz);
  if (!format || format.sourceRateHz == null) {
    return device ? tr("尚未載入曲目（輸出 {0}）", [device]) : tr('尚未載入曲目');
  }
  const steps = [formatSampleRate(format.sourceRateHz)];
  if (format.dseeHx === 'active') {
    steps.push(`${formatSampleRate(format.dseeOutputRateHz ?? 96_000)}（DSEE HX）`);
  }
  const carried = format.dseeHx === 'active' ? (format.dseeOutputRateHz ?? 96_000) : format.sourceRateHz;
  if ((format.conversion === 'highQuality' || format.conversion === 'basic') && format.outputRateHz !== carried) {
    steps.push(`${formatSampleRate(format.outputRateHz)}（${tr('高品質')}）`);
  }
  if (converted && device) steps.push(`${device}（${tr('高品質')}）`);
  if (steps.length === 1) return tr("原生 {0}（未轉換）", [steps[0]]);
  return steps.join(' → ');
}

/**
 * Describe the exclusive-mode stream (one device, the samples as the file has them when nothing had to change).
 * @param {OutputFormat | null | undefined} format
 * @returns {OutputFormatStatus | null}  null when the output is not in exclusive mode
 */
export function describeExclusive(format) {
  const exclusive = format?.exclusive;
  if (!format || !exclusive) return null;
  const rate = formatSampleRate(format.outputRateHz);
  const head = tr("獨佔．{0}-bit {1}", [exclusive.bits, rate]);
  if (exclusive.bitPerfect) return { text: `${head}．${tr('位元完美')}`, notice: null };
  /** @type {string[]} */
  const reasons = [];
  if (format.sourceRateHz != null && format.sourceRateHz !== format.outputRateHz) {
    reasons.push(tr("裝置不接受 {0}，已轉換為 {1}", [formatSampleRate(format.sourceRateHz), rate]));
  }
  if (exclusive.sourceChannels !== exclusive.channels) {
    reasons.push(tr("裝置不接受 {0} 聲道，已混成 {1} 聲道", [exclusive.sourceChannels, exclusive.channels]));
  }
  if (exclusive.volumeApplied) {
    reasons.push(tr('播放器音量低於 100%，已套用軟體音量；調到 100% 才會原樣送出'));
  }
  return { text: head, notice: reasons.length ? `${reasons.join('；')}（${tr('不是位元完美')}）` : null };
}
