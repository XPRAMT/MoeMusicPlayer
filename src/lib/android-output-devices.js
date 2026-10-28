// Android output devices (ADR 0044): which device the player is pinned to, and what each attached device takes. The
// identity rule (which devices are listed, what is stored, which device a stored choice is) lives in the Kotlin
// `AudioOutputRules`; this file only turns its answer into rows.
import { tr } from './i18n.js';
import { describeAndroidExclusive, describeAndroidPath, dsdFallbackNotice, formatSupportedFormats } from './output-format.js';

/** `androidUsbExclusive` when settings.json does not say (Rust `AppSettings::default`, ADR 0047): the shared mode. */
export const DEFAULT_ANDROID_USB_EXCLUSIVE = false;

/** `androidBluetoothRateFollow` when settings.json does not say (Rust `AppSettings::default`, ADR 0051): off. */
export const DEFAULT_ANDROID_BLUETOOTH_RATE_FOLLOW = false;

/**
 * Whether the Bluetooth sample-rate follow can run: it needs root or a Shizuku this app may use (ADR 0051; the same
 * sources as the real mixer rate, ADR 0048). Null while the sources have not been read.
 * @param {import('./ipc').AndroidMixerRateSourceStatus | null | undefined} sources
 * @returns {boolean | null}
 */
export function bluetoothRateSourceAvailable(sources) {
  if (!sources) return null;
  return sources.root === 'available' || sources.shizuku === 'granted';
}

/**
 * The short notice under the switch: why it does nothing yet. Null when it can work (or the sources are not known).
 * @param {boolean} enabled
 * @param {import('./ipc').AndroidMixerRateSourceStatus | null | undefined} sources
 * @returns {string | null}
 */
export function bluetoothRateNotice(enabled, sources) {
  const available = bluetoothRateSourceAvailable(sources);
  if (available !== false) return null;
  return enabled
    ? tr('需要 root 或已授權的 Shizuku 才能切換藍牙取樣率；目前沒有，所以不會變更。')
    : tr('此功能需要 root 或已授權的 Shizuku。');
}

/**
 * The two output modes under 「播放裝置」, as on Windows (`.output-mode`, ADR 0047): `exclusive` is
 * `androidUsbExclusive`.
 * @returns {Array<{ exclusive: boolean, title: string, description: string }>}
 */
export function androidOutputModes() {
  return [
    {
      exclusive: false,
      title: tr('共享'),
      description: tr('與其他 App 一起播放，由 Android 系統混音；系統可能轉換取樣率，不是位元完美。'),
    },
    {
      exclusive: true,
      title: tr('獨佔優先'),
      description: tr('USB 音訊裝置由本程式直接輸出（位元完美）；其他 App 與通知音改從別處出聲，系統音量無效。'),
    },
  ];
}

/**
 * The text under the 「播放裝置」 header, by mode (as the Windows page's).
 * @param {boolean} exclusive
 */
export function androidOutputHeader(exclusive) {
  return exclusive
    ? tr('選「系統預設」或 USB 音訊裝置時，由本程式獨佔輸出（有多台時用第一台）；選其他裝置則以共享方式播放。')
    : tr('選擇要輸出的裝置；拔除後改由系統預設播放，重新接上時自動切回。音訊經 Android 系統混音器輸出，系統可能再轉換取樣率，不是位元完美。');
}

/**
 * @typedef {import('./ipc').AndroidOutputDevice} AndroidOutputDevice
 * @typedef {import('./ipc').AndroidOutputDeviceInfo} AndroidOutputDeviceInfo
 * @typedef {import('./ipc').AndroidAudioOutputStatus} AndroidAudioOutputStatus
 * @typedef {import('./ipc').AndroidAudioTrackOutput} AndroidAudioTrackOutput
 * @typedef {{ text: string, tone: 'ok' | 'warn' | 'error' | 'idle', notice?: string | null }} RowPath
 * @typedef {{ text: string, needsPermission: boolean }} RowHint
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
 *   hint: RowHint | null,
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
 * The path of the USB row the exclusive mode drives (ADR 0047): its own line while it plays the track
 * (「44.1 kHz → 44.1 kHz（獨佔）．2 聲道．16bit．位元完美」, what keeps it from bit-perfect as the notice), or the
 * Android path with the reason it is not used. Null in the shared mode or when nothing plays.
 * @param {AndroidAudioOutputStatus | null} status
 * @param {string | null} androidPath the AudioTrack path line
 * @returns {RowPath | null}
 */
export function usbExclusivePath(status, androidPath) {
  const exclusive = status?.exclusive;
  if (!exclusive?.enabled) return null;
  if (exclusive.active) {
    const described = describeAndroidExclusive(exclusive.engine, exclusive.dsdFallbackReason);
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
 * A path line with the reason a DSD track plays as PCM although native passthrough is on (ADR 0049, 「DSD 直通無法使用：…」)
 * added to its notice. The exclusive line already carries it ([usbExclusivePath]), so this is for the Android lines.
 * @param {RowPath | null} path
 * @param {AndroidAudioOutputStatus | null} status
 * @returns {RowPath | null}
 */
export function withDsdNotice(path, status) {
  const notice = dsdFallbackNotice(status?.exclusive?.dsdFallbackReason);
  if (!path || !notice) return path;
  return { ...path, notice: [path.notice, notice].filter(Boolean).join('；') };
}

/**
 * The listed USB row the exclusive mode plays the saved choice on (ADR 0047, `exclusive.route` from the service, which
 * applies the routing table): the chosen row for a chosen USB device; for the system default the USB row named like
 * the route's USB device, else the first USB row. Null when the route plays nothing on USB.
 * @param {AndroidAudioOutputStatus | null} status
 * @param {AndroidOutputDevice | null} saved
 * @returns {AndroidOutputDeviceInfo | null}
 */
export function exclusiveTargetDevice(status, saved) {
  const route = status?.exclusive?.route;
  const target = route?.usbDevice ?? null;
  const devices = status?.devices ?? [];
  if (saved) {
    // A chosen USB card is its own target unless the service says it plays on no USB device.
    if (saved.kind !== 'usb' || (route && !target)) return null;
    return devices.find((device) => device.id === status?.chosenId) ?? null;
  }
  if (!target) return null;
  const usbRows = devices.filter((device) => device.kind === 'usb');
  const name = (target.productName ?? '').trim().toLowerCase();
  return usbRows.find((device) => name && (device.productName ?? '').trim().toLowerCase() === name) ?? usbRows[0] ?? null;
}

/**
 * The exclusive-mode hint of a chosen row (ADR 0047): no USB audio device for the system default or a USB device
 * (shared as chosen), a non-USB device (shared on it), the USB device of the system default when several are attached,
 * or the USB permission still to grant. Null in the shared mode or when there is nothing to say.
 * @param {AndroidAudioOutputStatus | null} status
 * @param {AndroidOutputDevice | null} saved
 * @param {boolean} exclusiveMode
 * @param {'system' | 'device' | 'missing'} row
 * @param {AndroidOutputDeviceInfo | null} [device] the row's device
 * @returns {RowHint | null}
 */
export function exclusiveHint(status, saved, exclusiveMode, row, device = null) {
  if (!exclusiveMode) return null;
  const route = status?.exclusive?.route ?? null;
  if (row === 'device' && device && device.kind !== 'usb') {
    return { text: tr('獨佔模式：這台裝置不支援獨佔，以共享方式播放'), needsPermission: false };
  }
  if (route?.noUsbDevice) return { text: tr('獨佔模式：沒有 USB 音訊裝置，以共享方式播放'), needsPermission: false };
  const target = route?.usbDevice;
  if (!target) return null;
  if (!target.permitted) return { text: tr('獨佔模式需要 USB 權限才能直接驅動這台裝置'), needsPermission: true };
  if (row === 'system' && (route?.usbDevices ?? 0) > 1) {
    const listed = exclusiveTargetDevice(status, saved);
    const name = listed ? deviceName(listed) : (target.productName ?? tr('USB'));
    return { text: tr('獨佔模式：有多台 USB 音訊裝置，由第一台 {0} 獨佔播放', [name]), needsPermission: false };
  }
  return null;
}

/**
 * The rows the list shows: 「系統預設」, then each attached device, then the chosen device when it is not attached (the
 * player then plays on the system's choice, and the row says which device that is). In the exclusive mode
 * ([exclusiveMode], `androidUsbExclusive`; the service's own view when omitted) the USB row the track plays on
 * exclusively shows the exclusive line, also when 「系統預設」 is chosen (its 「目前是：…」 then names that device), and
 * chosen rows carry the routing table's hint.
 * @param {AndroidAudioOutputStatus | null} status
 * @param {AndroidOutputDevice | null} saved The choice in settings.json.
 * @param {boolean} [exclusiveMode]
 * @returns {AndroidOutputRow[]}
 */
export function buildAndroidRows(status, saved, exclusiveMode = status?.exclusive?.enabled ?? false) {
  const devices = status?.devices ?? [];
  const routed = status?.routed ?? null;
  const chosenId = saved ? (status?.chosenId ?? null) : null;
  const missing = Boolean(saved) && chosenId === null;
  const path = playingPath(status);
  const target = exclusiveMode ? exclusiveTargetDevice(status, saved) : null;
  // The system default plays on a USB device the app drives itself: that USB card shows the line, this one names it.
  const systemExclusive = !saved && target !== null && Boolean(status?.exclusive?.active);
  // The system default's track could not play on that USB device: the Android path with the reason, on this card.
  const systemFallback = !saved && target !== null && !systemExclusive ? usbExclusivePath(status, path) : null;
  /** @type {AndroidOutputRow[]} */
  const rows = [{
    key: 'system',
    identity: null,
    label: tr('系統預設'),
    sub: systemExclusive && target
      ? tr('目前是：{0}．USB 獨佔', [deviceLabel(target)])
      : !saved && routed ? tr('目前是：{0}', [deviceLabel(routed)]) : tr('跟隨 Android 的預設播放裝置'),
    chosen: !saved,
    connected: true,
    capabilities: '',
    bluetooth: null,
    path: systemFallback ? withDsdNotice(systemFallback, status) : (!saved && path && !systemExclusive ? withDsdNotice({ text: path, tone: 'ok' }, status) : null),
    hint: saved ? null : exclusiveHint(status, saved, exclusiveMode, 'system'),
  }];
  for (const device of devices) {
    const chosen = device.id === chosenId;
    const exclusiveTarget = target !== null && device.id === target.id;
    // Only a playing track's route says where the chosen device's track goes; the system's route (while paused) does
    // not follow the player's pinned device, so it must not raise 「目前由 … 播放」.
    const trackRoute = status?.routedFrom === 'system' ? null : routed;
    // The chosen row shows the path whenever a track is open (Android plays on the chosen device once it is pinned);
    // unknown routing (no routed device reported) does not make it idle.
    const usbExclusive = exclusiveTarget ? usbExclusivePath(status, chosen ? path : null) : null;
    const playing = (chosen && path !== null) || usbExclusive !== null;
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
      rowPath = withDsdNotice(elsewhere && trackRoute
        ? { text: tr('目前由 {0} 播放：{1}', [deviceLabel(trackRoute), path]), tone: 'warn' }
        : { text: own, tone: 'ok' }, status);
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
      hint: chosen ? exclusiveHint(status, saved, exclusiveMode, 'device', device) : null,
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
      hint: exclusiveHint(status, saved, exclusiveMode, 'missing'),
    });
  }
  return rows;
}
