import { tr } from './i18n.js';
const REPEAT_MODE_LABELS = Object.freeze({
  off: tr('循環：關閉；點擊切換全部循環'),
  all: tr('循環：全部；點擊切換單曲循環'),
  one: tr('循環：單曲；點擊關閉循環'),
});

/** Return tooltips that describe the playback controls' current actions. */
/**
 * @param {{
 *   isPlaying?: boolean,
 *   positionMs?: number,
 *   repeatMode?: 'off' | 'all' | 'one',
 *   currentTrack?: unknown,
 *   shuffle?: boolean,
 *   canPrevious?: boolean,
 *   canNext?: boolean,
 * } | null | undefined} playback
 */
export function getPlaybackControlTooltips(playback) {
  const isPlaying = playback?.isPlaying === true;
  const positionMs = typeof playback?.positionMs === 'number' && Number.isFinite(playback.positionMs)
    ? playback.positionMs
    : 0;
  const repeatMode = playback?.repeatMode ?? 'off';

  return {
    playPause: isPlaying ? tr('暫停') : playback?.currentTrack ? tr('播放') : tr('尚未選擇歌曲'),
    shuffle: playback?.shuffle ? tr('關閉隨機播放') : tr('開啟隨機播放'),
    previous: positionMs > 3000
      ? tr('從頭播放目前歌曲')
      : playback?.canPrevious
        ? tr('播放佇列上一首')
        : tr('沒有可播放的上一首'),
    next: playback?.canNext ? tr('播放佇列下一首') : tr('沒有可播放的下一首'),
    repeat: REPEAT_MODE_LABELS[repeatMode] ?? REPEAT_MODE_LABELS.off,
  };
}
