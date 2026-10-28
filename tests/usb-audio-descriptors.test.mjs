import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  describeEndpoint,
  describeStream,
  describeSync,
  describeUsbDevice,
  spacedHex,
} from '../src/lib/usb-audio-descriptors.js';
import { chooseCardOnClick } from '../src/lib/output-device-card.js';

const out = { address: 0x01, direction: 'out', transfer: 'isochronous', sync: 'async', usage: 'data', maxPacketSize: 1024, extraTransactions: 0, interval: 1 };
const feedback = { address: 0x81, direction: 'in', transfer: 'isochronous', sync: 'none', usage: 'feedback', maxPacketSize: 4, extraTransactions: 0, interval: 4 };

/** The user's Sony "Sony Audio" (054C:0B8C) as the Kotlin parser answers it. */
const sony = {
  name: '/dev/bus/usb/001/002',
  vendorId: 0x054c,
  productId: 0x0b8c,
  manufacturer: 'Sony',
  product: 'Sony Audio',
  permission: 'granted',
  uacVersion: 2,
  controls: {
    featureUnits: [{ masterMute: true, masterVolume: true, channelMute: false, channelVolume: false }],
    clockSources: [{ id: 0x29, type: 'internalProgrammable', frequencyControl: 3 }],
  },
  streams: [
    { interface: 1, alt: 0, terminalLink: null, format: 'unknown', formatType: null, subslotBytes: null, bits: null, channels: null, rates: null, rateRange: null, endpoints: [] },
    { interface: 1, alt: 1, terminalLink: 2, format: 'pcm', formatType: 1, subslotBytes: 2, bits: 16, channels: 2, rates: null, rateRange: null, endpoints: [out, feedback] },
    { interface: 1, alt: 6, terminalLink: 2, format: 'raw', formatType: 1, subslotBytes: 4, bits: 32, channels: 2, rates: null, rateRange: null, endpoints: [out, feedback] },
  ],
  otherConfigurations: [],
  raw: '1201000200',
};

test('the Sony UAC2 DAC reads as UAC 2, asynchronous with feedback, per alternate setting', () => {
  const view = describeUsbDevice(sony);
  assert.equal(view.title, 'Sony Audio（VID 054C／PID 0B8C）');
  assert.equal(view.lines[0], 'UAC 2．非同步（有回饋）');
  assert.ok(view.lines.includes('硬體音量：有．靜音：有'));
  assert.ok(view.lines.includes('時脈：內部可程式（可設定頻率）'));
  assert.ok(view.lines.includes('取樣率由時脈來源在播放時提供，未讀取'));
  assert.equal(view.streams[0], '設定 0：零頻寬（閒置）');
  assert.equal(
    view.streams[1],
    '設定 1：PCM．16bit（2 byte）．2 聲道（端點 0x01 輸出．非同步．1024 byte．間隔 1；端點 0x81 回饋．4 byte．間隔 4）',
  );
  assert.match(view.streams[2], /^設定 6：原始資料（DSD 等）．32bit（4 byte）．2 聲道/);
  assert.equal(view.raw, '1201000200');
});

test('UAC1 rates are listed, a range is low–high, a capture stream says so', () => {
  const discrete = { ...sony.streams[1], rates: [44100, 48000], endpoints: [{ ...out, sync: 'adaptive', maxPacketSize: 196 }] };
  assert.equal(describeStream(discrete), '設定 1：PCM．16bit（2 byte）．2 聲道．44.1, 48 kHz（端點 0x01 輸出．自適應．196 byte．間隔 1）');
  const range = { ...discrete, rates: null, rateRange: [8000, 96000] };
  assert.match(describeStream(range), /．8–96 kHz（/);
  const capture = { ...discrete, endpoints: [{ ...out, address: 0x82, direction: 'in' }] };
  assert.match(describeStream(capture), /．錄音（端點 0x82 輸入/);
});

test('sync types: adaptive, async without a feedback endpoint, no isochronous stream', () => {
  assert.equal(describeSync([{ ...sony.streams[1], endpoints: [{ ...out, sync: 'adaptive' }] }]), '自適應');
  assert.equal(describeSync([{ ...sony.streams[1], endpoints: [out] }]), '非同步（無回饋端點）');
  assert.equal(describeSync([sony.streams[0]]), null);
});

test('high-bandwidth endpoints show the transactions per microframe', () => {
  assert.equal(describeEndpoint({ ...out, extraTransactions: 2 }), '端點 0x01 輸出．非同步．1024 byte ×3．間隔 1');
});

test('permission and read failures are one line, without streams or raw bytes', () => {
  assert.deepEqual(describeUsbDevice({ ...sony, permission: 'denied' }).lines, ['未取得 USB 權限']);
  assert.deepEqual(describeUsbDevice({ ...sony, permission: 'notAsked' }).streams, []);
  assert.deepEqual(describeUsbDevice({ ...sony, error: 'unreadable', raw: undefined }).lines, ['無法讀取 USB 描述元']);
  const unnamed = describeUsbDevice({ ...sony, product: null, uacVersion: null, streams: [], controls: null });
  assert.equal(unnamed.title, '/dev/bus/usb/001/002（VID 054C／PID 0B8C）');
  assert.equal(unnamed.lines[0], '不是 USB Audio 裝置');
});

test('the raw hex is shown in byte pairs', () => {
  assert.equal(spacedHex('1201000200'), '12 01 00 02 00');
  assert.equal(spacedHex(''), '');
});

test('the USB reader area and its 原始描述元 summary never choose the card', () => {
  for (const inside of ['summary', '[data-own-click]']) {
    const input = { clicks: 0, click() { this.clicks += 1; } };
    const target = { closest: (/** @type {string} */ selector) => (selector.split(', ').includes(inside) ? {} : null) };
    chooseCardOnClick({ currentTarget: { querySelector: () => input }, target });
    assert.equal(input.clicks, 0, inside);
  }
  const component = readFileSync(new URL('../src/lib/AndroidOutputDevicesSettings.svelte', import.meta.url), 'utf8');
  assert.match(component, /class="output-device-usb" data-own-click/);
  // Permission is asked only from the button (request: true), never on load or by the poll.
  assert.equal(component.match(/android_usb_audio_descriptors/g)?.length, 1);
  assert.match(component, /invokeCommand\('android_usb_audio_descriptors', \{ request: true \}\)/);
});
