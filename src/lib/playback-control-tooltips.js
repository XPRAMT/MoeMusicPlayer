const REPEAT_MODE_LABELS = Object.freeze({
  off: '循環：關閉；點擊切換全部循環',
  all: '循環：全部；點擊切換單曲循環',
  one: '循環：單曲；點擊關閉循環',
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
    playPause: isPlaying ? '暫停' : playback?.currentTrack ? '播放' : '尚未選擇歌曲',
    shuffle: playback?.shuffle ? '關閉隨機播放' : '開啟隨機播放',
    previous: positionMs > 3000
      ? '從頭播放目前歌曲'
      : playback?.canPrevious
        ? '播放佇列上一首'
        : '沒有可播放的上一首',
    next: playback?.canNext ? '播放佇列下一首' : '沒有可播放的下一首',
    repeat: REPEAT_MODE_LABELS[repeatMode] ?? REPEAT_MODE_LABELS.off,
  };
}
