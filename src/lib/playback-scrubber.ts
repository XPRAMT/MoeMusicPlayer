export interface PlaybackSeekDraft {
  trackId: string;
  startPositionMs: number;
  positionMs: number;
}

export interface PendingPlaybackSeek {
  trackId: string;
  positionMs: number;
  requestedAtMs: number;
  snapshotVersionAtRequest: number;
}

export interface PlaybackSeekSnapshot {
  trackId: string | null;
  positionMs: number;
  durationMs: number | null;
  isPlaying: boolean;
}

const PENDING_SEEK_TIMEOUT_MS = 3_000;

export function isPlaybackSeekableDuration(durationMs: number | null): durationMs is number {
  return durationMs !== null && Number.isFinite(durationMs) && durationMs > 0;
}

export function beginPlaybackSeekDraft(
  trackId: string | null,
  positionMs: number,
  durationMs: number | null,
): PlaybackSeekDraft | null {
  if (!trackId || !isPlaybackSeekableDuration(durationMs)) return null;
  const position = clampPlaybackPosition(positionMs, durationMs);
  return {
    trackId,
    startPositionMs: position,
    positionMs: position,
  };
}

export function updatePlaybackSeekDraft(
  draft: PlaybackSeekDraft | null,
  trackId: string | null,
  currentPositionMs: number,
  durationMs: number | null,
  positionMs: number,
): PlaybackSeekDraft | null {
  const current = draft?.trackId === trackId
    ? draft
    : beginPlaybackSeekDraft(trackId, currentPositionMs, durationMs);
  if (!current || !isPlaybackSeekableDuration(durationMs)) return null;
  return {
    ...current,
    positionMs: clampPlaybackPosition(positionMs, durationMs),
  };
}

export function commitPlaybackSeekDraft(
  draft: PlaybackSeekDraft | null,
  trackId: string | null,
  durationMs: number | null,
): number | null {
  if (!draft || draft.trackId !== trackId || !isPlaybackSeekableDuration(durationMs)) return null;
  const position = clampPlaybackPosition(draft.positionMs, durationMs);
  return position === draft.startPositionMs ? null : position;
}

export function playbackSeekDisplayPosition(
  snapshotPositionMs: number,
  durationMs: number | null,
  trackId: string | null,
  draft: PlaybackSeekDraft | null,
  pending: PendingPlaybackSeek | null,
): number {
  if (draft?.trackId === trackId) return clampPlaybackPosition(draft.positionMs, durationMs);
  if (pending?.trackId === trackId) return clampPlaybackPosition(pending.positionMs, durationMs);
  return clampPlaybackPosition(snapshotPositionMs, durationMs);
}

export function shouldClearPendingPlaybackSeek(
  pending: PendingPlaybackSeek,
  snapshot: PlaybackSeekSnapshot,
  snapshotVersion: number,
  nowMs: number,
): boolean {
  if (snapshotVersion <= pending.snapshotVersionAtRequest) return false;
  if (pending.trackId !== snapshot.trackId) return true;
  const expectedPosition = clampPlaybackPosition(pending.positionMs, snapshot.durationMs);
  const acceptableDriftMs = snapshot.isPlaying ? 1_000 : 50;
  if (Math.abs(snapshot.positionMs - expectedPosition) <= acceptableDriftMs) return true;
  return nowMs - pending.requestedAtMs >= PENDING_SEEK_TIMEOUT_MS;
}

function clampPlaybackPosition(positionMs: number, durationMs: number | null): number {
  const position = Number.isFinite(positionMs) ? Math.trunc(positionMs) : 0;
  const nonNegative = Math.max(0, position);
  return isPlaybackSeekableDuration(durationMs)
    ? Math.min(nonNegative, Math.floor(durationMs))
    : nonNegative;
}
