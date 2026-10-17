# Android All files access (MANAGE_EXTERNAL_STORAGE)

Status: accepted (2026-10-17). This partly supersedes the "MANAGE_EXTERNAL_STORAGE rejected" note in ADR 0008.

## Context

- Tag edits on MediaStore tracks go through `createWriteRequest`, which shows one system prompt per file (Android 11+). For batch-style editing this is unusable.
- Android 13/14+ blocks `WallpaperManager.getDrawable` unless the app holds `MANAGE_EXTERNAL_STORAGE` (or the privileged `READ_WALLPAPER_INTERNAL`). Without the pixels, the wallpaper backdrop cannot be sampled for automatic contrast, and the CSS blur does not apply to it.
- The app is side-loaded for its owner (not distributed on Play), so Play's All-files-access policy is not a blocker. The user explicitly chose this trade-off.

## Decision

- Declare `MANAGE_EXTERNAL_STORAGE` as an **optional** special permission. Settings → 音樂來源 shows its state (`Environment.isExternalStorageManager()`) and a button that opens `ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION` (fallback `ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION`). The state is re-checked whenever the app becomes visible again.
- When granted:
  - Tag saves resolve the track's content URI to its real path (MediaStore `DATA`; ExternalStorageProvider `primary:…` → `/storage/emulated/0/…`, `XXXX-XXXX:…` → `/storage/XXXX-XXXX/…`) and overwrite the file **in place**. The file is not replaced by temp + rename, because a rename makes MediaStore assign a new row id, which breaks the library's content locators. The existing byte verify / restore and work-copy editing are kept, and `MediaScannerConnection.scanFile` refreshes MediaStore.
  - System wallpaper mode copies the static wallpaper (as displayed) into the app cache as JPEG, and the frontend renders it like the Windows wallpaper, with blur, opacity, and automatic contrast. It is re-checked every 60 s.
- When not granted, or when the path cannot be resolved, everything falls back to the existing ContentResolver / SAF / `createWriteRequest` flow and to the `FLAG_SHOW_WALLPAPER` backdrop (ADR 0008). A live wallpaper always uses the backdrop.
- Reading track bytes still uses the ContentResolver. It needs no prompt and keeps one code path.

## Consequences

- One manual system toggle replaces per-file prompts. Revoking the toggle at any time silently returns to the fallback paths.
- Direct writes bypass SAF grants. They are used only for files the library already indexes, and only through the tag-save path.
