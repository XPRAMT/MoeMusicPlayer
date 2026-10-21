# Backup export and import

Status: Accepted

Decision date: 2026-10-19, Asia/Taipei (UTC+08:00)

## Context

Everything the player knows (settings, the library database with listening statistics, lyric choices and playlists, the favourites file) lives in one private folder. On Android that folder cannot be reached from outside, so reinstalling the app (for example after the signing key change of ADR 0023) loses it, and Android's automatic backup is unreliable for sideloaded apps.

## Decision

- Settings → About → "Backup and restore" has **Export** and **Import** (Windows and Android).
- **Export** writes one zip (`MoeMusicPlayer-backup-<UTC date>-<time>.zip`): `manifest.json` (app, format version 1, platform, build version, time), `settings.json`, the library database, `MyLove.m3u8` and the main background image. The database is a consistent snapshot made with `VACUUM INTO` through the open connection, so the running player is not disturbed and the file includes what is still in the WAL. **Album covers are not exported** (a cache rebuilt from the music files), nor logs or update files. Windows saves through the system save dialog; Android through the document picker and a streaming copy into the chosen document (`copyFileToUri` of the media-index plugin, no size cap).
- **Import** takes that zip **or any single file from it** (`settings.json`, the database `.sqlite3`, `MyLove.m3u8`) and only replaces the parts it is given. Several files can be imported one after the other (Windows can select several at once). A zip must carry a manifest of this app, format version and **platform**, and may contain only the known entries (any other name or a path that leaves the folder refuses it), each within a size limit. Every part is checked the way the player reads it (the settings file is parsed, the database is opened with the normal startup validation) before it is accepted, so a bad file never reaches the next start.
- Nothing is replaced while the player runs. An accepted import waits in `UserData/import-pending` (`READY.json` lists the parts); **at the next start, before the settings and the database are opened**, the current files are moved to `before-import` (the previous `before-import` is dropped) and the imported ones put in place; a failure puts everything back. The page tells the user to close the app completely and open it again.

## Consequences

- A backup cannot move between Windows and Android (paths and device settings differ); a single file is not checked for this.
- The favourites source in `settings.json` stores an absolute path; restoring a Windows backup into a differently located portable folder leaves that path pointing to the old place.
- The import is a replacement, not a merge: the imported database replaces the current one, so listening statistics made after the backup are lost (they remain in `before-import`).

## Design change (2026-10-21): export panel, accounts, no background image

- **Export opens a panel** where the user ticks what to include: library database, settings, accounts (remote servers with their passwords), favourites list. At least one item is required. The main background image is **not** part of the backup any more (it was never reliably covered: the single image is the file `UserData/background`); the settings keep the slideshow folder path, and a restored setting that points at an image or folder this device does not have falls back quietly to the plain background (no error, no blank page).
- **Accounts**: only when ticked, the panel asks for a backup passphrase (at least 8 characters, typed twice). The accounts are sealed under it (XChaCha20-Poly1305, the existing `CredentialVault::export`) into the entry `remote-accounts.sealed` inside the zip. `settings.json` never contains a password; without the accounts ticked its remote servers stay as "needs a password". Accounts can be exported without the settings; on import they then merge into the current settings (with the settings, into the staged `settings.json`). The stand-alone `.moeremote` export/import in Settings → Music sources and the `.moeremote` file type on import are removed.
- **Manifest** gains `parts` (`settings`, `library`, `accounts`, `favourites`), absent in older backups. A zip may hold any subset; import replaces only the parts present and refuses a zip with no content.
- **Import** of a zip with the accounts entry asks for the passphrase (wrong passphrase: an error and the accounts keep waiting; "skip" forgets them). Old backups are still accepted: the `background/…` entries are ignored and the old entry name `remote-accounts.moeremote` is read like the new one.
