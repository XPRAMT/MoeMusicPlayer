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
 *   bluetooth: { text: string, needsPermission: boolean } | null,
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
 * A device's name alone ("WF-1000XM6"); the built-in speaker and an unnamed device are their kind.
 * @param {{ kind: string, productName?: string | null }} device
 */
export function deviceName(device) {
  const name = (device.productName ?? '').trim();
  return device.kind === 'speaker' || !name ? kindLabel(device.kind) : name;
}

/**
 * What the device declares it takes (`AudioDeviceInfo`), labelled so it is not taken for what the device receives:
 * 「系統宣告：48 kHz．2 聲道．16 位元」.
 * @param {Pick<AndroidOutputDeviceInfo, 'sampleRates' | 'channelCounts' | 'encodings'>} device
 */
export function describeDeclaration(device) {
  return tr('系統宣告：{0}', [describeCapabilities(device)]);
}

/**
 * What the player hands to Android and where Android routes it: 「播放器輸出：44.1 kHz．32 位元浮點．2 聲道 →
 * WF-1000XM6」 (the AudioTrack format). The system mixer and a Bluetooth codec may still convert it, so it is never
 * called what the device receives, nor bit-perfect.
 * @param {AndroidAudioTrackOutput | null | undefined} output
 * @param {AndroidOutputDeviceInfo | null | undefined} routed
 */
export function describePlayerOutput(output, routed) {
  if (!output) return tr('音訊輸出尚未開啟。');
  const format = [
    `${kilohertz(output.sampleRate)} kHz`,
    encodingLabel(output.encoding),
    tr('{0} 聲道', [output.channelCount]),
  ];
  if (output.offload) format.push(tr('硬體解碼'));
  const text = format.join('．');
  return routed ? tr('播放器輸出：{0} → {1}', [text, deviceName(routed)]) : tr('播放器輸出：{0}', [text]);
}

/**
 * The codec a Bluetooth headset streams with: 「藍牙編碼：LDAC．96 kHz．32 位元．990 kbps」 (LDAC adds its quality:
 * the bitrate, or 最佳效果（自動） for the adaptive setting).
 * @param {import('./ipc').AndroidBluetoothCodec} codec
 */
export function describeBluetoothCodec(codec) {
  const parts = [codec.codec];
  if (codec.sampleRate) parts.push(`${kilohertz(codec.sampleRate)} kHz`);
  if (codec.bitsPerSample) parts.push(tr('{0} 位元', [codec.bitsPerSample]));
  if (codec.ldacKbps) parts.push(`${codec.ldacKbps} kbps`);
  else if (codec.ldacAdaptive) parts.push(tr('最佳效果（自動）'));
  return tr('藍牙編碼：{0}', [parts.join('．')]);
}

/**
 * The Bluetooth codec line of a device row: the codec when the stack has reported it, a request for 「附近的裝置」
 * when that permission is missing, nothing otherwise (not Bluetooth, Android 12 or older, or not reported yet).
 * @param {AndroidOutputDeviceInfo} device
 * @param {AndroidAudioOutputStatus | null} status
 * @returns {{ text: string, needsPermission: boolean } | null}
 */
export function bluetoothCodecLine(device, status) {
  if (device.kind !== 'bluetooth') return null;
  if (status?.bluetoothPermission === 'needed') {
    return { text: tr('需要『附近的裝置』權限才能讀取藍牙編碼'), needsPermission: true };
  }
  if (status?.bluetoothPermission !== 'granted') return null;
  const codec = status.bluetoothCodecs?.[device.address.trim().toUpperCase()];
  return codec ? { text: describeBluetoothCodec(codec), needsPermission: false } : null;
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
    bluetooth: null,
    path: !saved ? { text: describePlayerOutput(output, routed), tone: output ? 'ok' : 'idle' } : null,
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
      capabilities: describeDeclaration(device),
      bluetooth: bluetoothCodecLine(device, status),
      path: chosen
        ? { text: describePlayerOutput(output, routed), tone: elsewhere ? 'warn' : output ? 'ok' : 'idle' }
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
      bluetooth: null,
      path: {
        text: routed ? tr('已中斷連線，目前改由 {0} 播放', [deviceLabel(routed)]) : tr('已中斷連線'),
        tone: 'error',
      },
    });
  }
  return rows;
}
