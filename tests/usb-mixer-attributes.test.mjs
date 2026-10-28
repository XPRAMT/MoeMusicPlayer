import assert from 'node:assert/strict';
import test from 'node:test';
import { describeMixerAttributes, describeMixerFormat } from '../src/lib/usb-audio-descriptors.js';

/** @param {string} behavior @param {string} encoding @param {number | null} bits @param {number[]} sampleRates */
function group(behavior, encoding, bits, sampleRates, extra = {}) {
  return {
    behavior,
    behaviorValue: behavior === 'bitPerfect' ? 1 : 0,
    encoding,
    encodingValue: { pcm16: 2, pcm32: 22, pcmFloat: 4 }[encoding] ?? 5,
    bits,
    channelMask: 12,
    channels: 2,
    sampleRates,
    ...extra,
  };
}

/** @param {object} device */
function result(device) {
  return { supported: true, devices: [{ id: 7, type: 11, productName: 'Sony Audio', bitPerfect: false, preferred: null, ...device }] };
}

test('a group lists its rates with the output-format separators', () => {
  assert.equal(
    describeMixerFormat(group('bitPerfect', 'pcm32', 32, []), [192000, 48000, 96000, 48000, 0]),
    '位元完美：PCM 32bit．48, 96, 192 kHz．2 聲道',
  );
  assert.equal(describeMixerFormat(group('default', 'pcm16', 16, []), [48000]), '一般：PCM 16bit．48 kHz．2 聲道');
  assert.equal(describeMixerFormat(group('default', 'pcmFloat', 32, []), [44100]), '一般：浮點 32bit．44.1 kHz．2 聲道');
});

test('unknown behavior and encoding show their values', () => {
  const odd = { ...group('unknown', 'other', null, []), behaviorValue: 9, encodingValue: 5, channels: null };
  assert.equal(describeMixerFormat(odd, []), '未知模式（9）：編碼 5');
});

test('bit-perfect entries come as they are grouped, the preferred one last', () => {
  const view = describeMixerAttributes(result({
    bitPerfect: true,
    groups: [group('bitPerfect', 'pcm32', 32, [48000, 96000, 192000]), group('default', 'pcm16', 16, [48000])],
    preferred: { ...group('bitPerfect', 'pcm32', 32, []), sampleRate: 96000 },
  }));
  assert.equal(view.notice, null);
  assert.deepEqual(view.devices[0].lines, [
    '位元完美：PCM 32bit．48, 96, 192 kHz．2 聲道',
    '一般：PCM 16bit．48 kHz．2 聲道',
    '目前偏好：位元完美：PCM 32bit．96 kHz．2 聲道',
  ]);
});

test('no bit-perfect entry says so and keeps the default entries', () => {
  assert.deepEqual(describeMixerAttributes(result({ groups: [] })).devices[0].lines, ['此系統未提供位元完美模式']);
  assert.deepEqual(describeMixerAttributes(result({ groups: [group('default', 'pcm16', 16, [48000])] })).devices[0].lines, [
    '此系統未提供位元完美模式',
    '一般：PCM 16bit．48 kHz．2 聲道',
  ]);
});

test('below Android 14, no device and a failed query', () => {
  assert.equal(describeMixerAttributes({ supported: false, reason: 'requiresAndroid14', devices: [] }).notice, '需要 Android 14 以上才能查詢');
  assert.equal(describeMixerAttributes({ supported: true, devices: [] }).notice, '沒有找到 USB 播放裝置');
  assert.deepEqual(describeMixerAttributes(result({ groups: [], error: 'java.lang.SecurityException: x' })).devices[0].lines, [
    '無法查詢系統混音器格式：java.lang.SecurityException: x',
  ]);
  assert.deepEqual(describeMixerAttributes(null), { notice: null, devices: [] });
});

test('the page asks for the card device only to read, never to set', async () => {
  const { readFileSync } = await import('node:fs');
  const kotlin = readFileSync(
    new URL('../plugins/tauri-plugin-media-index/android/src/main/java/com/moemusicplayer/mediaindex/UsbAudioReader.kt', import.meta.url),
    'utf8',
  );
  assert.match(kotlin, /getSupportedMixerAttributes/);
  assert.doesNotMatch(kotlin, /setPreferredMixerAttributes|clearPreferredMixerAttributes|AudioTrack/);
});
