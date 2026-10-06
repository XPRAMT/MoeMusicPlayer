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

export const PLAYBACK_SEEK_SETTLE_TOLERANCE_MS = 400;

/** Keep a committed click on screen until the worker snapshot lands near it. */
export function shouldReleasePlaybackSeekDraft(
  snapshotPositionMs: number,
  draftPositionMs: number | null,
  pointerActive: boolean,
  seekSettled: boolean,
  baselinePositionMs: number | null,
): boolean {
  if (!seekSettled || pointerActive || draftPositionMs === null || baselinePositionMs === null) return false;
  return Math.abs(snapshotPositionMs - draftPositionMs) <= PLAYBACK_SEEK_SETTLE_TOLERANCE_MS;
}

export function playbackSeekDisplayPosition(
  snapshotPositionMs: number,
  durationMs: number | null,
  draftPositionMs: number | null,
): number {
  if (draftPositionMs !== null) return clampPlaybackPosition(draftPositionMs, durationMs);
  return clampPlaybackPosition(snapshotPositionMs, durationMs);
}
