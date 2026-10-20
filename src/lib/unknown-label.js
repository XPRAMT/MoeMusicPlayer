import { tr } from './i18n.js';

/** The library backend names the group of tracks without a name, artist or album "未知" (it is data, not UI text). */
export const BACKEND_UNKNOWN_LABEL = '未知';

/**
 * What to show for a group or item name that came from the backend: the "未知" placeholder is translated
 * (Simplified and Japanese show 未知, English shows unknown); every real name is shown as it is.
 * @param {string} name
 */
export function displayUnknown(name) {
  if (name === BACKEND_UNKNOWN_LABEL) return tr('未知');
  // The source of a track on this device is "本地" (a fixed word from the backend); a server shows its own name.
  if (name === BACKEND_LOCAL_SOURCE_LABEL) return tr('本地');
  return name;
}

/** The backend's name for tracks in folders on this device (the "source" column and its groups). */
export const BACKEND_LOCAL_SOURCE_LABEL = '本地';
