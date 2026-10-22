/**
 * The one place a seek position becomes what `playback_seek` accepts: whole milliseconds (Rust `u64`), never negative,
 * never past the track. Positions estimated by the playback clock (ADR 0028) come from `performance.now()` and carry
 * fractions such as 6325.30000000447, which the command rejects.
 * @param {number} positionMs
 * @param {number | null | undefined} [durationMs]
 * @returns {number}
 */
export function normalizeSeekPositionMs(positionMs, durationMs) {
  let position = Number.isFinite(positionMs) ? Math.round(positionMs) : 0;
  if (typeof durationMs === 'number' && Number.isFinite(durationMs) && durationMs >= 0) {
    position = Math.min(position, Math.floor(durationMs));
  }
  return Math.max(0, position);
}