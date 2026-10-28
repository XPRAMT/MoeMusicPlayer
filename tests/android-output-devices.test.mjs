import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  buildAndroidRows,
  describeSupport,
  deviceLabel,
  sameChoice,
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
  // The USB exclusive test (ADR 0046) is its own development-only control, not the Windows shared/exclusive mode.
  const shown = component.replace(/USB 獨佔試播/g, '');
  for (const windowsOnly of ['共享', '獨佔', '方案', '音量同步', '聲道映射', '延遲', 'DSEE']) {
    assert.ok(!shown.includes(windowsOnly), `${windowsOnly} is not on Android`);
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
