import { tr } from './i18n.js';
// Spectrogram window settings (ADR 0013): persisted per device in localStorage and sent
// to the Rust renderer as `SpectrogramOptions`.

const KEY = 'moe.spectrogramSettings';

export const FFT_SIZES = [1024, 2048, 4096, 8192, 16384];
export const WINDOW_FUNCTIONS = [
  { value: 'hann', label: 'Hann' },
  { value: 'hamming', label: 'Hamming' },
  { value: 'blackman', label: 'Blackman' },
  { value: 'blackmanHarris', label: 'Blackman-Harris' },
  { value: 'rectangular', label: tr('矩形（無窗）') },
];
/** CSS gradients mirror the Rust colormaps (crates/player-tags/src/spectrogram.rs; Spek's is computed there, sampled here). */
export const COLORMAPS = [
  { value: 'spek', label: 'Spek', gradient: 'linear-gradient(90deg,#000000 0%,#3a0087 8%,#3200ff 16%,#0020ff 25%,#008cff 33%,#00ffee 42%,#09ff00 50%,#56ff00 58%,#adff00 67%,#faff00 75%,#ffb100 83%,#ff0000 100%)' },
  { value: 'magma', label: 'Magma', gradient: 'linear-gradient(90deg,#000004,#3b0f70 20%,#8c2981 40%,#de4968 60%,#fe9f6d 80%,#fcfdbf)' },
  { value: 'inferno', label: 'Inferno', gradient: 'linear-gradient(90deg,#000004,#420a68 20%,#932667 40%,#dd513a 60%,#fca50a 80%,#fcffa4)' },
  { value: 'viridis', label: 'Viridis', gradient: 'linear-gradient(90deg,#440154,#3b528b 25%,#21918c 50%,#5ec962 75%,#fde725)' },
  { value: 'grayscale', label: tr('灰階'), gradient: 'linear-gradient(90deg,#000,#fff)' },
];

/**
 * @typedef {{ mode: 'mix' } | { mode: 'separate' } | { mode: 'channel', index: number }} ChannelSelect
 * @typedef {{ fftSize: number, window: string, colormap: string, channel: ChannelSelect,
 *   dbFloor: number, dbCeiling: number, logFrequency: boolean, maxFrequencyHz: number | null }} SpectrogramSettings
 */

/** @type {SpectrogramSettings} */
export const DEFAULT_SPECTROGRAM_SETTINGS = {
  fftSize: 4096,
  window: 'hann',
  colormap: 'spek',
  channel: { mode: 'mix' },
  dbFloor: -120,
  dbCeiling: 0,
  logFrequency: false,
  maxFrequencyHz: null,
};

/** @param {unknown} value @returns {SpectrogramSettings} */
export function normalizeSpectrogramSettings(value) {
  const raw = /** @type {Record<string, any>} */ (value && typeof value === 'object' ? value : {});
  const d = DEFAULT_SPECTROGRAM_SETTINGS;
  const num = (/** @type {unknown} */ v, /** @type {number} */ fallback, /** @type {number} */ min, /** @type {number} */ max) => {
    const n = Number(v);
    return Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : fallback;
  };
  let dbFloor = num(raw.dbFloor, d.dbFloor, -200, -1);
  let dbCeiling = num(raw.dbCeiling, d.dbCeiling, -199, 20);
  if (dbCeiling <= dbFloor) dbCeiling = Math.min(20, dbFloor + 10);
  const channel = raw.channel?.mode === 'separate' ? { mode: 'separate' }
    : raw.channel?.mode === 'channel' && Number.isInteger(raw.channel.index) && raw.channel.index >= 0 ? { mode: 'channel', index: raw.channel.index }
    : { mode: 'mix' };
  const maxFrequencyHz = raw.maxFrequencyHz == null ? null : num(raw.maxFrequencyHz, 0, 100, 400000) || null;
  return {
    fftSize: FFT_SIZES.includes(Number(raw.fftSize)) ? Number(raw.fftSize) : d.fftSize,
    window: WINDOW_FUNCTIONS.some((w) => w.value === raw.window) ? raw.window : d.window,
    colormap: COLORMAPS.some((c) => c.value === raw.colormap) ? raw.colormap : d.colormap,
    channel: /** @type {ChannelSelect} */ (channel),
    dbFloor,
    dbCeiling,
    logFrequency: raw.logFrequency === true,
    maxFrequencyHz,
  };
}

/** @returns {SpectrogramSettings} */
export function loadSpectrogramSettings() {
  try {
    return normalizeSpectrogramSettings(JSON.parse(localStorage.getItem(KEY) ?? 'null'));
  } catch {
    return { ...DEFAULT_SPECTROGRAM_SETTINGS };
  }
}

/** @param {SpectrogramSettings} settings */
export function saveSpectrogramSettings(settings) {
  try {
    localStorage.setItem(KEY, JSON.stringify(normalizeSpectrogramSettings(settings)));
  } catch {
    // ignore
  }
}

/**
 * Renderer options for the Rust side.
 * @param {SpectrogramSettings} settings @param {number} columns @param {number} height
 */
export function toRenderOptions(settings, columns, height) {
  return {
    columns: Math.round(Math.min(8000, Math.max(16, columns))),
    height: Math.round(Math.min(4096, Math.max(64, height))),
    fftSize: settings.fftSize,
    window: settings.window,
    colormap: settings.colormap,
    channel: settings.channel,
    dbFloor: settings.dbFloor,
    dbCeiling: settings.dbCeiling,
    logFrequency: settings.logFrequency,
    maxFrequencyHz: settings.maxFrequencyHz,
    minLogFrequencyHz: 20,
  };
}

/**
 * Fraction (0 = bottom, 1 = top) of a frequency on the axis.
 * @param {number} hz @param {number} maxHz @param {boolean} log
 */
export function frequencyToFraction(hz, maxHz, log, minLog = 20) {
  if (!log) return hz / maxHz;
  const lo = Math.min(minLog, maxHz / 2);
  if (hz <= lo) return 0;
  return Math.log(hz / lo) / Math.log(maxHz / lo);
}

/** @param {number} fraction @param {number} maxHz @param {boolean} log */
export function fractionToFrequency(fraction, maxHz, log, minLog = 20) {
  if (!log) return fraction * maxHz;
  const lo = Math.min(minLog, maxHz / 2);
  return lo * Math.pow(maxHz / lo, fraction);
}

/** Axis ticks in Hz. @param {number} maxHz @param {boolean} log */
export function frequencyTicks(maxHz, log) {
  if (log) {
    return [50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000, 50000, 100000].filter((f) => f < maxHz * 0.98);
  }
  const step = [500, 1000, 2000, 5000, 10000, 20000].find((s) => maxHz / s <= 12) ?? 50000;
  const ticks = [];
  for (let f = 0; f <= maxHz; f += step) ticks.push(f);
  return ticks;
}
