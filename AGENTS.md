# AGENTS.md

## 專案現況與目標

專案已有可建置的 Tauri 2、Svelte 5、TypeScript 與 Vite 骨架，並包含 Rust Core、SQLite 曲庫、Windows 與 Android 媒體來源 adapter，以及雙語 README。

目前已完成：

- Windows 可新增來源資料夾，透過 Core 增量同步與 Lofty 讀取標籤；SQLite 保存曲目、來源映射與同步狀態，前端只取分頁結果。
- Windows 來源設定使用原生資料夾選擇器；程式在取消時不新增來源。使用者已在隔離 app-data 實際選取資料夾並完成掃描；Cancel 不新增來源尚未單獨確認。Tauri picker 及 M3U/M3U8 檔案對話框都以呼叫端 WebviewWindow 設為 owner，主視窗 config 與啟動時也明確關閉 always-on-top。
- 啟動同步由 renderer 在註冊 `library-sync-progress` 與 `library-sync-finished` 後觸發，避免空來源或快速掃描在 listener 建立前完成；不定總數的列舉只顯示已處理數。前端 call-site regression 與來源事件彙整測試通過。
- 對 `WindowsSystemIndex` root 會探測 WSearch 服務、範圍規則與 catalog 狀態供診斷；目前不把 Search 查詢結果當完整清單，曲目與刪除對帳只依完整檔案系統走訪。
- Android MediaStore/SAF 外掛、Rust IPC 與來源管理畫面已接線；Android ARM64 Debug APK 曾成功建置。最新版 insets 修正尚未在裝置複驗。
- Windows 音訊 crate 使用 Rodio/CPAL；目前 17 項單元與 2 項隱藏 HWND 系統媒體測試通過。Tauri 已將 `TrackId` 解析為 SQLite 中啟用來源的本機路徑，再呼叫 `PlayerHandle`。actor 現提供非阻塞命令 ticket，Play/Pause/Seek/Volume ACK 於 worker 更新後快照；等待逾時會回明確錯誤，Tauri IPC 使用 `spawn_blocking` 等待，避免阻塞 async runtime。播放進度滑桿由 window 層 pointerup/cancel 收束拖曳，CSS 命中高度 24px、可視軌道 3px；headless Svelte DOM/鼠標合成橋接測試通過。Rust session queue 已接上曲庫查詢與播放清單來源、自然播完、前後首、隨機及 off/all/one 循環；播放清單佇列保留重複項，並以 entry position 選取所點項目。系統媒體控制的 Next/Previous 能力依目前 queue 狀態提供。這些測試不代表實際 Tauri 視窗或音訊輸出驗收。
- 曲庫列表已接入獨立 `src/lib/TrackList.svelte`：每次只取後端 40 首頁面，虛擬化可見列並以六頁／最多 240 首作 LRU 快取；查詢重置、過期回應隔離、錯誤重試與鍵盤瀏覽都已實作。七項 Node 測試涵蓋分頁競態與 100,000 首合成捲動；模擬 2,778 個捲動位置時最多 28 列、六頁快取。這是資料窗格測試，不代表 WebView 畫面影格效能；瀏覽器 DOM 掃描仍待執行。
- SQLite v4 保存播放清單及順序項目與背景／主色偏好；部分索引標記未匹配項目，播放清單頁面或 queue 查詢前會在單一交易內嘗試重對應整份清單，只填入唯一匹配且仍為 NULL 的 TrackId，不覆蓋已匹配項目或人工欄位。Windows 前端以分頁 IPC 瀏覽清單，透過 Rust 原生對話框匯入 M3U/M3U8、匯出 UTF-8 M3U/M3U8；未匹配曲目保留原 locator。相對匯出會拒絕逃出所選共同根目錄的路徑。Unicode M3U/M3U8 匯入、分頁及匯出 helper 整合測試通過；播放清單原生對話框仍未以 GUI 驗收。
- 設定頁將來源管理收在「設定 → 音樂來源」，另有「設定 → 外觀」可自訂背景色與主色；預設純黑 `#000000`、水藍 `#55D9FF`。外觀只透過窄 Tauri IPC 讀寫 SQLite typed `ThemePreferences`，不使用 localStorage；Renderer 依對比度選擇黑／白文字、主色按鈕文字及焦點色。十六進位色值驗證、黑白背景／低亮度主色對比測試通過；`player-db` 25 項測試包含非法值不改寫、預設 migration、關閉重開後偏好一致及 playlist locator reconciliation。設定頁的實際 GUI 視覺、縮放及偏好重啟回讀尚未由使用者驗收。
- Windows 正在播放曲目封面透過 `library_get_track_artwork` 單首二進位 IPC 延遲讀取，只使用啟用來源映射；優先 Lofty 內嵌圖片，再找同資料夾 `cover.jpg`、`folder.jpg`。保留原始編碼位元組與像素尺寸，不經 JSON/base64、不重編碼；單一 Blob URL 共用於「正在播放」與底部播放列，切歌、停止或元件卸載時釋放，過期回應會丟棄。上限 32 MiB、單邊 16,384、64 百萬像素；後端會以 `ARTWORK_TOO_LARGE` 區分超限與缺圖，前端顯示對應說明；有效 sidecar 仍可作為超限內嵌圖的 fallback。封面 Rust 測試 20 項、前端測試 7 項通過；真實 Tauri 畫面尚未驗收，Android Content URI artwork 尚未接線。
- 使用者已在隔離 Tauri/app-data 中透過原生資料夾選擇器選取唯讀來源 `\\?\D:\Music`；隔離資料庫保存 1 個 root、5,362 個曲目與來源映射，掃描狀態為 complete，另有 3 項 WSearch 診斷。正式 App SQLite 未觸碰。原生對話框的取消路徑尚未單獨確認。
- Windows 系統媒體控制已從 Tauri 主視窗 HWND 接入 WinRT worker；Shell 以播放器快照每 750 ms 至少間隔更新標題、演出者、專輯與播放狀態，並把 Play/Pause/Stop/Seek/Next/Previous 事件送回音訊服務。Next/Previous 能力依 active queue 狀態更新。crate 的 17 項單元與 2 項隱藏 HWND 整合測試通過；Shell fake-event 測試涵蓋控制映射、無佇列能力時停用佇列事件及 attach/update 失敗與音訊隔離。實際 Tauri 視窗、媒體 flyout 與硬體按鍵仍未驗收。
- Svelte 型別檢查、Vite 正式版建置、完整 `cargo test --workspace --locked`、嚴格 Clippy 與 Windows Tauri release 建置均通過；輸出為 `target/release/moemusicplayer.exe`。Windows shell 整合測試涵蓋 Unicode 資料夾新增、合成 MP3 同步、SQLite 分頁/搜尋、關閉重開，以及 TrackId 到保存路徑再到播放 handle 的命令流程；另有 queue IPC JSON camelCase 測試。

仍未完成或未驗證：

- Windows 原生資料夾選擇器已由使用者在隔離 app-data 選取 `D:\Music` 並完成唯讀同步；正式 AppData 未觸碰。Cancel 不新增來源的路徑尚未單獨確認；播放清單原生對話框仍待使用者驗收。系統媒體控制尚未由實際 Tauri 視窗、Windows 媒體 flyout 或硬體媒體按鍵端到端驗收。
- Windows Tauri/音訊 IPC 的命令 ACK 與 queue/mode 已接線；`npm run check/build`、workspace Rust tests、嚴格 Tauri Clippy 及 Windows release build 通過。服務整合測試以臨時 Unicode 音樂資料夾與合成 MP3 驗證 TrackId 路徑、worker 播放、暫停與音量，並驗證重複 playlist TrackId 依 entry position 建 queue。實際 Tauri 視窗／音訊輸出、手動滑桿及 SMTC flyout/硬體鍵仍待隔離 app-data 驗收。headless DOM probe 確認滑桿 hit box 24px 且滑桿外 pointerup 可送 seek。worker 超時會回錯誤，但正在執行中的命令不會取消，之後仍可能完成；UI 尚需確認錯誤回報與後續快照更新行為。合成 DOM/輸出裝置測試不代表實際 Tauri 音訊或聽感驗收。
- 設定頁與真實 Tauri 視窗的自訂色對比、重啟回讀及多視窗尺寸版面尚未人工驗收；目前只通過前端 helper、SQLite 重開測試、Svelte 型別檢查與正式版建置。
- Hanser `hanser.m3u8` 已以唯讀隔離來源資料庫透過 SQLite backup API 複製到 `target/` 新資料庫，未觸碰正式資料庫或音樂原檔。嚴格 UTF-8 驗證為 176 個絕對路徑且全數存在。來源舊清單 ID `8bf3f89c-04c0-418e-b29b-a33d867b15ab` 原有 176 項、0 匹配；觸發頁面/queue 查詢後全清單重對應為 176/176，queue 為 176 項。實際匯入另建立新清單 ID `ca39c101-c843-48a4-a100-9d72cf351736`，176/176 匹配；來源清單數 1→2，兩份清單項目順序 0..175，關閉重開後仍一致。Core 共用 Windows locator key 正規化涵蓋一般與 extended path、Unicode、空白及長路徑；Android 音訊尚未實作。
- Android 最新頂部/底部 system insets 修正及 MediaStore/SAF 真機掃描尚未驗證。先前 ARM64 APK 建置早於本輪共用 shell 變更；本輪直接 `cargo check --target aarch64-linux-android` 因目前 shell 找不到 `clang.exe` 而停在 SQLite native build，未得到 Android app 編譯結果。Android 播放未實作。
- Windows 封面目前只讀取已啟用本機檔案來源；Android MediaStore/SAF 封面、使用者實際 Tauri 視窗圖片解碼／縮放與封面載入失敗畫面仍待平台接線及驗收。封面上限內仍可能遇到瀏覽器無法解碼的容器資料，該路徑會回到占位圖。
- 100,000 首真實曲庫端到端效能、WebView 實際 DOM／捲動影格表現及完整 Windows SystemIndex 使用判準尚無實測結論；目前只有列表資料窗格的合成測試。

下一步由 root 在隔離 app-data/Tauri 視窗驗收已自動重對應的舊 Hanser 清單播放與 queue 控制；本輪 Rust 隔離 DB 已驗證舊清單 0→176、新匯入 176/176、順序與重開持久性。再驗收 queue next/previous、自然播完、off/all/one、shuffle、封面與 ACK 輪詢；不得操作使用者正在使用的視窗或正式曲庫。之後由使用者手動確認滑桿 thumb、拖曳 seek 與底部控制穩定性。另續驗來源選擇器 Cancel、外觀頁、播放清單對話框、系統媒體 flyout/按鍵、封面真實畫面與應用音訊輸出；`D:\Music` 已掃描完成，不重複掃描或寫入音樂檔。另於 `tests/track-list-harness.html` 確認 100,000 首列表的實際 DOM 列數、捲動與鍵盤操作。Android 驗收時以 Tauri Android CLI/NDK 環境重新建置目前 shell，確認安全區與 MediaStore/SAF 真機掃描。

下一輪待辦：曲目資訊顯示 codec、sample rate、bitrate；只有無損格式且來源 bit depth 可可靠辨識時才顯示 bit depth，不得把 AAC/MP3 等有損格式的解碼 PCM depth 當來源深度。統計每首實際播放累計毫秒；未來 shuffle 優先較小的 `played_ms / track_duration_ms`，duration 為 null 或 0 的處理方式須另行定義並測試。以上均為未實作規劃。

產品以 Windows 11 與 Android 的大型本地音樂庫為核心，目標規模為 100,000 首。啟動、搜尋與播放不得等待全庫掃描；封面、歌詞與動畫不得造成記憶體或 DOM 持續成長。使用者資料須可攜；本地音樂庫與本地播放永遠優先，線上串流服務不屬於核心目標。

以下「必須／不得」是功能或安全邊界；「預設／優先／建議」是待原型與測量驗證的方案。不得把尚未實作的需求從規格中移除，或把規劃文字當成已完成狀態。

## 必須維持的產品行為

- **音樂庫：**使用者可加入多個來源。啟動時先顯示上次保存在 App SQLite 的可用曲庫，再背景同步並增量更新畫面；提供手動重新整理，但正確性不可依賴手動操作。每次啟動自動同步是最低要求，無需為此強制常駐檔案監控。來源暫時離線、根目錄不可讀或 SAF 授權失效時，不得將歌曲當成已永久刪除。
- **增量同步：**Windows 一般檔案至少比較正規化路徑、大小與修改時間；只解析新增或變更檔案。只有來源成功完成掃描且可確認缺席後，才能處理刪除。單一壞檔須記錄錯誤並繼續整批同步。Android 一般共享媒體優先使用 MediaStore，特殊授權來源使用 SAF；不得套用 Windows 式任意路徑掃描。
- 同步結果必須區分「檔案消失」「來源不可用」「掃描未完成」；NAS 離線、卸除式磁碟拔除或 SAF 權限失效時保留既有曲目與人工資料，恢復後再對帳。不得因切換 SystemIndex 與 fallback 而建立同一歌曲的重複身份。
- **基本中繼資料：**新檔或變更檔需取得路徑或 URI、大小、修改時間、標題、演出者、專輯、專輯演出者、曲序、碟序、長度、編碼或容器；可取得時補上位元率與取樣率。掃描階段不批次解碼封面或抓取網路歌詞。
- **封面：**延遲載入，依序考慮內嵌、同資料夾 sidecar、選用的網路來源。縮圖按畫面需求產生並共用；不批次保存高解析原圖至 SQLite、不透過 IPC 批量傳送 base64。Renderer 的圖片快取須有上限且可清除。
- 縮圖可按 128/256/512 像素等畫面需求生成；同專輯或相同 artwork 儘量共用。封面、歌詞、分析結果等快取不影響資料正確性，需有容量上限與釋放路徑。
- **播放清單：**支援 M3U/M3U8 匯入與匯出；M3U8 使用 UTF-8。正確處理 Unicode、空白、長路徑、相對與絕對路徑；共同根目錄下可選相對路徑匯出。內部資料表不得取代標準交換格式。
- **搜尋：**100,000 首仍可快速查詢，使用分頁或虛擬列表；不得先把全庫載入前端再篩選。SQLite FTS5 是預設候選，需由實測確認。
- **播放：**播放/暫停、前後首、跳轉、音量、循環、隨機與佇列。架構需容納無縫播放。Windows 整合系統媒體控制；Android 使用 Media3/MediaSessionService，支援背景、通知列與藍牙控制，Activity 關閉後仍可依平台規範播放。
- **歌詞：**支援本地 LRC、可取得的內嵌歌詞、網易雲與 QQ Provider、同步顯示、候選手動搜尋/指定及持久快取。Provider 與 UI、Library Core 分離，需有逾時、可取消與受控重試；網路失敗不得阻礙啟動、曲庫瀏覽、本地歌詞或播放。自動匹配須綜合標題、演出者、長度、專輯、版本與 feat. 資訊；低信心不自動套用。使用者指定與本地歌詞優先於網路自動結果。
- 歌詞匹配要有可解釋分數與信心門檻。標題、演出者權重高；長度差異有實質影響；專輯為低權重。正規化大小寫、全半形、空白、常見括號及 feat./featuring/ft.，但須保留 Live、Cover、Acoustic、Instrumental、TV Size、Remaster 等版本差異；手動指定須持久保存並優先於後續自動搜尋。播放期間不得批量觸發網路補全。
- **歌詞渲染：**僅渲染可見行及少量緩衝；只有目前行及必要相鄰行可拆成逐字節點。高頻進度不觸發整棵 UI 重繪。實際 DOM、計時器與 GPU 資源用量不得隨歌曲長度或切歌次數無限增長。
- 可從目前行前後各約 5～8 行作為窗口起點，再依實測調整；禁止整首逐字歌詞一次建立動畫 DOM。進度優先透過 requestAnimationFrame 更新少量 CSS/DOM 屬性；避免大範圍動態模糊、大量 filter、逐字 GPU layer 與無限制 will-change。
- **響應式介面：**依可用寬高、比例及容器空間安排版面；播放控制始終可用，列表、歌詞、抽屜與對話框不得溢出。版面切換不得重設播放、佇列或歌詞狀態。優先使用 CSS Grid/Flex/容器查詢；若需 JavaScript 判定整體模式，由單一服務集中計算並節流。
- 至少涵蓋寬橫向、一般橫向、近正方形、窄直向，以及 Windows 可縮放小視窗與 Android 橫直向；不可只用單一寬度斷點或裝置名稱決定整體布局。空間不足時改為堆疊或抽屜，封面尺寸受 viewport 約束；高 DPI、拖曳縮放及方向切換後仍須可讀、可操作且不卡頓。
- 播放頁、側邊欄、佇列、設定、歌詞與專輯頁都要遵循相同布局原則；具體閾值由畫面實測決定。布局模式改變只重新排布 UI，不得重建播放核心或中斷音訊。

響應式驗收至少覆蓋：3840×2160、2560×1440、1920×1080、1600×900、1366×768、1280×1024、1024×768、900×900、800×1200、720×1280、412×915、360×800；同時檢查無水平溢出、重要按鈕可見、歌詞可讀、封面比例、列表操作、抽屜/對話框邊界、視窗縮放與方向切換後的狀態保持。

## 架構邊界

預設使用 **Svelte 5 + TypeScript + Vite → Tauri 2 IPC → Rust Core → 媒體索引與 App SQLite 層**。Tauri 2 的選擇服務於 Windows/Android 共用 Web UI、原生核心、較低基礎記憶體與安裝體積；Windows 音訊評估 Symphonia、cpal 與 WASAPI，Android 音訊由 Kotlin 插件接 Media3/MediaSessionService。改用其他主框架或改變重大邊界前，須有原型、效能證據與 ADR；不得僅因方便改用 Electron。

前三者同屬前端工具組合：Svelte 5 建立介面元件，TypeScript 檢查前端資料與 IPC 型別，Vite 提供開發伺服器並建置正式版靜態檔案；正式執行時由 Tauri WebView 載入前端，Vite 不是播放或資料處理層。

- **前端**只處理畫面、輸入、少量檢視狀態、虛擬列表與動畫；不直接遍歷檔案、解析標籤、讀寫 SQLite、解碼音訊或持有全庫。IPC 命令需窄、明確、可版本化，回傳分頁資料並節流高頻事件；不可暴露任意檔案或 SQL 能力，也不可批量傳回曲庫或封面 BLOB。
- **Rust Core**負責跨平台 Library、播放清單、歌詞匹配、播放政策及佇列。平台系統索引、檔案 API、SQLite 交易與同步細節由媒體資料層及平台介面封裝；Core 不直接依賴 SystemIndex、MediaStore、Android Cursor 或 Windows Property Key。
- **媒體資料層**提供統一的曲目查詢、搜尋、增量變更、專輯/演出者與同步介面；負責系統索引查詢、來源可用性、metadata 補全、系統 ID ↔ internal ID 映射、schema migration、交易、FTS 投影、快取失效與診斷。Core 不用平台分支實作兩套 Library 邏輯。
- **Windows 媒體來源**對已索引且結果完整、及時的指定資料夾優先使用 SystemIndex；未索引、服務停用、NAS/卸除式磁碟或欄位缺失時使用檔案系統掃描與必要的中繼資料解析。是否要求使用者加入索引範圍須明確告知並保留 fallback；不可把 SystemIndex 當成唯一真相。USN 僅在實測需要時以 ADR 引入。
- **Windows 現況：**SystemIndex 目前只有唯讀健康/範圍診斷用途，尚未採用其曲目列舉結果。`WindowsSystemIndex` root 仍由完整檔案系統走訪產生 seen-set；只有完整走訪才可確認缺席並刪除映射，部分或不可用時保留既有資料。
- **Android 媒體來源**一般共享媒體使用 MediaStore；未涵蓋或需授權的來源使用 SAF，並在平台允許時保存持久 URI 權限。MediaStore 欄位不足時才針對檔案補解析。
- **Android 現況：**native `WindowInsetsCompat` 以 system bars、display cutout 與 mandatory system gestures 計算動態安全區，CSS 不再重複加 safe-area padding；最新修正未經 NX809J 直橫向幾何驗收，不得宣稱已解決遮擋。
- 系統索引失效或回退掃描時，不能要求使用者重建整個 App SQLite；平台 adapter 需將 SystemIndex/MediaStore/SAF 結果正規化為共同的曲目模型，再由同步層與 App 資料合併。
- **App SQLite**保存內部 ID、啟動/離線顯示所需的曲目投影、來源映射、使用者覆寫、播放清單、歌詞/封面快取、設定與同步狀態。系統媒體索引用於確認來源現況，但不得覆蓋使用者資料。使用 migration、批次交易、索引與必要的 FTS；不得逐曲提交交易。快取有上限、可清除、可觀察。
- SQLite 建議 WAL、prepared statements 與分頁查詢；啟動投影要足夠顯示上次曲庫，但不無條件複製系統資料庫所有欄位。播放次數、最後播放時間、收藏、歷史及人工匹配若實作，均屬 App 資料，不得被系統 metadata 更新清掉。
- **身份與同步：**全平台以穩定 internal track ID 關聯播放清單與使用者資料；來源類型、來源 ID 與 locator 分開保存，不能單用路徑當全平台身份。系統索引切換到 fallback 或來源暫離線時，須保留對同一曲目的映射與人工資料。
- 身份來源例：Windows SystemIndex 項目、Windows filesystem fallback、Android MediaStore `_ID`、Android SAF document URI。這些來源 ID 與 locator 是平台資料，UI 與跨平台 Core 只使用 internal track ID；不得直接依賴 Content URI 或 Windows Property Key。
- **音訊平台邊界：**共用播放政策與 AudioBackend 介面，不強迫共用底層解碼/輸出。Windows 評估原生音訊與 WASAPI；Android 的 MediaSessionService 是背景播放和系統控制的權威來源，WebView/Activity 銷毀不得使其錯誤停止。音訊路徑不得受掃描、圖片或 Provider 工作阻塞。
- AudioBackend 至少容納載入、播放、暫停、停止、跳轉、音量、位置/長度查詢、前後首與裝置狀態；實作無縫播放時仍由音訊層維持時序，Renderer 不成為音訊真相來源。
- **路徑與時間：**Windows 支援中文、日文、Emoji、空白與長路徑；Rust 內部避免將 Path/OsStr 過早轉為有損 UTF-8。持久化時間使用有明確時區的格式或 UTC epoch；顯示依使用者時區，未知時預設台北時間。既有無時區資料不得猜測時區。

## 效能、錯誤與測試

- 啟動顯示舊曲庫不等待同步；未變更歌曲不重讀標籤，封面與歌詞按需載入。背景掃描、圖片處理與 Provider 請求須限制並行，不可搶占音訊。每次播放進度更新不得做 O(N) 曲庫操作、掃描全部歌詞 DOM，或造成全應用重繪。
- 離開畫面或切歌後須取消 requestAnimationFrame、移除 listener、取消可中止請求並釋放 object URL 與大型物件。封面瀏覽、切歌與歌詞切換不得使 Renderer/Rust heap、DOM 節點或 GPU surface 無界增長。
- 背景錯誤依可恢復、可重試、權限、中繼資料、格式、資料庫、網路、音訊裝置、致命等類別處理；單檔失敗不中止整批。UI 提供可理解的診斷，詳細資訊留在日誌。
- 修改 scanner、metadata parser、資料庫查詢、音訊路徑、歌詞渲染或封面快取時，新增對應的風險測試；重大效能改動另以 benchmark 驗證，不能只憑體感。曲庫基準涵蓋 10k/50k/100k 首、首次建庫、全未變更重啟、新增/修改/刪除各 100 首、Unicode/長路徑及大量小檔。實作時測試來源離線與恢復、SystemIndex 缺漏、MediaStore/SAF 權限變化、身份映射與人工資料保存。
- Rust 測試至少涵蓋 migration、掃描差異、M3U/M3U8 解析與 Unicode 路徑、歌詞匹配；前端至少涵蓋元件、虛擬列表、歌詞窗口及記憶體生命週期。跨模組整合要覆蓋曲庫匯入、播放清單匯入/匯出、重啟同步、刪檔與離線根目錄、手動歌詞覆寫、Windows 系統媒體控制、Android MediaSession 背景播放。
- 媒體資料層需分別驗證 Windows 索引完整/未索引/停用/未完成/欄位缺失、卸除式來源與 fallback；Android MediaStore 新增/修改/刪除、SAF 文件樹、URI 權限失效與來源暫不可用；兩平台皆檢查 playlist reference、lyrics cache 與人工資料在同步後仍穩定。
- 功能完成須驗證正常與失敗路徑、不阻塞 UI、不破壞本地播放、快取與資源用量有界，且文件與實際行為一致。跨平台整合需在相應系統上驗收；編譯成功不等於播放或畫面驗收。

## 可參考專案與授權邊界

以下連結是研究行為、架構與使用體驗的來源，不是本專案的程式碼基底。新增依賴、改作或引用程式碼前，需重新查看對應版本的 `LICENSE` 及第三方授權；在 GitHub 能看到原始碼不代表可自由複製。若正式公開原創程式碼，優先評估 MIT 或 Apache-2.0，並在選用 GPL/AGPL 類元件前確認對分發方式的影響。

### [ECHO](https://github.com/Moekotori/ECHO)

- 可研究 Library Core／Audio Core／Renderer／Native Host 的責任分離、SQLite 曲庫、`path + size + mtime` 增量掃描、大型列表分頁、背景任務降載與診斷方式；以實測檢查哪些做法適合 100,000 首及跨平台目標。
- 不沿用 Electron 作為預設框架，也不照搬高 DOM 或高 GPU 合成成本的逐字動畫歌詞實作。
- [ECHO NEXT 授權檔](https://github.com/Moekotori/ECHO/blob/main/LICENSE)標示為 source-available，並非一般開源授權；不得直接複製其原始碼，亦不得以其 fork 作為本專案可公開分發的基底，除非取得作者明確許可。

### [Any Listen](https://github.com/any-listen/any-listen)

- 可研究 Provider／擴充套件分層、本地列表同步的操作體驗、現代 Web 播放器介面，以及跨模組共用資料模型；須自行驗證對本地優先與低資源用量的適用性。
- [Any Listen 授權檔](https://github.com/any-listen/any-listen/blob/main/LICENSE)以 AGPLv3 為基礎並加入商業使用限制；不要直接複製程式碼，除非已確認授權範圍及本專案分發方式可接受。

### [MusicPlayer2](https://github.com/zhongyang219/MusicPlayer2)

- 可研究 M3U/M3U8 匯入匯出行為、本地播放器功能完整度、網易雲／QQ 歌詞來源的使用體驗，以及 Windows 桌面播放控制細節；以行為測試重新實作，不移植其程式碼。
- [MusicPlayer2 授權檔](https://github.com/zhongyang219/MusicPlayer2/blob/master/LICENSE)為 GPLv3；如考慮使用其程式碼，須先評估 GPL 義務與本專案的授權策略。

## 工作方式與下一步

1. 開工先閱讀本文件、相關程式碼與 ADR；以最新需求、程式和測試結果修正過時敘述。較大修改先說明目標、落點層級與硬限制，區分平台硬限制、工程限制和慣例假設；比較至少兩種方案，必要時用原型或 benchmark 找瓶頸。重大架構變更寫入 `docs/adr/`，優先修正根因，不以 UI 權宜作法掩蓋後端問題。
2. 變更保持聚焦，不自行改變硬性需求或更換核心框架。引入依賴前確認必要性、重量、維護狀況與授權；參考外部專案時遵守上節邊界。臨時做法要標記 temporary、建立 issue/TODO 並避免成為長期依賴。
3. 每次修改後提交 Git，包含程式碼、`AGENTS.md`、雙語 README 與必要 ADR；執行暫存資料和使用者設定加入 `.gitignore`。新文字檔優先 UTF-8 with BOM，除非工具鏈要求其他編碼。README 使用英文 `README.md` 與繁體中文 `README_TW.md`，頂端皆放 `[English](README.md) | [繁體中文](README_TW.md)`，內容保持同步且只寫使用者需要的用途、功能、安裝與基本用法。技術文件的時間紀錄必須帶時區。
4. 每次收尾更新本文件的**目前狀態、未解問題與下一步**；後續使用者新增的功能、限制與驗收需求必須同步納入本文件的當前需求與狀態，不得只留在對話，並移除已過時資訊；不累積版本歷史。需要重啟任何裝置時，先說明原因並取得使用者明確同意，或由使用者親自操作。

階段順序保留原規劃的範圍：

1. Tauri/Svelte 骨架、媒體資料抽象、Windows SystemIndex 與 Android MediaStore 適配、App SQLite、來源管理、Windows fallback、metadata、分頁列表與基本播放。
2. M3U/M3U8、搜尋/FTS、專輯與演出者分組、封面快取、佇列。
3. 本地與內嵌歌詞、窗口渲染、網易雲與 QQ Provider、可解釋匹配及手動覆寫。
4. Windows 系統整合、Android Media3/MediaSession 背景播放與 SAF 來源。
5. 核心穩定後評估無縫播放、進階音訊裝置、DSP/EQ、遠端曲庫/同步與插件；Phase 1 不先投入大量視覺特效。

目前可重現的驗證命令為 `npm run check`、`npm run build`、`node --test`、`cargo check -p moemusicplayer --locked`、`cargo test -p player-db --locked`、`cargo check --workspace --locked`、`cargo test --workspace --locked`、`cargo clippy --workspace --all-targets -- -D warnings` 與 `npm run tauri -- build --ci`。本輪前端檢查與建置通過，Node 測試 27/27、`cargo check -p moemusicplayer --locked` 通過，`cargo test -p player-db --locked` 25/25。虛擬曲庫的本機瀏覽器測試頁為 `tests/track-list-harness.html`，提供合成 100,000 首分頁來源、DOM 列數/捲動和鍵盤播放檢查；Node 資料窗格測試最高 28 列、240 個快取項，尚未完成瀏覽器互動驗收。Windows shell E2E 是 Rust 服務層測試，會建立臨時 Unicode 音樂資料夾與合成 MP3，將來源及曲目存入臨時 SQLite、驗證分頁/搜尋，再關閉並重新開啟資料庫核對來源路徑與 TrackId；播放測試從該 TrackId 查詢啟用來源路徑，將其交給 `PlayerHandle`，並驗證播放、暫停、跳轉與音量命令。Fake SMTC event 測試確認 Play/Pause/Stop/Seek 路由到既有播放命令、Next/Previous 依 queue capability 路由，以及 attach/update 失敗只降低系統媒體控制能力，不會阻塞獨立播放器。另有 IPC JSON 測試覆蓋 queue source、PlaybackSnapshot 與 runtime capability 欄位的 camelCase。`D:\Music` 已由使用者在隔離 app-data 透過原生資料夾選擇器唯讀掃描完成，正式資料庫未碰觸；取消流程、播放清單原生對話框、實際 Tauri SMTC 視窗／flyout／硬體按鍵與應用音訊輸出仍未完整驗證。Rodio/CPAL 自產 WAV 預設裝置 smoke 是獨立測試。Android 最新安全區與裝置媒體掃描仍待驗收；直接 Android Cargo target check 需先配置 NDK Clang，不能把其失敗當成 app code 編譯錯誤。
