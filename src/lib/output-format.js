import { tr } from './i18n.js';
import { formatDsdName } from './dsd-settings.js';
// The output path of a playback device, one line in the same format on Windows and Android (ADR 0020, ADR 0044):
// the file's own rate, then each step the samples go through as 「取樣率（步驟）」 joined by 「 → 」, then 「．N 聲道」.
// Steps a platform does not have are simply not there. Both device lists build every path and idle line here.
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
 *   exclusive?: { bits: number, channels: number, sourceChannels: number, bitPerfect: boolean, volumeApplied: boolean, underruns: number, dopDsdRateHz?: number | null, dopFallback?: string | null, otherReasons?: string[] } | null,
 *   exclusiveFallback?: string | null,
 * }} OutputFormat
 */
/** @typedef {{ text: string, notice: string | null }} OutputFormatStatus */
/** @typedef {{ rateHz: number | null, name: string }} PathStep */

/** DSEE HX's output rate when the backend does not say. */
const DSEE_DEFAULT_RATE_HZ = 96_000;

/**
 * 44100 -> "44.1", 96000 -> "96" (no unit).
 * @param {number} hz
 */
function kilohertz(hz) {
  return String(Number((hz / 1000).toFixed(3)));
}

/**
 * 44100 -> "44.1 kHz", 96000 -> "96 kHz", 22050 -> "22.05 kHz".
 * @param {number} hz
 * @returns {string}
 */
export function formatSampleRate(hz) {
  return `${kilohertz(hz)} kHz`;
}

/**
 * The one path line: 「44.1 kHz → 48 kHz（FFT） → 48 kHz（系統）．2 聲道」. A step without a known rate is its name
 * alone (「（藍牙）」); `extras` follow the channels (「24bit」「位元完美」「硬體解碼」).
 * @param {string} first the file's own rate (or 「DSD64」), without brackets
 * @param {PathStep[]} steps
 * @param {number | null | undefined} channels
 * @param {(string | null | false | undefined)[]} [extras]
 */
export function formatOutputPath(first, steps, channels, extras = []) {
  const path = [first, ...steps.map((step) => (step.rateHz ? `${formatSampleRate(step.rateHz)}（${step.name}）` : `（${step.name}）`))]
    .join(' → ');
  return [path, channels ? tr('{0} 聲道', [channels]) : null, ...extras].filter(Boolean).join('．');
}

/**
 * What an idle device takes: every sample rate, then the channel counts, no bit depth: 「44.1, 48, 88.2, 96 kHz．2 聲道」.
 * Nothing known at all is 「格式不限」 (Android converts anything), or `unspecified` when the caller knows better; a
 * missing half says so.
 * @param {number[]} rates
 * @param {number[]} channels
 * @param {string} [unspecified] the line when nothing is known
 */
export function formatSupportedFormats(rates, channels, unspecified = tr('格式不限')) {
  const sortedRates = [...new Set(rates.filter((rate) => rate > 0))].sort((a, b) => a - b);
  const sortedChannels = [...new Set(channels.filter((count) => count > 0))].sort((a, b) => a - b);
  if (!sortedRates.length && !sortedChannels.length) return unspecified;
  return [
    sortedRates.length ? `${sortedRates.map(kilohertz).join(', ')} kHz` : tr('取樣率不限'),
    sortedChannels.length ? tr('{0} 聲道', [sortedChannels.join(', ')]) : tr('聲道數不限'),
  ].join('．');
}

/**
 * The player's own steps of a Windows track: DSEE HX, then the rate conversion to the output rate (rubato's FFT, or the
 * basic converter when FFT cannot take the format), and the rate the stream carries after them.
 * @param {OutputFormat} format with a track loaded
 * @returns {{ steps: PathStep[], carried: number }}
 */
function playerSteps(format) {
  /** @type {PathStep[]} */
  const steps = [];
  let carried = /** @type {number} */ (format.sourceRateHz);
  if (format.dseeHx === 'active') {
    carried = format.dseeOutputRateHz ?? DSEE_DEFAULT_RATE_HZ;
    steps.push({ rateHz: carried, name: 'DSEE HX' });
  }
  if ((format.conversion === 'highQuality' || format.conversion === 'basic') && format.outputRateHz !== carried) {
    steps.push({ rateHz: format.outputRateHz, name: format.conversion === 'basic' ? tr('基本轉換') : 'FFT' });
    carried = format.outputRateHz;
  }
  return { steps, carried };
}

/**
 * Why a Windows shared-mode track is converted the way it is, when that needs saying.
 * @param {OutputFormat} format
 * @returns {string | null}
 */
function sharedNotice(format) {
  if (format.dseeHx === 'active') return format.dseeNotice;
  const source = formatSampleRate(/** @type {number} */ (format.sourceRateHz));
  const output = formatSampleRate(format.outputRateHz);
  /** @type {string | null} */
  let notice = null;
  if (format.conversion === 'highQuality' && format.mode === 'windowsBuiltin') {
    notice = format.fallbackReason
      ? tr("Windows 無法以 {0} 開啟共享模式輸出，已改用 {1} 並以高品質轉換。", [source, output])
      : tr("輸出裝置未以 {0} 開啟，暫以高品質轉換。", [source]);
  } else if (format.conversion === 'basic') {
    notice = tr('高品質轉換器無法處理此格式，已改用基本轉換。');
  }
  if (format.dseeHx === 'bypassed' || format.dseeHx === 'unavailable') return format.dseeNotice ?? notice;
  return notice;
}

/**
 * The Windows main stream in shared mode: 「44.1 kHz → 48 kHz（FFT） → 48 kHz（系統）．2 聲道」. The 系統 rate is what
 * the Windows audio engine runs at, so a conversion done by Windows is that step alone.
 * @param {OutputFormat | null | undefined} format
 * @param {number | null} [channels] the device stream's channel count
 * @returns {OutputFormatStatus}
 */
export function describeOutputFormat(format, channels = null) {
  if (!format) return { text: tr('音訊輸出尚未開啟。'), notice: null };
  if (format.sourceRateHz == null) {
    return { text: tr("尚未載入曲目（輸出 {0}）", [formatSampleRate(format.outputRateHz)]), notice: null };
  }
  const { steps } = playerSteps(format);
  steps.push({ rateHz: format.deviceRateHz, name: tr('系統') });
  return { text: formatOutputPath(formatSampleRate(format.sourceRateHz), steps, channels), notice: sharedNotice(format) };
}

/**
 * A synced (non-main) Windows device: the track's steps, then this device's own FFT conversion when its rate differs
 * from the stream's, then its own 系統 rate.
 * @param {OutputFormat | null | undefined} format the main stream's format (the track's path)
 * @param {number | null} deviceRateHz the rate this device runs at
 * @param {boolean} converted the device's rate differs from the stream's
 * @param {number | null} [channels]
 * @returns {string}
 */
export function describeMirrorPath(format, deviceRateHz, converted, channels = null) {
  if (!format || format.sourceRateHz == null) {
    return deviceRateHz == null ? tr('尚未載入曲目') : tr("尚未載入曲目（輸出 {0}）", [formatSampleRate(deviceRateHz)]);
  }
  const { steps, carried } = playerSteps(format);
  const device = deviceRateHz ?? carried;
  if (converted && device !== carried) steps.push({ rateHz: device, name: 'FFT' });
  steps.push({ rateHz: device, name: tr('系統') });
  return formatOutputPath(formatSampleRate(format.sourceRateHz), steps, channels);
}

/**
 * The exclusive-mode stream: 「96 kHz → 96 kHz（獨佔）．2 聲道．24bit．位元完美」; what kept it from being bit-perfect
 * is the notice. DSD as DoP starts at the DSD rate: 「DSD64 → 176.4 kHz（DoP） → 176.4 kHz（獨佔）．2 聲道．24bit」.
 * @param {OutputFormat | null | undefined} format
 * @returns {OutputFormatStatus | null} null when the output is not in exclusive mode
 */
export function describeExclusive(format) {
  const exclusive = format?.exclusive;
  if (!format || !exclusive) return null;
  const bits = `${exclusive.bits}bit`;
  const exclusiveStep = { rateHz: format.outputRateHz, name: tr('獨佔') };
  if (exclusive.dopDsdRateHz) {
    // DSD as DoP: every bit of the file goes to the DAC; the player's gain and volume do not apply.
    const steps = [{ rateHz: format.outputRateHz, name: 'DoP' }, exclusiveStep];
    return { text: formatOutputPath(formatDsdName(exclusive.dopDsdRateHz), steps, exclusive.channels, [bits]), notice: null };
  }
  const resampled = format.sourceRateHz != null && format.sourceRateHz !== format.outputRateHz;
  const steps = resampled ? [{ rateHz: format.outputRateHz, name: 'FFT' }, exclusiveStep] : [exclusiveStep];
  const first = formatSampleRate(format.sourceRateHz ?? format.outputRateHz);
  const text = formatOutputPath(first, steps, exclusive.channels, [bits, exclusive.bitPerfect && tr('位元完美')]);
  const fallbackNote = exclusive.dopFallback ? tr("DoP 無法使用，已轉成 PCM：{0}", [exclusive.dopFallback]) : null;
  const withNote = (/** @type {string | null} */ notice) => [notice, fallbackNote].filter(Boolean).join('；') || null;
  if (exclusive.bitPerfect) return { text, notice: withNote(null) };
  /** @type {string[]} */
  const reasons = [];
  if (resampled) {
    reasons.push(tr("裝置不接受 {0}，已轉換為 {1}", [formatSampleRate(/** @type {number} */ (format.sourceRateHz)), formatSampleRate(format.outputRateHz)]));
  }
  if (exclusive.sourceChannels !== exclusive.channels) {
    reasons.push(tr("裝置不接受 {0} 聲道，已混成 {1} 聲道", [exclusive.sourceChannels, exclusive.channels]));
  }
  reasons.push(...(exclusive.otherReasons ?? []));
  if (exclusive.volumeApplied) {
    reasons.push(tr('播放器音量低於 100%，已套用軟體音量；調到 100% 才會原樣送出'));
  }
  return { text, notice: withNote(reasons.length ? `${reasons.join('；')}（${tr('不是位元完美')}）` : null) };
}

/**
 * An output that is in exclusive mode but plays the current track in shared mode, and why.
 * @param {OutputFormat | null | undefined} format
 * @param {number | null} [channels]
 * @returns {OutputFormatStatus | null} null when exclusive mode played the track itself
 */
export function describeExclusiveFallback(format, channels = null) {
  if (!format?.exclusiveFallback) return null;
  const shared = describeOutputFormat(format, channels);
  return {
    text: tr("已改用共享模式：{0}", [shared.text]),
    notice: tr("獨佔模式無法播放這首：{0}", [format.exclusiveFallback]),
  };
}

/**
 * The name of an Android Bluetooth codec step: 「LDAC 990 kbps」 (a fixed mode), 「LDAC 自適應」 (the adaptive mode),
 * 「LDAC 預設」 (the mode is not set: the stack's default), 「LDAC」 (the mode was not read: no root / Shizuku), 「AAC」.
 * @param {import('./ipc').AndroidBluetoothCodec} codec
 */
export function codecStepName(codec) {
  if (codec.ldacKbps) return `${codec.codec} ${codec.ldacKbps} kbps`;
  if (codec.ldacAdaptive) return tr('{0} 自適應', [codec.codec]);
  if (codec.ldacDefault) return tr('{0} 預設', [codec.codec]);
  return codec.codec;
}

/**
 * The Android playing row: the AudioTrack rate → the mixer rate（系統） → the Bluetooth codec rate（codec）．N 聲道, e.g.
 * 「96 kHz → 48 kHz（系統） → 96 kHz（LDAC）．2 聲道」.
 * - An unreadable mixer rate leaves the 系統 step out (never a guessed number).
 * - A Bluetooth route ends with the codec step, also at the same rate, because it names the codec; before the stack
 *   has reported the codec (or without the permission) the step is 「（藍牙）」 with no rate.
 * - Nothing here claims what the device receives bit for bit.
 * @param {import('./ipc').AndroidAudioTrackOutput | null | undefined} output
 * @param {{ bluetooth: boolean, codec: import('./ipc').AndroidBluetoothCodec | null }} [route]
 * @returns {string | null} null when no AudioTrack is open
 */
export function describeAndroidPath(output, route = { bluetooth: false, codec: null }) {
  if (!output) return null;
  /** @type {PathStep[]} */
  const steps = [];
  const mixer = output.mixerSampleRate;
  if (mixer && mixer > 0) steps.push({ rateHz: mixer, name: tr('系統') });
  if (route.bluetooth) {
    const codec = route.codec;
    steps.push(codec ? { rateHz: codec.sampleRate || null, name: codecStepName(codec) } : { rateHz: null, name: tr('藍牙') });
  }
  return formatOutputPath(formatSampleRate(output.sampleRate), steps, output.channelCount, [output.offload && tr('硬體解碼')]);
}

/**
 * Why the Android USB exclusive output is not bit-perfect, other than the volume (ADR 0047).
 * @param {import('./ipc').AndroidUsbExclusiveEngine['notBitPerfect'][number]} reason
 */
function androidExclusiveReason(reason) {
  switch (reason) {
    case 'float32': return tr('32 bit 來源經過浮點處理，無法逐位元原樣送出');
    case 'unknownBits': return tr('無法得知來源的位元深度');
    case 'dsd': return tr('DSD 已轉成 PCM');
    case 'truncated': return tr('USB 音訊裝置的格式位元數少於來源');
    default: return null;
  }
}

/**
 * Why a DSD track plays as PCM although native passthrough is on (ADR 0049), as the card's notice.
 * @param {string | null | undefined} reason
 * @returns {string | null}
 */
export function dsdFallbackNotice(reason) {
  return reason ? tr('DSD 直通無法使用：{0}', [translateDsdReason(reason)]) : null;
}

/**
 * The reasons the native side gives for a DSD track that plays as PCM (Kotlin `DsdPassthroughRules`, Rust
 * `player_dsd::raw` and `player_usb_audio::format`), in the interface language. An unknown text stays as it is.
 * @param {string} reason
 * @returns {string}
 */
export function translateDsdReason(reason) {
  switch (reason) {
    case '只支援雙聲道 DSD64、DSD128、DSD256 的 DSF 與 WavPack-DSD': return tr('只支援雙聲道 DSD64、DSD128、DSD256 的 DSF 與 WavPack-DSD');
    case '不是 DSF 或 WavPack-DSD 檔案': return tr('不是 DSF 或 WavPack-DSD 檔案');
    case '這不是 DSD 的 WavPack 檔案': return tr('這不是 DSD 的 WavPack 檔案');
    case '不是獨佔模式': return tr('不是獨佔模式');
    case '沒有 USB 音訊裝置': return tr('沒有 USB 音訊裝置');
    case '播放裝置不是 USB': return tr('播放裝置不是 USB');
    case '未取得 USB 權限': return tr('未取得 USB 權限');
    case '不是雙聲道': return tr('不是雙聲道');
    case '不是 44.1 kHz 系列的 DSD': return tr('不是 44.1 kHz 系列的 DSD');
    case '音響沒有 DSD 直通格式': return tr('音響沒有 DSD 直通格式');
    case '無法讀取 USB 描述元': return tr('無法讀取 USB 描述元');
    case '無法讀取 DSD 檔案': return tr('無法讀取 DSD 檔案');
    case '不支援直通': return tr('不支援直通');
    case 'USB 獨佔輸出無法使用': return tr('USB 獨佔輸出無法使用');
    default: break;
  }
  const tooFast = /^DSD(\d+) 超出 USB 音訊的取樣率上限$/.exec(reason);
  if (tooFast) return tr('DSD{0} 超出 USB 音訊的取樣率上限', [tooFast[1]]);
  const rate = /^音響沒有 DSD 取樣率 ([\d.]+) kHz$/.exec(reason);
  if (rate) return tr('音響沒有 DSD 取樣率 {0} kHz', [rate[1]]);
  return reason;
}

/**
 * The Android USB exclusive output (ADR 0047) in the Windows exclusive line's format (no conversion: the device runs
 * at the file's rate): 「44.1 kHz → 44.1 kHz（獨佔）．2 聲道．16bit．位元完美」; what kept it from being bit-perfect is
 * the notice. Native DSD (ADR 0049) is the DSD rate, then the frame rate on the bus: 「DSD64 → 88.2 kHz（DSD 直通）．
 * 2 聲道」, with no bit depth, no 「位元完美」 and no volume reason (the volume does not apply).
 * @param {import('./ipc').AndroidUsbExclusiveEngine | null | undefined} engine
 * @param {string | null} [dsdFallbackReason] why a DSD track plays as PCM although native passthrough is on
 * @returns {OutputFormatStatus | null} null while no format is set
 */
export function describeAndroidExclusive(engine, dsdFallbackReason = null) {
  if (!engine?.rate || !engine.bits || !engine.channels) return null;
  if (engine.dsdNative) {
    const steps = [{ rateHz: engine.rate, name: tr('DSD 直通') }];
    return { text: formatOutputPath(formatDsdName(engine.rate * 32), steps, engine.channels), notice: null };
  }
  const described = describePcmExclusive(engine);
  const fallback = dsdFallbackNotice(dsdFallbackReason);
  if (!described || !fallback) return described;
  return { text: described.text, notice: [described.notice, fallback].filter(Boolean).join('；') };
}

/**
 * @param {import('./ipc').AndroidUsbExclusiveEngine} engine
 * @returns {OutputFormatStatus | null}
 */
function describePcmExclusive(engine) {
  if (!engine.rate || !engine.bits || !engine.channels) return null;
  const reasons = engine.notBitPerfect ?? [];
  /** @type {string[]} */
  const otherReasons = reasons
    .filter((reason) => reason !== 'volume')
    .map(androidExclusiveReason)
    .filter((text) => text !== null);
  return describeExclusive({
    mode: 'highQuality',
    sourceRateHz: engine.rate,
    outputRateHz: engine.rate,
    deviceRateHz: engine.rate,
    conversion: 'none',
    fallbackReason: null,
    dseeHx: 'off',
    dseeNotice: null,
    dseeOutputRateHz: null,
    exclusive: {
      bits: engine.bits,
      channels: engine.channels,
      sourceChannels: engine.channels,
      bitPerfect: reasons.length === 0,
      volumeApplied: reasons.includes('volume'),
      underruns: engine.underruns,
      otherReasons,
    },
  });
}
