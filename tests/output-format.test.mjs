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

test('missing output or track has a neutral status', () => {
  assert.equal(describeOutputFormat(null).text, '音訊輸出尚未開啟。');
  assert.deepEqual(describeOutputFormat(format({ sourceRateHz: null, conversion: 'none' })), {
    text: '尚未載入曲目（輸出 96 kHz）',
    notice: null,
  });
});
