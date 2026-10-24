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
  Remote sources keep deleting unseen mappings. (Superseded by the Amendment below: remote folder sources now keep and
  hide unseen mappings too; `keeps_missing_files` in `player-db` includes `Remote`.)
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
## Amendment (2026-10-23): remote sources

- Remote playlist scans: a song that answers not-found counts as missing only when the account's top folder (the SMB
  share or the WebDAV root, `list("")`) is reachable; otherwise the error is an ordinary one and nothing changes
  (offline). A scan whose only errors are missing files is Complete, so those entries are hidden like local ones.
- Remote folder sources keep their mappings too (`keeps_missing_files` includes `Remote`): files not seen by a
  Complete scan are hidden and come back when found again. An unreachable server never produces a Complete scan.
  There is deliberately no guard against hiding many files at once.
- "Remove from playlist" also works for a playlist on a server: the server is asked first (the file exists: refused;
  any error other than not-found: refused with that error), then the same remote favorite job as the unfavorite
  button rewrites the server file (keeping `<name>.bak`) and synchronizes the source. The item key is the song's
  path from the account root, as the playlist names it.