// Which speaker icon the bottom bar shows for a volume: the more volume, the more sound waves.

/** @typedef {'off' | 'low' | 'medium' | 'high'} VolumeIconLevel */

/** The highest percent shown as "low" and as "medium". */
export const VOLUME_ICON_LOW_MAX_PERCENT = 33;
export const VOLUME_ICON_MEDIUM_MAX_PERCENT = 66;

/**
 * @param {number | null | undefined} volume  0 to 1 (null or not a number counts as 0)
 * @returns {VolumeIconLevel}  off at 0 %, low up to 33 %, medium up to 66 %, high above (by the rounded percent shown)
 */
export function getVolumeIconLevel(volume) {
  const percent = typeof volume === 'number' && Number.isFinite(volume) ? Math.round(Math.min(1, Math.max(0, volume)) * 100) : 0;
  if (percent <= 0) return 'off';
  if (percent <= VOLUME_ICON_LOW_MAX_PERCENT) return 'low';
  if (percent <= VOLUME_ICON_MEDIUM_MAX_PERCENT) return 'medium';
  return 'high';
}
