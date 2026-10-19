# Android APK updates from the Windows release feed

Status: Accepted (the separate Android manifest was removed by ADR 0022: the APK is identified by the Release tag and verified by GitHub's asset digest)

Decision date: 2026-10-17, Asia/Taipei (UTC+08:00)

## Context

Windows updates come from the fixed repository's public latest GitHub Release (ADR 0007): `moemusicplayer-update.json`, which is verified by GitHub's asset SHA-256 digest, plus the two executables. Android was updated only by sideloading APKs by hand. The app is sideloaded for its owner, not distributed on Play.

## Decision

- Use the same feed: the same latest Release of `XPRAMT/MoeMusicPlayer`, the same redirect, host and size limits, the same digest validation, and the same build ordering by `buildTimestampUtc`.
- Give Android a separate manifest asset, `moemusicplayer-android-update.json`: `schemaVersion` 1, `version`, `buildTimestampUtc`, `gitCommit`, `architecture` `android-arm64`, and `apk` {`name` `moemusicplayer-android-arm64.apk`, `sha256`, `size`}. The Windows manifest uses `deny_unknown_fields`, so adding fields to it would break older Windows clients. A Release without the Android manifest counts as "unpublished" on Android.
- Download the APK into app-private `cache/updates/`. Bytes are streamed, and size and SHA-256 must match the manifest. Hand the APK to the system installer with `ACTION_VIEW` through the plugin's own `MoeUpdateFileProvider` (authority `${applicationId}.moe.updates`), which shares only that folder and the clipboard cover cache `cache/clipboard-share/`. The manifest declares `REQUEST_INSTALL_PACKAGES`.
- Without "install unknown apps" for this app, open `ACTION_MANAGE_UNKNOWN_APP_SOURCES` and report `installPermission`. The user allows it there and presses 「下載並安裝」 again, which reuses the verified download. The app never grants this itself, and the system installer always asks the user to confirm.
- Publishing: `npm run release:android` (`scripts/release-android.ps1`) pins `MOE_BUILD_TIMESTAMP_UTC` and records the commit. It builds `npm run android:build:arm64` and writes the APK and manifest to `release/update-assets/`. Upload happens only with an explicit `-Upload -ReleaseTag <tag>`, into an existing Release next to the Windows assets.

## Consequences

- Android only installs an update signed with the same key as the installed app. Today that is the debug keystore of the build machine, so a build from another machine or key fails at the system installer.
- Integrity stays within GitHub's trust boundary, as on Windows. Android's signature check additionally ties the update to the installed app's key.
- The version code is fixed. Android allows an update with the same version code, so ordering is by build timestamp only.
