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
  clockOrder: 'altThenClock',
  feedback: { min: 5.5124, max: 5.5128, avg: 5.51262, accepted: 7480, rejected: 20, shift: 0, rescales: 0, firstRaw: 5.5126 },
  feedbackUrbs: {
    reaped: 7500,
    withData: 7500,
    zeroLength: 0,
    packetStatus: [],
    urbStatus: [],
    firstPackets: ['9C 58 05 00'],
  },
  dataPackets: {
    reaped: 3750,
    urbStatus: [{ status: 0, name: null, count: 3750 }],
    packetStatus: [{ status: 0, name: null, count: 30000 }],
    packets: 30000,
    lengthMismatch: 0,
    requestedBytes: 5292000,
    actualBytes: 5292000,
  },
  pcm: { firstSeconds: 3, peakFirst: 22654, peakFirstDbfs: -3.2056, peak: 22654, peakDbfs: -3.2056, firstSamples: [0, 0] },
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

test('a finished test reads as one line with the clock, alt, feedback, PCM peak, underruns, errors and the hand-back', () => {
  assert.equal(
    describeUsbExclusiveReport(finished),
    '時鐘 44100（讀回 44100）．alt 6．回授 7480／7500．平均 5.5126．峰值 -3.2 dBFS．欠載 0．錯誤 0．已交還',
  );
  assert.equal(
    describeUsbExclusiveReport({ ...finished, pcm: { ...finished.pcm, peakDbfs: -0.0003 } }),
    '時鐘 44100（讀回 44100）．alt 6．回授 7480／7500．平均 5.5126．峰值 0.0 dBFS．欠載 0．錯誤 0．已交還',
  );
});

test('feedback that never came back shows how many URBs were reaped, without an average; silence is named', () => {
  const silent = {
    ...finished,
    feedback: { ...finished.feedback, avg: null, min: null, max: null, accepted: 0, rejected: 0 },
    feedbackUrbs: { ...finished.feedbackUrbs, withData: 0, zeroLength: 7500, firstPackets: [] },
    pcm: { ...finished.pcm, peak: 0, peakDbfs: null, peakFirst: 0, peakFirstDbfs: null },
  };
  assert.equal(
    describeUsbExclusiveReport(silent),
    '時鐘 44100（讀回 44100）．alt 6．回授 0／7500．PCM 全為靜音．欠載 0．錯誤 0．已交還',
  );
  assert.equal(
    describeUsbExclusiveReport({ ...silent, feedbackUrbs: undefined, pcm: null }),
    '時鐘 44100（讀回 44100）．alt 6．回授 0／0．欠載 0．錯誤 0．已交還',
  );
});

test('a stopped test says so first; errors add URB and packet errors', () => {
  assert.equal(
    describeUsbExclusiveReport({ ...finished, outcome: 'stopped', underruns: 2, urbErrors: 1, packetErrors: 3 }),
    '已停止．時鐘 44100（讀回 44100）．alt 6．回授 7480／7500．平均 5.5126．峰值 -3.2 dBFS．欠載 2．錯誤 4．已交還',
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
  // It may be a speaker: the warning names a USB audio device, not a DAC.
  const panel = component.slice(component.indexOf('id="usb-exclusive-title"'));
  assert.doesNotMatch(panel.slice(0, panel.indexOf('</dialog>')), /DAC/);
  assert.match(panel, /USB 音訊裝置/);
});
