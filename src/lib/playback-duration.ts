/** Select a usable duration for display: prefer the decoder, then track tags. */
export function effectivePlaybackDurationMs(
  audioDurationMs: number | null | undefined,
  trackDurationMs: number | null | undefined,
): number | null {
  if (typeof audioDurationMs === 'number' && Number.isFinite(audioDurationMs) && audioDurationMs > 0) {
    return Math.floor(audioDurationMs);
  }
  if (typeof trackDurationMs === 'number' && Number.isFinite(trackDurationMs) && trackDurationMs > 0) {
    return Math.floor(trackDurationMs);
  }
  return null;
}
