// The USB exclusive test (ADR 0046, P0, development only): the one-line summary of a finished test on the USB card in
// Settings → playback. The run itself is in Rust (`player-usb-audio`); this only turns its report into text.
import { tr } from './i18n.js';

/**
 * @typedef {{ min: number | null, max: number | null, avg: number | null, accepted: number, rejected: number }} UsbExclusiveFeedback
 * @typedef {{
 *   outcome: 'finished' | 'stopped' | 'failed', error: string | null, fileRate: number | null, rateSet: number | null,
 *   rateReadBack: number | null, interface: number | null, alt: number | null, highSpeed: boolean | null,
 *   feedback: UsbExclusiveFeedback, framesTotal: number, framesSent: number, underruns: number, urbErrors: number,
 *   packetErrors: number, dataUrbs: number, packetsPerUrb: number, handedBack: boolean | null,
 *   handBackErrors: string[], elapsedMs: number, steps: string[],
 * }} UsbExclusiveReport
 * @typedef {{ running: boolean, report: UsbExclusiveReport | null }} UsbExclusiveStatus
 */

/** How often the page reads the status while a test runs. */
export const USB_EXCLUSIVE_POLL_MS = 500;

/** @param {UsbExclusiveReport} report */
function handBackPart(report) {
  if (report.handedBack === null || report.handedBack === undefined) return null;
  if (report.handedBack) return tr('已交還');
  const reason = report.handBackErrors?.[0];
  return reason ? tr('交還失敗：{0}', [reason]) : tr('交還失敗');
}

/**
 * One line, e.g. 「時鐘 44100（讀回 44100）．alt 6．回授平均 5.5126．欠載 0．錯誤 0．已交還」.
 * @param {UsbExclusiveReport | null | undefined} report
 * @returns {string | null}
 */
export function describeUsbExclusiveReport(report) {
  if (!report) return null;
  /** @type {string[]} */
  const parts = [];
  if (report.outcome === 'failed') {
    parts.push(tr('失敗：{0}', [report.error || tr('原因不明')]));
  } else if (report.outcome === 'stopped') {
    parts.push(tr('已停止'));
  }
  if (report.rateSet !== null && report.rateSet !== undefined) {
    const readBack = report.rateReadBack ?? '?';
    parts.push(tr('時鐘 {0}（讀回 {1}）', [report.rateSet, readBack]));
  }
  if (report.outcome !== 'failed' || report.framesSent > 0) {
    if (report.alt !== null && report.alt !== undefined) parts.push(tr('alt {0}', [report.alt]));
    const average = report.feedback?.avg;
    parts.push(
      typeof average === 'number' ? tr('回授平均 {0}', [average.toFixed(4)]) : tr('沒有回授'),
    );
    parts.push(tr('欠載 {0}', [report.underruns ?? 0]));
    parts.push(tr('錯誤 {0}', [(report.urbErrors ?? 0) + (report.packetErrors ?? 0)]));
  }
  const handBack = handBackPart(report);
  if (handBack) parts.push(handBack);
  return parts.join('．');
}
