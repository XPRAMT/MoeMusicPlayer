# Hide tracks whose files are missing, keep their records

Status: Accepted and implemented on 2026-10-23

Decision date: 2026-10-23, Asia/Taipei (UTC+08:00)

## Context

When a file listed in a playlist, or found earlier in a watched folder, is deleted or moved, the library kept showing
it (playlists) or deleted its mapping together with user overrides and play history (folders). The maintainer wants
such tracks hidden everywhere but restorable: if a later sync finds the file again, it reappears with its data.

## Decision

- Schema v20 adds `source_mappings.missing_since_utc_ms` (NULL = present). The app's normal migration adds the column;
  existing rows stay visible.
- Every list query (library, albums, artists, playlists, search, counts) that filters by `r.enabled=1 AND
  moe_in_scope(...)` also requires `m.missing_since_utc_ms IS NULL`.
- A missing file is told apart from an offline source: `MediaSourceErrorKind::MissingFile` is used only when opening
  the file fails with NotFound **and** its volume root (`volume_root`: drive root on Windows, `/storage/emulated/N`,
  `/storage/<VOL>` or `/mnt/media_rw/<VOL>` on Android) still exists. An unplugged drive or card gives `Other`, so the
  track stays visible as before.
- Playlist sources: entries with a MissingFile error are marked missing (first time only). The playlist file is never
  edited automatically.
- Folder sources (Windows filesystem / system index, Android filesystem / MediaStore / SAF): on a Complete scan,
  mappings not seen any more are marked missing instead of deleted. Unavailable or Incomplete scans change nothing.
  Remote sources keep deleting unseen mappings.
- Any later scan that observes the file clears the mark (`persist_mapping` sets it to NULL).
- `SyncApplyStats.hidden_missing_mappings` counts newly hidden mappings.

- The sync summary (Settings, manage music sources) shows a "Remove from playlist" button next to each missing-file
  error. `sync_remove_missing_playlist_entry` reuses the favorite button's unfavorite rewrite
  (`rewrite_local_playlist_membership`, keeping `<name>.bak`), only for local playlist files and only while the file
  is still missing; the page then synchronizes again. This is the only way a playlist file is edited for a missing file.

## Consequences

- Moved or renamed folder files leave a hidden mapping behind (the new path is a new track).
- Files under a newly excluded subfolder become hidden rather than removed.
- Hidden rows keep growing until the source is removed; there is no purge yet.