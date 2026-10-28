// Android output devices (ADR 0044): which device the player is pinned to, and what each attached device takes. The
// identity rule (which devices are listed, what is stored, which device a stored choice is) lives in the Kotlin
// `AudioOutputRules`; this file only turns its answer into rows.
import { tr } from './i18n.js';
import { describeAndroidExclusive, describeAndroidPath, formatSupportedFormats } from './output-format.js';

/** `androidUsbExclusive` when settings.json does not say (Rust `AppSettings::default`, ADR 0047). */
export const DEFAULT_ANDROID_USB_EXCLUSIVE = false;

/**
 * @typedef {import('./ipc').AndroidOutputDevice} AndroidOutputDevice
 * @typedef {import('./ipc').AndroidOutputDeviceInfo} AndroidOutputDeviceInfo
 * @typedef {import('./ipc').AndroidAudioOutputStatus} AndroidAudioOutputStatus
 * @typedef {import('./ipc').AndroidAudioTrackOutput} AndroidAudioTrackOutput
 * @typedef {{ text: string, tone: 'ok' | 'warn' | 'error' | 'idle', notice?: string | null }} RowPath
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
 * What a device declares it takes (`AudioDeviceInfo`), the line of a row that is not playing, in the Windows list's
 * format: every sample rate, then the channel counts (「44.1, 48, 96 kHz．2 聲道」). An empty list from Android means
 * "any" (the platform converts): 「格式不限」 when nothing is declared, and for a Bluetooth device (Bluetooth, LE Audio,
 * hearing aid) 「由藍牙編碼決定」, because the codec the stack negotiates sets its format.
 * @param {Pick<AndroidOutputDeviceInfo, 'sampleRates' | 'channelCounts'> & { kind?: string }} device
 */
export function describeSupport(device) {
  const wireless = device.kind === 'bluetooth' || device.kind === 'bleAudio' || device.kind === 'hearingAid';
  return formatSupportedFormats(device.sampleRates, device.channelCounts, wireless ? tr('由藍牙編碼決定') : undefined);
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
 * The codec the stack reported for a device, by its address.
 * @param {AndroidOutputDeviceInfo} device
 * @param {AndroidAudioOutputStatus | null} status
 */
function codecOf(device, status) {
  if (status?.bluetoothPermission !== 'granted') return null;
  return status.bluetoothCodecs?.[device.address.trim().toUpperCase()] ?? null;
}

/**
 * The path line of the playing row, for the device the track plays on (`device`, by default the routed one); null when
 * no AudioTrack is open.
 * @param {AndroidAudioOutputStatus | null} status
 * @param {AndroidOutputDeviceInfo | null} [routed]
 */
function playingPath(status, routed = status?.routed ?? null) {
  const bluetooth = routed?.kind === 'bluetooth';
  return describeAndroidPath(status?.output ?? null, {
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
 * Whether two listed devices are one device: the same id, or the same identity (`AudioOutputRules.identityOf`).
 * @param {AndroidOutputDeviceInfo} a
 * @param {AndroidOutputDeviceInfo} b
 */
function isSameDevice(a, b) {
  return a.id === b.id || (a.identity != null && b.identity != null && sameChoice(a.identity, b.identity));
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
 * The path of the chosen USB row when the USB exclusive output is on (ADR 0047): its own line while it plays the track
 * (「44.1 kHz → 44.1 kHz（獨佔）．2 聲道．16bit．位元完美」, what keeps it from bit-perfect as the notice), or the
 * Android path with the reason it is not used. Null when the switch is off or nothing plays.
 * @param {AndroidAudioOutputStatus | null} status
 * @param {string | null} androidPath the AudioTrack path line
 * @returns {RowPath | null}
 */
export function usbExclusivePath(status, androidPath) {
  const exclusive = status?.exclusive;
  if (!exclusive?.enabled) return null;
  if (exclusive.active) {
    const described = describeAndroidExclusive(exclusive.engine);
    if (described) return { text: described.text, tone: 'ok', notice: described.notice };
    // No format yet (the device is being claimed), or the device went back to Android while paused.
    const idle = exclusive.state === 'handedBack' ? tr('USB 獨佔：裝置已暫時交還系統，播放時再取得') : tr('USB 獨佔：準備中');
    return { text: idle, tone: 'idle', notice: null };
  }
  if (exclusive.fallbackReason && androidPath) {
    return { text: androidPath, tone: 'warn', notice: tr('USB 獨佔無法播放這首：{0}', [exclusive.fallbackReason]) };
  }
  return null;
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
    // Only a playing track's route says where the chosen device's track goes; the system's route (while paused) does
    // not follow the player's pinned device, so it must not raise 「目前由 … 播放」.
    const trackRoute = status?.routedFrom === 'system' ? null : routed;
    // The chosen row shows the path whenever a track is open (Android plays on the chosen device once it is pinned);
    // unknown routing (no routed device reported) does not make it idle.
    const usbExclusive = chosen && device.kind === 'usb' ? usbExclusivePath(status, path) : null;
    const playing = chosen && (path !== null || usbExclusive !== null);
    // The chosen device is attached but the track plays elsewhere: pinning did not take (yet). The same device can be
    // reported under another id (Android lists some devices twice), so the identity decides, not the id.
    const elsewhere = playing && trackRoute !== null && !isSameDevice(trackRoute, device);
    /** @type {RowPath | null} */
    let rowPath = null;
    if (usbExclusive) {
      // The app drives the device itself: Android's routing (the device is gone from its list) does not apply.
      rowPath = usbExclusive;
    } else if (playing && path !== null) {
      // Without a routing report the track plays where the player is pinned: this device (its codec step included).
      const own = trackRoute === null ? playingPath(status, device) ?? path : path;
      rowPath = elsewhere && trackRoute
        ? { text: tr('目前由 {0} 播放：{1}', [deviceLabel(trackRoute), path]), tone: 'warn' }
        : { text: own, tone: 'ok' };
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
