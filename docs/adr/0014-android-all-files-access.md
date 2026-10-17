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

## Update (2026-10-17): All files access is the only storage permission

- The separate MediaStore read permission flow (`READ_MEDIA_AUDIO`, the 「共享音樂 / MediaStore」 authorize button and its prompt) is removed, along with the `READ_MEDIA_AUDIO` manifest entry and the `requestMediaReadPermission` / `android_media_request_permission` bridge.
- Gate for shared-music (MediaStore) scanning and direct edits: `Environment.isExternalStorageManager()` on Android 11+. On Android 10 and older, the gate is the legacy `READ_EXTERNAL_STORAGE` / `WRITE_EXTERNAL_STORAGE` runtime permission (both declared with `maxSdkVersion="29"`), requested from the same 「前往授權」 button.
- MediaStore remains the index for shared music; All files access allows querying it.
- The 所有檔案存取權 card is shown only while access is missing. Once access is granted, the MediaStore volume list loads automatically.
- Existing MediaStore sources whose access is missing report `permissionRevoked` and keep their library projection until access is granted.

## Update (2026-10-17, later): real-path sources, no per-item grants

Decision: on Android, All files access (Android ≤10: the legacy storage permission) is the
**only** storage authorization. Every per-item grant is removed:

- SAF tree/document grants for music folders, playlists, 我的最愛 and the slideshow folder
  (`pickSafTree`, `pickMyLoveFolder`, `pickPlaylistImport/Create/Export`, `releaseSafTree`,
  `hasSafPermission`, `resolveSafPlaylistEntry`, `readSafLyricSibling`, `listSafImages`, …).
- MediaStore sources (`scanMediaStore`, `mediaStoreVolumes`, the 「共享音樂 MediaStore」 block).
- Per-file MediaStore write prompts (`createWriteRequest`, `RecoverableSecurityException`,
  `writeAudioCacheLease`). Tag edits write the real file in place; without a real path the save
  fails with a request to enable 所有檔案存取權.

Folders and playlists are real filesystem paths, like desktop sources:

- 設定 → 音樂來源 on Android shows 「選擇資料夾」 and 「選擇播放清單」. The system picker is used
  only to point at a location; the tree/document URI is converted to `/storage/emulated/0/…`
  (`storage_paths`, Kotlin `externalStoragePathFor` / `documentPathFor`) and no grant is persisted.
- Folder sources have kind `android_filesystem`, a `FileSystem` locator, and are scanned by
  `fs_scan::scan_directory` (skips hidden entries, symlinks, `.nomedia` folders and `Android/` at a
  volume root; an unreadable subfolder makes the scan `Incomplete`, so nothing is deleted).
  Item id and locator key are both `unix_path_key(canonical path)`.
- Playlist sources (incl. 我的最愛) share the Windows path-based sync
  (`playlist_source_sync::sync_playlist_file_source_with`); favorites are rewritten through the
  real path (temp + rename). 我的最愛 is created as `<picked folder>/MyLove.m3u8` and an existing
  file is never overwritten.
- Playback, artwork (embedded via `file://`, then `cover.jpg`/`folder.jpg`/`cover.png`/`folder.png`)
  and `.lrc` sidecars read the real path.

Migration (`android_path_migration`, run at the start of `library_sync`, only while access is
granted, idempotent):

1. A full copy of the database (`VACUUM INTO`) and `settings.json` is written to
   `<app data>/pre-path-migration-<ms>/`, with `migration-report.txt`. If the backup fails, nothing
   is migrated.
2. Each legacy MediaStore / SAF folder source and each SAF playlist source is converted in place
   (same source id): every `source_mappings` row is rewritten to the path key the scanner will
   produce (MediaStore: `_data` via `resolveDirectPaths`; SAF: document id → path). `TrackId`s do
   not change, so play statistics, lyrics, playlist entries and the playback session stay attached.
   A MediaStore volume becomes its storage root (`/storage/emulated/0`), so files outside `Music/`
   that were in the library stay in it.
3. A legacy slideshow tree URI becomes its folder path.

Until migrated (access missing), legacy sources are reported unavailable and are not scanned, so
their projection is kept.

Root cause of 「已撤銷此播放清單的 SAF 文件樹權限」 on 我的最愛: the import picker released the
persisted grant of a tree after a cancelled/empty pick (`releasePersistedTreeReadGrant`) even when
that same tree (`primary:Music`) backed the existing 我的最愛 source. Without per-item grants this
class of failure no longer exists.

## Consequences

- One manual system toggle replaces per-file prompts. Revoking the toggle at any time silently returns to the fallback paths.
- Direct writes bypass SAF grants. They are used only for files the library already indexes, and only through the tag-save path.
