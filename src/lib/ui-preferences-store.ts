// IPC side of the panel and dialog choices kept in settings.json (`uiPreferences`; see ui-preferences.js). The first
// load in a window carries the old localStorage values over once. Saves are coalesced (a short delay) so a dragged
// slider or column border writes the file once.
import { invokeCommand, type UiPreferences } from './ipc';
import { browserStorage, legacyPatchToCarry, mergeUiPreferencesPatch, readLegacyUiPreferences, removeLegacyUiPreferences } from './ui-preferences.js';

const SAVE_DELAY_MS = 250;
let loaded: Promise<UiPreferences> | null = null;
let queued: Partial<UiPreferences> = {};
let timer: ReturnType<typeof setTimeout> | null = null;
let writing: Promise<unknown> = Promise.resolve();

async function readAndCarryOver(): Promise<UiPreferences> {
  const stored = await invokeCommand('settings_get_ui_preferences', {});
  const storage = browserStorage();
  if (!storage) return stored;
  const carry = legacyPatchToCarry(stored as unknown as Record<string, unknown>, readLegacyUiPreferences(storage));
  const result = Object.keys(carry).length > 0
    ? await invokeCommand('settings_set_ui_preferences', { patch: carry as Partial<UiPreferences> })
    : stored;
  // Only once settings.json holds them are the old keys removed.
  removeLegacyUiPreferences(storage);
  return result;
}

/** The stored choices (read once per window; a failed read is tried again next time). */
export function loadUiPreferences(): Promise<UiPreferences> {
  if (!loaded) {
    const attempt = readAndCarryOver();
    loaded = attempt;
    attempt.catch(() => {
      if (loaded === attempt) loaded = null;
    });
  }
  return loaded;
}

function flush(): void {
  timer = null;
  const patch = queued;
  queued = {};
  if (Object.keys(patch).length === 0) return;
  const run = writing.then(() => invokeCommand('settings_set_ui_preferences', { patch }));
  writing = run.catch(() => {});
  loaded = run;
  run.catch(() => {
    if (loaded === run) loaded = null;
  });
}

/** Saves some of the choices (only the given keys change). */
export function saveUiPreferences(patch: Partial<UiPreferences>): void {
  queued = mergeUiPreferencesPatch(queued, patch) as Partial<UiPreferences>;
  if (timer) clearTimeout(timer);
  timer = setTimeout(flush, SAVE_DELAY_MS);
}

if (typeof window !== 'undefined') {
  // A window closing within the delay still writes its last change.
  window.addEventListener('pagehide', () => {
    if (timer) {
      clearTimeout(timer);
      flush();
    }
  });
}