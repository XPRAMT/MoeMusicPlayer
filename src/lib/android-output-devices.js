// Android output devices (ADR 0044): which device the player is pinned to, and what each attached device takes. The
// identity rule (which devices are listed, what is stored, which device a stored choice is) lives in the Kotlin
// `AudioOutputRules`; this file only turns its answer into rows.
import { tr } from './i18n.js';

/**
 * @typedef {import('./ipc').AndroidOutputDevice} AndroidOutputDevice
 * @typedef {import('./ipc').AndroidOutputDeviceInfo} AndroidOutputDeviceInfo
 * @typedef {import('./ipc').AndroidAudioOutputStatus} AndroidAudioOutputStatus
 * @typedef {import('./ipc').AndroidAudioTrackOutput} AndroidAudioTrackOutput
 * @typedef {{ text: string, tone: 'ok' | 'warn' | 'error' | 'idle' }} RowPath
 * @typedef {{
 *   key: string,
 *   identity: AndroidOutputDevice | null,
 *   label: string,
 *   sub: string | null,
 *   chosen: boolean,
 *   connected: boolean,
 *   capabilities: string,
 *   path: RowPath | null,
 * }} AndroidOutputRow
 */

// AudioFormat.ENCODING_* of linear PCM: the bit depth each one carries.
/** @type {Readonly<Record<number, number>>} */
const PCM_BITS = Object.freeze({ 3: 8, 2: 16, 21: 24, 22: 32 });
const ENCODING_PCM_FLOAT = 4;

/**
 * The kind of device, in words (the kinds are `AudioOutputRules.kindOf`).
 * @param {string} kind
 */
export function kindLabel(kind) {
  switch (kind) {
    case 'speaker': return tr('內建喇叭');
    case 'wired': return tr('有線耳機');
    case 'usb': return tr('USB');
    case 'bluetooth': return tr('藍牙');
    case 'bleAudio': return tr('藍牙 LE Audio');
    case 'hearingAid': return tr('助聽器');
    case 'hdmi': return tr('HDMI');
    case 'line': return tr('線路輸出');
    case 'dock': return tr('底座');
    default: return tr('其他裝置');
  }
}

/**
 * What a device is called: its product name with its kind ("KA3（USB）"). The built-in speaker is named after the
 * phone, so it is the kind alone; a device without a name (a hidden Bluetooth name) is its kind.
 * @param {{ kind: string, productName?: string | null }} device
 */
export function deviceLabel(device) {
  const kind = kindLabel(device.kind);
  const name = (device.productName ?? '').trim();
  if (device.kind === 'speaker' || !name) return kind;
  return tr('{0}（{1}）', [name, kind]);
}

/**
 * 44100 -> "44.1"; the unit is added once for the whole list.
 * @param {number} hz
 */
function kilohertz(hz) {
  return String(Number((hz / 1000).toFixed(3)));
}

/**
 * The bit depths of a list of encodings ("16／24 位元／浮點"); null when none of them is linear PCM.
 * @param {number[]} encodings
 */
function bitDepths(encodings) {
  const bits = [...new Set(encodings.map((encoding) => PCM_BITS[encoding]).filter(Boolean))].sort((a, b) => a - b);
  const float = encodings.includes(ENCODING_PCM_FLOAT);
  if (!bits.length && !float) return null;
  const parts = [];
  if (bits.length) parts.push(tr('{0} 位元', [bits.join('／')]));
  if (float) parts.push(tr('浮點'));
  return parts.join('／');
}

/**
 * What a device takes, in the row style of the Windows list: "44.1／48／96 kHz．2 聲道．16／24 位元". An empty list from
 * Android means "any" (the platform converts), shown as 不限.
 * @param {Pick<AndroidOutputDeviceInfo, 'sampleRates' | 'channelCounts' | 'encodings'>} device
 */
export function describeCapabilities(device) {
  const rates = [...new Set(device.sampleRates)].sort((a, b) => a - b);
  const channels = [...new Set(device.channelCounts)].sort((a, b) => a - b);
  if (!rates.length && !channels.length && !device.encodings.length) return tr('格式不限');
  const depths = device.encodings.length ? bitDepths(device.encodings) : tr('位元深度不限');
  return [
    rates.length ? `${rates.map(kilohertz).join('／')} kHz` : tr('取樣率不限'),
    channels.length ? tr('{0} 聲道', [channels.join('／')]) : tr('聲道數不限'),
    depths,
  ].filter(Boolean).join('．');
}

/**
 * One AudioTrack encoding in words.
 * @param {number} encoding
 */
function encodingLabel(encoding) {
  if (encoding === ENCODING_PCM_FLOAT) return tr('32 位元浮點');
  const bits = PCM_BITS[encoding];
  return bits ? tr('{0} 位元', [bits]) : tr('編碼 {0}', [encoding]);
}

/**
 * What the AudioTrack really opened and where it plays: "實際輸出：48 kHz．24 位元．2 聲道 → KA3（USB）". This is the
 * stream the player hands to Android; the system mixer may still convert it, so it never claims to be bit-perfect.
 * @param {AndroidAudioTrackOutput | null | undefined} output
 * @param {AndroidOutputDeviceInfo | null | undefined} routed
 */
export function describeActualOutput(output, routed) {
  if (!output) return tr('音訊輸出尚未開啟。');
  const format = [
    `${kilohertz(output.sampleRate)} kHz`,
    encodingLabel(output.encoding),
    tr('{0} 聲道', [output.channelCount]),
  ];
  if (output.offload) format.push(tr('硬體解碼'));
  const text = format.join('．');
  return routed ? tr('實際輸出：{0} → {1}', [text, deviceLabel(routed)]) : tr('實際輸出：{0}', [text]);
}

/**
 * Whether two stored choices are the same device.
 * @param {AndroidOutputDevice | null} a
 * @param {AndroidOutputDevice | null} b
 */
export function sameChoice(a, b) {
  if (!a || !b) return a === b;
  return a.kind === b.kind && a.productName === b.productName && (a.address ?? null) === (b.address ?? null);
}

/**
 * The rows the list shows: 「系統預設」, then each attached device, then the chosen device when it is not attached (the
 * player then plays on the system's choice, and the row says which device that is).
 * @param {AndroidAudioOutputStatus | null} status
 * @param {AndroidOutputDevice | null} saved The choice in settings.json.
 * @returns {AndroidOutputRow[]}
 */
export function buildAndroidRows(status, saved) {
  const devices = status?.devices ?? [];
  const routed = status?.routed ?? null;
  const output = status?.output ?? null;
  const chosenId = saved ? (status?.chosenId ?? null) : null;
  const missing = Boolean(saved) && chosenId === null;
  /** @type {AndroidOutputRow[]} */
  const rows = [{
    key: 'system',
    identity: null,
    label: tr('系統預設'),
    sub: !saved && routed ? tr('目前是：{0}', [deviceLabel(routed)]) : tr('跟隨 Android 的預設播放裝置'),
    chosen: !saved,
    connected: true,
    capabilities: '',
    path: !saved ? { text: describeActualOutput(output, routed), tone: output ? 'ok' : 'idle' } : null,
  }];
  for (const device of devices) {
    const chosen = device.id === chosenId;
    // The chosen device is attached but the track plays elsewhere: pinning did not take (yet).
    const elsewhere = chosen && output && routed && routed.id !== device.id;
    rows.push({
      key: `device-${device.id}`,
      identity: device.identity,
      label: deviceLabel(device),
      sub: null,
      chosen,
      connected: true,
      capabilities: describeCapabilities(device),
      path: chosen
        ? { text: describeActualOutput(output, routed), tone: elsewhere ? 'warn' : output ? 'ok' : 'idle' }
        : null,
    });
  }
  if (missing && saved) {
    rows.push({
      key: 'missing',
      identity: saved,
      label: deviceLabel(saved),
      sub: null,
      chosen: true,
      connected: false,
      capabilities: '',
      path: {
        text: routed ? tr('已中斷連線，目前改由 {0} 播放', [deviceLabel(routed)]) : tr('已中斷連線'),
        tone: 'error',
      },
    });
  }
  return rows;
}
