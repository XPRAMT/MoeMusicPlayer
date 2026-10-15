# AGENTS.md

## 專案目標與架構

MoeMusicPlayer 是以本機音樂庫為核心的 Windows／Android 播放器。桌面與行動 UI 使用 Svelte 5、TypeScript、Vite；Tauri 2 IPC 連接 Rust Core、SQLite 曲庫及 Windows／Android 原生媒體 adapter。使用者音樂資料不依賴線上服務。

- `src/`：Svelte 前端、共用元件及 UI 狀態。
- `src-tauri/`：Tauri IPC、應用生命週期、設定與原生橋接。
- `crates/`：共用核心、資料庫、音訊及平台 adapter。
- `CONTEXT.md`：共享領域詞彙；`docs/adr/`：已採納的架構決策。
- `待辦清單.md`：由使用者維護；除使用者明確要求外，不得編輯。

## 目前工作狀態

專輯詳情的曲目捲動視窗固定從「封面完全收合後的標題高度＋欄標題高度」（Y0）開始，捲動期間位置與大小不變，捲軸比例固定；內容最前面有等於收合行程（桌面 120 CSS px、窄版 80 CSS px）的空白，原生 scrollTop 前段收合封面（桌面 200、窄版 160 → 80 CSS px；標題高度 = 封面 + 上下內距（桌面 36、窄版 28）），之後才捲曲目，往上時同樣先回到清單頂端再展開封面，收合量恆為 `min(scrollTop, 收合行程)`。列不會被標題蓋住，因此不做逐列 clip-path、不做原生／邏輯捲動映射，也不攔截滾輪；封面、標題與欄標題位置由 CSS scroll timeline 依 scrollTop 驅動，不支援時以 scroll 事件換算的收合量更新。2026-10-16 已用 Release WebView2 CDP 驗證：快速拖曳捲軸 1,565 幀中沒有任何列出現在欄標題下緣以上，`clientHeight`、`scrollHeight`、捲動視窗頂端位置固定。架構決策見 [`docs/adr/0012-stable-album-detail-scrollport.md`](docs/adr/0012-stable-album-detail-scrollport.md)。

專輯首頁卡片依「曲目欄位」分頁的「專輯卡片最大寬度」（`trackListColumns.albumCardMaxWidthPx`，預設 180 CSS px，範圍 100–500）填滿整列：欄數 = ceil((可用寬度 + 間距) / (最大寬度 + 間距))、至少 1，卡片平分剩餘寬度，不留右側空隙且不超過最大寬度；封面與文字隨卡片寬度縮放，虛擬列高由同一寬度計算。純函式為 `album-index-layout.js` 的 `getAlbumCardGrid`。Release CDP 多視窗寬度驗收尚未完成（建置時使用者的播放器正在執行，未強制關閉）。 專輯首頁排序列四欄標籤一律置中：排序鈕以 `1fr｜標籤｜1fr` 三欄格線排版，▲▼ 放在右欄，標籤不因指示符號或欄位非數值而偏移；垂直 −3.25 px 光學偏移不變。

快速查找面板（`PagedVirtualList` 與 `AlbumIndex`）每列項目數先取近正方形 round(√標籤數)，再以可用寬度上限 floor((可用寬度 − 2×12 內距 + 4 間距) / (2em 格寬 + 4 間距)) 截斷、至少 1；可用寬度 = min(清單區寬, 視窗寬) − 24 px，格寬以 `--font-title` × 2 計算。寬度優先：放不下時改為更多列而非橫向捲動。純函式在 `quick-index.js`（`getQuickIndexColumnCount`／`getQuickIndexMaxFittingColumns`／`getQuickIndexAvailableWidth`），Release CDP 驗收尚待使用者播放器關閉後補做。

左側導覽：品牌與搜尋列固定在上方，導覽項目（曲庫、演出者、專輯、播放佇列、播放清單群組）在 `.sidebar-scroll` 內垂直捲動，「設定」固定在底部不隨之捲動。播放清單群組恆為展開，沒有收合按鈕或狀態（先前的收合只存在元件記憶體，未持久化，無需遷移）。視窗寬度 ≤ 820 CSS px（沿用既有斷點，`src/lib/nav-panel.js` 的 `NAV_PANEL_MEDIA_QUERY`）時不再有圖示欄或頂部橫向導覽列：工作區佔滿寬度，頂欄左側顯示 MOE 標誌按鈕（`.nav-panel-toggle`）切換導覽面板；面板即同一個 `<aside class="sidebar">` 加上 `.floating-surface`（與快速設定相同的背景、模糊、陰影與 `--floating-surface-radius` 圓角），以 fixed 疊在內容上：上緣貼齊標題列下緣、左緣貼齊視窗、下緣貼在底部播放列之上（播放列保持可見），僅右側保留圓角與陰影；Android safe-area 以 top/bottom 偏移與左側內距計入，開啟時 160 ms 淡入滑入（遵循 reduced-motion）。點擊外部（透明 scrim）、Esc、點選任何導覽項目或播放清單後關閉；放寬視窗或開啟正在播放時自動關閉；關閉時面板為 inert。Android 手機寬度都在斷點內，因此同樣使用標誌按鈕＋疊層面板，原本 ≤ 620 px 的頂部導覽列已移除。同一斷點下頂欄「重新整理」只顯示圖示（36 px 方鈕），保留 aria-label「重新整理音樂來源」與 tooltip。Release CDP 寬／窄驗收尚待使用者播放器關閉後補做。

下一步（未來最佳化，非目前缺陷）：快速拖曳捲軸時虛擬列表可能短暫出現空白列，原因是合成器先捲動、主執行緒重繪與資料頁載入較晚。可考慮的緩解：overscan 改為約一個視窗高度、尚未載入的列一律繪製輕量佔位列、捲動時只在可見範圍改變才通知上層、列加上 `contain: content`。

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
- Bug／功能驗收一律啟動固定路徑的 Release 執行檔，以該程序的 WebView2 CDP 檢查實際 DOM、IPC 或錯誤；不得改用 Chrome、Playwright 或無資料隔離副本。完成後關閉本次診斷程序並確認偵錯埠已關閉。CDP 啟動方式：先執行 `netsh int ipv4 show excludedportrange protocol=tcp` 確認埠不在 Windows 保留範圍（例如 9326–9425 會讓 9337 無法監聽），預設使用 9222；設定 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` 後啟動 `release/moemusicplayer.exe`，再以 `netstat -ano | findstr :9222` 確認 LISTENING，並以 `http://127.0.0.1:9222/json/version` 確認有回應後才開始驗收。建置／單元測試不等於實際 App 驗收。Android 必須在 Android 目標環境驗收。
- 不得強制終止使用中的播放器；Windows 新版按既定背景替換程序與備份規則更新，絕不觸碰 `UserData/`。不得未經同意重新啟動裝置。
- 不以 Python sqlite3 讀寫使用中的正式 SQLite；先確認播放器關閉，或使用已驗證的唯讀／備份流程。
- 依變更範圍執行 `npm run check`、`node --test`、相關 Rust tests、workspace Clippy 與 rustfmt；完成工作後更新本文件「目前工作狀態」，只保留仍有效的狀態與下一步，不記錄歷史流水帳。

## 按需閱讀的領域規則

- UI、列表互動、播放頁、歌詞、背景及視覺 token：[`src/lib/AGENTS.md`](src/lib/AGENTS.md)
- Tauri IPC、使用者資料、播放生命週期及更新流程：[`src-tauri/AGENTS.md`](src-tauri/AGENTS.md)
- Rust Core、SQLite、排序／分組、跨平台資料與音訊邊界：[`crates/AGENTS.md`](crates/AGENTS.md)
- 產品詞彙：[`CONTEXT.md`](CONTEXT.md)；架構決策：[`docs/adr/`](docs/adr/)
