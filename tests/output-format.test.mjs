import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  codecStepName,
  describeAndroidPath,
  describeExclusive,
  describeExclusiveFallback,
  describeMirrorPath,
  describeOutputFormat,
  formatSampleRate,
  formatSupportedFormats,
} from '../src/lib/output-format.js';

/** @param {Partial<import('../src/lib/output-format.js').OutputFormat>} overrides */
const format = (overrides) => ({
  mode: 'highQuality',
  sourceRateHz: 44_100,
  outputRateHz: 48_000,
  deviceRateHz: 48_000,
  conversion: 'highQuality',
  fallbackReason: null,
  dseeHx: 'off',
  dseeNotice: null,
  dseeOutputRateHz: null,
  ...overrides,
});
const exclusive = { bits: 24, channels: 2, sourceChannels: 2, bitPerfect: true, volumeApplied: false, underruns: 0 };
const track = (sampleRate, mixerSampleRate) => ({ encoding: 4, sampleRate, channelCount: 2, offload: false, mixerSampleRate });
const ldac = { codec: 'LDAC', sampleRate: 96_000, bitsPerSample: 32, channels: 2, ldacKbps: null, ldacAdaptive: false };
const bt = (codec) => ({ bluetooth: true, codec });

test('sample rates are shown in kHz without trailing zeros', () => {
  assert.equal(formatSampleRate(44_100), '44.1 kHz');
  assert.equal(formatSampleRate(96_000), '96 kHz');
  assert.equal(formatSampleRate(176_400), '176.4 kHz');
  assert.equal(formatSampleRate(22_050), '22.05 kHz');
});

test('a track nothing converts reads the same on both platforms', () => {
  assert.equal(describeOutputFormat(format({ sourceRateHz: 48_000, conversion: 'none' }), 2).text, '48 kHz → 48 kHz（系統）．2 聲道');
  assert.equal(describeAndroidPath(track(48_000, 48_000)), '48 kHz → 48 kHz（系統）．2 聲道');
});

test('Windows names the FFT conversion, a conversion by Windows is the 系統 step alone', () => {
  assert.deepEqual(describeOutputFormat(format({}), 2), { text: '44.1 kHz → 48 kHz（FFT） → 48 kHz（系統）．2 聲道', notice: null });
  assert.equal(
    describeOutputFormat(format({ outputRateHz: 44_100, conversion: 'windows' }), 2).text,
    '44.1 kHz → 48 kHz（系統）．2 聲道',
  );
  assert.equal(describeAndroidPath(track(44_100, 48_000)), '44.1 kHz → 48 kHz（系統）．2 聲道');
  // The basic converter is a different converter: its own step name, and its notice stays.
  assert.deepEqual(describeOutputFormat(format({ conversion: 'basic' }), 2), {
    text: '44.1 kHz → 48 kHz（基本轉換） → 48 kHz（系統）．2 聲道',
    notice: '高品質轉換器無法處理此格式，已改用基本轉換。',
  });
  for (const text of [describeOutputFormat(format({}), 2).text, describeOutputFormat(format({ conversion: 'none', sourceRateHz: 48_000 })).text]) {
    assert.doesNotMatch(text, /原生|未轉換|交由 Windows|高品質/);
  }
});

test('the Windows built-in fallback is still called out below the line', () => {
  const status = describeOutputFormat(format({ mode: 'windowsBuiltin', fallbackReason: 'unsupported format' }), 2);
  assert.equal(status.text, '44.1 kHz → 48 kHz（FFT） → 48 kHz（系統）．2 聲道');
  assert.equal(status.notice, 'Windows 無法以 44.1 kHz 開啟共享模式輸出，已改用 48 kHz 並以高品質轉換。');
});

test('DSEE HX is its own step, before a later FFT conversion', () => {
  assert.equal(
    describeOutputFormat(format({ sourceRateHz: 48_000, dseeHx: 'active', dseeOutputRateHz: 96_000, outputRateHz: 96_000, deviceRateHz: 96_000 }), 2).text,
    '48 kHz → 96 kHz（DSEE HX） → 96 kHz（系統）．2 聲道',
  );
  assert.equal(
    describeOutputFormat(format({ sourceRateHz: 48_000, dseeHx: 'active', dseeOutputRateHz: 96_000, outputRateHz: 192_000, deviceRateHz: 192_000 }), 2).text,
    '48 kHz → 96 kHz（DSEE HX） → 192 kHz（FFT） → 192 kHz（系統）．2 聲道',
  );
  assert.equal(
    describeOutputFormat(format({ dseeHx: 'unavailable', dseeNotice: '請先安裝 Sony Music Center。' })).notice,
    '請先安裝 Sony Music Center。',
  );
});

test('missing output or track has a neutral status', () => {
  assert.equal(describeOutputFormat(null).text, '音訊輸出尚未開啟。');
  assert.deepEqual(describeOutputFormat(format({ sourceRateHz: null, conversion: 'none', outputRateHz: 96_000 })), {
    text: '尚未載入曲目（輸出 96 kHz）',
    notice: null,
  });
  assert.equal(describeMirrorPath(null, 96_000, false), '尚未載入曲目（輸出 96 kHz）');
});

test('a synced device ends with its own 系統 rate, with an FFT step when it converts', () => {
  assert.equal(describeMirrorPath(format({ conversion: 'none', sourceRateHz: 48_000 }), 48_000, false, 2), '48 kHz → 48 kHz（系統）．2 聲道');
  assert.equal(describeMirrorPath(format({}), 48_000, false, 2), '44.1 kHz → 48 kHz（FFT） → 48 kHz（系統）．2 聲道');
  assert.equal(describeMirrorPath(format({}), 96_000, true, 2), '44.1 kHz → 48 kHz（FFT） → 96 kHz（FFT） → 96 kHz（系統）．2 聲道');
  assert.equal(
    describeMirrorPath(format({ sourceRateHz: 48_000, dseeHx: 'active', dseeOutputRateHz: 96_000, outputRateHz: 96_000 }), 96_000, false, 2),
    '48 kHz → 96 kHz（DSEE HX） → 96 kHz（系統）．2 聲道',
  );
});

test('the exclusive stream ends with 獨佔, bit depth and 位元完美, or the reasons as the notice', () => {
  assert.deepEqual(describeExclusive(format({ sourceRateHz: 96_000, outputRateHz: 96_000, exclusive })), {
    text: '96 kHz → 96 kHz（獨佔）．2 聲道．24bit．位元完美',
    notice: null,
  });
  const converted = describeExclusive(format({ sourceRateHz: 44_100, outputRateHz: 96_000, exclusive: { ...exclusive, bitPerfect: false } }));
  assert.equal(converted?.text, '44.1 kHz → 96 kHz（FFT） → 96 kHz（獨佔）．2 聲道．24bit');
  assert.equal(converted?.notice, '裝置不接受 44.1 kHz，已轉換為 96 kHz（不是位元完美）');
  const volume = describeExclusive(format({ sourceRateHz: 96_000, outputRateHz: 96_000, exclusive: { ...exclusive, bitPerfect: false, volumeApplied: true } }));
  assert.equal(volume?.text, '96 kHz → 96 kHz（獨佔）．2 聲道．24bit');
  assert.match(volume?.notice ?? '', /軟體音量/);
  assert.equal(describeExclusive(format({})), null, 'a shared-mode output has no exclusive stream');
});

test('DSD as DoP starts at the DSD rate; a refused DoP says why the track was converted', () => {
  assert.deepEqual(
    describeExclusive(format({ sourceRateHz: 176_400, outputRateHz: 176_400, exclusive: { ...exclusive, dopDsdRateHz: 2_822_400 } })),
    { text: 'DSD64 → 176.4 kHz（DoP） → 176.4 kHz（獨佔）．2 聲道．24bit', notice: null },
  );
  const refused = describeExclusive(format({ sourceRateHz: 176_400, outputRateHz: 176_400, exclusive: { ...exclusive, dopFallback: '這個裝置不接受 24-bit 176400 Hz 的 DoP 格式。' } }));
  assert.equal(refused?.text, '176.4 kHz → 176.4 kHz（獨佔）．2 聲道．24bit．位元完美');
  assert.equal(refused?.notice, 'DoP 無法使用，已轉成 PCM：這個裝置不接受 24-bit 176400 Hz 的 DoP 格式。');
});

test('a track exclusive mode could not take says it plays in shared mode and why', () => {
  assert.equal(describeExclusiveFallback(format({})), null);
  const described = describeExclusiveFallback(format({ conversion: 'none', sourceRateHz: 48_000, exclusiveFallback: '這個裝置正被其他程式使用。' }), 2);
  assert.equal(described?.text, '已改用共享模式：48 kHz → 48 kHz（系統）．2 聲道');
  assert.equal(described?.notice, '獨佔模式無法播放這首：這個裝置正被其他程式使用。');
});

test('Android Bluetooth ends with the codec step', () => {
  // G8142: the mixer runs at the track's rate, LDAC at its best-effort setting.
  assert.equal(describeAndroidPath(track(96_000, 96_000), bt({ ...ldac, ldacAdaptive: true })), '96 kHz → 96 kHz（系統） → 96 kHz（LDAC 最佳效果）．2 聲道');
  // RedMagic / tablet: the mixer at 48 kHz, LDAC without a reported quality (codecSpecific1 0).
  assert.equal(describeAndroidPath(track(96_000, 48_000), bt(ldac)), '96 kHz → 48 kHz（系統） → 96 kHz（LDAC）．2 聲道');
  assert.equal(describeAndroidPath(track(96_000, 48_000), bt({ ...ldac, ldacKbps: 990 })), '96 kHz → 48 kHz（系統） → 96 kHz（LDAC 990 kbps）．2 聲道');
  // The codec not broadcast yet: 藍牙 without a rate.
  assert.equal(describeAndroidPath(track(96_000, 48_000), bt(null)), '96 kHz → 48 kHz（系統） → （藍牙）．2 聲道');
  // The mixer rate unreadable: no 系統 step, never a guessed number.
  assert.equal(describeAndroidPath(track(96_000, null), bt({ ...ldac, ldacAdaptive: true })), '96 kHz → 96 kHz（LDAC 最佳效果）．2 聲道');
  assert.equal(describeAndroidPath(track(44_100, 0)), '44.1 kHz．2 聲道');
  const sbc = { codec: 'SBC', sampleRate: null, bitsPerSample: null, channels: null, ldacKbps: null, ldacAdaptive: false };
  assert.equal(describeAndroidPath(track(48_000, 48_000), bt(sbc)), '48 kHz → 48 kHz（系統） → （SBC）．2 聲道');
  assert.equal(describeAndroidPath(null, bt(ldac)), null);
  assert.doesNotMatch(describeAndroidPath(track(96_000, 96_000)) ?? '', /位元|bit-perfect|系統混音/);
});

test('a codec step names the codec and LDAC quality', () => {
  assert.equal(codecStepName({ ...ldac, ldacKbps: 990 }), 'LDAC 990 kbps');
  assert.equal(codecStepName({ ...ldac, ldacAdaptive: true }), 'LDAC 最佳效果');
  assert.equal(codecStepName({ codec: 'AAC', sampleRate: 44_100, bitsPerSample: 16, channels: 2, ldacKbps: null, ldacAdaptive: false }), 'AAC');
});

test('an idle device lists every supported rate then the channels, no bit depth', () => {
  assert.equal(formatSupportedFormats([96_000, 44_100, 48_000, 88_200, 48_000], [2]), '44.1, 48, 88.2, 96 kHz．2 聲道');
  assert.equal(formatSupportedFormats([48_000], [2]), '48 kHz．2 聲道');
  assert.equal(formatSupportedFormats([176_400, 352_800], [8, 2]), '176.4, 352.8 kHz．2, 8 聲道');
  assert.equal(formatSupportedFormats([], []), '格式不限');
  assert.equal(formatSupportedFormats([], [], '由藍牙編碼決定'), '由藍牙編碼決定');
  assert.doesNotMatch(formatSupportedFormats([44_100], [2]), /位元|bit/);
});
