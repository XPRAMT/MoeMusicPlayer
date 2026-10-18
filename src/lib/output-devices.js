// Output devices (ADR 0020, phase 0): which playback devices play, with a volume, a delay and an optional channel map
// each. The backend (settings.rs `OutputDeviceEntry`, audio crate `output_devices`) enforces the same ranges.

export const OUTPUT_DELAY_MAX_MS = 2000;
export const OUTPUT_DELAY_STEP_MS = 5;
/** Channels of the widest layout the channel map names (7.1). */
export const OUTPUT_MAX_SOURCE_CHANNELS = 8;

/**
 * @typedef {{ source: number | null, gainPercent: number }} ChannelRoute
 * @typedef {{ id: string | null, name: string, enabled: boolean, delayMs: number, channelMap: ChannelRoute[] | null }} OutputDeviceEntry
 * @typedef {{ id: string, name: string, isDefault: boolean, channels: number, sampleRate: number }} AudioOutputDevice
 * @typedef {{ key: string, id: string | null, name: string, connected: boolean, isDefault: boolean, channels: number, sampleRate: number, entry: OutputDeviceEntry | null }} DeviceRow
 */

const LABELS = Object.freeze({
  1: ['C'],
  2: ['FL', 'FR'],
  3: ['FL', 'FR', 'C'],
  4: ['FL', 'FR', 'BL', 'BR'],
  5: ['FL', 'FR', 'C', 'BL', 'BR'],
  6: ['FL', 'FR', 'C', 'LFE', 'BL', 'BR'],
  7: ['FL', 'FR', 'C', 'LFE', 'BC', 'SL', 'SR'],
  8: ['FL', 'FR', 'C', 'LFE', 'BL', 'BR', 'SL', 'SR'],
});

/**
 * Names of the channels of a standard layout with `count` channels ("" for a count with no standard layout).
 * @param {number} count
 * @returns {string[]}
 */
export function channelLabels(count) {
  const known = /** @type {Record<number, string[]>} */ (LABELS)[count];
  if (known) return [...known];
  return Array.from({ length: Math.max(0, count) }, () => '');
}

/** A device that has not been configured yet: plays at full volume, no delay, automatic channels. */
/**
 * @param {string | null} [id]
 * @param {string} [name]
 * @returns {OutputDeviceEntry}
 */
export function defaultEntry(id = null, name = '') {
  return { id, name, enabled: true, delayMs: 0, channelMap: null };
}

/** @param {unknown} value @param {number} min @param {number} max @param {number} fallback */
function clampNumber(value, min, max, fallback) {
  return typeof value === 'number' && Number.isFinite(value) ? Math.min(max, Math.max(min, Math.round(value))) : fallback;
}

/**
 * Repair one entry the way the backend does.
 * @param {unknown} value
 * @returns {OutputDeviceEntry}
 */
export function normalizeEntry(value) {
  const source = typeof value === 'object' && value !== null ? /** @type {Record<string, any>} */ (value) : {};
  const map = Array.isArray(source.channelMap)
    ? source.channelMap.slice(0, 32).map((route) => ({
      source: typeof route?.source === 'number' && Number.isInteger(route.source) && route.source >= 0 && route.source < 256
        ? route.source
        : null,
      gainPercent: clampNumber(route?.gainPercent, 0, 100, 100),
    }))
    : null;
  return {
    id: typeof source.id === 'string' && source.id.trim() ? source.id : null,
    name: typeof source.name === 'string' ? source.name : '',
    enabled: source.enabled !== false,
    delayMs: clampNumber(source.delayMs, 0, OUTPUT_DELAY_MAX_MS, 0),
    channelMap: map,
  };
}

/**
 * The rows of the device list, in the order that decides the main output: the system default, the devices Windows
 * lists now, then saved devices that are not connected. A row carries its saved entry, or `null` when never
 * configured.
 * @param {OutputDeviceEntry[]} entries
 * @param {AudioOutputDevice[]} devices
 * @returns {DeviceRow[]}
 */
export function buildDeviceRows(entries, devices) {
  const saved = new Map(entries.map((entry) => [entry.id, entry]));
  const current = devices.find((device) => device.isDefault) ?? null;
  /** @type {DeviceRow[]} */
  const rows = [{
    key: 'default',
    id: null,
    name: current?.name ?? '',
    connected: true,
    isDefault: true,
    channels: current?.channels ?? 2,
    sampleRate: current?.sampleRate ?? 0,
    entry: saved.get(null) ?? null,
  }];
  for (const device of devices) {
    rows.push({
      key: device.id,
      id: device.id,
      name: device.name,
      connected: true,
      isDefault: device.isDefault,
      channels: device.channels,
      sampleRate: device.sampleRate,
      entry: saved.get(device.id) ?? null,
    });
  }
  for (const entry of entries) {
    if (entry.id !== null && !devices.some((device) => device.id === entry.id)) {
      rows.push({
        key: entry.id,
        id: entry.id,
        name: entry.name,
        connected: false,
        isDefault: false,
        channels: entry.channelMap?.length || 2,
        sampleRate: 0,
        entry,
      });
    }
  }
  return rows;
}

/**
 * Whether a row plays: its entry says so, and with no entry enabled at all the system default plays.
 * @param {DeviceRow[]} rows
 * @param {number} index
 */
export function rowEnabled(rows, index) {
  const row = rows[index];
  if (row.entry) return row.entry.enabled;
  return index === 0 && !rows.some((candidate) => candidate.entry?.enabled);
}

/**
 * Index of the main output (the first row that plays).
 * @param {DeviceRow[]} rows
 */
export function mainRowIndex(rows) {
  const index = rows.findIndex((_, position) => rowEnabled(rows, position));
  return index < 0 ? 0 : index;
}

/**
 * How many rows play.
 * @param {DeviceRow[]} rows
 */
export function playingCount(rows) {
  return rows.filter((_, position) => rowEnabled(rows, position)).length;
}

/**
 * The entries to save: every configured row, in row order, with the device's current name remembered.
 * @param {DeviceRow[]} rows
 * @returns {OutputDeviceEntry[]}
 */
export function entriesFromRows(rows) {
  return rows
    .filter((row) => row.entry)
    .map((row) => ({ ...(/** @type {OutputDeviceEntry} */ (row.entry)), id: row.id, name: row.id === null ? '' : row.name }));
}

/**
 * Update one row's entry (creating it from the defaults the first time).
 * @param {DeviceRow[]} rows
 * @param {number} index
 * @param {Partial<OutputDeviceEntry>} patch
 * @returns {DeviceRow[]}
 */
export function patchRow(rows, index, patch) {
  const next = rows.map((row) => ({ ...row }));
  // The system default plays implicitly while nothing is configured; make that explicit before another device joins.
  if (index !== 0 && rows[0].entry === null && rowEnabled(rows, 0)) next[0].entry = defaultEntry(null, '');
  const row = next[index];
  const base = row.entry ?? { ...defaultEntry(row.id, row.name), enabled: rowEnabled(rows, index) };
  row.entry = normalizeEntry({ ...base, ...patch, id: row.id });
  return next;
}

/**
 * Turn a row on or off. The system default and chosen devices exclude each other: choosing a device turns the system
 * default off (the same endpoint would play twice), and choosing the system default turns the chosen devices off.
 * @param {DeviceRow[]} rows
 * @param {number} index
 * @param {boolean} enabled
 * @returns {DeviceRow[]}
 */
export function setRowEnabled(rows, index, enabled) {
  let next = patchRow(rows, index, { enabled });
  if (!enabled) return next;
  for (let other = 0; other < next.length; other += 1) {
    if (other === index) continue;
    const conflicts = index === 0 ? other !== 0 : other === 0;
    if (conflicts && rowEnabled(next, other)) next = patchRow(next, other, { enabled: false });
  }
  return next;
}

/**
 * Whether turning this row off would leave nothing playing.
 * @param {DeviceRow[]} rows
 * @param {number} index
 */
export function isLastPlaying(rows, index) {
  return rowEnabled(rows, index) && playingCount(rows) <= 1;
}

/**
 * The identity map for `channels` output channels (output n takes source n).
 * @param {number} channels
 * @returns {ChannelRoute[]}
 */
export function standardRoutes(channels) {
  return Array.from({ length: channels }, (_, index) => ({
    source: index < OUTPUT_MAX_SOURCE_CHANNELS ? index : null,
    gainPercent: 100,
  }));
}

/**
 * Ready-made channel maps for a device with `channels` output channels.
 * @param {number} channels
 * @returns {{ id: string, routes: ChannelRoute[] }[]}
 */
export function routePresets(channels) {
  /** @type {{ id: string, routes: ChannelRoute[] }[]} */
  const presets = [{ id: 'standard', routes: standardRoutes(channels) }];
  if (channels >= 2) {
    const swapped = standardRoutes(channels);
    swapped[0].source = 1;
    swapped[1].source = 0;
    presets.push({ id: 'swap', routes: swapped });
    const left = standardRoutes(channels);
    left[1].source = 0;
    presets.push({ id: 'left', routes: left });
    const right = standardRoutes(channels);
    right[0].source = 1;
    presets.push({ id: 'right', routes: right });
  }
  return presets;
}

/**
 * Whether two channel maps are the same (`null` is automatic).
 * @param {ChannelRoute[] | null} a
 * @param {ChannelRoute[] | null} b
 */
export function routesEqual(a, b) {
  if (a === null || b === null) return a === b;
  return a.length === b.length && a.every((route, index) => route.source === b[index].source && route.gainPercent === b[index].gainPercent);
}

/**
 * Id of the preset the map matches, or `null`.
 * @param {number} channels
 * @param {ChannelRoute[] | null} map
 * @returns {string | null}
 */
export function activePreset(channels, map) {
  if (map === null) return null;
  return routePresets(channels).find((preset) => routesEqual(preset.routes, map))?.id ?? null;
}

/** "250 ms" @param {number} delayMs */
export function formatDelay(delayMs) {
  return `${Math.round(delayMs)} ms`;
}

/**
 * @typedef {{ id: string | null, state: 'playing' | 'opening' | 'lost', message: string | null, deviceRateHz: number | null, channels: number | null, converted: boolean }} DeviceStatus
 */

/**
 * The map after clicking the matrix cell (source channel `source`, output channel `output`): the cell turns on
 * (that output takes that source) or, when it was on, the output goes silent.
 * @param {ChannelRoute[] | null} map
 * @param {number} channels
 * @param {number} source
 * @param {number} output
 * @returns {ChannelRoute[]}
 */
export function toggleCell(map, channels, source, output) {
  const next = (map ?? standardRoutes(channels)).map((route) => ({ ...route }));
  while (next.length < channels) next.push({ source: null, gainPercent: 100 });
  next[output] = next[output].source === source
    ? { source: null, gainPercent: next[output].gainPercent }
    : { source, gainPercent: next[output].source === null ? 100 : next[output].gainPercent };
  return next;
}

/**
 * The map after changing the gain of an active cell by `delta` percent; null when the cell is off.
 * @param {ChannelRoute[] | null} map
 * @param {number} channels
 * @param {number} source
 * @param {number} output
 * @param {number} delta
 * @returns {ChannelRoute[] | null}
 */
export function nudgeCell(map, channels, source, output, delta) {
  const base = map ?? standardRoutes(channels);
  if (base[output]?.source !== source) return null;
  const next = base.map((route) => ({ ...route }));
  next[output].gainPercent = Math.min(100, Math.max(0, next[output].gainPercent + delta));
  return next;
}

/**
 * How many source-channel rows the matrix shows: the stereo pair, more when the map uses higher channels, all eight
 * when expanded.
 * @param {ChannelRoute[] | null} map
 * @param {boolean} expanded
 */
export function sourceRowCount(map, expanded) {
  if (expanded) return OUTPUT_MAX_SOURCE_CHANNELS;
  const highest = (map ?? []).reduce((most, route) => (route.source === null ? most : Math.max(most, route.source)), 1);
  return Math.min(OUTPUT_MAX_SOURCE_CHANNELS, highest + 1);
}

/**
 * Whether two device lists are the same choice (names are only a display cache and do not count).
 * @param {OutputDeviceEntry[] | null} a
 * @param {OutputDeviceEntry[] | null} b
 */
export function sameDevices(a, b) {
  if (a === null || b === null) return a === b;
  const key = (/** @type {OutputDeviceEntry} */ entry) => JSON.stringify([entry.id, entry.enabled, entry.delayMs, entry.channelMap]);
  const left = a.map(key).sort();
  const right = b.map(key).sort();
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

/**
 * The status entry of a device, if the player reported one.
 * @param {DeviceStatus[]} statuses
 * @param {string | null} id
 * @returns {DeviceStatus | null}
 */
export function statusFor(statuses, id) {
  return statuses.find((status) => status.id === id) ?? null;
}

/**
 * The player reports the device behind its main stream first; that stream carries the track's own sample-rate path.
 * @param {DeviceStatus[]} statuses
 * @param {string | null} id
 */
export function isMainStream(statuses, id) {
  return statuses.length > 0 && statuses[0].state === 'playing' && statuses[0].id === id;
}

/**
 * The short form of a device name for tight places (matrix headers): the text in the last pair of parentheses
 * ("Headphones (WF-1000XM6)" -> "WF-1000XM6"), or the whole name.
 * @param {string} name
 */
export function shortDeviceName(name) {
  const match = /\(([^()]+)\)\s*$/.exec(name);
  return match ? match[1].trim() : name;
}

/**
 * The id the player reports volumes under for a row: the device itself, or for the system default row the device
 * that is the default right now.
 * @param {DeviceRow} row
 * @param {AudioOutputDevice[]} devices
 * @returns {string | null}
 */
export function volumeDeviceId(row, devices) {
  if (row.id !== null) return row.id;
  return devices.find((device) => device.isDefault)?.id ?? null;
}

/**
 * Exclusive mode plays on one device: the row becomes the only one that plays.
 * @param {DeviceRow[]} rows
 * @param {number} index
 * @returns {DeviceRow[]}
 */
export function selectOnly(rows, index) {
  let next = setRowEnabled(rows, index, true);
  for (let other = 0; other < next.length; other += 1) {
    if (other !== index && rowEnabled(next, other)) next = patchRow(next, other, { enabled: false });
  }
  return next;
}

/**
 * Rows that are meant to play but whose device is gone (a Bluetooth device that disconnected, an unplugged DAC).
 * @param {DeviceRow[]} rows
 * @returns {number[]}
 */
export function disconnectedPlayingRows(rows) {
  return rows.map((_, index) => index).filter((index) => !rows[index].connected && rowEnabled(rows, index));
}

/**
 * The row of the device that carries on when a playing device is gone: Windows moves the default to the next
 * device, and the player follows it. -1 when nothing is missing or no connected default exists.
 * @param {DeviceRow[]} rows
 * @returns {number}
 */
export function takeoverRow(rows) {
  if (disconnectedPlayingRows(rows).length === 0) return -1;
  return rows.findIndex((row) => row.id !== null && row.connected && row.isDefault);
}

/**
 * Whether two device lists are the same (so a poll that found nothing new does not redraw the page).
 * @param {AudioOutputDevice[]} a
 * @param {AudioOutputDevice[]} b
 */
export function sameDeviceList(a, b) {
  return a.length === b.length && a.every((device, index) => {
    const other = b[index];
    return device.id === other.id && device.name === other.name && device.isDefault === other.isDefault
      && device.channels === other.channels && device.sampleRate === other.sampleRate;
  });
}
