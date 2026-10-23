# Updates are identified by the version only (no manifest, no helper)

Status: Accepted (supersedes the manifest and helper parts of ADR 0007, ADR 0015 and ADR 0021)

Decision date: 2026-10-19, Asia/Taipei (UTC+08:00)

## Context

Updates were described by JSON manifests (`moemusicplayer-update.json`, `moemusicplayer-android-update.json`) carrying a build timestamp, commit and file hashes, plus a helper executable on Windows. Publishing meant producing and uploading several matching files, and an APK uploaded without its manifest (or the reverse) broke updates. Nobody had downloaded a release yet, so the format could still change freely.

## Decision

- A Release is identified by its tag, `v<version>`, where the version is the build date (`26.10.19`, `src-tauri/build-version.txt`). The player compares that tag with its own version as numbers (26.10.19 > 26.10.9); a tag that is not a version is ignored as an error. At most one update per day follows from this, which is intended: users are not nagged more than once a day.
- A Release carries only the platform files: `moemusicplayer-windows-x86_64.zip` (holding `moemusicplayer.exe`) and `moemusicplayer-android-arm64.apk`. There is no JSON manifest, no standalone executable and no updater executable.
- Integrity is GitHub's own SHA-256 digest of the asset, returned by the public Release API (`digest`). A Release whose asset has no digest cannot be installed automatically. Only the fixed repository, HTTPS redirects within GitHub hosts and bounded sizes and timeouts are accepted, as before.
- **Ignore** stores that version in `settings.json` (`ignoredUpdateVersion`). A startup check that finds the same version reports `ignored` and shows no prompt; only a check the user starts by hand (`update_check` with `manual: true`, Settings → About) offers it again. A newer version is offered as usual.
- Windows downloads the zip, verifies it, extracts `moemusicplayer.exe` (that entry by exact name, size-bounded, must start with `MZ`) into `UserData/updates/<nonce>/` and installs it by renaming (ADR 0021): the running file is renamed to a `.moemusicplayer.previous.<id>.bak` and the new file takes its name. The helper process, the pending journal and the next-launch handoff are gone; the old `moemusicplayer-updater.exe` beside the executable is removed on the first install.
- Android downloads the APK, verifies it and hands it to the system installer, as in ADR 0015.
- Publishing: `npm run release:windows` writes the zip to `release/update-assets/`, `npm run release:android` the APK; each uploads only with an explicit `-Upload -ReleaseTag v<version>` to an existing Release.

## Consequences

- Two builds on one day cannot update each other; a second release the same day needs a new version (the next date).
- Players older than this change look for the manifest and find none, so they report that no release is published; they must be updated by hand once.
- The exact build time and commit are still embedded and shown in Settings → About, but they no longer take part in update decisions.

## 目前設計細節（自 AGENTS.md 移入）

以下是目前仍有效的實作細節；AGENTS.md 只保留摘要與指向本節的連結。

自動更新（ADR 0022 版本辨識、ADR 0021 Windows 安裝、ADR 0015 Android）：版本號＝建置日期（`src-tauri/build-version.txt`），Release tag `v<版本>`，版本以數字比較；Release 只放 `moemusicplayer-windows-x86_64.zip`（內含 `moemusicplayer.exe`）與 `moemusicplayer-android-arm64.apk`，沒有 JSON 清單、沒有單獨的更新器，完整性靠 GitHub 回報的資產 SHA-256 digest（沒有 digest 就不自動更新）。啟動時檢查，有新版跳出 `UpdateCenter`：「忽略」（存 `settings.json` 的 `ignoredUpdateVersion`，同版本只在手動「檢查更新」`update_check` 的 `manual: true` 時再提示）、「自動安裝」（`nextLaunch`）、「自動安裝並重啟」（`restartNow`：先嚴格關閉，替換後以 `--update-relaunch <pid>` 啟動新版）。Windows 解出 `moemusicplayer.exe`（只取該 entry、有大小上限、須以 `MZ` 開頭），`install_in_place` 把目前 exe 改名為 `.moemusicplayer.previous.<id>.bak` 再換上新檔（失敗還原，只留最新一份，舊的 `moemusicplayer-updater.exe` 首次安裝時刪除）；Android 交給系統安裝程式。發布：`npm run release:windows`（產生 `release/update-assets/moemusicplayer-windows-x86_64.zip`，`scripts/package-windows-update.ps1`）、`npm run release:android`，只有加 `-Upload -ReleaseTag v<版本>` 才上傳。端到端更新要等有更新的 Release 才能驗證。
