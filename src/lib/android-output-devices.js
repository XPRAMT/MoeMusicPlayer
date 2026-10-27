// Android output devices (ADR 0044): which device the player is pinned to, and what each attached device takes. The
// identity rule (which devices are listed, what is stored, which device a stored choice is) lives in the Kotlin
// `AudioOutputRules`; this file only turns its answer into rows.
import { tr } from './i18n.js';
import { formatSampleRate } from './output-format.js';

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
 * 44100 -> "44.1"; the unit is added once for a range.
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
 * The declared sample rates as a range ("44.1–96 kHz"), or the one rate ("48 kHz").
 * @param {number[]} rates sorted, without duplicates, not empty
 */
function rateRange(rates) {
  const low = rates[0];
  const high = rates[rates.length - 1];
  return low === high ? formatSampleRate(low) : `${kilohertz(low)}–${formatSampleRate(high)}`;
}

/**
 * What a device declares it takes (`AudioDeviceInfo`), one line for a row that is not playing, in the style of the
 * Windows 「預設格式 …」 line: 「支援 44.1–96 kHz．2 聲道．16／24 位元」. An empty list from Android means "any" (the
 * platform converts): 「格式不限」 when nothing is declared.
 * @param {Pick<AndroidOutputDeviceInfo, 'sampleRates' | 'channelCounts' | 'encodings'>} device
 */
export function describeSupport(device) {
  const rates = [...new Set(device.sampleRates)].sort((a, b) => a - b);
  const channels = [...new Set(device.channelCounts)].sort((a, b) => a - b);
  if (!rates.length && !channels.length && !device.encodings.length) return tr('格式不限');
  const depths = device.encodings.length ? bitDepths(device.encodings) : tr('位元深度不限');
  const parts = [
    rates.length ? rateRange(rates) : tr('取樣率不限'),
    channels.length ? tr('{0} 聲道', [channels.join('／')]) : tr('聲道數不限'),
    depths,
  ].filter(Boolean).join('．');
  return tr('支援 {0}', [parts]);
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
 * The name of a Bluetooth codec step: 「LDAC 990 kbps」, 「LDAC 最佳效果」 (the adaptive setting), 「AAC」.
 * @param {import('./ipc').AndroidBluetoothCodec} codec
 */
export function codecStepName(codec) {
  if (codec.ldacKbps) return `${codec.codec} ${codec.ldacKbps} kbps`;
  if (codec.ldacAdaptive) return tr('{0} 最佳效果', [codec.codec]);
  return codec.codec;
}

/**
 * The sample-rate path of the row that plays, one line like the Windows list: the AudioTrack rate → the Android
 * mixer rate（系統混音） → the Bluetooth codec rate（codec）．N 聲道, e.g.
 * 「96 kHz → 48 kHz（系統混音） → 96 kHz（LDAC 990 kbps）．2 聲道」.
 * - The mixer step is shown only when its rate is known and differs from the AudioTrack rate (an equal rate is no
 *   conversion); an unknown mixer rate is left out, never guessed.
 * - A Bluetooth route always ends with the codec step, even at the same rate, because it names the codec; before the
 *   stack has reported the codec (or without the permission) the step is 「（藍牙）」 with no rate.
 * - Without any step it is the rate alone: 「96 kHz．2 聲道」. Bit depth is not part of the path (as on Windows), and
 *   nothing here claims what the device receives bit for bit.
 * @param {AndroidAudioTrackOutput | null | undefined} output
 * @param {{ bluetooth: boolean, codec: import('./ipc').AndroidBluetoothCodec | null }} [route]
 * @returns {string | null} null when no AudioTrack is open
 */
export function describeOutputPath(output, route = { bluetooth: false, codec: null }) {
  if (!output) return null;
  const steps = [formatSampleRate(output.sampleRate)];
  const mixer = output.mixerSampleRate;
  if (mixer && mixer > 0 && mixer !== output.sampleRate) steps.push(`${formatSampleRate(mixer)}（${tr('系統混音')}）`);
  if (route.bluetooth) {
    const codec = route.codec;
    if (codec) {
      const name = codecStepName(codec);
      steps.push(codec.sampleRate ? `${formatSampleRate(codec.sampleRate)}（${name}）` : `（${name}）`);
    } else {
      steps.push(`（${tr('藍牙')}）`);
    }
  }
  const parts = [steps.join(' → '), tr('{0} 聲道', [output.channelCount])];
  if (output.offload) parts.push(tr('硬體解碼'));
  return parts.join('．');
}

/**
 * The codec the stack reported for a device, by its address.
 * @param {AndroidOutputDeviceInfo} device
 * @param {AndroidAudioOutputStatus | null} status
 */
function codecOf(device, status) {
  if (status?.bluetoothPermission !== 'granted') return null;
  return status.bluetoothCodecs?.[device.address.trim().toUpperCase()] ?? null;
}

/**
 * The path line of the playing row, for the device the track is routed to; null when no AudioTrack is open.
 * @param {AndroidAudioOutputStatus | null} status
 */
function playingPath(status) {
  const routed = status?.routed ?? null;
  const bluetooth = routed?.kind === 'bluetooth';
  return describeOutputPath(status?.output ?? null, {
    bluetooth,
    codec: bluetooth && routed ? codecOf(routed, status) : null,
  });
}

/**
 * The 「附近的裝置」 line of a Bluetooth row: shown when the permission is missing (the codec cannot be read without
 * it), nothing otherwise (the codec itself is part of the playing row's path).
 * @param {AndroidOutputDeviceInfo} device
 * @param {AndroidAudioOutputStatus | null} status
 * @returns {{ text: string, needsPermission: boolean } | null}
 */
export function bluetoothCodecLine(device, status) {
  if (device.kind !== 'bluetooth' || status?.bluetoothPermission !== 'needed') return null;
  return { text: tr('需要『附近的裝置』權限才能讀取藍牙編碼'), needsPermission: true };
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
  const chosenId = saved ? (status?.chosenId ?? null) : null;
  const missing = Boolean(saved) && chosenId === null;
  const path = playingPath(status);
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
    path: !saved && path ? { text: path, tone: 'ok' } : null,
  }];
  for (const device of devices) {
    const chosen = device.id === chosenId;
    const playing = chosen && path !== null;
    // The chosen device is attached but the track plays elsewhere: pinning did not take (yet).
    const elsewhere = playing && routed !== null && routed.id !== device.id;
    /** @type {RowPath | null} */
    let rowPath = null;
    if (playing && path !== null) {
      rowPath = elsewhere && routed
        ? { text: tr('目前由 {0} 播放：{1}', [deviceLabel(routed), path]), tone: 'warn' }
        : { text: path, tone: 'ok' };
    }
    rows.push({
      key: `device-${device.id}`,
      identity: device.identity,
      label: deviceLabel(device),
      sub: null,
      chosen,
      connected: true,
      capabilities: playing ? '' : describeSupport(device),
      bluetooth: bluetoothCodecLine(device, status),
      path: rowPath,
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
