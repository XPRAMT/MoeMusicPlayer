// Panel and dialog choices kept in settings.json (`uiPreferences`, src-tauri/src/settings.rs): the spectrogram options
// and A/B compare view (ADR 0013), the cover edit options of the song details panel, the dragged artist column width
// and the last tab of Settings -> lyrics. Older builds kept them in this window's localStorage; those keys are read
// once (only for values settings.json does not have yet) and then removed. The IPC side is ui-preferences-store.ts.
import { normalizeSpectrogramSettings } from './spectrogram-settings.js';
import { normalizeComparePrefs } from './spectrogram-compare.js';
import { normalizeCoverEditOptions } from './cover-edit-options.js';

const SPECTROGRAM_KEY = 'moe.spectrogramSettings.v2';
/** The first version: its colour map, window and dB range carry over; FFT size and channel start from the defaults. */
const SPECTROGRAM_V1_KEY = 'moe.spectrogramSettings';
const COMPARE_KEY = 'moe.spectrogramCompare';
const COVER_EDIT_KEY = 'moe.coverEditOptions.v2';
/** Dropped without carrying it over (the v2 defaults replaced it). */
const COVER_EDIT_V1_KEY = 'moe.coverEditOptions';
const ARTIST_TITLE_WIDTH_KEY = 'moe.artistIndex.titleWidthPx';
const LYRICS_TAB_KEY = 'moe.settings.lyricsTab';

/** The keys of `uiPreferences`. */
export const UI_PREFERENCE_KEYS = Object.freeze(['spectrogram', 'spectrogramCompare', 'coverEdit', 'artistIndexTitleWidthPx', 'lyricsSettingsTab']);

/** Dragged 演出者 column width (same range as settings.rs `ARTIST_INDEX_TITLE_WIDTH_RANGE`). */
export const ARTIST_TITLE_WIDTH_MIN = 120;
export const ARTIST_TITLE_WIDTH_MAX = 1200;
/** @param {unknown} value @returns {number | null} */
export function normalizeArtistTitleWidth(value) {
  if (value === null || value === undefined || value === '') return null;
  const width = Number(value);
  return Number.isFinite(width) && width > 0 ? Math.round(Math.min(ARTIST_TITLE_WIDTH_MAX, Math.max(ARTIST_TITLE_WIDTH_MIN, width))) : null;
}

/** @returns {Storage | null} */
export function browserStorage() {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}

/**
 * The old localStorage values as a `uiPreferences` patch (only the ones that exist and can be read).
 * @param {{ getItem(key: string): string | null }} storage
 * @returns {Record<string, unknown>}
 */
export function readLegacyUiPreferences(storage) {
  /** @param {string | null} text */
  const parse = (text) => {
    try {
      const value = text === null ? null : JSON.parse(text);
      return value && typeof value === 'object' ? value : null;
    } catch {
      return null;
    }
  };
  /** @type {Record<string, unknown>} */
  const patch = {};
  const spectrogram = parse(storage.getItem(SPECTROGRAM_KEY));
  const spectrogramV1 = spectrogram ? null : parse(storage.getItem(SPECTROGRAM_V1_KEY));
  if (spectrogram) patch.spectrogram = normalizeSpectrogramSettings(spectrogram);
  else if (spectrogramV1) {
    const { fftSize: _fft, channel: _channel, logFrequency: _log, ...kept } = spectrogramV1;
    patch.spectrogram = normalizeSpectrogramSettings(kept);
  }
  const compare = parse(storage.getItem(COMPARE_KEY));
  if (compare) patch.spectrogramCompare = normalizeComparePrefs(compare);
  const coverEdit = parse(storage.getItem(COVER_EDIT_KEY));
  if (coverEdit) patch.coverEdit = normalizeCoverEditOptions(coverEdit);
  const width = normalizeArtistTitleWidth(storage.getItem(ARTIST_TITLE_WIDTH_KEY));
  if (width !== null) patch.artistIndexTitleWidthPx = width;
  const tab = storage.getItem(LYRICS_TAB_KEY);
  if (tab === 'general' || tab === 'taskbar' || tab === 'floating') patch.lyricsSettingsTab = tab;
  return patch;
}

/** Removes every old key (after settings.json holds what was worth keeping). @param {{ removeItem(key: string): void }} storage */
export function removeLegacyUiPreferences(storage) {
  for (const key of [SPECTROGRAM_KEY, SPECTROGRAM_V1_KEY, COMPARE_KEY, COVER_EDIT_KEY, COVER_EDIT_V1_KEY, ARTIST_TITLE_WIDTH_KEY, LYRICS_TAB_KEY]) {
    try {
      storage.removeItem(key);
    } catch {
      // storage unavailable
    }
  }
}

/**
 * The part of the old values to write: only keys settings.json has no value for (a value already there wins).
 * @param {Record<string, unknown> | null | undefined} stored
 * @param {Record<string, unknown>} legacy
 */
export function legacyPatchToCarry(stored, legacy) {
  /** @type {Record<string, unknown>} */
  const carry = {};
  for (const key of UI_PREFERENCE_KEYS) {
    if (legacy[key] !== undefined && (stored?.[key] === null || stored?.[key] === undefined)) carry[key] = legacy[key];
  }
  return carry;
}

/**
 * Merges queued changes (several quick changes, e.g. a dragged slider, become one write).
 * @param {Record<string, unknown>} queued @param {Record<string, unknown>} patch
 */
export function mergeUiPreferencesPatch(queued, patch) {
  return { ...queued, ...patch };
}