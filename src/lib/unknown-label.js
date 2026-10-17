import { tr } from './i18n.js';

/** The library backend names the group of tracks without a name, artist or album "未知" (it is data, not UI text). */
export const BACKEND_UNKNOWN_LABEL = '未知';

/**
 * What to show for a group or item name that came from the backend: the "未知" placeholder is translated
 * (Simplified and Japanese show 未知, English shows unknown); every real name is shown as it is.
 * @param {string} name
 */
export function displayUnknown(name) {
  return name === BACKEND_UNKNOWN_LABEL ? tr('未知') : name;
}
