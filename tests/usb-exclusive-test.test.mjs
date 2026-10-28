import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { describeUsbExclusiveReport } from '../src/lib/usb-exclusive-test.js';

/** A finished 30 s test on the Sony DAC as `player-usb-audio` reports it (ADR 0046). */
const finished = {
  outcome: 'finished',
  error: null,
  fileRate: 44100,
  rateSet: 44100,
  rateReadBack: 44100,
  interface: 1,
  alt: 6,
  highSpeed: true,
  feedback: { min: 5.5124, max: 5.5128, avg: 5.51262, accepted: 29990, rejected: 0 },
  framesTotal: 1323000,
  framesSent: 1323000,
  underruns: 0,
  urbErrors: 0,
  packetErrors: 0,
  dataUrbs: 16,
  packetsPerUrb: 8,
  handedBack: true,
  handBackErrors: [],
  elapsedMs: 30000,
  steps: [],
};

test('a finished test reads as one line with the clock, alt, feedback, underruns, errors and the hand-back', () => {
  assert.equal(
    describeUsbExclusiveReport(finished),
    '時鐘 44100（讀回 44100）．alt 6．回授平均 5.5126．欠載 0．錯誤 0．已交還',
  );
});

test('a stopped test says so first; errors add URB and packet errors', () => {
  assert.equal(
    describeUsbExclusiveReport({ ...finished, outcome: 'stopped', underruns: 2, urbErrors: 1, packetErrors: 3 }),
    '已停止．時鐘 44100（讀回 44100）．alt 6．回授平均 5.5126．欠載 2．錯誤 4．已交還',
  );
  assert.equal(
    describeUsbExclusiveReport({ ...finished, feedback: { ...finished.feedback, avg: null } }),
    '時鐘 44100（讀回 44100）．alt 6．沒有回授．欠載 0．錯誤 0．已交還',
  );
});

test('a failed start gives the reason, the clock when it was set, and whether the DAC went back', () => {
  const failed = {
    ...finished,
    outcome: 'failed',
    error: '時脈讀回 48000 Hz，不是 44100 Hz',
    rateReadBack: 48000,
    framesSent: 0,
  };
  assert.equal(
    describeUsbExclusiveReport(failed),
    '失敗：時脈讀回 48000 Hz，不是 44100 Hz．時鐘 44100（讀回 48000）．已交還',
  );
  assert.equal(
    describeUsbExclusiveReport({
      ...failed,
      rateSet: null,
      handedBack: false,
      handBackErrors: ['介面 0 交還系統驅動程式失敗：x'],
    }),
    '失敗：時脈讀回 48000 Hz，不是 44100 Hz．交還失敗：介面 0 交還系統驅動程式失敗：x',
  );
  // Refused before the DAC was touched: nothing to hand back.
  assert.equal(
    describeUsbExclusiveReport({ ...failed, error: '沒有找到 USB 音訊裝置', rateSet: null, handedBack: null }),
    '失敗：沒有找到 USB 音訊裝置',
  );
  assert.equal(describeUsbExclusiveReport(null), null);
});

test('the page starts the test only from the confirm panel, and the area never chooses the card', () => {
  const component = readFileSync(new URL('../src/lib/AndroidOutputDevicesSettings.svelte', import.meta.url), 'utf8');
  assert.equal(component.match(/invokeCommand\('android_usb_exclusive_start'/g)?.length, 1);
  // The start button opens the volume warning; only its 「開始試播」 calls startExclusive.
  assert.match(component, /onclick=\{\(\) => \{ exclusivePanelOpen = true; \}\}/);
  assert.equal(component.match(/void startExclusive\(\)/g)?.length, 1);
  assert.match(component, /class="output-device-usb" data-own-click/);
});
