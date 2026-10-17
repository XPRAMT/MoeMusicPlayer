# Interface language: source-text keys, Simplified generated and Japanese/English hand-written, reload to apply

Status: Accepted

Decision date: 2026-10-18, Asia/Taipei (UTC+08:00)

## Context

Every interface string was hard-coded Traditional Chinese (about 730 distinct strings in 40 frontend files). The user wants Simplified Chinese first (done 2026-10-18), then Japanese and English (added the same day), without caring about longer text; layout of the longer English and Japanese text is checked by the user afterwards. Languages follow the system by default and can be chosen in Settings → Main, in the "Language" block at the bottom.

## Decision

- **Source-text keys.** Each user-visible string is written `tr('繁體中文原文')` (`src/lib/i18n.js`). The Traditional original is the key, so the source language needs no catalog, a missing translation shows the readable original, and the code stays readable. Strings with values use positional placeholders: `tr('共 {0} 首', [count])`. Other languages are catalogs keyed by the source text, `src/lib/i18n/<language>.json`.
- **Simplified catalog is generated.** `node scripts/i18n-catalog.mjs` finds every `tr()` call, converts each source string with OpenCC (`twp` to `cn`, Taiwan phrases to Mainland), applies Mainland software wording from `zh-CN.terms.json` (for example 播放清單 to 播放列表, 演出者 to 艺术家, 預設 to 默认, 目前 to 当前) and finally the hand-written `zh-CN.overrides.json` (which wins, used where OpenCC is wrong). `tests/i18n-catalog.test.mjs` fails when the committed catalog is out of date or loses a placeholder. Japanese (`ja.json`) and English (`en.json`) are hand-written catalogs keyed by the same source strings. `tests/i18n-catalog.test.mjs` fails when one of them lacks a string or keeps a string no `tr()` uses, or when placeholders differ; `node scripts/i18n-catalog.mjs --missing` lists the gaps. Sentences are never built from fragments around a value (fragments such as `目標亮度（` + number + `%）` were turned into whole-sentence keys with placeholders) so that word order can change per language.
- **The language is fixed when a page loads.** Changing it saves the choice (`settings.json` → `language`: `system`, `zh-TW` or `zh-CN`; commands `settings_get_language` and `settings_set_language`), copies it to `localStorage` (`moe.languageChoice`, so the first paint of every window already uses it) and reloads the window. Module-level constants and plain helper functions can therefore call `tr()` with no reactivity. The Rust command broadcasts `app-language-changed`; the taskbar-lyrics window and the spectrogram window follow it by reloading. At startup the main window compares the saved choice with the one it loaded with and reloads once if they differ (guarded against loops).
- **System language.** `navigator.languages`, the first tag that has an interface wins: Chinese (Simplified for `zh-CN`, `zh-SG`, `zh-Hans` and plain `zh`, Traditional otherwise), `ja`, `en`; tags without an interface are skipped and a system with none of ours uses English (no navigator at all uses the source language). `<html lang>` is set (`zh-Hans`, `zh-Hant`, `ja`, `en`) so the browser picks the right glyph variants; date formatting uses the active language as its locale.
- **Migration.** The existing strings were wrapped mechanically by a one-time script that parsed the Svelte and JavaScript sources (text runs, attribute values, string and template literals; comparisons, property keys and the OpenCC dictionary code were skipped). Source-reading tests (which grep for literals such as `aria-label="播放"`) import `tests/helpers/untr-fs.mjs`, which removes the wrapping from `src/` files they read.

## Not covered (yet)

- Messages produced by the Rust backend (errors, file dialog titles) stay Traditional. They need either error codes translated in the frontend or a Rust-side catalog.
- Data is never translated: track titles, artist and album names, lyrics, playlist names, release notes from the update server.
- Operating-system chrome (window control tooltips, native dialogs) follows the system language.
- Rust backend messages stay Traditional by the user's decision (for now).
- Quick-index group labels come from the backend. 漢字 only exists for Japanese collation and is not translated. The 未知 placeholder (tracks without a name, artist or album) is shown through `displayUnknown()` (`src/lib/unknown-label.js`, which calls `tr('未知')`): Simplified and Japanese show 未知, English shows `unknown`.
