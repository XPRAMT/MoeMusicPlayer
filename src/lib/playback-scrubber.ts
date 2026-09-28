export function isPlaybackSeekableDuration(durationMs: number | null): durationMs is number {
  return durationMs !== null && Number.isFinite(durationMs) && durationMs > 0;
}

export function clampPlaybackPosition(positionMs: number, durationMs: number | null): number {
  const position = Number.isFinite(positionMs) ? Math.trunc(positionMs) : 0;
  const nonNegative = Math.max(0, position);
  return isPlaybackSeekableDuration(durationMs)
    ? Math.min(nonNegative, Math.floor(durationMs))
    : nonNegative;
}

export function playbackSeekDisplayPosition(
  snapshotPositionMs: number,
  durationMs: number | null,
  draftPositionMs: number | null,
): number {
  if (draftPositionMs !== null) return clampPlaybackPosition(draftPositionMs, durationMs);
  return clampPlaybackPosition(snapshotPositionMs, durationMs);
}
