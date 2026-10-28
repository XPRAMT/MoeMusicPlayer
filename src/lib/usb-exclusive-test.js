// The USB exclusive test (ADR 0046, P0, development only): the one-line summary of a finished test on the USB card in
// Settings → playback. The run itself is in Rust (`player-usb-audio`); this only turns its report into text.
import { tr } from './i18n.js';

/**
 * @typedef {{
 *   min: number | null, max: number | null, avg: number | null, accepted: number, rejected: number,
 *   shift: number | null, rescales: number, firstRaw: number | null,
 * }} UsbExclusiveFeedback
 * @typedef {{ status: number, name: string | null, count: number }} UsbStatusCount
 * @typedef {{
 *   reaped: number, withData: number, zeroLength: number, packetStatus: UsbStatusCount[],
 *   urbStatus: UsbStatusCount[], firstPackets: string[],
 * }} UsbExclusiveFeedbackUrbs
 * @typedef {{
 *   reaped: number, urbStatus: UsbStatusCount[], packetStatus: UsbStatusCount[], packets: number,
 *   lengthMismatch: number, requestedBytes: number, actualBytes: number,
 * }} UsbExclusiveDataPackets
 * @typedef {{
 *   firstSeconds: number, peakFirst: number, peakFirstDbfs: number | null, peak: number, peakDbfs: number | null,
 *   firstSamples: number[],
 * }} UsbExclusivePcm
 * @typedef {{
 *   outcome: 'finished' | 'stopped' | 'failed', error: string | null, fileRate: number | null, rateSet: number | null,
 *   rateReadBack: number | null, interface: number | null, alt: number | null, highSpeed: boolean | null,
 *   clockOrder: 'altThenClock' | 'clockThenAlt', feedback: UsbExclusiveFeedback,
 *   feedbackUrbs: UsbExclusiveFeedbackUrbs, dataPackets: UsbExclusiveDataPackets, pcm: UsbExclusivePcm | null,
 *   framesTotal: number, framesSent: number, underruns: number, urbErrors: number,
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
 * The PCM peak over the whole buffer: dBFS with one decimal, or silence (null: nothing was decoded).
 * @param {UsbExclusivePcm | null | undefined} pcm
 * @returns {string | null}
 */
function peakPart(pcm) {
  if (!pcm) return null;
  if (typeof pcm.peakDbfs !== 'number') return tr('PCM 全為靜音');
  const text = pcm.peakDbfs.toFixed(1);
  return tr('峰值 {0} dBFS', [text === '-0.0' ? '0.0' : text]);
}

/**
 * One line, e.g. 「時鐘 44100（讀回 44100）．alt 6．回授 7480／7500．平均 5.5125．峰值 -3.2 dBFS．欠載 0．錯誤 0．已交還」:
 * feedback values accepted ／ feedback URBs reaped, the average only when one was accepted.
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
    parts.push(tr('回授 {0}／{1}', [report.feedback?.accepted ?? 0, report.feedbackUrbs?.reaped ?? 0]));
    const average = report.feedback?.avg;
    if (typeof average === 'number') parts.push(tr('平均 {0}', [average.toFixed(4)]));
    const peak = peakPart(report.pcm);
    if (peak) parts.push(peak);
    parts.push(tr('欠載 {0}', [report.underruns ?? 0]));
    parts.push(tr('錯誤 {0}', [(report.urbErrors ?? 0) + (report.packetErrors ?? 0)]));
  }
  const handBack = handBackPart(report);
  if (handBack) parts.push(handBack);
  return parts.join('．');
}
