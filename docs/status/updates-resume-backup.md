# 現況：更新、接續播放與備份

本文件是此領域「目前狀態」的快照，不是日誌；維護規則同根目錄 `AGENTS.md`「本文件維護規則」：就地改寫既有條目、不寫日期／量測數字／除錯經過，設計細節放 `docs/adr/`。

### 更新、接續播放與備份

自動更新（ADR 0022 版本辨識、ADR 0021 Windows 安裝、ADR 0015 Android）：只用版本號辨識（版本＝建置日期 `src-tauri/build-version.txt`，Release tag `v<版本>`）；Release 只放 `moemusicplayer-windows-x86_64.zip` 與 `moemusicplayer-android-arm64.apk`，沒有 JSON 清單、沒有單獨的更新器 exe，完整性靠 GitHub 資產 SHA-256 digest（沒有 digest 就不自動更新）。`UpdateCenter`：「忽略」（`ignoredUpdateVersion`）、「自動安裝」、「自動安裝並重啟」。Windows `install_in_place` 改名替換，只留一份 `.moemusicplayer.previous.<id>.bak`。發布流程見 `docs/RELEASE.md`。端到端更新要等有更新的 Release 才能驗證。

接續播放：`settings.json` 的 `resumePlayback`（預設 false）。正常關閉或更新重啟時若正在播放，`set_resume_playback(true)`（更新流程在暫停前記錄，中止時改回 false）；下次啟動還原 session 成功（`ready`）後 `take_resume_playback` 重設並 `request_play`；強制結束或當機不寫入。Android 播放 actor 在狀態改變時經 `ResumeHook.sink` 即時寫入，啟動時 `restore_session` 依欄位以 `play_when_ready` 續播（`AppState.settings` 為 `Arc<SettingsStore>`）。Release 與裝置實測待確認。

備份匯出／匯入（ADR 0024，設定 → 關於 → 備份與還原）：勾選面板（`AboutPage.svelte`、`src/lib/backup-export.js`）匯出只含勾選項目的 zip；帳號只以備份密碼封存（`remote-accounts.sealed`），任何項目都不得含明文密碼；不含封面與背景圖。匯入只替換給的部分，先放 `import-pending`，下次啟動、開設定與資料庫之前才換入（舊檔移到 `before-import`）。測試 `src-tauri/src/backup.rs`。Release 實機與手機實測待確認。
