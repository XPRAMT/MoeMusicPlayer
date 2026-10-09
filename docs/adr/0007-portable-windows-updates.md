# Portable Windows updates

Status: Accepted

Decision date: 2026-10-09, Asia/Taipei (UTC+08:00)

## Context

Windows uses a portable executable with `UserData` beside it. Settings and SQLite already have stable ownership. The displayed version is a date, package SemVer is 0.1.0, and multiple builds on the same date must be distinguishable. The user chose to preserve this structure and publish `XPRAMT/MoeMusicPlayer` for actual Release updates.

## Decision

- Embed UTC time with milliseconds and the full git commit. Order by UTC time and identify by both values, not display date or fixed SemVer.
- Query only the fixed repository's public latest Release API without a token. Validate the manifest using GitHub's asset SHA-256 digest, then verify the two raw executable downloads against the manifest. HTTPS redirects are restricted to GitHub asset hosts; metadata, transfer sizes, and waits are bounded. Missing public releases are a normal unavailable state.
- Keep `moemusicplayer.exe`, `moemusicplayer-updater.exe`, and `UserData/`. Use `UserData/updates` for staged bytes, pending journal, executable backups, and logs. No source paths, SQLite, settings, or Sony filter are packaged. The ZIP contains exactly two executables and the manifest; the updater downloads raw assets and does not extract ZIP entries.
- Ignore dismisses only this notification. Restart Now downloads/verifies, drains database work, acknowledges pause/accounting, flushes statistics, and saves the session/WAL strictly. Persistence failure cancels the update, reopens admission, and leaves current services usable. Ordinary close keeps its existing behavior. Next Launch downloads/verifies immediately, then hands off before Tauri/database initialization on next start.
- A small helper uses Windows process handles and file APIs. It validates the parent executable and creation time, reports READY, waits at most 60 seconds, and requires an explicit persistence permit. It never force-kills the parent or other app instances. The helper runs from the stage so the installed helper can be replaced too.
- Back up both original executables before replacement. Restore them on replacement or launch failure when no uncertain live child is involved, and restart the old main executable. A live new child without the five-second launch acknowledgement is kept with backups; starting the old app could put two processes on the same database.
- The new app acknowledges before opening its database. This proves handoff only. File rollback cannot reverse a later schema migration and does not guarantee UI or audio health.

## Alternatives considered

- Tauri's standard Windows updater uses MSI/NSIS installer releases, which do not match this portable pair.
- Velopack supplies an updater but changes installation layout and the stable data-root boundary; the user chose to preserve the existing structure.
- `self_replace` replaces its own executable, so a separate helper cannot use it to replace the main executable. `self_update` network helpers would duplicate the existing reqwest transport while retaining the same process/persistence coordination.

## Verification and publishing

The Release API and manifest use a 15-second request limit. Binary staging uses 10-second connection establishment, a 30-second no-data watchdog (including initial response headers), and a 30-minute total limit around both transfers. An interrupted download removes its nonce stage and creates no pending journal. The bounded transfer budget permits real slow GitHub asset connections while preventing an indefinite wait.

`cargo test -p moemusicplayer-updater --features test-fixtures -- --test-threads=1` exercises real temporary child processes for parent wait, replacement/relaunch, timeout, permit refusal, rollback, and identity. Fixtures preserve a UserData sentinel. Strict-save orchestration separately tests finalizer failure and gate/coordinator reopening.

`npm run release:windows` builds both executables and packages assets. `scripts/package-windows-update.ps1` reads the exact executable's pre-app `--update-build-info` metadata, drains output asynchronously, and preserves the raw UTC ISO string independently of PowerShell's JSON date conversion. Publication requires explicit `-Upload -ReleaseTag` or a separate `gh release` command. The app never reads local GitHub credentials. Diagnostic CLI `--update-check-json` and `--update-stage-next-launch` share the production Release metadata/download validation and have no configurable endpoint; they support live public Release checks in isolated directories.

Integrity is within GitHub's trust boundary, not an independent publisher signature. Releases without the manifest digest are rejected. Report GUI/modal evidence, real GitHub transfers, process handoff, and audio/UI acceptance separately.
