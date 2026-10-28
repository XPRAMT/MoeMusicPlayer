// DSD→PCM playback settings shown in Settings → output (ADR 0016): the digital gain and the PCM output rate.
// The backend (settings.rs `DsdPlaybackSettings`, audio crate `DsdSettings`) enforces the same ranges.

export const DSD_GAIN_MIN_DB = 0;
export const DSD_GAIN_MAX_DB = 6;
export const DSD_GAIN_STEP_DB = 0.5;
export const DSD_DEFAULT_GAIN_DB = 3;
export const DSD_OUTPUT_RATES_HZ = Object.freeze([176_400, 352_800]);
export const DSD_DEFAULT_OUTPUT_RATE_HZ = 176_400;

/**
 * @typedef {{ gainDb: number, outputRateHz: number, dop: boolean, nativePassthrough: boolean, nativeByteOrder: 'a' | 'b' }} DsdSettings
 * `nativePassthrough` (Android, ADR 0049): a DSF's raw DSD goes to the USB audio device in the exclusive mode, no PCM.
 * `nativeByteOrder` is hidden (no UI): how the four DSD bytes of a 32-bit slot are laid out, `'a'` by default.
 */

/** `dsd.nativePassthrough` when settings.json does not say (Rust `DsdPlaybackSettings::default`). */
export const DSD_DEFAULT_NATIVE_PASSTHROUGH = false;
/** `dsd.nativeByteOrder` when settings.json does not say. */
export const DSD_DEFAULT_NATIVE_BYTE_ORDER = 'a';

/**
 * Repair a settings object: the gain is clamped to 0 to 6 dB (a non-number becomes 3 dB), an unsupported rate becomes
 * 176.4 kHz.
 * @param {unknown} value
 * @returns {DsdSettings}
 */
export function normalizeDsdSettings(value) {
  const source = typeof value === 'object' && value !== null ? /** @type {Record<string, unknown>} */ (value) : {};
  const gain = typeof source.gainDb === 'number' && Number.isFinite(source.gainDb)
    ? Math.min(DSD_GAIN_MAX_DB, Math.max(DSD_GAIN_MIN_DB, source.gainDb))
    : DSD_DEFAULT_GAIN_DB;
  const rate = DSD_OUTPUT_RATES_HZ.includes(/** @type {number} */ (source.outputRateHz))
    ? /** @type {number} */ (source.outputRateHz)
    : DSD_DEFAULT_OUTPUT_RATE_HZ;
  return {
    gainDb: gain,
    outputRateHz: rate,
    dop: source.dop === true,
    nativePassthrough: source.nativePassthrough === true,
    nativeByteOrder: typeof source.nativeByteOrder === 'string' && source.nativeByteOrder.trim().toLowerCase() === 'b' ? 'b' : DSD_DEFAULT_NATIVE_BYTE_ORDER,
  };
}

/** "DSD64", "DSD128" for a 1-bit rate that is a multiple of 44.1 kHz. @param {number} dsdRateHz */
export function formatDsdName(dsdRateHz) {
  const multiple = Math.round(dsdRateHz / 44_100);
  return multiple > 0 ? `DSD${multiple}` : `${(dsdRateHz / 1e6).toFixed(1)} MHz DSD`;
}

/** "+3 dB", "0 dB" @param {number} gainDb */
export function formatDsdGain(gainDb) {
  const rounded = Math.round(gainDb * 10) / 10;
  return `${rounded > 0 ? '+' : ''}${Number.isInteger(rounded) ? rounded : rounded.toFixed(1)} dB`;
}

/** "176.4 kHz" @param {number} rateHz */
export function formatDsdOutputRate(rateHz) {
  return `${(rateHz / 1000).toFixed(1)} kHz`;
}
