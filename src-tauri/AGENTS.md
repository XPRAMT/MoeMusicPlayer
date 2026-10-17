# src-tauri/AGENTS.md

適用於 Tauri host、IPC、Rust app lifecycle 與 Android 原生橋接；同時遵守根目錄 `AGENTS.md`。

## 邊界與資料來源

- Tauri 命令負責平台能力與協調；領域政策留在共用 Rust Core／crates。IPC 使用穩定 Track ID 與明確 DTO，不把 Windows 路徑或 Android Content URI 當跨平台曲目身份。
- `settings.json` 是主題、播放偏好及來源登錄的權威；SQLite 是音樂資料、播放 session、歌詞與統計的權威。Windows 便攜資料放在執行檔同層 `UserData/`；不得再搬回 Roaming／LocalCache。
- 使用者音樂、UserData、資料庫或設定不得加入 Git、Release ZIP 或診斷快照；診斷資訊要版本化並排除曲名、路徑及設定內容。
- 正式 SQLite 只能由應用資料庫生命週期寫入。避免跨命令長時間持有鎖；資料庫 shutdown／checkpoint 未完成時不可丟棄 worker 或統計寫入。

## 非同步與播放生命週期

- SQLite／ICU4X 重工作使用 `run_database_work`／`spawn_blocking` 或既有非同步服務，不在 UI command thread 執行大量聚合。
- 音訊 backend 是播放狀態、位置與長度的權威來源；掃描、封面、歌詞、SMTC 或 renderer 不得阻塞音訊 worker。
- 使用者明確 Play、Pause、Seek、切歌等命令要表達 intent；切歌不得因前一首的暫停狀態或隱式 restore 自動改變使用者意圖。播放 session 保存 traversal、目前 queue entry、位置與播放狀態。
- 正常關閉與更新關閉是不同流程。更新替換必須 drain DB 工作、成功保存 session／統計／WAL 後才簽發 permit；失敗時恢復服務，不得強制 kill 正在使用的主程式。

## Windows／Android

- Windows 外部檔案開啟先檢查正式資料庫，未收錄曲目才進本次佇列的記憶體 SQLite；外部佇列不取代磁碟中已保存的播放 session。
- Windows 音訊副檔名與實際 decoder capability 分開維護；MP4 影片不可當作音訊來源，M4A 是音訊容器。新增 codec 前同步測 metadata、duration、decode、seek 與打包授權。
- Android SAF URI 權限與 MediaStore locator 都是平台資料；權限失效或來源離線時保留舊投影和使用者資料。背景播放由 Media3 `MediaSessionService`／Rust playback actor 維持，不得依賴 Activity 存活。
- Android 主背景（ADR 0008）：`appearance_pick_main_background` 先複製文件內容再安裝；幻燈片的 `StoredPath::Uri` 經 `next_slideshow_bytes` 和 `MainBackgroundRuntime::*_with` 注入 `listSafImages` 掃描，每張圖以 cache lease 讀取；系統桌布模式回傳空位元組，由原生 `setWallpaperBackdrop` 合成桌布。取得與切換背景的命令為 async，並在 spawn_blocking 中執行，因為 Android plugin 呼叫不可在主執行緒阻塞。
- 更新功能沿用現行便攜雙 EXE 結構與固定 GitHub release trust boundary；詳細流程參照 `docs/adr/0007-portable-windows-updates.md`。

## 驗證

執行與修改模組相符的 Rust tests、`cargo fmt --all -- --check` 與 strict Clippy。Release 與 WebView2 GUI 驗收依根目錄規則；Android 改動須交叉編譯並在目標裝置驗證，不能以 Windows 測試推斷 Android 行為。