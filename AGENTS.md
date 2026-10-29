# AGENTS.md

## 專案目標與架構

MoeMusicPlayer 是以本機音樂庫為核心的 Windows／Android 播放器。桌面與行動 UI 使用 Svelte 5、TypeScript、Vite；Tauri 2 IPC 連接 Rust Core、SQLite 曲庫及 Windows／Android 原生媒體 adapter。使用者音樂資料不依賴線上服務。

- `src/`：Svelte 前端、共用元件及 UI 狀態。
- `src-tauri/`：Tauri IPC、應用生命週期、設定與原生橋接。
- `crates/`：共用核心、資料庫、音訊及平台 adapter。
- `CONTEXT.md`：共享領域詞彙；`docs/adr/`：已採納的架構決策。
- `待辦清單.md`：由使用者維護；除使用者明確要求外，不得編輯。

## 授權與定位

本專案是「原始碼公開（source-available）」而非開源，採 PolyForm Noncommercial 1.0.0（根目錄 `LICENSE`、`NOTICE`；各 crate 與 `package.json` 的 license 為 SPDX `PolyForm-Noncommercial-1.0.0`）：可自由修改、自用，重新散佈須保留授權與 `Required Notice:` 行，禁止商用。文件與介面不得稱本專案為「open source／開源專案」（感謝第三方「開源」套件的文字不受影響）。**不得引入 GPL／AGPL（含 rdsd2pcm、dsd2dxd、GPL 版 FFmpeg）程式碼或連結**，因為它們不允許再加「禁止商用」；MIT／Apache／BSD／MPL／LGPL（動態連結）的相依可用，MPL 檔案維持 MPL。素材：`icon/SilverWolfIcon.png`、`icon/DefaultCover.jpg` 與 `hi-res-badge.svg` 都是作者以 AI 生成／重繪的非官方圖，隨專案授權提供；但圖中角色（銀狼）與 Hi-Res Audio 標誌／商標的權利屬原權利人，授權不涵蓋。

## 本文件維護規則

- 本文件是「路由與硬性規則」，不是功能日誌：全文最多 20,000 字元，任何單一段落或項目最多 1,500 字元（`tests/agents-md-size.test.mjs` 檢查）。
- 各功能「目前狀態」在 `docs/status/<領域>.md`（見「功能索引」）：就地改寫既有條目、刪掉被取代的內容、不另起新段落追加；同一功能只有一個位置；不寫日期、量測數字或除錯經過（放 commit message 或 ADR）。
- 設計細節（演算法、參數、門檻、理由、替代方案、量測）放 `docs/adr/`，status 只留目前行為、硬性規則、關鍵名稱、待辦／未驗證項目與「見 ADR NNNN」。
- 新增或移動功能時同步更新「功能索引」的指標；真正「每個工作階段都必須看到」的硬性規則才放本文件（一行，並指向 status 檔）。
- 每次發布前整理本文件與 `docs/status/`（見「工作與驗收規則」）。

## 硬性規則（每個工作階段都要看到；細節見 status 檔）

- **Android 裝置**（`docs/status/android-platform.md`）：手機 NX809J（`adb connect 192.168.1.10:5555`，位址可能改變）與平板 23046RP50C（192.168.1.92，無線偵錯埠每次不同，先 `adb mdns services` 找埠再 `adb connect`）；連得上的都以 `adb -s <序號> install -r <apk>` 安裝並測試驗收，連不上的略過，結案報告註明哪台已安裝與測試、哪台略過。任何裝置都不得重新啟動。啟動 App 一律 `adb -s <序號> shell am start -n com.moemusicplayer.app/.MainActivity`，**不得用 `adb shell monkey`**（會打開 NX809J 的自動旋轉）。用 adb 操作畫面前先確認方向，不在使用者手持操作時點擊。使用者測試音訊時，不在裝置上執行 `dumpsys media.audio_flinger` 這類重型 dump（除非事先談好；dump 規則見 ADR 0048）。
- **Android 建置**：`npm run android:build:arm64`（debug，可除錯；debug、fast、release 三種建置共用同一把 release 金鑰，可互相直接覆蓋安裝）；日常小修改用 `npm run android:build:arm64:fast`（不做 LTO、不產 AAB，輸出 `release/moemusicplayer-android-arm64-fast.apk`，可直接覆蓋安裝）；使用者說「上傳 github」時才跑完整最佳化版 `npm run android:build:arm64:release`（ADR 0023）。Android 專屬程式碼只在 aarch64-linux-android 目標編譯，變更共用 Rust 後須實際跑一次 APK 建置。release 版不可除錯，WebView CDP 驗收用 debug 版。
- **共同功能不得區分平台（使用者定案）**：Windows 與 Android 都有的功能，行為、規則、介面、數值與資料流必須相同，共用同一份程式（Rust 核心、前端元件、純函式）；只有平台才有的功能（例如 Windows 的 DoP／工作列歌詞、Android 的藍牙／USB 獨佔）可以不同。不得為共同功能寫只在某平台生效的分支、預設值或規則；發現既有分支一律改為共用。兩份實作時必須有共用測試向量。
- **Android 權限**：懸浮窗權限只能由使用者在系統設定頁開啟，禁止 adb 授權；權限提示一律先顯示說明面板、按「前往授權」才請求（ADR 0039，規則見 `src/lib/AGENTS.md`）。
- **授權補充**：不得複製 Spek 的 GPL 程式碼（頻譜圖「Spek」配色是依色階重新實作）；GPL／AGPL 禁令見「授權與定位」。
- **設定與資料只存在指定位置（使用者定案）**：使用者設定（凡使用者看得到或能改的，含面板寬度、選取的分頁、開關）只放 `settings.json`（Rust `src-tauri/src/settings.rs`，Windows 與 Android 同一份實作，並納入備份匯出）；跟歌曲綁在一起的使用者資料（例如逐曲歌詞延遲）放曲庫資料庫。`localStorage`、`sessionStorage`、IndexedDB、Android SharedPreferences／DataStore 只准用於除錯旗標、純快取、裝置專屬狀態與白名單內的項目。新增任何 key／檔案／preferences 名稱都必須同時加進 `tests/fixtures/storage-locations.json`，並寫明 `where`、`key`、`category` 與一行 `reason`；使用者看得到的設定不得以永久例外列入白名單（`pending-migration` 只給待遷移的既有項目，遷移完成就刪除）。`tests/storage-locations.test.mjs` 會掃描前端與 Android Kotlin 並在違規時失敗，訊息說明該怎麼改。
- **機密與使用者資料**：SMB 測試帳密（使用者的 `Downloads\smb.txt`）不得寫進程式、文件或日誌；不得觸碰 `UserData/`；`待辦清單.md` 除使用者明確要求外不得編輯。
- **動畫設計規則**（使用者定案，所有動畫適用；細節 `docs/status/lyrics-windows.md`）：（1）歌詞動畫必須跑滿螢幕更新率（120 Hz＝120 fps、60 Hz＝60 fps），所有歌詞視窗與主視窗同時動畫時也一樣，這是驗收標準，不得改成 60 fps 滑動；（2）CSS 連續位置動畫只在移動至少 1 個實體像素時才更新（落後不到 1 px、每個目標精確到達）；例外：Windows 原生歌詞與 Android 浮動歌詞（GL 渲染）以 1/8 px 次像素位移（靜止時取整數像素）；播放進度條與底欄／聲道矩陣跑馬燈（`dock-marquee.ts`）以 `linear` 次像素移動（`tests/linear-position-animations.test.mjs`）；透明度淡化不受此限；另一例外是緩衝中進度條圓點每秒一週期的亮度脈動；（3）每幀 CPU 越低越好，每幀只依時鐘算位置並提交有變的值，不在每幀點陣化、排版或模糊，能快取就快取，閒置時睡眠。連續歌詞捲動條不可把整首歌放進去（只繪製會出現的行）。
- **列表佇列**（ADR 0037，`docs/status/ui.md`）：從任何列表建立的播放佇列必須正好是該列表頁面顯示的列與順序（範圍、搜尋、篩選、遺失檔案、停用來源都相同），佇列共用頁面查詢、不另寫 SQL；唯一例外是點的那首一定播放。
- **播放與 seek**（ADR 0036，`docs/status/now-playing-and-lyrics.md`）：前端所有 `playback_seek` 只能經 `src/lib/ipc.ts` 的 `seekPlayback`；換曲先備妥再切換、失敗不改目前播放；跳到結尾一律當曲目自然結束，絕不變成錯誤或停止；點擊不留焦點是全 App 規則（`installPointerFocusRelease`），新控制項不需另外處理。
- **封面縮放圖層**（ADR 0036）：`.cover-stage` 永久 `z-index: 1`＋`will-change: transform`，手勢或動畫開始／結束不得切換合成／堆疊屬性。
- **使用者決定（不得違反）**：前端不得再引入任何簡繁轉換，簡體「只」一律不轉「隻」（ADR 0030）；Rust 後端訊息維持繁體；取樣率轉換一律 rubato FFT、不提供選項；左滑＝下一首、右滑＝上一首，勿對調；WavPack 短名 `WVLL`／`WVL`／`WVDSD` 為定案；Android 輸出路徑不得稱 bit-perfect、藍牙編碼不得用隱藏 API；SMB 根目錄＝整台伺服器先不做；連續模式透明度為實驗性，可整筆還原（ADR 0041 D10）。
- **遠端來源**（ADR 0026，`docs/status/remote-sources.md`）：伺服器連不上不得影響本機曲目的播放、封面、列表與歌詞；任何連伺服器的檢查不得在非同步執行緒或資料庫鎖內執行；遠端播放卡住時不要重裝或重開 App，先取 `adb logcat -d | grep "\[remote\]"` 與 `/proc/net/tcp6`；備份與匯出不得含明文密碼。
- **面板與樣式**：浮動面板只經 `.floating-surface` 取得背景，`.sidebar` 的背景規則必須加 `:not(.floating-surface)`（`tests/floating-surface.test.mjs`）；貼邊內容用 `--android-safe-*`（規則見 `src/lib/AGENTS.md`）。

## 功能索引

每項：內容與關鍵名稱 → 現況檔（`docs/status/`）；括號內為 ADR 編號。

**歌詞視窗** → `lyrics-windows.md`
- Windows 原生歌詞繪製：`crates/player-lyrics-native`（單一 `moe-lyrics-native` 執行緒）、規則與共用向量 `crates/player-lyrics-rules`、`tests/fixtures/lyrics-rules-vectors.json`；資料流 `taskbar-lyrics-data` 等事件經 `src-tauri/src/native_lyrics.rs`（ADR 0034、0041 D10）。
- 工作列歌詞（僅 Windows）：`taskbar_lyrics.rs`、設定 `taskbarLyrics`、`displayMode`、`hideWhenPaused`（ADR 0018、0031）。
- 浮動歌詞：Windows `floating_lyrics.rs`、Android `FloatingLyricsOverlay.kt`＋GL 渲染；`widthPx` 為歌詞區寬、位置存歌詞區左上角（ADR 0032、0034）。
- 連續動畫與透明度：`animation`＝`none`／`continuous`、`word_strip_knots`／`knot_cubic`、靜態遮罩 `centre_mask_stops`（ADR 0041 D10）；動畫設計規則見上。

**播放頁與歌詞** → `now-playing-and-lyrics.md`
- 歌詞設定三分頁 `LyricsLookSettings.svelte`、字體粗細與透明度欄位；歌詞只有一份（`App.svelte` 歌詞控制器、`lyricsDisplayOffsetMs`）。
- 播放頁同步歌詞與瀏覽跳轉（`lyrics-window.js`、`LYRIC_CUE_LEAD_MS`、`lyrics-browse.js`；ADR 0029）。
- 播放進度時鐘 `playback-clock.ts`、進度條 `progress-run.js`、緩衝提示（ADR 0028）。
- 跳轉與焦點 `seekPlayback`、播放鍵與 seek 期間、換曲先備妥再切換 `track_start.rs`（ADR 0036）。
- 切歌封面 `active-track-artwork.ts`、封面縮放 `cover-zoom.js`／`cover-swipe-feedback.js`、手勢 `swipe-dismiss.js`（ADR 0036）。
- 歌詞來源：線上自動套用 `auto_lyric_candidate` 與 AMLL QRC（ADR 0041 D6）、本機歌詞 `parse_any_lyrics`／`lyric_roles.rs`（ADR 0040、0041）、簡繁轉換 `crates/player-zh`（ADR 0030）。

**介面** → `ui.md`
- 介面語言 `tr()`／`zh-CN.json`（ADR 0019）；字型選擇 `FontPicker.svelte`／`crates/player-fonts`（ADR 0042）。
- 左側導覽與窄版導覽面板、浮動面板外觀 `.floating-surface`、睡眠定時 `sleep_timer.rs`（ADR 0036）。
- 記住上次的頁面並於啟動時還原 `lastPage`、`start-page.js`（ADR 0053）。
- 專輯詳情捲動（ADR 0012）與資料、專輯首頁、列表、曲目欄位、歌曲來源篩選開關 `LibrarySourceScope`（ADR 0037）、同步結果顯示、頻譜圖（ADR 0038）。

**歌曲資訊面板** → `track-details.md`：`TrackDetailsDialog`、`crates/player-tags` 寫檔流程、封面計畫、儲存後重讀（ADR 0013）。

**格式與解碼** → `formats-and-decoding.md`
- 格式能力表 `crates/player-formats`（新增格式只改 `FORMATS`／`EXTENSIONS`）。
- DSD（ADR 0016、0049）與 DoP（ADR 0020 P2）、Opus（ADR 0017）、WavPack（ADR 0025）、Android PCM 與轉換橋、FLAC（ADR 0043）。

**Windows 音訊輸出** → `windows-audio-output.md`：多裝置輸出 `outputDevices`、聲道矩陣、音量同步、rubato 取樣率轉換、WASAPI 獨佔 `outputExclusive`（ADR 0020 P0／P1）。

**Android 平台** → `android-platform.md`：建置與安裝、本機來源與資料夾瀏覽器（ADR 0014、0039）、Media3 `PlaybackService`（ADR 0006、0039）、播放裝置／藍牙／USB 獨佔／混音器取樣率（ADR 0044–0048、0051、0052）、系統列安全區、權限說明面板（ADR 0039）、「開啟方式」開啟音訊檔（ADR 0054）。

**曲庫同步與資料** → `library-sync-and-data.md`：同步與取消續掃、歌詞不存內容（schema 21）、播放時檢查檔案、冷建庫效能、遺失檔案隱藏（ADR 0027、0033、0040）。

**遠端來源** → `remote-sources.md`：SMB／WebDAV、加密帳號、磁碟快取優先、`RemoteConnections`、遠端播放清單與收藏、單一備份、測試伺服器（ADR 0026、0035）。

**更新、接續播放與備份** → `updates-resume-backup.md`：版本號辨識與就地更新（ADR 0015、0021、0022）、`resumePlayback`、備份匯出／匯入（ADR 0024）。

## 程式預設值

設定的預設值取自維護者當時的 `settings.json`，涵蓋主介面（`theme`：主色 `#A9C3FE`、邊框／分隔線 10%、自動背景透明度模式、背景清晰度 65／封面 50、快速設定透明度 20%／模糊 5 px、主背景模糊 5 px／透明度 60%、幻燈片間隔 2 分鐘）、曲目欄位（欄位順序「長度」在「音訊格式」前、欄寬 184／144／257、字尾省略、專輯卡片最大寬度 200、曲庫預設依新增時間排序）、播放頁（封面背景模糊 10 px／透明度 80%）、歌詞（顯示譯文、簡轉繁開、主要透明度 100%／次要透明度 50%（播放頁、工作列、浮動各自儲存）、原文 20 px／譯文與羅馬拼音 90%（相對原文，`auxiliaryScalePercent`）、字體粗細 500（`fontWeight`）、字型皆為預設字型（`null`，含主介面 `interfaceFontFamily`）、行距 10 px、工作列歌詞預設啟用且置中、20 px、譯文 100%、自訂色 `#00e1ff`、陰影；工作列與浮動歌詞「暫停時隱藏」預設開）、輸出（音量同步預設開）與快捷鍵（鍵盤加手把按鍵／搖桿，沒有滑鼠綁定）；個別播放裝置設定、音樂來源、排除資料夾、視窗位置與語言不屬於預設值。預設值有兩份且必須一致：Rust `src-tauri/src/settings.rs`（`Default` 與 `#[serde(default = …)]` 函式，舊檔缺欄位時也套用）與前端 `theme.js`、`lyrics-preferences.js`、`now-playing-appearance.js`、`taskbar-lyrics-settings.js`、`shortcuts.js`、`track-columns.js`、`album-index-layout.js`、`App.svelte`（文字省略模式）、`OutputDevicesSettings.svelte`（音量同步）與 `app.css` 的主色變數；測試鎖定這些值。

## 重要資料與架構邊界

- 檔案本身的內容是最新事實，資料庫只是索引與快取：標籤、內嵌歌詞、旁檔、封面等以檔案當下內容為準，資料庫內容只在讀不到檔案時才使用；任何檔案（本機與遠端）開始播放時都要在背景檢查是否為最新（大小／修改時間），有變就立即更新不影響播放的資料，不得卡住播放（ADR 0040）。
- `settings.json` 是介面偏好、隨機／循環狀態及來源登錄的權威；SQLite 保存音樂資料、來源對應、播放清單投影、歌詞、聆聽統計與播放 session。Windows 便攜版使用 `release/UserData/`；不得改用 LocalCache／Roaming，也不得提交使用者資料或 Release 輸出。
- SQLite 負責搜尋、過濾、排序、分組、計數及 artist／album 聚合快照；前端只取固定 1,000 筆資料頁並以虛擬列表限制 DOM。詳見 [`crates/AGENTS.md`](crates/AGENTS.md)。
- 曲名、演出者、專輯名稱與快速索引須共用 ICU4X locale-aware natural collation；平台原生索引 API 不參與排序／分組。細節見 [`crates/AGENTS.md`](crates/AGENTS.md) 及相關 ADR。
- Renderer 不持有媒體路徑作為跨平台身份；穩定 Track ID、來源對應與平台 locator 分層。Windows 檔案總管開啟的未匯入曲目使用該佇列生命週期內的記憶體 SQLite。
- SQLite／音訊生命週期細節、播放意圖及 platform bridge 規則分別見 [`src-tauri/AGENTS.md`](src-tauri/AGENTS.md) 與 [`crates/AGENTS.md`](crates/AGENTS.md)。

## 工作與驗收規則

- 每次開始工作先讀根目錄及工作路徑上適用的 `AGENTS.md`；採信最新程式碼、測試與使用者需求，文件衝突時修正文件。需求不清楚時先查證或提問，不得擅自補需求。
- 先追查資料流與真正根因；跨領域／大型設計按需讀取 ADR 並釐清需求。保留工作樹中與任務無關的修改。
- **介面修改先臨時、再永久（使用者定案）**：使用者要求改 UI 時，先用 CDP 在執行中的頁面臨時注入樣式或腳本（Windows 用 9222 埠；Android 要裝 debug 版 `npm run android:build:arm64`，`fast` 與 release 版連不上 WebView），讓使用者在真機上看效果；使用者滿意並明確同意後，才派工寫成永久程式。臨時改動只在頁面上，重新載入即消失，量測用的探針量完立刻關掉。
- 每次修改程式碼、設定或文件後提交 Git；只 stage 本次修改。新檔優先 UTF-8 BOM，依工具鏈既有格式例外處理。README 變更需同步 `README.md`、`README_TW.md`。
- 建置、替換 Windows 執行檔、重新啟動播放器與安裝 APK 由主工作階段執行；子代理只修改程式碼與執行測試。
- 建置時間上限：`release:windows:fast` 5 分鐘、`android:build:arm64:fast` 10 分鐘、`release:windows` 15 分鐘、`android:build:arm64:release` 20 分鐘。建置輸出一律導向記錄檔（`> <log> 2>&1`），不得接管線（`| Select-Object`），因為 Gradle daemon 會一直佔住管線，指令不會結束；背景建置要同時設定到期檢查。到了上限就檢查產物（exe／APK 的修改時間是否晚於開始時間、記錄檔最後幾行、`cargo`／`rustc`／Gradle 是否還在執行）：產物已產生且沒有編譯程序就視為完成，停止該背景工作並繼續；仍在編譯就回報實際進度；卡住或失敗就讀記錄檔找原因，不得乾等。
- **啟動背景建置的同一個動作裡，必須同時啟動「完成即通知」的等待程序**（`run_in_background` 的等待迴圈，等完成檔或產物變新），收到通知立刻安裝並回報，不等使用者詢問；到時間上限仍未完成就主動檢查產物與編譯程序並回報。
- 修改程式碼、資源或建置設定時，先完成測試並提交來源，再執行一次 Windows 建置：日常小修改用 `npm run release:windows:fast`（opt-level 1、256 個編譯單元、增量編譯，獨立的 `target/fast`，不產更新 zip），使用者說「上傳 github」時才跑完整最佳化的 `npm run release:windows`，避免執行檔內嵌舊 commit；純文件修改不建置。固定 Windows 執行檔為 `release/moemusicplayer.exe`。播放器執行中也照樣背景替換新版（含回滾與 SHA-256 驗證），替換完成後須正常關閉舊程序並立即啟動固定路徑的新 EXE，核對新版視窗及 build identity；不得強制終止播放器。**建置前先備份目前的 exe**：一律放在 `release/backups/`，檔名 `backup-<yyyyMMdd-HHmmss>-<舊檔 SHA-256 前 8 碼（大寫）>.exe`（時間為台北時間的備份當下），例如 `backup-20261022-163948-6D0314E9.exe`；只保留最新 5 個（依檔名時間排序），每次備份後自動刪除較舊的，不需詢問使用者。腳本自己留在 `release/` 的 `.moemusicplayer.previous.<id>.bak` 不屬於上述備份規則，不得改名或搬動；每次建置後只保留最新 3 個（依修改時間），較舊的直接刪除，不需詢問使用者。
- Bug／功能驗收一律啟動固定路徑的 Release 執行檔，以該程序的 WebView2 CDP 檢查實際 DOM、IPC 或錯誤；不得改用 Chrome、Playwright 或無資料隔離副本。開發期間主播放器一律以偵錯埠 9222 啟動並保持開啟，驗收後不必關閉。CDP 啟動方式：先執行 `netsh int ipv4 show excludedportrange protocol=tcp` 確認埠不在 Windows 保留範圍（例如 9326–9425 會讓 9337 無法監聽），預設使用 9222；設定 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` 後啟動 `release/moemusicplayer.exe`，再以 `netstat -ano | findstr :9222` 確認 LISTENING，並以 `http://127.0.0.1:9222/json/version` 確認有回應後才開始驗收。建置／單元測試不等於實際 App 驗收。Android 必須在 Android 目標環境驗收，並在所有有連線的裝置上測試（見上方「硬性規則」的 Android 裝置規則與 `docs/status/android-platform.md`）。
- 不得強制終止使用中的播放器；Windows 新版按既定背景替換程序與備份規則更新，絕不觸碰 `UserData/`。
- 工作結束時必須提供完整結案報告：實際修改內容、執行的建置命令與版本、安裝／啟動在哪些裝置、無法安裝或未實測項目及原因；不得把建置成功說成已安裝，也不得把單元測試說成實機驗收。
- 不以 Python sqlite3 讀寫使用中的正式 SQLite；先確認播放器關閉，或使用已驗證的唯讀／備份流程。
- 依變更範圍執行 `npm run check`、`node --test`、相關 Rust tests、workspace Clippy 與 rustfmt；完成工作後依「本文件維護規則」就地更新對應的 `docs/status/` 檔（必要時更新「功能索引」）。
- 發布（使用者說「上傳 github」）：依 [`docs/RELEASE.md`](docs/RELEASE.md) 執行（版本與命名、發布說明 `docs/release-notes/v<版本>.md` 的撰寫規則與先給使用者看過、發布前整理本文件並通過 `node --test tests/agents-md-size.test.mjs`、建置、上傳與 digest 確認、回報）。

## 按需閱讀的領域規則

- UI、列表互動、播放頁、歌詞、背景及視覺 token：[`src/lib/AGENTS.md`](src/lib/AGENTS.md)
- Tauri IPC、使用者資料、播放生命週期及更新流程：[`src-tauri/AGENTS.md`](src-tauri/AGENTS.md)
- Rust Core、SQLite、排序／分組、跨平台資料與音訊邊界：[`crates/AGENTS.md`](crates/AGENTS.md)
- 各功能目前狀態（行為、關鍵名稱、待驗收項目）：`docs/status/` 下依領域分檔，入口見「功能索引」。
- 產品詞彙：[`CONTEXT.md`](CONTEXT.md)；架構決策：[`docs/adr/`](docs/adr/)
