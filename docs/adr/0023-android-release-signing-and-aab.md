# Android release signing and the Google Play bundle

Status: Accepted

Decision date: 2026-10-19, Asia/Taipei (UTC+08:00)

## Context

Until now the Android release APK was signed with the build machine's debug key, which only works for sideloading from one computer and cannot be uploaded to Google Play. The maintainer has a release keystore (`C:\APP\@Android\Play KEY.jks`, alias `anime theme key`, created 2017-12-31) and uploads the bundle to Google Play by hand.

## Decision

- `npm run android:build:arm64:release` builds an APK (sideloading and the in-app update from GitHub) **and an AAB** (Google Play) in one run, both arm64, size-optimized, signed with the release key. Outputs: `release/moemusicplayer-android-arm64-release.apk` and `.aab`. The AAB is never uploaded by a script.
- The key comes from environment variables read by Gradle (`MOE_ANDROID_KEYSTORE`, `MOE_ANDROID_KEY_ALIAS`, `MOE_ANDROID_KEYSTORE_PASSWORD`, `MOE_ANDROID_KEY_PASSWORD`); the build script fills them from the maintainer's key files (the password from the `Password` line of `C:\APP\@Android\Play KEY.txt`) only for its own process and restores the environment afterwards. Nothing secret is in the repository. Without a keystore the build falls back to the debug key, so contributors can still build.
- The debug build (`npm run android:build:arm64`, `build-android-release.ps1 -Debuggable`, output `release/moemusicplayer-android-arm64-debug.apk`) uses the same key and environment handling as the fast and full release builds (the Gradle `debug` build type takes the `moeRelease` signing config when it exists), so debug, fast and release APKs install over each other without uninstalling and app data survives. It stays debuggable for WebView CDP.
- The version name is the build version (26.10.19) and the version code is its date as a number (261019), so each day's Google Play upload has a higher code, matching the one-release-a-day rule of ADR 0022.

- `MOE_ANDROID_DEBUG_KEY=1` builds with the old debug key instead (files get a `-debugkey` suffix and never overwrite the release-key ones): that is the build to install over an app that was installed before this change, so its data survives and the new Backup export can be used before moving to the release key.

## Consequences

- A phone with an app signed by the old debug key cannot be updated in place: it must be uninstalled once (its settings are lost, the library is rebuilt by a scan). The in-app update works from then on, for builds signed with this key. All future APKs must keep using this key.
- An APK from GitHub and the same app installed from Google Play are signed by different keys when Play App Signing is on, so they cannot update each other.
