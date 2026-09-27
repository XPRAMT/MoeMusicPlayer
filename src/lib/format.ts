export function formatDuration(durationMs: number | null | undefined): string {
  if (durationMs === null || durationMs === undefined || !Number.isFinite(durationMs)) {
    return '—:—';
  }

  const totalSeconds = Math.max(0, Math.floor(durationMs / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, '0')}`;
}

export function formatTrackIndex(trackNumber: number | null, discNumber: number | null): string {
  const track = trackNumber?.toString().padStart(2, '0') ?? '—';
  if (discNumber === null) return track;
  return `${discNumber}-${track}`;
}

export function formatVolume(volume: number): string {
  return `${Math.round(Math.max(0, Math.min(1, volume)) * 100)}%`;
}
