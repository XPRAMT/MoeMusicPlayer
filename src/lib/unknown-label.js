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

/**
 * The backend's fixed name of the favourites playlist (MyLove.m3u8): the source, its playlist and the favourite
 * picker all carry "我的最愛" as data (the backend also recognises the list by it), so it is translated only here.
 */
export const BACKEND_MY_LOVE_LABEL = '我的最愛';

/**
 * What to show for a playlist or source name from the backend: the favourites list is translated, an empty name
 * reads "未命名播放清單", every other name is shown as it is.
 * @param {string} name
 */
export function displayPlaylistName(name) {
  const trimmed = name.trim();
  if (trimmed === BACKEND_MY_LOVE_LABEL) return tr('我的最愛');
  return trimmed || tr('未命名播放清單');
}

/** The backend's name for tracks in folders on this device (the "source" column and its groups). */
export const BACKEND_LOCAL_SOURCE_LABEL = '本地';
