/** @typedef {import('./ipc').NowPlayingAppearancePreferences} NowPlayingAppearancePreferences */
/** @typedef {'loading' | 'saved' | 'saving' | 'error' | 'preview'} SettingsState */

export const DEFAULT_NOW_PLAYING_APPEARANCE_PREFERENCES = Object.freeze({
  backgroundBlurPx: 20,
  backgroundBrightnessPercent: 40,
  coverCornerStyle: /** @type {const} */ ('rounded'),
  timelineStyle: /** @type {const} */ ('line'),
});

/**
 * @param {Partial<NowPlayingAppearancePreferences> | null | undefined} [value]
 * @returns {NowPlayingAppearancePreferences}
 */
export function normalizeNowPlayingAppearancePreferences(value = {}) {
  const blur = Number(value?.backgroundBlurPx);
  const brightness = Number(value?.backgroundBrightnessPercent);
  /** @type {import('./ipc').CoverCornerStyle} */
  const coverCornerStyle = value?.coverCornerStyle === 'square' ? 'square' : 'rounded';
  /** @type {import('./ipc').TimelineStyle} */
  const timelineStyle = value?.timelineStyle === 'edge' ? 'edge' : 'line';
  return {
    backgroundBlurPx: Number.isFinite(blur)
      ? Math.round(Math.max(0, Math.min(40, blur)))
      : DEFAULT_NOW_PLAYING_APPEARANCE_PREFERENCES.backgroundBlurPx,
    backgroundBrightnessPercent: Number.isFinite(brightness)
      ? Math.round(Math.max(0, Math.min(100, brightness)))
      : DEFAULT_NOW_PLAYING_APPEARANCE_PREFERENCES.backgroundBrightnessPercent,
    coverCornerStyle,
    timelineStyle,
  };
}

/**
 * Keep one settings write in flight and at most one replaceable latest value.
 * @param {{
 *   write: (preferences: NowPlayingAppearancePreferences) => Promise<NowPlayingAppearancePreferences>,
 *   onState?: (state: SettingsState) => void,
 *   onSaved?: (preferences: NowPlayingAppearancePreferences) => void,
 *   onError?: (error: unknown) => void,
 *   delayMs?: number,
 * }} options
 */
export function createNowPlayingAppearanceWriter({
  write,
  onState = () => {},
  onSaved = () => {},
  onError = () => {},
  delayMs = 180,
}) {
  let revision = 0;
  /** @type {ReturnType<typeof setTimeout> | undefined} */
  let timer;
  let saving = false;
  /** @type {{ revision: number, preferences: NowPlayingAppearancePreferences } | null} */
  let pending = null;
  let pendingReady = false;

  async function drain() {
    if (saving || pending === null || !pendingReady) return;
    saving = true;
    while (pending !== null && pendingReady) {
      const request = pending;
      pending = null;
      pendingReady = false;
      try {
        const saved = await write(request.preferences);
        if (request.revision === revision) onSaved(normalizeNowPlayingAppearancePreferences(saved));
      } catch (error) {
        if (request.revision === revision && pending === null) onError(error);
      }
    }
    saving = false;
  }

  /** @param {NowPlayingAppearancePreferences} preferences @param {boolean} [immediate] */
  function schedule(preferences, immediate = false) {
    revision += 1;
    pending = { revision, preferences: normalizeNowPlayingAppearancePreferences(preferences) };
    pendingReady = immediate;
    onState('saving');
    if (timer !== undefined) clearTimeout(timer);
    if (immediate) {
      timer = undefined;
      void drain();
    } else {
      timer = setTimeout(() => {
        timer = undefined;
        pendingReady = true;
        void drain();
      }, delayMs);
    }
  }

  return {
    schedule,
    invalidate() {
      revision += 1;
      pending = null;
      pendingReady = false;
      if (timer !== undefined) clearTimeout(timer);
      timer = undefined;
    },
  };
}
