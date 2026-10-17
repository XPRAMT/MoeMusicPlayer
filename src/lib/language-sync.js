// Keeps the interface language of a window in step with settings.json (ADR 0019). The language is fixed when a page
// loads, so every change is applied by reloading the page. The settings file is the authority; the choice is also
// copied into localStorage so the first paint of every window already uses the right language.
import { getLanguageChoice, normalizeLanguageChoice, storeLanguageChoice } from './i18n.js';

const RELOAD_GUARD_KEY = 'moe.languageReloadFor';

/** @param {{ reload: () => void }} [target] */
function defaultTarget(target) {
  return target ?? globalThis.location;
}

/**
 * Adopt the saved choice at startup: when it differs from what this page loaded with, remember it and reload once
 * (the guard stops a reload loop if the storage cannot be written).
 * @param {unknown} saved
 * @param {{ reload: () => void }} [target]
 * @returns {boolean} true when the page is reloading
 */
export function adoptSavedLanguage(saved, target) {
  const choice = normalizeLanguageChoice(saved);
  if (choice === getLanguageChoice()) return false;
  try {
    if (globalThis.sessionStorage?.getItem(RELOAD_GUARD_KEY) === choice) return false;
    globalThis.sessionStorage?.setItem(RELOAD_GUARD_KEY, choice);
  } catch {
    // No session storage: reload anyway, the stored copy below stops a second mismatch.
  }
  storeLanguageChoice(choice);
  defaultTarget(target).reload();
  return true;
}

/**
 * Save a new choice and apply it by reloading this window.
 * @param {unknown} next @param {(choice: string) => Promise<unknown>} save @param {{ reload: () => void }} [target]
 */
export async function changeLanguage(next, save, target) {
  const choice = normalizeLanguageChoice(next);
  if (choice === getLanguageChoice()) return;
  await save(choice);
  storeLanguageChoice(choice);
  defaultTarget(target).reload();
}

/**
 * A window that did not start the change (another window did) follows it.
 * @param {unknown} payload @param {{ reload: () => void }} [target]
 */
export function followLanguageChange(payload, target) {
  const choice = normalizeLanguageChoice(payload);
  if (choice === getLanguageChoice()) return;
  storeLanguageChoice(choice);
  defaultTarget(target).reload();
}
