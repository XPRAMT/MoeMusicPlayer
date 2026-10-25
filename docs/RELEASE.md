# 發布規則

使用者說「上傳 github」時依本文件執行，不需再交代。本文件只寫目前的發布規則；更新機制見 ADR 0015、0021、0022、0023。

## 1. 版本與命名

- 版本號＝`src-tauri/build-version.txt`（建置日期，例如 `26.10.25`）。
- Git tag＝`v<版本>`；Release 標題＝`MoeMusicPlayer <版本>`。
- Release 只放兩個資產：`moemusicplayer-windows-x86_64.zip`、`moemusicplayer-android-arm64.apk`。沒有 JSON 清單、沒有單獨的更新器。

## 2. 發布說明

- 檔案：`docs/release-notes/v<版本>.md`（UTF-8 不含 BOM，因為原文直接作為 Release 內文）。
- 範圍：上一個 GitHub Release 的 commit 到 HEAD（`gh release list --repo XPRAMT/MoeMusicPlayer --limit 1` 找上一版，`git fetch --tags` 後 `git log v<上一版>..HEAD`）。
- 只寫相對上一版使用者感受得到的變化。上一版沒有、在這段期間內引入又修好的 bug 不寫：以 `git show <上一版 commit>:<檔案>` 確認上一版是否已有該問題；不確定時把新行為寫成功能，而不是修正。上一版就有、現在修好的 bug 才寫成「修正…」。
- 格式（照上一版 Release 內文）：`## 繁體中文` 在前、`## English` 在後；第一行「距離 v<上一版> 的變更。」／“Changes since v<上一版>.”；依功能分組，粗體標題，最明顯的分組在前；每條是簡短的使用者描述。
- 不寫提交編號、內部函式／檔案／設定鍵名稱；介面文字照 App 的繁體中文標籤；英文段翻譯同一份內容，必要時保留繁中介面標籤（例如 顯示譯文）。不得稱本專案為開源。
- 寫好後把繁體中文全文直接貼在對話裡給使用者看（不只給檔案路徑或摘要），得到同意才發布。

## 3. 發布前

1. 依 AGENTS.md「本文件維護規則」整理 AGENTS.md，並通過 `node --test tests/agents-md-size.test.mjs`。
2. 測試：`npm run check`、`node --test`、相關 Rust 測試（以及 workspace Clippy、rustfmt）。
3. 提交所有來源後才建置（執行檔內嵌 commit）：
   - Windows：`npm run release:windows`（完整最佳化；替換 `release/moemusicplayer.exe` 並產生 `release/update-assets/moemusicplayer-windows-x86_64.zip`）。建置前後照 AGENTS.md「工作與驗收規則」備份 exe、清理 `.bak`，正常關閉舊程序後以 CDP 9222 啟動新版並核對 build identity。
   - Android：`npm run android:build:arm64:release`（release 金鑰簽章，輸出 `src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk`）。把 release APK 安裝到所有連得上的裝置（見 AGENTS.md「Android 平台」），連不上的略過並在回報註明。

## 4. 發布

上傳腳本只會對「已存在」的 Release 執行 `gh release upload <tag> <檔案> --repo XPRAMT/MoeMusicPlayer --clobber`；它們不建立 Release、不建立 tag，也不寫內文。所以順序是：

1. `git push origin master`。
2. 建立草稿 Release（同時建立 tag、寫入內文）：
   `gh release create v<版本> --repo XPRAMT/MoeMusicPlayer --target master --title "MoeMusicPlayer <版本>" --notes-file docs/release-notes/v<版本>.md --draft`
3. 上傳資產：
   - Windows：`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package-windows-update.ps1 -Upload -ReleaseTag v<版本>`（從 `release/moemusicplayer.exe` 重新打包；版本取自該 exe 的 `--update-build-info`，必須與 tag 相同）。`npm run release:windows`（`release-windows.ps1`）本身不接受 `-Upload`。
   - Android：`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release-android.ps1 -SkipBuild -Upload -ReleaseTag v<版本>`（複製第 3 節的 release APK 為 `release/update-assets/moemusicplayer-android-arm64.apk` 再上傳；不加 `-SkipBuild` 會再跑一次 `android:build:arm64:release`）。
4. 公開：`gh release edit v<版本> --repo XPRAMT/MoeMusicPlayer --draft=false`。之後要改內文用 `gh release edit v<版本> --notes-file docs/release-notes/v<版本>.md`。
5. 確認兩個資產都有 SHA-256 digest：`gh release view v<版本> --repo XPRAMT/MoeMusicPlayer --json assets --jq '.assets[] | {name, digest}'`。自動更新只信任這個 digest，沒有 digest 就不會自動更新。

## 5. 發布後回報

- Release 網址。
- 兩個資產的 SHA-256（取自 GitHub digest）。
- 各裝置安裝狀態：Windows 新版是否已啟動並核對 build identity；每台 Android 裝置已安裝並測試，或略過及原因。
