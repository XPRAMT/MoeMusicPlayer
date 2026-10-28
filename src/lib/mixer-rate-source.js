// The explanation panel of the real mixer rate (ADR 0048, convention of ADR 0039: the explanation first, a request only
// from a button). The state itself comes from Kotlin (`MixerRateRules.panelState`); this file turns it into texts and
// buttons.
import { tr } from './i18n.js';

/**
 * @typedef {import('./ipc').AndroidMixerRateSourceStatus} MixerRateSourceStatus
 * @typedef {'checkRoot' | 'requestShizuku' | 'openShizuku' | 'recheck'} MixerRateAction
 * @typedef {{ action: MixerRateAction, label: string, primary: boolean }} MixerRateButton
 * @typedef {{ state: string, summary: string, title: string, paragraphs: string[], buttons: MixerRateButton[] }} MixerRatePanel
 */

/**
 * The one-line status under 「播放裝置」: which source gives the 「系統」 rate. Null while the state is not known yet.
 * @param {MixerRateSourceStatus | null} status
 * @returns {string | null}
 */
export function mixerRateSummary(status) {
  if (!status) return null;
  switch (status.state) {
    case 'rootActive': return tr('系統取樣率：以 root 讀取實際值');
    case 'shizukuActive': return tr('系統取樣率：以 Shizuku 讀取實際值');
    case 'tripped': return tr('系統取樣率：已暫停讀取實際值（音訊服務異常）');
    case 'disabled': return tr('系統取樣率：已停用讀取實際值');
    default: return tr('系統取樣率：Android 提供的串流值（可能與實際不同）');
  }
}

/**
 * @param {MixerRateAction} action
 * @param {string} label
 * @param {boolean} primary
 * @returns {MixerRateButton}
 */
const button = (action, label, primary) => ({ action, label, primary });

/**
 * The panel for a state: the title, the explanation and the buttons (the first primary one is the 「授權」 action).
 * @param {MixerRateSourceStatus} status
 * @returns {MixerRatePanel}
 */
export function mixerRatePanel(status) {
  const title = tr('需要 root 或 Shizuku');
  const summary = mixerRateSummary(status) ?? '';
  const why = tr('三項功能需要 root 或 Shizuku：讀取這首歌實際的系統（混音）取樣率、「藍牙取樣率跟隨曲目」、藍牙編碼切換。它們要讀取系統的音訊狀態或變更藍牙編碼，一般應用程式沒有這個權限，所以改由 root 或 Shizuku 代為執行；只在換歌、換輸出裝置或切換編碼時使用，不會持續輪詢。');
  switch (status.state) {
    case 'rootActive':
      return {
        state: status.state, summary, title,
        paragraphs: [tr('已可使用 root：上述三項功能會以 root 執行。'), why],
        buttons: [button('recheck', tr('重新檢查'), false)],
      };
    case 'shizukuActive':
      return {
        state: status.state, summary, title,
        paragraphs: [tr('已取得 Shizuku 授權：上述三項功能會透過 Shizuku 執行。'), why],
        buttons: [button('recheck', tr('重新檢查'), false)],
      };
    case 'shizukuNeedsPermission':
      return {
        state: status.state, summary, title,
        paragraphs: [why, tr('Shizuku 已在執行。按下「授權」後，Shizuku 會詢問是否允許本程式使用。')],
        buttons: [
          button('requestShizuku', tr('授權'), true),
          button('checkRoot', tr('檢查 root'), false),
        ],
      };
    case 'shizukuDenied':
      return {
        state: status.state, summary, title,
        paragraphs: [why, tr('你已選擇不再詢問 Shizuku 的授權。請開啟 Shizuku，在已授權的應用程式清單中允許 MoeMusicPlayer，再回到本程式。')],
        buttons: [
          button('openShizuku', tr('授權'), true),
          button('recheck', tr('重新檢查'), false),
        ],
      };
    case 'shizukuNotRunning':
      return {
        state: status.state, summary, title,
        paragraphs: [why, tr('已安裝 Shizuku，但它尚未執行（或版本過舊）。請開啟 Shizuku 並啟動服務（無線偵錯、adb 或 root），再回到本程式。')],
        buttons: [
          button('openShizuku', tr('授權'), true),
          button('checkRoot', tr('檢查 root'), false),
          button('recheck', tr('重新檢查'), false),
        ],
      };
    case 'tripped':
      return {
        state: status.state, summary, title,
        paragraphs: [tr('讀取實際取樣率後不久，Android 的音訊服務出現異常，為了安全，本次執行期間不再讀取，改用 Android 提供的串流值。重新啟動本程式後會恢復。')],
        buttons: [],
      };
    case 'disabled':
      return {
        state: status.state, summary, title,
        paragraphs: [tr('讀取實際取樣率的功能已被停用（設定檔的 androidRealMixerRate 為 false），目前使用 Android 提供的串流值。')],
        buttons: [],
      };
    default:
      return {
        state: 'none', summary, title,
        paragraphs: [
          why,
          tr('目前沒有可用的 root 或 Shizuku。若裝置已 root，按下「授權」，root 管理程式會詢問是否允許本程式；沒有 root 時可安裝 Shizuku 並啟動它。沒有授權時，這三項功能不會作用，系統取樣率維持顯示 Android 提供的串流值。'),
        ],
        buttons: [
          button('checkRoot', tr('授權'), true),
          ...(status.shizukuInstalled ? [button('openShizuku', tr('開啟 Shizuku'), false)] : []),
        ],
      };
  }
}

/**
 * Whether the 「前往授權」 entry under 「裝置」 is shown: only while a permission is missing (no root, and no authorized
 * Shizuku), hidden once one is granted. 「tripped」 and 「disabled」 have nothing to grant, so no entry either.
 * @param {MixerRateSourceStatus | null} status
 */
export function mixerRateEntryVisible(status) {
  if (!status) return false;
  return ['none', 'shizukuNotRunning', 'shizukuDenied', 'shizukuNeedsPermission'].includes(status.state);
}

/**
 * Whether the explanation panel closes by itself: root or Shizuku became available, so there is nothing left to grant
 * (the global rule: permission UI only while a permission is missing). Not for 「tripped」 and 「disabled」, which
 * can only be opened by hand and keep their 「關閉」 button.
 * @param {MixerRateSourceStatus | null} status
 */
export function mixerPanelShouldClose(status) {
  return status?.state === 'rootActive' || status?.state === 'shizukuActive';
}
