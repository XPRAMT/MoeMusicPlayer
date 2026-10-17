# Interface language: source-text keys, generated Simplified catalog, reload to apply

Status: Accepted

Decision date: 2026-10-18, Asia/Taipei (UTC+08:00)

## Context

Every interface string was hard-coded Traditional Chinese (about 730 distinct strings in 40 frontend files). The user wants a Simplified Chinese interface first, without caring about longer text, and Japanese and English later. Languages follow the system by default and can be chosen in Settings → Main, in a new "Language" block at the bottom.

## Decision

- **Source-text keys.** Each user-visible string is written `tr('繁體中文原文')` (`src/lib/i18n.js`). The Traditional original is the key, so the source language needs no catalog, a missing translation shows the readable original, and the code stays readable. Strings with values use positional placeholders: `tr('共 {0} 首', [count])`. Other languages are catalogs keyed by the source text, `src/lib/i18n/<language>.json`.
- **Simplified catalog is generated.** `node scripts/i18n-catalog.mjs` finds every `tr()` call, converts each source string with OpenCC (`twp` to `cn`, Taiwan phrases to Mainland), applies Mainland software wording from `zh-CN.terms.json` (for example 播放清單 to 播放列表, 演出者 to 艺术家, 預設 to 默认, 目前 to 当前) and finally the hand-written `zh-CN.overrides.json` (which wins, used where OpenCC is wrong). `tests/i18n-catalog.test.mjs` fails when the committed catalog is out of date or loses a placeholder. Japanese and English cannot be generated; they will be hand-written catalogs added to `CATALOGS` and `LANGUAGES`, with the choice list growing accordingly.
- **The language is fixed when a page loads.** Changing it saves the choice (`settings.json` → `language`: `system`, `zh-TW` or `zh-CN`; commands `settings_get_language` and `settings_set_language`), copies it to `localStorage` (`moe.languageChoice`, so the first paint of every window already uses it) and reloads the window. Module-level constants and plain helper functions can therefore call `tr()` with no reactivity. The Rust command broadcasts `app-language-changed`; the taskbar-lyrics window and the spectrogram window follow it by reloading. At startup the main window compares the saved choice with the one it loaded with and reloads once if they differ (guarded against loops).
- **System language.** `navigator.languages`: the first Chinese tag decides, Simplified for `zh-CN`, `zh-SG`, `zh-Hans` and plain `zh`, Traditional otherwise; every non-Chinese system uses Traditional (the source language) until English exists. `<html lang>` is set (`zh-Hans` or `zh-Hant`) so the browser picks the right glyph variants; date formatting uses the active language as its locale.
- **Migration.** The existing strings were wrapped mechanically by a one-time script that parsed the Svelte and JavaScript sources (text runs, attribute values, string and template literals; comparisons, property keys and the OpenCC dictionary code were skipped). Source-reading tests (which grep for literals such as `aria-label="播放"`) import `tests/helpers/untr-fs.mjs`, which removes the wrapping from `src/` files they read.

## Not covered (yet)

- Messages produced by the Rust backend (errors, file dialog titles) stay Traditional. They need either error codes translated in the frontend or a Rust-side catalog.
- Data is never translated: track titles, artist and album names, lyrics, playlist names, release notes from the update server.
- Operating-system chrome (window control tooltips, native dialogs) follows the system language.
- Quick-index group labels from the library (such as 漢字 and 未知) are produced by the backend.
