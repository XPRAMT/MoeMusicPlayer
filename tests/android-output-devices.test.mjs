import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  buildAndroidRows,
  describePlayerOutput,
  describeCapabilities,
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

test('what a device takes reads like the Windows rows, and an empty list is "any"', () => {
  assert.equal(describeCapabilities(usb), '44.1／48／96 kHz．2 聲道．16／24 位元／浮點');
  assert.equal(describeCapabilities(speaker), '格式不限');
  assert.equal(
    describeCapabilities({ sampleRates: [48000], channelCounts: [], encodings: [] }),
    '48 kHz．聲道數不限．位元深度不限',
  );
  assert.equal(
    describeCapabilities({ sampleRates: [], channelCounts: [1, 2], encodings: [22] }),
    '取樣率不限．1／2 聲道．32 位元',
  );
  // Only compressed encodings (passthrough): no bit depth to show.
  assert.equal(describeCapabilities({ sampleRates: [48000], channelCounts: [2], encodings: [5] }), '48 kHz．2 聲道');
});

test('the actual output names the format the AudioTrack opened and where it is routed', () => {
  assert.equal(describePlayerOutput(output, usb), '播放器輸出：48 kHz．24 位元．2 聲道 → KA3');
  assert.equal(describePlayerOutput({ ...output, encoding: 4, sampleRate: 44100 }, null), '播放器輸出：44.1 kHz．32 位元浮點．2 聲道');
  assert.equal(describePlayerOutput(null, usb), '音訊輸出尚未開啟。');
  assert.doesNotMatch(describePlayerOutput(output, usb), /位元完美|bit-perfect/);
});

test('the system default row comes first and shows what plays when nothing is chosen', () => {
  const rows = buildAndroidRows({ devices: [usb, speaker], chosenId: null, pinnedId: null, routed: speaker, output }, null);
  assert.deepEqual(rows.map((row) => row.key), ['system', 'device-57', 'device-2']);
  assert.equal(rows[0].chosen, true);
  assert.equal(rows[0].sub, '目前是：內建喇叭');
  assert.equal(rows[0].path?.text, '播放器輸出：48 kHz．24 位元．2 聲道 → 內建喇叭');
  assert.equal(rows[0].path?.tone, 'ok');
  assert.equal(rows[1].chosen, false);
  assert.equal(rows[1].path, null);
  assert.equal(rows[1].capabilities, "系統宣告：44.1／48／96 kHz．2 聲道．16／24 位元／浮點");
});

test('the chosen device carries the actual output; playing elsewhere is a warning', () => {
  const saved = { kind: 'usb', productName: 'KA3' };
  const rows = buildAndroidRows({ devices: [usb, speaker], chosenId: 57, pinnedId: 57, routed: usb, output }, saved);
  assert.equal(rows[0].chosen, false);
  assert.equal(rows[0].sub, '跟隨 Android 的預設播放裝置');
  assert.equal(rows[0].path, null);
  assert.equal(rows[1].chosen, true);
  assert.equal(rows[1].path?.tone, 'ok');
  assert.equal(rows.length, 3, 'no missing row while it is attached');
  const elsewhere = buildAndroidRows({ devices: [usb, speaker], chosenId: 57, pinnedId: 57, routed: speaker, output }, saved);
  assert.equal(elsewhere[1].path?.tone, 'warn');
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
  for (const windowsOnly of ['共享', '獨佔', '方案', '音量同步', '聲道映射', '延遲', 'DSEE']) {
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
const ldac = { codec: 'LDAC', sampleRate: 96000, bitsPerSample: 32, channels: 2, ldacKbps: 990, ldacAdaptive: false };

test('the Bluetooth codec line names the codec, its rate and bits, and LDAC quality', async () => {
  const { describeBluetoothCodec } = await import('../src/lib/android-output-devices.js');
  assert.equal(describeBluetoothCodec(ldac), '藍牙編碼：LDAC．96 kHz．32 位元．990 kbps');
  assert.equal(
    describeBluetoothCodec({ ...ldac, ldacKbps: null, ldacAdaptive: true }),
    '藍牙編碼：LDAC．96 kHz．32 位元．最佳效果（自動）',
  );
  assert.equal(
    describeBluetoothCodec({ codec: 'AAC', sampleRate: 44100, bitsPerSample: 16, channels: 2, ldacKbps: null, ldacAdaptive: false }),
    '藍牙編碼：AAC．44.1 kHz．16 位元',
  );
  assert.equal(
    describeBluetoothCodec({ codec: 'SBC', sampleRate: null, bitsPerSample: null, channels: null, ldacKbps: null, ldacAdaptive: false }),
    '藍牙編碼：SBC',
  );
});

test('a Bluetooth row shows the codec, asks for 附近的裝置 when needed, and says nothing otherwise', () => {
  const base = { devices: [headset, speaker], chosenId: null, pinnedId: null, routed: headset, output };
  const known = buildAndroidRows({ ...base, bluetoothPermission: 'granted', bluetoothCodecs: { 'AA:BB:CC:DD:EE:FF': ldac } }, null);
  assert.equal(known[1].capabilities, '系統宣告：48 kHz．2 聲道．16 位元');
  assert.deepEqual(known[1].bluetooth, { text: '藍牙編碼：LDAC．96 kHz．32 位元．990 kbps', needsPermission: false });
  assert.equal(known[0].path?.text, '播放器輸出：48 kHz．24 位元．2 聲道 → WF-1000XM6');
  assert.equal(known[2].bluetooth, null, 'the speaker has no codec line');
  const needed = buildAndroidRows({ ...base, bluetoothPermission: 'needed' }, null);
  assert.deepEqual(needed[1].bluetooth, { text: '需要『附近的裝置』權限才能讀取藍牙編碼', needsPermission: true });
  // Not reported yet, or Android 12 and older: no line.
  assert.equal(buildAndroidRows({ ...base, bluetoothPermission: 'granted', bluetoothCodecs: {} }, null)[1].bluetooth, null);
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
