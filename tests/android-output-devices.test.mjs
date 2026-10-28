import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  androidOutputHeader,
  androidOutputModes,
  buildAndroidRows,
  DEFAULT_ANDROID_USB_EXCLUSIVE,
  describeSupport,
  deviceLabel,
  exclusiveHint,
  exclusiveTargetDevice,
  sameChoice,
  usbExclusivePath,
} from '../src/lib/android-output-devices.js';

const usb = {
  id: 57,
  type: 22,
  kind: 'usb',
  productName: 'KA3',
  address: 'card=1;device=0',
  sampleRates: [96000, 44100, 48000],
  channelCounts: [2],
  encodings: [2, 21, 4],
  identity: { kind: 'usb', productName: 'KA3' },
};
const speaker = {
  id: 2,
  type: 2,
  kind: 'speaker',
  productName: 'NX809J',
  address: '',
  sampleRates: [],
  channelCounts: [],
  encodings: [],
  identity: { kind: 'speaker', productName: 'NX809J' },
};
const output = { encoding: 21, sampleRate: 48000, channelCount: 2, offload: false };

test('a device is its name and kind; the built-in speaker and an unnamed device are their kind', () => {
  assert.equal(deviceLabel(usb), 'KA3（USB）');
  assert.equal(deviceLabel(speaker), '內建喇叭');
  assert.equal(deviceLabel({ kind: 'bluetooth', productName: '  ' }), '藍牙');
  assert.equal(deviceLabel({ kind: 'mystery', productName: 'X' }), 'X（其他裝置）');
});

test('an idle row lists every declared rate and the channels, in the Windows list format, no bit depth', () => {
  assert.equal(describeSupport(usb), '44.1, 48, 96 kHz．2 聲道');
  assert.equal(describeSupport(speaker), '格式不限');
  for (const kind of ['bluetooth', 'bleAudio', 'hearingAid']) {
    assert.equal(describeSupport({ kind, sampleRates: [], channelCounts: [] }), '由藍牙編碼決定', kind);
  }
  assert.equal(describeSupport({ kind: 'bluetooth', sampleRates: [48000], channelCounts: [2] }), '48 kHz．2 聲道', 'declared values still win');
  assert.equal(describeSupport({ kind: 'usb', sampleRates: [], channelCounts: [] }), '格式不限');
  assert.equal(describeSupport({ sampleRates: [48000], channelCounts: [] }), '48 kHz．聲道數不限');
  assert.equal(describeSupport({ sampleRates: [], channelCounts: [1, 2] }), '取樣率不限．1, 2 聲道');
});

const ldac = { codec: 'LDAC', sampleRate: 96000, bitsPerSample: 32, channels: 2, ldacKbps: 990, ldacAdaptive: false };

test('the system default row comes first and shows what plays when nothing is chosen', () => {
  const rows = buildAndroidRows({ devices: [usb, speaker], chosenId: null, pinnedId: null, routed: speaker, output }, null);
  assert.deepEqual(rows.map((row) => row.key), ['system', 'device-57', 'device-2']);
  assert.equal(rows[0].chosen, true);
  assert.equal(rows[0].sub, '目前是：內建喇叭');
  assert.equal(rows[0].path?.text, '48 kHz．2 聲道');
  assert.equal(rows[0].path?.tone, 'ok');
  assert.equal(rows[1].chosen, false);
  assert.equal(rows[1].path, null);
  assert.equal(rows[1].capabilities, '44.1, 48, 96 kHz．2 聲道');
  // Nothing playing: no path line, every device row says what it supports.
  const idle = buildAndroidRows({ devices: [usb, speaker], chosenId: null, pinnedId: null, routed: speaker, output: null }, null);
  assert.equal(idle[0].path, null);
  assert.equal(idle[0].sub, '目前是：內建喇叭');
});

test('the chosen device carries the actual output; playing elsewhere is a warning', () => {
  const saved = { kind: 'usb', productName: 'KA3' };
  const rows = buildAndroidRows({ devices: [usb, speaker], chosenId: 57, pinnedId: 57, routed: usb, output }, saved);
  assert.equal(rows[0].chosen, false);
  assert.equal(rows[0].sub, '跟隨 Android 的預設播放裝置');
  assert.equal(rows[0].path, null);
  assert.equal(rows[1].chosen, true);
  assert.equal(rows[1].path?.text, '48 kHz．2 聲道');
  assert.equal(rows[1].path?.tone, 'ok');
  assert.equal(rows[1].capabilities, '', 'the playing row has the path line instead of 支援');
  const notPlaying = buildAndroidRows({ devices: [usb, speaker], chosenId: 57, pinnedId: 57, routed: usb, output: null }, saved);
  assert.equal(notPlaying[1].path, null);
  assert.equal(notPlaying[1].capabilities, '44.1, 48, 96 kHz．2 聲道');
  assert.equal(rows.length, 3, 'no missing row while it is attached');
  const elsewhere = buildAndroidRows({ devices: [usb, speaker], chosenId: 57, pinnedId: 57, routed: speaker, output }, saved);
  assert.equal(elsewhere[1].path?.tone, 'warn');
  assert.equal(elsewhere[1].path?.text, '目前由 內建喇叭 播放：48 kHz．2 聲道');
});

test('a chosen device that is unplugged shows which device plays instead, and can be dropped', () => {
  const saved = { kind: 'usb', productName: 'KA3' };
  const rows = buildAndroidRows({ devices: [speaker], chosenId: null, pinnedId: null, routed: speaker, output }, saved);
  const missing = rows.at(-1);
  assert.equal(missing?.key, 'missing');
  assert.equal(missing?.connected, false);
  assert.equal(missing?.chosen, true);
  assert.equal(missing?.label, 'KA3（USB）');
  assert.equal(missing?.path?.text, '已中斷連線，目前改由 內建喇叭 播放');
  assert.equal(missing?.path?.tone, 'error');
  assert.equal(rows[0].chosen, false);
  const unknown = buildAndroidRows({ devices: [speaker], chosenId: null, pinnedId: null, routed: null, output: null }, saved);
  assert.equal(unknown.at(-1)?.path?.text, '已中斷連線');
  // Before the first answer the list is the system default row alone.
  assert.deepEqual(buildAndroidRows(null, null).map((row) => row.key), ['system']);
});

test('two stored choices are the same device only with the same kind, name and address', () => {
  assert.ok(sameChoice(null, null));
  assert.ok(sameChoice({ kind: 'usb', productName: 'KA3' }, { kind: 'usb', productName: 'KA3', address: null }));
  assert.ok(!sameChoice({ kind: 'usb', productName: 'KA3' }, null));
  assert.ok(!sameChoice({ kind: 'bluetooth', productName: 'B', address: '1' }, { kind: 'bluetooth', productName: 'B', address: '2' }));
});

test('Android shows its own device list in Settings → playback; the Windows-only parts stay Windows-only', () => {
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  const component = readFileSync(new URL('../src/lib/AndroidOutputDevicesSettings.svelte', import.meta.url), 'utf8');
  assert.equal(app.match(/<AndroidOutputDevicesSettings\b/g)?.length, 1, 'inside the shared settingsPanels snippet once');
  const windows = app.indexOf("{#if capabilities?.platform === 'windows'}\n                <OutputDevicesSettings");
  const android = app.indexOf("{:else if capabilities?.platform === 'android'}\n                <AndroidOutputDevicesSettings />");
  assert.ok(windows > 0 && android > windows, 'Windows list (with DSEE HX) or the Android list');
  // Shared state lives in the module script (both copies draw the same list).
  assert.match(component, /<script module lang="ts">[\s\S]*const s = \$state\(/);
  assert.match(component, /<div class="output-device-list" role="radiogroup"/);
  assert.match(component, /type="radio"/);
  // The 共享／獨佔 mode group is the Windows one (ADR 0047); presets, volume sync, channel map, delay and DSEE HX stay
  // Windows-only.
  assert.match(component, /<div class="output-mode" role="group" aria-label=\{tr\('輸出模式'\)\}>/);
  assert.match(component, /class="layout-choice-button"[\s\S]*aria-pressed=\{usbExclusive\.enabled === mode\.exclusive\}/);
  for (const windowsOnly of ['方案', '音量同步', '聲道映射', '延遲', 'DSEE']) {
    assert.ok(!component.includes(windowsOnly), `${windowsOnly} is not on Android`);
  }
  assert.doesNotMatch(component, /\btitle=/, 'tooltips use appTooltip');
});

const headset = {
  id: 31,
  type: 8,
  kind: 'bluetooth',
  productName: 'WF-1000XM6',
  address: 'aa:bb:cc:dd:ee:ff',
  sampleRates: [48000],
  channelCounts: [2],
  encodings: [2],
  identity: { kind: 'bluetooth', productName: 'WF-1000XM6', address: 'AA:BB:CC:DD:EE:FF' },
};
test('a Bluetooth route puts the codec in the path; a missing permission keeps its own line', () => {
  const playing = { ...output, sampleRate: 96000, mixerSampleRate: 48000 };
  const base = { devices: [headset, speaker], chosenId: null, pinnedId: null, routed: headset, output: playing };
  const known = buildAndroidRows({ ...base, bluetoothPermission: 'granted', bluetoothCodecs: { 'AA:BB:CC:DD:EE:FF': ldac } }, null);
  assert.equal(known[0].path?.text, '96 kHz → 48 kHz（系統） → 96 kHz（LDAC 990 kbps）．2 聲道');
  assert.equal(known[1].capabilities, '48 kHz．2 聲道');
  assert.equal(known[1].bluetooth, null, 'no separate codec line');
  assert.equal(known[2].bluetooth, null, 'the speaker has no codec line');
  const needed = buildAndroidRows({ ...base, bluetoothPermission: 'needed' }, null);
  assert.deepEqual(needed[1].bluetooth, { text: '需要『附近的裝置』權限才能讀取藍牙編碼', needsPermission: true });
  assert.equal(needed[0].path?.text, '96 kHz → 48 kHz（系統） → （藍牙）．2 聲道');
  // Not reported yet, or Android 12 and older: 藍牙 without a rate, no permission line.
  const pending = buildAndroidRows({ ...base, bluetoothPermission: 'granted', bluetoothCodecs: {} }, null);
  assert.equal(pending[0].path?.text, '96 kHz → 48 kHz（系統） → （藍牙）．2 聲道');
  assert.equal(pending[1].bluetooth, null);
  assert.equal(buildAndroidRows({ ...base, bluetoothPermission: 'unavailable' }, null)[1].bluetooth, null);
  assert.equal(buildAndroidRows(base, null)[1].bluetooth, null);
});

test('the permission is asked for only from the explanation panel', () => {
  const component = readFileSync(new URL('../src/lib/AndroidOutputDevicesSettings.svelte', import.meta.url), 'utf8');
  assert.equal(component.match(/android_request_bluetooth_connect/g)?.length, 1);
  const request = component.indexOf("invokeCommand('android_request_bluetooth_connect'");
  assert.ok(request > component.indexOf('async function authorizeBluetooth()'), 'only inside the panel action');
  assert.match(component, /onclick=\{openPermissionPanel\}>\{tr\('前往授權'\)\}/);
  assert.match(component, /onclick=\{\(\) => void authorizeBluetooth\(\)\}/);
  const manifest = readFileSync(new URL('../plugins/tauri-plugin-media-index/android/src/main/AndroidManifest.xml', import.meta.url), 'utf8');
  assert.match(manifest, /android\.permission\.BLUETOOTH_CONNECT/);
});

test('the chosen Bluetooth row shows the path while a track plays, whatever the routing report says', () => {
  const saved = headset.identity;
  const playing = { ...output, sampleRate: 96000, mixerSampleRate: 48000 };
  const codecs = { bluetoothPermission: 'granted', bluetoothCodecs: { 'AA:BB:CC:DD:EE:FF': { ...ldac, ldacKbps: null } } };
  const base = { devices: [headset, speaker], chosenId: 31, pinnedId: 31, output: playing, ...codecs };
  // Routed to the chosen device: its row has the path, the system default row only its subtitle.
  const routed = buildAndroidRows({ ...base, routed: headset }, saved);
  assert.equal(routed[1].path?.text, '96 kHz → 48 kHz（系統） → 96 kHz（LDAC）．2 聲道');
  assert.equal(routed[1].path?.tone, 'ok');
  assert.equal(routed[1].capabilities, '');
  assert.equal(routed[0].path, null);
  // No routed device reported: the chosen, attached device still carries the path (it is where the player is pinned).
  const unknown = buildAndroidRows({ ...base, routed: null }, saved);
  assert.equal(unknown[1].path?.text, '96 kHz → 48 kHz（系統） → 96 kHz（LDAC）．2 聲道');
  assert.equal(unknown[1].capabilities, '');
  // The same headset reported under another id is not "playing elsewhere".
  const twin = buildAndroidRows({ ...base, routed: { ...headset, id: 32 } }, saved);
  assert.equal(twin[1].path?.tone, 'ok');
  // Another device really plays: a warning naming it.
  const other = buildAndroidRows({ ...base, routed: speaker }, saved);
  assert.equal(other[1].path?.tone, 'warn');
});

test('while paused the system route names 「目前是」 but never warns that the chosen device plays elsewhere', () => {
  const paused = { ...output, sampleRate: 96000, mixerSampleRate: 48000 };
  // Nothing chosen, paused, a headset just connected: 「目前是」 follows the system's media route.
  const followed = buildAndroidRows({ devices: [headset, speaker], chosenId: null, pinnedId: null, routed: headset, routedFrom: 'system', output: paused }, null);
  assert.equal(followed[0].sub, '目前是：WF-1000XM6（藍牙）');
  // The headset is chosen and pinned; the system's route (the speaker) does not follow the pin: no warning.
  const pinned = buildAndroidRows({ devices: [headset, speaker], chosenId: 31, pinnedId: 31, routed: speaker, routedFrom: 'system', output: paused }, headset.identity);
  assert.equal(pinned[1].path?.tone, 'ok');
  assert.equal(pinned[1].path?.text, '96 kHz → 48 kHz（系統） → （藍牙）．2 聲道');
  // A playing track routed elsewhere still warns.
  const playing = buildAndroidRows({ devices: [headset, speaker], chosenId: 31, pinnedId: 31, routed: speaker, routedFrom: 'track', output: paused }, headset.identity);
  assert.equal(playing[1].path?.tone, 'warn');
});

/** A USB exclusive output that plays the track (ADR 0047), as the service reports it. */
const engine = (overrides = {}) => ({
  state: 'streaming',
  rate: 44_100,
  rateReadBack: 44_100,
  channels: 2,
  alt: 6,
  subslotBytes: 2,
  bits: 16,
  sourceBits: 16,
  dsd: false,
  notBitPerfect: [],
  bitPerfect: true,
  gain: 1,
  underruns: 0,
  feedbackAvg: 5.5123,
  urbErrors: 0,
  packetErrors: 0,
  error: null,
  handedBack: null,
  handBackErrors: [],
  ...overrides,
});
const exclusiveStatus = (exclusive, extra = {}) => ({
  devices: [usb, speaker],
  chosenId: 57,
  pinnedId: 57,
  routed: speaker,
  routedFrom: 'system',
  output,
  exclusive,
  ...extra,
});

test('the chosen USB row shows the exclusive line while the app drives the device', () => {
  const saved = { kind: 'usb', productName: 'KA3' };
  const playing = buildAndroidRows(exclusiveStatus({ enabled: true, active: true, state: 'streaming', fallbackReason: null, engine: engine() }), saved);
  assert.deepEqual(playing[1].path, {
    text: '44.1 kHz → 44.1 kHz（獨佔）．2 聲道．16bit．位元完美',
    tone: 'ok',
    notice: null,
  });
  assert.equal(playing[1].capabilities, '');
  // The device is gone from Android's list while it is held, but routing never raises 「目前由…」 for it.
  const tracked = buildAndroidRows(
    exclusiveStatus({ enabled: true, active: true, state: 'streaming', fallbackReason: null, engine: engine() }, { routedFrom: 'track' }),
    saved,
  );
  assert.equal(tracked[1].path?.tone, 'ok');
  const quiet = buildAndroidRows(
    exclusiveStatus({ enabled: true, active: true, state: 'paused', fallbackReason: null, engine: engine({ notBitPerfect: ['volume'], bitPerfect: false, gain: 0.5 }) }),
    saved,
  );
  assert.equal(quiet[1].path?.text, '44.1 kHz → 44.1 kHz（獨佔）．2 聲道．16bit');
  assert.equal(quiet[1].path?.notice, '播放器音量低於 100%，已套用軟體音量；調到 100% 才會原樣送出（不是位元完美）');
  const preparing = usbExclusivePath(exclusiveStatus({ enabled: true, active: true, state: 'claimed', fallbackReason: null, engine: engine({ rate: null }) }), null);
  assert.equal(preparing?.tone, 'idle');
  assert.equal(preparing?.text, 'USB 獨佔：準備中');
  const away = usbExclusivePath(exclusiveStatus({ enabled: true, active: true, state: 'handedBack', fallbackReason: null, engine: engine({ rate: null }) }), null);
  assert.equal(away?.text, 'USB 獨佔：裝置已暫時交還系統，播放時再取得');
});

test('a track the USB output cannot play keeps the Android path with the reason', () => {
  const saved = { kind: 'usb', productName: 'KA3' };
  const rows = buildAndroidRows(
    exclusiveStatus({ enabled: true, active: false, state: 'handedBack', fallbackReason: 'USB 音訊裝置不支援 705600 Hz', engine: null }, { routed: usb }),
    saved,
  );
  assert.deepEqual(rows[1].path, {
    text: '48 kHz．2 聲道',
    tone: 'warn',
    notice: 'USB 獨佔無法播放這首：USB 音訊裝置不支援 705600 Hz',
  });
  // Switch off: the row is the plain Android row again.
  const off = buildAndroidRows(exclusiveStatus({ enabled: false, active: false, state: null, fallbackReason: 'x', engine: null }, { routed: usb }), saved);
  assert.deepEqual(off[1].path, { text: '48 kHz．2 聲道', tone: 'ok' });
  assert.equal(usbExclusivePath(null, '48 kHz'), null);
  // Only the chosen USB row takes the exclusive line.
  const speakerChosen = buildAndroidRows(
    { ...exclusiveStatus({ enabled: true, active: true, state: 'streaming', fallbackReason: null, engine: engine() }), chosenId: 2 },
    { kind: 'speaker', productName: 'NX809J' },
  );
  assert.equal(speakerChosen[1].path, null);
});

test('USB exclusive is off by default on both sides, and turned on only through its explanation panel', () => {
  assert.equal(DEFAULT_ANDROID_USB_EXCLUSIVE, false);
  const settings = readFileSync(new URL('../src-tauri/src/settings.rs', import.meta.url), 'utf8');
  assert.match(settings, /android_usb_exclusive: false,/);
  assert.match(settings, /#\[serde\(default\)\]\s+pub android_usb_exclusive: bool,/);
  const component = readFileSync(new URL('../src/lib/AndroidOutputDevicesSettings.svelte', import.meta.url), 'utf8');
  assert.equal(component.match(/android_request_usb_exclusive_permission/g)?.length, 1);
  const request = component.indexOf("invokeCommand('android_request_usb_exclusive_permission'");
  assert.ok(request > component.indexOf('async function authorizeUsbExclusive()'), 'only inside the panel action');
  assert.match(component, /onclick=\{\(\) => void authorizeUsbExclusive\(\)\}/);
  // The mode buttons replace the per-card switch; the P0 development test is gone.
  assert.doesNotMatch(component, /USB 獨佔（App 直接驅動，不經過系統）|toggleUsbExclusive|type="checkbox"/);
  assert.doesNotMatch(component, /試播|android_usb_exclusive_start/);
  // 獨佔 opens the panel (no save before it); 共享 saves at once.
  const setMode = component.slice(component.indexOf('function setMode('), component.indexOf('function openUsbExclusivePanel('));
  assert.match(setMode, /if \(!exclusive\) \{[\s\S]*saveUsbExclusive\(false\)[\s\S]*openUsbExclusivePanel\(\);/);
  assert.doesNotMatch(setMode, /saveUsbExclusive\(true\)/);
  const authorize = component.slice(component.indexOf('async function authorizeUsbExclusive('));
  assert.ok(authorize.indexOf('android_request_usb_exclusive_permission') < authorize.indexOf('saveUsbExclusive(true)'));
});

test('the two output modes and the header follow the Windows page, in Android words', () => {
  const [shared, exclusive] = androidOutputModes();
  assert.deepEqual(shared, {
    exclusive: false,
    title: '共享',
    description: '與其他 App 一起播放，由 Android 系統混音；系統可能轉換取樣率，不是位元完美。',
  });
  assert.deepEqual(exclusive, {
    exclusive: true,
    title: '獨佔',
    description: '接上 USB 音訊裝置時由本程式直接驅動，不經 Android 系統，取樣率與位元深度照檔案原樣送出（位元完美）。其他 App 與通知音會改從其他裝置出聲；藍牙與內建喇叭不支援，照常以共享方式播放。',
  });
  assert.match(androidOutputHeader(false), /^選擇要輸出的裝置；[\s\S]*不是位元完美。$/);
  assert.match(androidOutputHeader(true), /^選「系統預設」或 USB 音訊裝置時，播放到 USB 音訊裝置由本程式獨佔/);
  assert.notEqual(androidOutputHeader(true), androidOutputHeader(false));
});

const ka3Route = (extra = {}) => ({ usbDevices: 1, noUsbDevice: false, usbDevice: { key: '/dev/bus/usb/001/002', productName: 'KA3', permitted: true }, ...extra });
const systemStatus = (exclusive, extra = {}) => ({
  devices: [usb, speaker],
  chosenId: null,
  pinnedId: null,
  routed: speaker,
  routedFrom: 'system',
  output,
  exclusive,
  ...extra,
});

test('in the exclusive mode the system default plays on the USB device: its card shows the line, 「目前是」 names it', () => {
  const rows = buildAndroidRows(
    systemStatus({ enabled: true, active: true, state: 'streaming', fallbackReason: null, engine: engine(), route: ka3Route() }),
    null,
    true,
  );
  assert.equal(rows[0].chosen, true);
  assert.equal(rows[0].sub, '目前是：KA3（USB）．USB 獨佔');
  assert.equal(rows[0].path, null);
  assert.equal(rows[0].hint, null);
  assert.equal(rows[1].chosen, false);
  assert.deepEqual(rows[1].path, { text: '44.1 kHz → 44.1 kHz（獨佔）．2 聲道．16bit．位元完美', tone: 'ok', notice: null });
  assert.equal(rows[1].capabilities, '');
  assert.equal(rows[2].path, null);
  // The shared mode: the system default row is the plain Android row.
  const shared = buildAndroidRows(
    systemStatus({ enabled: false, active: false, state: null, fallbackReason: null, engine: null, route: ka3Route() }),
    null,
    false,
  );
  assert.equal(shared[0].sub, '目前是：內建喇叭');
  assert.deepEqual(shared[0].path, { text: '48 kHz．2 聲道', tone: 'ok' });
  assert.equal(shared[0].hint, null);
  assert.equal(shared[1].path, null);
  // A track the USB device cannot take: the Android path with the reason on the system default card.
  const fallback = buildAndroidRows(
    systemStatus({ enabled: true, active: false, state: 'handedBack', fallbackReason: 'USB 音訊裝置不支援 705600 Hz', engine: null, route: ka3Route() }),
    null,
    true,
  );
  assert.deepEqual(fallback[0].path, { text: '48 kHz．2 聲道', tone: 'warn', notice: 'USB 獨佔無法播放這首：USB 音訊裝置不支援 705600 Hz' });
  assert.equal(fallback[1].path, null);
});

test('the exclusive target is the chosen USB card, or for the system default the named or the first USB card', () => {
  const ka3b = { ...usb, id: 58, productName: 'Sony Audio', identity: { kind: 'usb', productName: 'Sony Audio' } };
  const status = (route, extra = {}) => systemStatus({ enabled: true, active: false, state: null, fallbackReason: null, engine: null, route }, extra);
  assert.equal(exclusiveTargetDevice(status(ka3Route(), { devices: [ka3b, usb, speaker] }), null)?.id, 57);
  assert.equal(exclusiveTargetDevice(status(ka3Route({ usbDevice: { key: 'k', productName: 'Other', permitted: true } }), { devices: [ka3b, usb] }), null)?.id, 58);
  assert.equal(exclusiveTargetDevice(status(ka3Route(), { chosenId: 57 }), { kind: 'usb', productName: 'KA3' })?.id, 57);
  assert.equal(exclusiveTargetDevice(status(ka3Route(), { chosenId: 2 }), { kind: 'speaker', productName: 'NX809J' }), null);
  assert.equal(exclusiveTargetDevice(status({ usbDevices: 0, noUsbDevice: true, usbDevice: null }), null), null);
});

test('the exclusive mode hints of the routing table', () => {
  const status = (route) => systemStatus({ enabled: true, active: false, state: null, fallbackReason: null, engine: null, route });
  const none = { usbDevices: 0, noUsbDevice: true, usbDevice: null };
  // No USB audio device for the system default or a chosen USB device: shared as chosen.
  assert.deepEqual(exclusiveHint(status(none), null, true, 'system'), { text: '獨佔模式：沒有 USB 音訊裝置，以共享方式播放', needsPermission: false });
  assert.deepEqual(exclusiveHint(status(none), { kind: 'usb', productName: 'KA3' }, true, 'missing'), { text: '獨佔模式：沒有 USB 音訊裝置，以共享方式播放', needsPermission: false });
  // A non-USB device chosen: shared on it.
  assert.deepEqual(
    exclusiveHint(status({ usbDevices: 1, noUsbDevice: false, usbDevice: null }), { kind: 'speaker', productName: 'NX809J' }, true, 'device', speaker),
    { text: '獨佔模式：這台裝置不支援獨佔，以共享方式播放', needsPermission: false },
  );
  // Several USB devices for the system default: which one.
  assert.deepEqual(exclusiveHint(status(ka3Route({ usbDevices: 2 })), null, true, 'system'), { text: '獨佔模式：有多台 USB 音訊裝置，由第一台 KA3 獨佔播放', needsPermission: false });
  assert.equal(exclusiveHint(status(ka3Route()), null, true, 'system'), null);
  // A USB device attached after the mode was chosen, without permission: the card asks.
  assert.deepEqual(
    exclusiveHint(status(ka3Route({ usbDevice: { key: 'k', productName: 'KA3', permitted: false } })), null, true, 'system'),
    { text: '獨佔模式需要 USB 權限才能直接驅動這台裝置', needsPermission: true },
  );
  // The shared mode says nothing.
  assert.equal(exclusiveHint(status(none), null, false, 'system'), null);
  // The rows carry it: the no-USB hint on the chosen system default card.
  const rows = buildAndroidRows(
    { ...status(none), devices: [speaker] },
    null,
    true,
  );
  assert.equal(rows[0].hint?.text, '獨佔模式：沒有 USB 音訊裝置，以共享方式播放');
  assert.equal(rows[1].hint, null);
});
