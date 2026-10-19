# Updates are identified by the version only (no manifest, no helper)

Status: Accepted (supersedes the manifest and helper parts of ADR 0007, ADR 0015 and ADR 0021)

Decision date: 2026-10-19, Asia/Taipei (UTC+08:00)

## Context

Updates were described by JSON manifests (`moemusicplayer-update.json`, `moemusicplayer-android-update.json`) carrying a build timestamp, commit and file hashes, plus a helper executable on Windows. Publishing meant producing and uploading several matching files, and an APK uploaded without its manifest (or the reverse) broke updates. Nobody had downloaded a release yet, so the format could still change freely.

## Decision

- A Release is identified by its tag, `v<version>`, where the version is the build date (`26.10.19`, `src-tauri/build-version.txt`). The player compares that tag with its own version as numbers (26.10.19 > 26.10.9); a tag that is not a version is ignored as an error. At most one update per day follows from this, which is intended: users are not nagged more than once a day.
- A Release carries only the platform files: `moemusicplayer-windows-x86_64.zip` (holding `moemusicplayer.exe`) and `moemusicplayer-android-arm64.apk`. There is no JSON manifest, no standalone executable and no updater executable.
- Integrity is GitHub's own SHA-256 digest of the asset, returned by the public Release API (`digest`). A Release whose asset has no digest cannot be installed automatically. Only the fixed repository, HTTPS redirects within GitHub hosts and bounded sizes and timeouts are accepted, as before.
- Windows downloads the zip, verifies it, extracts `moemusicplayer.exe` (that entry by exact name, size-bounded, must start with `MZ`) into `UserData/updates/<nonce>/` and installs it by renaming (ADR 0021): the running file is renamed to a `.moemusicplayer.previous.<id>.bak` and the new file takes its name. The helper process, the pending journal and the next-launch handoff are gone; the old `moemusicplayer-updater.exe` beside the executable is removed on the first install.
- Android downloads the APK, verifies it and hands it to the system installer, as in ADR 0015.
- Publishing: `npm run release:windows` writes the zip to `release/update-assets/`, `npm run release:android` the APK; each uploads only with an explicit `-Upload -ReleaseTag v<version>` to an existing Release.

## Consequences

- Two builds on one day cannot update each other; a second release the same day needs a new version (the next date).
- Players older than this change look for the manifest and find none, so they report that no release is published; they must be updated by hand once.
- The exact build time and commit are still embedded and shown in Settings → About, but they no longer take part in update decisions.
