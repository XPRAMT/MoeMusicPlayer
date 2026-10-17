import { tr } from './i18n.js';
export function formatDuration(durationMs: number | null | undefined): string {
  if (durationMs === null || durationMs === undefined || !Number.isFinite(durationMs)) {
    return '—:—';
  }

  const totalSeconds = Math.max(0, Math.floor(durationMs / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, '0')}`;
}

export function formatDurationHours(durationMs: number | null | undefined): string {
  if (durationMs === null || durationMs === undefined || !Number.isFinite(durationMs)) {
    return '—:—:—';
  }
  const totalSeconds = Math.max(0, Math.floor(durationMs / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  return `${hours.toString().padStart(2, '0')}:${minutes.toString().padStart(2, '0')}:${seconds.toString().padStart(2, '0')}`;
}

export function formatAlbumDetailSummary(trackCount: number, playedMs: number, durationMs: number): string {
  return tr('{0}首曲目．累積播放 {1}．播放時長 {2}', [trackCount.toLocaleString(), formatDurationHours(playedMs), formatDurationHours(durationMs)]);
}

/** Show only the track number; album Disc groups already identify the disc. */
export function formatAlbumTrackNumber(trackNumber: number | null): string {
  return trackNumber?.toString() ?? '—';
}

export function formatVolume(volume: number): string {
  return `${Math.round(Math.max(0, Math.min(1, volume)) * 100)}%`;
}
