import assert from 'node:assert/strict';
import { test } from 'node:test';
import { describeOutputFormat, formatSampleRate } from '../src/lib/output-format.js';

/** @param {Partial<import('../src/lib/output-format.js').OutputFormat>} overrides */
const format = (overrides) => ({
  mode: 'highQuality',
  sourceRateHz: 44_100,
  outputRateHz: 96_000,
  deviceRateHz: 96_000,
  conversion: 'highQuality',
  fallbackReason: null,
  dseeHx: 'off',
  dseeNotice: null,
  dseeOutputRateHz: null,
  ...overrides,
});

test('sample rates are shown in kHz without trailing zeros', () => {
  assert.equal(formatSampleRate(44_100), '44.1 kHz');
  assert.equal(formatSampleRate(96_000), '96 kHz');
  assert.equal(formatSampleRate(176_400), '176.4 kHz');
  assert.equal(formatSampleRate(22_050), '22.05 kHz');
});

test('output status names who converts the current track', () => {
  assert.deepEqual(describeOutputFormat(format({})), {
    text: '44.1 kHz → 96 kHz（高品質）',
    notice: null,
  });
  assert.deepEqual(
    describeOutputFormat(format({ sourceRateHz: 96_000, conversion: 'none' })),
    { text: '原生 96 kHz（未轉換）', notice: null },
  );
  assert.deepEqual(
    describeOutputFormat(format({ mode: 'windowsBuiltin', outputRateHz: 44_100, conversion: 'windows' })),
    { text: '44.1 kHz（交由 Windows 轉換為 96 kHz）', notice: null },
  );
  assert.equal(describeOutputFormat(format({ conversion: 'basic' })).text, '44.1 kHz → 96 kHz（基本轉換）');
});

test('windows built-in fallback is called out', () => {
  const status = describeOutputFormat(
    format({ mode: 'windowsBuiltin', fallbackReason: 'unsupported format' }),
  );
  assert.equal(status.text, '44.1 kHz → 96 kHz（高品質）');
  assert.equal(status.notice, 'Windows 無法以 44.1 kHz 開啟共享模式輸出，已改用 96 kHz 並以高品質轉換。');
});

test('DSEE HX names the enhancement and keeps a later device conversion', () => {
  assert.equal(
    describeOutputFormat(format({ dseeHx: 'active' })).text,
    '44.1 kHz → 96 kHz（DSEE HX）',
  );
  assert.equal(
    describeOutputFormat(format({
      dseeHx: 'active',
      outputRateHz: 48_000,
      deviceRateHz: 48_000,
    })).text,
    '44.1 kHz → 96 kHz（DSEE HX） → 48 kHz（高品質）',
  );
  assert.equal(
    describeOutputFormat(format({
      dseeHx: 'active',
      dseeOutputRateHz: 176_400,
    })).text,
    '44.1 kHz → 176.4 kHz（DSEE HX） → 96 kHz（高品質）',
  );
  assert.equal(
    describeOutputFormat(format({
      dseeHx: 'unavailable',
      dseeNotice: '請先安裝 Sony Music Center。',
    })).notice,
    '請先安裝 Sony Music Center。',
  );
});

test('missing output or track has a neutral status', () => {
  assert.equal(describeOutputFormat(null).text, '音訊輸出尚未開啟。');
  assert.deepEqual(describeOutputFormat(format({ sourceRateHz: null, conversion: 'none' })), {
    text: '尚未載入曲目（輸出 96 kHz）',
    notice: null,
  });
});

test('a synced device shows the track path plus its own conversion', async () => {
  const { describeMirrorPath } = await import('../src/lib/output-format.js');
  assert.equal(describeMirrorPath(format({ conversion: 'none', outputRateHz: 44_100 }), 44_100, false), '原生 44.1 kHz（未轉換）');
  assert.equal(describeMirrorPath(format({}), 96_000, false), '44.1 kHz → 96 kHz（高品質）');
  assert.equal(describeMirrorPath(format({}), 48_000, true), '44.1 kHz → 96 kHz（高品質） → 48 kHz（高品質）');
  assert.equal(
    describeMirrorPath(format({ sourceRateHz: 48_000, dseeHx: 'active', dseeOutputRateHz: 96_000, outputRateHz: 96_000 }), 96_000, false),
    '48 kHz → 96 kHz（DSEE HX）',
  );
  assert.equal(describeMirrorPath(null, 96_000, false), '尚未載入曲目（輸出 96 kHz）');
});

test('the exclusive stream says bit-perfect, or what changed the samples', async () => {
  const { describeExclusive } = await import('../src/lib/output-format.js');
  const exclusive = { bits: 24, channels: 2, sourceChannels: 2, bitPerfect: true, volumeApplied: false, underruns: 0 };
  assert.deepEqual(describeExclusive(format({ sourceRateHz: 96_000, outputRateHz: 96_000, exclusive })), {
    text: '獨佔．24-bit 96 kHz．位元完美',
    notice: null,
  });
  const converted = describeExclusive(format({ sourceRateHz: 44_100, outputRateHz: 96_000, exclusive: { ...exclusive, bitPerfect: false } }));
  assert.equal(converted.text, '獨佔．24-bit 96 kHz');
  assert.equal(converted.notice, '裝置不接受 44.1 kHz，已轉換為 96 kHz（不是位元完美）');
  const volume = describeExclusive(format({ sourceRateHz: 96_000, outputRateHz: 96_000, exclusive: { ...exclusive, bitPerfect: false, volumeApplied: true } }));
  assert.match(volume.notice, /軟體音量/);
  assert.equal(describeExclusive(format({})), null, 'a shared-mode output has no exclusive stream');
});

test('a track exclusive mode could not take says it plays in shared mode and why', async () => {
  const { describeExclusiveFallback } = await import('../src/lib/output-format.js');
  assert.equal(describeExclusiveFallback(format({})), null);
  const described = describeExclusiveFallback(format({ conversion: 'none', outputRateHz: 44_100, exclusiveFallback: '這個裝置正被其他程式使用。' }));
  assert.equal(described.text, '已改用共享模式：原生 44.1 kHz（未轉換）');
  assert.equal(described.notice, '獨佔模式無法播放這首：這個裝置正被其他程式使用。');
});
