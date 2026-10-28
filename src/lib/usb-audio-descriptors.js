// The USB DAC descriptor reader (ADR 0045): the Kotlin `UsbAudioDescriptors` parser's answer turned into the compact
// read-only lines of a USB card in Settings → playback. Nothing here talks to the device.
import { tr } from './i18n.js';
import { formatSampleRate } from './output-format.js';

/**
 * @typedef {{ address: number, direction: 'in' | 'out', transfer: string, sync: string, usage: string,
 *   maxPacketSize: number, extraTransactions: number, interval: number }} UsbEndpoint
 * @typedef {{ interface: number, alt: number, terminalLink: number | null, format: string, formatType: number | null,
 *   subslotBytes: number | null, bits: number | null, channels: number | null, rates: number[] | null,
 *   rateRange: number[] | null, endpoints: UsbEndpoint[] }} UsbStream
 * @typedef {{ id: number, type: string, frequencyControl: number }} UsbClockSource
 * @typedef {{ masterMute: boolean, masterVolume: boolean, channelMute: boolean, channelVolume: boolean }} UsbFeatureUnit
 * @typedef {{ featureUnits?: UsbFeatureUnit[], clockSources?: UsbClockSource[] }} UsbControls
 * @typedef {{
 *   name: string, vendorId: number, productId: number, manufacturer: string | null, product: string | null,
 *   permission: 'granted' | 'denied' | 'notAsked', error?: string, uacVersion?: number | null,
 *   controls?: UsbControls | null, streams?: UsbStream[], otherConfigurations?: unknown[], raw?: string,
 * }} UsbAudioDevice
 * @typedef {{ title: string, lines: string[], streams: string[], raw: string }} UsbDeviceView
 */

/** @param {number} value @param {number} digits */
function hex(value, digits) {
  return Number(value).toString(16).toUpperCase().padStart(digits, '0');
}

/** @param {string} format */
export function formatName(format) {
  switch (format) {
    case 'pcm': return 'PCM';
    case 'pcm8': return 'PCM8';
    case 'float': return tr('浮點');
    case 'alaw': return 'A-law';
    case 'mulaw': return 'μ-law';
    case 'raw': return tr('原始資料（DSD 等）');
    case 'typeII': return tr('Type II 壓縮格式');
    case 'typeIII': return tr('Type III（IEC 61937）');
    case 'typeIV': return 'Type IV';
    default: return tr('未知格式');
  }
}

/** @param {string} sync */
function syncName(sync) {
  switch (sync) {
    case 'async': return tr('非同步');
    case 'adaptive': return tr('自適應');
    case 'sync': return tr('同步');
    default: return tr('無同步型態');
  }
}

/**
 * Whether a stream plays to the device (its data endpoint is OUT).
 * @param {UsbStream} stream
 */
function isPlayback(stream) {
  return stream.endpoints.some((endpoint) => endpoint.direction === 'out' && endpoint.usage !== 'feedback');
}

/**
 * How the playback stream keeps time: the sync type of the first playback data endpoint, and for an asynchronous one
 * whether the device sends feedback (a feedback endpoint, or implicit feedback from a capture stream).
 * @param {UsbStream[]} streams
 */
export function describeSync(streams) {
  for (const stream of streams) {
    const data = stream.endpoints.find((endpoint) => endpoint.direction === 'out' && endpoint.usage !== 'feedback');
    if (!data || data.transfer !== 'isochronous') continue;
    if (data.sync !== 'async') return syncName(data.sync);
    const feedback = stream.endpoints.some((endpoint) => endpoint.usage === 'feedback');
    return feedback ? tr('非同步（有回饋）') : tr('非同步（無回饋端點）');
  }
  return null;
}

/**
 * The rates a UAC1 stream lists; UAC2 rates come from the clock source at runtime and are not read.
 * @param {UsbStream} stream
 */
function ratesOf(stream) {
  if (stream.rates?.length) return `${stream.rates.map((rate) => formatSampleRate(rate).replace(' kHz', '')).join(', ')} kHz`;
  if (stream.rateRange?.length === 2) {
    const [low, high] = stream.rateRange;
    return `${formatSampleRate(low).replace(' kHz', '')}–${formatSampleRate(high)}`;
  }
  return null;
}

/** @param {UsbEndpoint} endpoint */
export function describeEndpoint(endpoint) {
  const size = endpoint.extraTransactions > 0
    ? tr('{0} byte ×{1}', [endpoint.maxPacketSize, endpoint.extraTransactions + 1])
    : tr('{0} byte', [endpoint.maxPacketSize]);
  const role = endpoint.usage === 'feedback'
    ? tr('回饋')
    : endpoint.direction === 'out' ? tr('輸出') : tr('輸入');
  const sync = endpoint.transfer === 'isochronous' && endpoint.usage !== 'feedback' ? syncName(endpoint.sync) : null;
  return [tr('端點 0x{0} {1}', [hex(endpoint.address, 2), role]), sync, size, tr('間隔 {0}', [endpoint.interval])]
    .filter(Boolean)
    .join('．');
}

/**
 * One alternate setting: 「設定 1：PCM．16bit（2 byte）．2 聲道」, its endpoints after it; alt 0 without endpoints is
 * the zero-bandwidth setting the device idles in.
 * @param {UsbStream} stream
 */
export function describeStream(stream) {
  if (!stream.endpoints.length) return tr('設定 {0}：零頻寬（閒置）', [stream.alt]);
  const parts = [formatName(stream.format)];
  if (stream.bits) {
    parts.push(stream.subslotBytes ? tr('{0}bit（{1} byte）', [stream.bits, stream.subslotBytes]) : tr('{0}bit', [stream.bits]));
  }
  if (stream.channels) parts.push(tr('{0} 聲道', [stream.channels]));
  const rates = ratesOf(stream);
  if (rates) parts.push(rates);
  if (!isPlayback(stream)) parts.push(tr('錄音'));
  const endpoints = stream.endpoints.map(describeEndpoint).join('；');
  return `${tr('設定 {0}：{1}', [stream.alt, parts.join('．')])}（${endpoints}）`;
}

/** @param {UsbClockSource} clock */
function clockName(clock) {
  const kind = {
    external: tr('外部'),
    internalFixed: tr('內部固定'),
    internalVariable: tr('內部可變'),
    internalProgrammable: tr('內部可程式'),
  }[clock.type] ?? tr('未知');
  return clock.frequencyControl === 3 ? tr('{0}（可設定頻率）', [kind]) : kind;
}

/**
 * The lines of one device: the version and timing, the controls, and each alternate setting.
 * @param {UsbAudioDevice} device
 * @returns {UsbDeviceView}
 */
export function describeUsbDevice(device) {
  const name = device.product?.trim() || device.name;
  const title = `${name}（VID ${hex(device.vendorId, 4)}／PID ${hex(device.productId, 4)}）`;
  /** @type {string[]} */
  const lines = [];
  /** @type {string[]} */
  const streams = [];
  if (device.permission !== 'granted') {
    lines.push(device.permission === 'denied' ? tr('未取得 USB 權限') : tr('尚未授予 USB 權限'));
    return { title, lines, streams, raw: '' };
  }
  if (device.error) {
    lines.push(tr('無法讀取 USB 描述元'));
    return { title, lines, streams, raw: device.raw ?? '' };
  }
  const all = device.streams ?? [];
  const head = [device.uacVersion ? `UAC ${device.uacVersion}` : tr('不是 USB Audio 裝置'), describeSync(all)];
  lines.push(head.filter(Boolean).join('．'));
  const units = device.controls?.featureUnits ?? [];
  if (units.length) {
    const volume = units.some((unit) => unit.masterVolume || unit.channelVolume);
    const mute = units.some((unit) => unit.masterMute || unit.channelMute);
    lines.push(tr('硬體音量：{0}．靜音：{1}', [volume ? tr('有') : tr('無'), mute ? tr('有') : tr('無')]));
  }
  const clocks = device.controls?.clockSources ?? [];
  if (clocks.length) lines.push(tr('時脈：{0}', [clocks.map(clockName).join('、')]));
  if (device.uacVersion === 2 && all.some((stream) => stream.endpoints.length)) {
    lines.push(tr('取樣率由時脈來源在播放時提供，未讀取'));
  }
  const others = device.otherConfigurations?.length ?? 0;
  if (others) lines.push(tr('另有 {0} 個 USB 設定未列出', [others]));
  for (const stream of all) streams.push(describeStream(stream));
  return { title, lines, streams, raw: device.raw ?? '' };
}

/**
 * The raw descriptors as space-separated byte pairs, for reading and copying.
 * @param {string} raw
 */
export function spacedHex(raw) {
  return (raw.match(/.{1,2}/g) ?? []).join(' ');
}

// ---- The system mixer formats (Android 14+, `getSupportedMixerAttributes`): what the audio policy is willing to open
// for the device, grouped by the Kotlin `MixerAttributesRules`; read only, nothing is ever set (ADR 0045). ----

/**
 * @typedef {{ behavior: 'default' | 'bitPerfect' | 'unknown', behaviorValue: number, encoding: string,
 *   encodingValue: number, bits: number | null, channelMask: number, channels: number | null }} MixerFormat
 * @typedef {MixerFormat & { sampleRates: number[] }} MixerGroup
 * @typedef {MixerFormat & { sampleRate: number | null }} MixerEntry
 * @typedef {{ id: number, type: number, productName: string, groups: MixerGroup[], bitPerfect: boolean,
 *   preferred: MixerEntry | null, error?: string }} MixerDevice
 * @typedef {{ supported: boolean, reason?: string, devices: MixerDevice[] }} MixerAttributesResult
 * @typedef {{ title: string, lines: string[] }} MixerDeviceView
 */

/** @param {MixerFormat} format */
function mixerBehaviorName(format) {
  switch (format.behavior) {
    case 'bitPerfect': return tr('位元完美');
    case 'default': return tr('一般');
    default: return tr('未知模式（{0}）', [format.behaviorValue]);
  }
}

/** @param {MixerFormat} format */
function mixerEncodingName(format) {
  if (format.encoding === 'pcmFloat') return tr('浮點 {0}bit', [format.bits ?? 32]);
  if (format.bits && format.encoding.startsWith('pcm')) return `PCM ${format.bits}bit`;
  return tr('編碼 {0}', [format.encodingValue]);
}

/**
 * One format: 「位元完美：PCM 32bit．48, 96, 192 kHz．2 聲道」 (rates and channels as `output-format.js` writes them).
 * @param {MixerFormat} format
 * @param {number[]} rates
 */
export function describeMixerFormat(format, rates) {
  const sorted = [...new Set(rates.filter((rate) => rate > 0))].sort((a, b) => a - b);
  const parts = [
    mixerEncodingName(format),
    sorted.length ? `${sorted.map((rate) => formatSampleRate(rate).replace(' kHz', '')).join(', ')} kHz` : null,
    format.channels ? tr('{0} 聲道', [format.channels]) : null,
  ];
  return `${mixerBehaviorName(format)}：${parts.filter(Boolean).join('．')}`;
}

/**
 * The 「系統混音器格式」 lines of each queried output: below Android 14 one notice; a device without any bit-perfect
 * entry says so (its default entries still listed); the preferred format, when one is set, last.
 * @param {MixerAttributesResult | null | undefined} result
 * @returns {{ notice: string | null, devices: MixerDeviceView[] }}
 */
export function describeMixerAttributes(result) {
  if (!result) return { notice: null, devices: [] };
  if (!result.supported) {
    return { notice: result.reason === 'requiresAndroid14' ? tr('需要 Android 14 以上才能查詢') : tr('無法查詢系統混音器格式'), devices: [] };
  }
  if (!result.devices.length) return { notice: tr('沒有找到 USB 播放裝置'), devices: [] };
  const devices = result.devices.map((device) => {
    /** @type {string[]} */
    const lines = [];
    if (device.error) {
      lines.push(tr('無法查詢系統混音器格式：{0}', [device.error]));
    } else {
      if (!device.groups.some((group) => group.behavior === 'bitPerfect')) lines.push(tr('此系統未提供位元完美模式'));
      for (const group of device.groups) lines.push(describeMixerFormat(group, group.sampleRates));
      if (device.preferred) {
        const rate = device.preferred.sampleRate;
        lines.push(tr('目前偏好：{0}', [describeMixerFormat(device.preferred, rate ? [rate] : [])]));
      }
    }
    return { title: device.productName || tr('USB 裝置 {0}', [device.id]), lines };
  });
  return { notice: null, devices };
}
