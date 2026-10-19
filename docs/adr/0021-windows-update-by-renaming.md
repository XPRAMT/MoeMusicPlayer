# Windows updates install by renaming the running executable

Status: Accepted (supersedes the helper-process part of ADR 0007; the manifest and helper executable it mentions were removed again by ADR 0022)

Decision date: 2026-10-19, Asia/Taipei (UTC+08:00)

## Context

ADR 0007 installed an update with a helper executable: the app exited, the helper waited for the exact parent process, replaced both files, restarted the player and rolled back on failure. The user wants the plain approach the release script already uses: Windows lets a running executable be **renamed** (not deleted or overwritten), so the installed file can be renamed away and the new file put under the original name while the player keeps running.

## Decision

- The update prompt offers three choices: **Ignore**, **Install automatically** (`nextLaunch`) and **Install and restart** (`restartNow`).
- Both install choices download and verify the release exactly as before (Release API, manifest digest, SHA-256 of both files), then `install_in_place`: copy and verify `moemusicplayer.exe` and `moemusicplayer-updater.exe` beside the installed files under temporary names, rename the installed files to `.moemusicplayer.previous.<id>.bak` / `.moemusicplayer-updater.previous.<id>.bak`, rename the verified copies to the original names. A failure at any step puts the original names back and removes the copies. Only the newest backup is kept; older ones (and copies of an interrupted attempt) are removed at the start of the next install.
- **Install automatically** touches nothing else: playback continues and the new version starts on the next launch (status `deferred`).
- **Install and restart** first runs the existing strict shutdown (drain database work, pause and flush statistics, save session and WAL; failure cancels the update and leaves the running services usable), then installs, then starts the new executable with `--update-relaunch <pid>` and exits. The new process waits (at most a minute) for that pid to exit before it starts up, so the single-instance check never sees the old process. The old process's backup stays beside the executable.
- The helper executable is no longer run by the player, and there is no pending journal or next-launch handoff before the app starts. It is still downloaded, verified, replaced and published, because players installed before this change update through it and read the same manifest (the manifest schema is unchanged).

## Consequences

- No rollback orchestration after a restart: if the new version fails to start, the previous file is the `.bak` beside it. The new version does not restore it by itself.
- A file replacement can fail when antivirus holds the file; the error is shown in the update dialog and nothing is replaced.
- Verified by unit tests of the swap (install, only one backup kept, failed verification replaces nothing) and a test that a running executable can be renamed but not deleted. A real end-to-end update (an older new-style build updating from a published release) can only be exercised once a newer release exists.
