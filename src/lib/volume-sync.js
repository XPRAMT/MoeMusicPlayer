// Keeping several playback devices' real volumes in proportion (like the reference channel mapper): when one device's
// volume changes, the other chosen devices move so that their ratio to it stays what it was when the sync started.
// Volumes are percent (0 to 100). The ratios are remembered per pair, so going down to 0 and coming back restores the
// proportions instead of leaving every device at the same level.

/** Ratios within this band of 1 snap to exactly 1 (two devices set to "about the same" move together). */
const SNAP_LOW = 0.9;
const SNAP_HIGH = 1.1;

/**
 * How `id` compares to `reference` (its volume divided by the reference's), learned the first time and remembered.
 * The map is directional: `${reference}>${id}` is id's volume over the reference's.
 * @param {Map<string, number>} scales
 * @param {string} id
 * @param {string} reference
 * @param {number} idVolume
 * @param {number} referenceVolume
 */
function ratioTo(scales, id, reference, idVolume, referenceVolume) {
  const key = `${reference}>${id}`;
  const known = scales.get(key);
  if (known !== undefined) return known;
  let ratio = referenceVolume > 0 ? idVolume / referenceVolume : 1;
  if (ratio > SNAP_LOW && ratio < SNAP_HIGH) ratio = 1;
  scales.set(key, ratio);
  // The same pair seen from the other side (a device at 0 cannot be divided by, so it falls back to 1).
  scales.set(`${id}>${reference}`, ratio > 0 ? 1 / ratio : 1);
  return ratio;
}

/**
 * The volumes the other devices should take after `changedId` moved to `next`.
 * @param {{
 *   changedId: string,
 *   next: number,
 *   volumes: Map<string, number>,
 *   ids: string[],
 *   scales: Map<string, number>,
 * }} request  `volumes` are the volumes before the change; `ids` the devices that take part
 * @returns {Map<string, number>}
 */
export function syncVolumes({ changedId, next, volumes, ids, scales }) {
  /** @type {Map<string, number>} */
  const updates = new Map();
  const before = volumes.get(changedId) ?? next;
  for (const id of ids) {
    if (id === changedId || !volumes.has(id)) continue;
    const ratio = ratioTo(scales, id, changedId, /** @type {number} */ (volumes.get(id)), before);
    updates.set(id, Math.min(100, Math.max(0, Math.round(next * ratio))));
  }
  return updates;
}

/**
 * Forget the ratios of devices that no longer take part.
 * @param {Map<string, number>} scales
 * @param {string[]} ids
 */
export function pruneScales(scales, ids) {
  for (const key of [...scales.keys()]) {
    const [a, b] = key.split('>');
    if (!ids.includes(a) || !ids.includes(b)) scales.delete(key);
  }
}
