# AGENTS.md

## 專案目標與架構

MoeMusicPlayer 是以本機音樂庫為核心的 Windows／Android 播放器。桌面與行動 UI 使用 Svelte 5、TypeScript、Vite；Tauri 2 IPC 連接 Rust Core、SQLite 曲庫及 Windows／Android 原生媒體 adapter。使用者音樂資料不依賴線上服務。

- `src/`：Svelte 前端、共用元件及 UI 狀態。
- `src-tauri/`：Tauri IPC、應用生命週期、設定與原生橋接。
- `crates/`：共用核心、資料庫、音訊及平台 adapter。
- `CONTEXT.md`：共享領域詞彙；`docs/adr/`：已採納的架構決策。
- `待辦清單.md`：由使用者維護；除使用者明確要求外，不得編輯。

## 目前工作狀態

專輯詳情使用固定範圍的原生捲動視窗；封面桌面尺寸 148 CSS px、窄版 116 CSS px，最小 64 CSS px。向下先逐像素收合封面，再捲曲目；清單回頂後多出的向上捲動才展開封面。`clientHeight`、`scrollHeight`、捲軸比例及捲動範圍不因收合改變。列項只裁切越過清單標題邊界的部分，裁切值在原生 scroll event 及滾輪動畫影格中依實際畫面座標同步；不裁切 viewport。專輯詳情的標題外框寬度扣除原生捲動槽，標題格線與列項同寬；標題上方疊層的空白區不攔截捲動槽事件。固定 Release WebView2 實測：15 CSS px 捲動槽在標題區的點擊將 `scrollTop` 從 0 移至 820.8 px；拖曳使其由 2753.2 移至 3403.6 px。標題外框與 viewport `clientWidth` 同為 1269 px，欄位格線與列項逐欄對齊。快速連續捲動抽樣 1,793 次列項幾何，裁切與標題邊界最大差異 0.00011 CSS px，文字未越界。`npm run check` 0 errors／warnings、`node --test` 274/274 通過。固定 Release build `2026-10-15T17:14:03.545Z@58e586d64fe82cc33ee3570ed7cf2c7ab4c228f6`，SHA-256 `E9F3D2EE6EBAE62B03E38770FFCEAC2CDA947BE72115C71982E8651FF6FFB5EF`；架構決策見 [`docs/adr/0012-stable-album-detail-scrollport.md`](docs/adr/0012-stable-album-detail-scrollport.md)。

## 重要資料與架構邊界

- `settings.json` 是介面偏好、隨機／循環狀態及來源登錄的權威；SQLite 保存音樂資料、來源對應、播放清單投影、歌詞、聆聽統計與播放 session。Windows 便攜版使用 `release/UserData/`；不得改用 LocalCache／Roaming，也不得提交使用者資料或 Release 輸出。
- SQLite 負責搜尋、過濾、排序、分組、計數及 artist／album 聚合快照；前端只取固定 1,000 筆資料頁並以虛擬列表限制 DOM。詳見 [`crates/AGENTS.md`](crates/AGENTS.md)。
- 曲名、演出者、專輯名稱與快速索引須共用 ICU4X locale-aware natural collation；平台原生索引 API 不參與排序／分組。細節見 [`crates/AGENTS.md`](crates/AGENTS.md) 及相關 ADR。
- Renderer 不持有媒體路徑作為跨平台身份；穩定 Track ID、來源對應與平台 locator 分層。Windows 檔案總管開啟的未匯入曲目使用該佇列生命週期內的記憶體 SQLite。
- SQLite／音訊生命週期細節、播放意圖及 platform bridge 規則分別見 [`src-tauri/AGENTS.md`](src-tauri/AGENTS.md) 與 [`crates/AGENTS.md`](crates/AGENTS.md)。

## 工作與驗收規則

- 每次開始工作先讀根目錄及工作路徑上適用的 `AGENTS.md`；採信最新程式碼、測試與使用者需求，文件衝突時修正文件。需求不清楚時先查證或提問，不得擅自補需求。
- 先追查資料流與真正根因；跨領域／大型設計按需讀取 ADR 並釐清需求。保留工作樹中與任務無關的修改。
- 每次修改程式碼、設定或文件後提交 Git；只 stage 本次修改。新檔優先 UTF-8 BOM，依工具鏈既有格式例外處理。README 變更需同步 `README.md`、`README_TW.md`。
- 修改程式碼、資源或建置設定時，先完成測試並提交來源，再執行一次 `npm run release:windows`，避免執行檔內嵌舊 commit；純文件修改不建置。固定 Windows 執行檔為 `release/moemusicplayer.exe`，helper 與之同層。
- Bug／功能驗收一律啟動固定路徑的 Release 執行檔，以該程序的 WebView2 CDP 檢查實際 DOM、IPC 或錯誤；不得改用 Chrome、Playwright 或無資料隔離副本。完成後關閉本次診斷程序並確認偵錯埠已關閉。建置／單元測試不等於實際 App 驗收。Android 必須在 Android 目標環境驗收。
- 不得強制終止使用中的播放器；Windows 新版按既定背景替換程序與備份規則更新，絕不觸碰 `UserData/`。不得未經同意重新啟動裝置。
- 不以 Python sqlite3 讀寫使用中的正式 SQLite；先確認播放器關閉，或使用已驗證的唯讀／備份流程。
- 依變更範圍執行 `npm run check`、`node --test`、相關 Rust tests、workspace Clippy 與 rustfmt；完成工作後更新本文件「目前工作狀態」，只保留仍有效的狀態與下一步，不記錄歷史流水帳。

## 按需閱讀的領域規則

- UI、列表互動、播放頁、歌詞、背景及視覺 token：[`src/lib/AGENTS.md`](src/lib/AGENTS.md)
- Tauri IPC、使用者資料、播放生命週期及更新流程：[`src-tauri/AGENTS.md`](src-tauri/AGENTS.md)
- Rust Core、SQLite、排序／分組、跨平台資料與音訊邊界：[`crates/AGENTS.md`](crates/AGENTS.md)
- 產品詞彙：[`CONTEXT.md`](CONTEXT.md)；架構決策：[`docs/adr/`](docs/adr/)
