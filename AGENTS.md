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

睡眠定時：正在播放頂欄「羅」切換與歌曲資訊（ⓘ）按鈕之間的鬧鐘圖示按鈕（面板向下開啟）（aria-label／tooltip「睡眠定時」，啟用時 `.is-active` 並在 tooltip 顯示剩餘時間）開啟 `.floating-surface` 浮出面板；點外部或 Esc 關閉。面板（定位與音量／收藏浮出面板共用 `getPopoverHorizontalShiftPx`：置中於按鈕、窄視窗時夾在 8 px 邊距內）以 hh:mm 顯示剩餘時間（向上取整到分鐘、每秒更新）；小時與分鐘是兩個獨立的 `role="slider"` 欄位，在欄位上上下拖曳（觸控相同，`touch-action: none`，每 14 px 一格）、滾輪（像素模式每 50 px 一格）或方向鍵（作用於聚焦欄位），每格 1 小時或 1 分鐘；小時 0–23、分鐘 0–59 各自夾住，不循環也不進位（`adjustSleepTimerField`），上限 23:59（Rust `MAX_SLEEP_TIMER_MS` 同步）；計時中調整即設為新的剩餘時間，且恰為 hh:mm:00。調整以「手勢」為單位：pointerdown 時鎖定欄位並 setPointerCapture，滾輪／方向鍵在 800 ms 無動作後結束；手勢開始時凍結當下顯示的 hh:mm（`beginSleepTimerEdit`），每一格都從這個快照計算（`stepSleepTimerEdit`，超出範圍的格數不累積），手勢期間顯示手勢值而非即時倒數，所以秒數向上取整（00:59:30 顯示 01:00）或倒數跨分鐘都不會影響另一欄；`sleep_timer_set` 的回應以序號丟棄過期結果；設定 > 0 即啟動，調到 0 或「取消定時」即關閉。期限由 Rust 核心 `src-tauri/src/sleep_timer.rs` 持有（Unix epoch 毫秒截止時間＋世代號），背景任務每秒重讀系統時鐘，不累加 tick；到期時清除定時、送出 `sleep-timer-changed` 並呼叫 `playback_pause`（暫停而非停止）。前端 `src/lib/sleep-timer.js` 只做顯示與調整換算，WebView 節流也不影響到期。

浮動面板外觀：快速設定、窄版導覽面板（`.sidebar.floating-surface`）與睡眠定時面板都只透過 `.floating-surface` 取得背景（`--text-inverse` 依 `--quick-settings-opacity` 混合）、`--quick-settings-blur` 模糊、陰影與圓角；這兩個變數由 App.svelte 掛在 `.app-shell` 上。任何以 `.sidebar` 為主體、設定 background／backdrop-filter 的規則都必須加 `:not(.floating-surface)`（原本 `.sidebar { background: transparent }` 寫在 `.floating-surface` 之後、同等特異度而覆蓋掉設定），各面板自身規則也不可再設背景；`tests/floating-surface.test.mjs` 會檢查。

曲目表格水平捲動：所有 `PagedVirtualList` 表格（曲庫、專輯、播放清單、演出者、播放佇列）的列寬為 `max(100%, --track-list-min-width)`，該值統一取 `max(minContentWidth, getGridTemplateMinWidthPx(gridTemplate, emPx))`（`src/lib/grid-min-width.js`：px／em、minmax 取下限、fr/auto 視為 0，加上列 gap 10 px 與左右內距 28 px；em 取 viewport 字級），因此即使頁面沒傳 `minContentWidth`（原本演出者頁就沒傳，窄視窗時列被夾在 100% 寬而裁掉欄位、無法水平捲動）也會溢出並可捲動，表頭由 `scrollLeft` 同步。可長按重排的列（播放佇列）使用 `touch-action: pan-x pan-y`，觸控可雙向捲動。`tests/grid-min-width.test.mjs` 為回歸測試。

長按拖曳重排（觸控）：播放佇列（`PagedVirtualList.svelte`）與設定「曲目欄位」（`App.svelte` 的 `.track-column-setting-row`）共用 `src/lib/long-press-reorder.js`。列保持 `touch-action: pan-x pan-y`（曲目欄位列另加 `-webkit-touch-callout: none`），快速滑動照常原生捲動；按住 `LONG_PRESS_REORDER_MS`（300 ms）且移動不超過 `LONG_PRESS_REORDER_SLOP_PX`（8 px，`exceedsLongPressSlop`）才進入拖曳，計時前超過門檻即取消長按、維持捲動。單靠 pointer 事件擋不住瀏覽器接手平移（會送 `pointercancel` 中斷拖曳），因此 `longPressDragGuard` action 掛非 passive 的 `touchmove`，只在拖曳中 `preventDefault`。`queue-reorder.js` 的長按常數改由此模組轉出。`tests/long-press-reorder.test.mjs` 為回歸測試。

觸控滑動關閉：共用 Svelte action `src/lib/swipe-dismiss.js`（`use:swipeDismiss={{ direction, enabled, onDismiss }}`，純函式有 `tests/swipe-dismiss.test.mjs`）。只處理 pointerType touch/pen，滑鼠行為不變；移動超過 10 px 後鎖定主軸，非關閉方向的軸交還瀏覽器（窄版導覽面板與快速設定 `touch-action: pan-y` 可垂直捲動，封面 `touch-action: none`）；從 input／select／textarea／`[role="slider"]`／`[data-swipe-ignore]` 開始的手勢一律忽略。元素以 CSS `translate` 屬性跟手（不與既有 transform 動畫衝突），放開時位移 ≥ 30% 尺寸或速度 ≥ 0.5 px/ms 即關閉，否則 180 ms 彈回；滑動後吞掉一次 click；`prefers-reduced-motion` 時不播動畫。綁定：窄版導覽面板左滑 → `closeNavPanel`，快速設定右滑 → `closeQuickSettings`，正在播放封面使用 `follow: false` 多方向 `actions`：封面完全不移動、不跟手，只在放開時依同樣的距離／速度門檻判斷——左滑（deltaX < 0）→ 下一首（`playback_next`），右滑 → 上一首（`playback_previous`）；此對應經使用者在手機實測確認，勿再對調，下滑 → `closeNowPlaying`。（更新：取代先前「封面不移動」規則——現加 `damped: true`，封面以橡皮筋曲線跟手，位移上限約 24 px、輕微縮放/透明度，放開或觸發後彈回；reduced-motion 時不動。）

Android APK：`npm run android:build:arm64`（debug、aarch64、`--apk --ci`），輸出 `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`，並複製為 `release/moemusicplayer-android-arm64-debug.apk`；安裝到 NX809J：`adb connect 192.168.1.10:5555` 後 `adb -s 192.168.1.10:5555 install -r <apk>`，不得重新啟動裝置。Android 專屬程式碼只在 aarch64-linux-android 目標編譯，Windows 建置不會發現其錯誤（例如佇列快速查找曾誤用 `PlaybackQueue` 私有欄位），變更共用 Rust 後須實際跑一次 APK 建置。2026-10-16 以 0420f68 建置 0.1.0 並成功安裝到 NX809J（未做裝置上手動驗收）。

Android 媒體通知：播放在 `plugins/tauri-plugin-media-index` 的 Media3 `PlaybackService`（MediaSessionService＋ExoPlayer，Rust 經 JNI 輪詢指令，佇列仍在 Rust；`QueueAwarePlayer` 依 Rust 送來的 `set_capabilities` 開放上一首／下一首，按鈕以 `transport` 事件回報 Rust）。載入指令已帶標題／演出者／專輯與縮小後的封面位元組（`setArtworkData`）。服務以 startService 啟動、App 內沒有 MediaController 連線，因此 `onCreate` 必須 `addSession(session)`，Media3 `DefaultMediaNotificationProvider` 才會建立通知頻道並發出 MediaStyle 通知（通知欄＋鎖定畫面：封面、標題、演出者、上一首／播放暫停／下一首），播放時升為 `mediaPlayback` 前景服務，stop（清空項目）後移除；`onDestroy` 先 `removeSession`，`onTaskRemoved` 在未播放時 `stopSelf`。Android 13+ 需 `POST_NOTIFICATIONS`（宣告在外掛 manifest），`MainActivity` 啟動時依 `NotificationPermissionPolicy` 每個行程最多詢問一次；拒絕只會讓通知不顯示，不影響播放。

線上歌詞自動套用（`player-core` `auto_lyric_candidate`，2026-10-17 使用者規則）：候選必須同時通過兩道門檻才算合格——標題正規化相似度 ≥ 0.88（`AUTO_MATCH_TITLE_MIN_SIMILARITY`，與「標題相符」說明同一門檻）且時長差 ≤ 2 秒（`AUTO_MATCH_MAX_DURATION_DIFF_MS`，任一方時長未知即不合格），並須為同步歌詞；加權分數（標題 0.55／演出者 0.25／時長 0.15／專輯 0.05）只用來排序，在合格者中取最高分，不再以分數 > 0.8 決定通過。手動選擇永遠優先，不會被自動結果覆寫。

歌詞簡轉繁（`src/lib/lyrics-traditional.js`，opencc-js STPhrases＋STCharacters＋TWVariants）遵守使用者規則：簡體「只」一律不轉「隻」。OpenCC 本身有詞組轉換，但採由左而右最長匹配分詞，量詞詞組（这只／一只／船只…）會吃掉後面「只有」的「只」（这只有 → 這隻有）；`buildKeepZhiOverrides` 由 STPhrases 推導覆寫表，把值中對應「只」位置的「隻」還原為「只」，其餘轉換不變（一只猫 → 一只貓）。opencc-js 以反序載入同組字典，先列者優先，因此覆寫表置於最前。覆寫建構器集中在 `src/lib/opencc-keep-zhi.js`，歌詞與搜尋變體（`search-variants.js` 的簡轉繁）共用；搜尋的繁轉簡不受影響（輸入「隻」仍產生「只」變體）。

左側導覽：品牌與搜尋列固定在上方，導覽項目（曲庫、演出者、專輯、播放佇列、播放清單群組）在 `.sidebar-scroll` 內垂直捲動，「設定」固定在底部不隨之捲動。播放清單群組恆為展開，沒有收合按鈕或狀態（先前的收合只存在元件記憶體，未持久化，無需遷移）。視窗寬度 ≤ 820 CSS px（沿用既有斷點，`src/lib/nav-panel.js` 的 `NAV_PANEL_MEDIA_QUERY`）時不再有圖示欄或頂部橫向導覽列：工作區佔滿寬度，頂欄左側顯示 MOE 標誌按鈕（`.nav-panel-toggle`）切換導覽面板；面板即同一個 `<aside class="sidebar">` 加上 `.floating-surface`（與快速設定相同的背景、模糊、陰影與 `--floating-surface-radius` 圓角），以 fixed 疊在內容上：上緣貼齊標題列下緣、左緣貼齊視窗、下緣停在底部播放列與時間軸之上（line 樣式時間軸在播放列內，偏移為 0；邊緣樣式可見的 3 px 線條置中於點擊帶、底緣落在播放列頂端，因此 `--nav-panel-dock-overhang` = `--dock-edge-progress-track`，面板下緣剛好停在線條頂端，不避開不可見的點擊帶），四角皆直角、保留陰影；Android safe-area 以 top/bottom 偏移與左側內距計入，開啟時 160 ms 淡入滑入（遵循 reduced-motion）。點擊外部（透明 scrim）、Esc、點選任何導覽項目或播放清單後關閉；放寬視窗或開啟正在播放時自動關閉；關閉時面板為 inert。Android 手機寬度都在斷點內，因此同樣使用標誌按鈕＋疊層面板，原本 ≤ 620 px 的頂部導覽列已移除。MOE 標誌按鈕左右間距共用 `--nav-toggle-gap`（14 px，自圖示可見像素起算，已扣除 PNG 左 73／右 21 px 透明邊依顯示尺寸換算的內縮）。同一斷點下頂欄「重新整理」只顯示圖示（36 px 方鈕），保留 aria-label「重新整理音樂來源」與 tooltip。Release CDP 寬／窄驗收尚待使用者播放器關閉後補做。

歌曲資訊面板（ADR 0013 Phase 1，Windows／Android）：播放頁頂欄「羅」與「快速設定」之間的資訊圖示按鈕開啟目前曲目的 `TrackDetailsDialog`；欄位暫存、按「儲存」（或 Ctrl+S）才經 `track_details_save` 寫檔（`crates/player-tags`：同目錄 `.moetmp-<檔名>` 暫存→lofty 寫入→重讀驗證→rename 取代，mtime 照常更新；MP3 一律 ID3v2.4；演出者純文字原樣寫入；歌詞一律寫內嵌（ID3 為 USLT），不建立 `.lrc`；「導入使用中歌詞」匯入播放器目前使用的歌詞（任何來源：線上、.lrc、手動選擇、內嵌），同步時間戳以 LRC 保留、翻譯／羅馬音以同時間戳的額外行輸出，無使用中歌詞或與欄位相同時停用，暫存至儲存；單張封面可取代／匯出／移除，封面 JPG 品質（預設 100）與自動縮放短邊（預設 2160 px）存在 localStorage `moe.coverEditOptions.v2`（舊 key 載入時刪除，一次性改回新預設）；封面按鈕：選擇封面、貼上封面（剪貼簿有圖片才出現，開啟面板與視窗取得焦點時檢查，暫存為 base64 `replaceCoverBase64`）、匯出封面、複製封面（轉 PNG 寫入剪貼簿）、移除封面，剪貼簿：Windows 用 WebView 的 navigator.clipboard；Android WebView 會拒絕以 `navigator.clipboard.read()` 讀圖片，WebView 的 `navigator.clipboard.write()` 寫圖片時雖然 resolve，系統剪貼簿卻收不到，所以讀寫都改走原生 ClipboardManager。
  - 偵測：`track_details_clipboard_has_image` 只讀 clip description，不會觸發「已貼上」提示。這只是提示用，因為 App 沒有焦點時 Android 會隱藏剪貼簿內容；Android 的「貼上」按鈕保持可按。
  - 貼上：`track_details_read_clipboard_image` 讀取 item URI 或 text/uri-list 裡的 content:// 或 file:// URI，以檔頭判斷 JPEG／PNG／WebP／GIF／BMP，複製到 cache `clipboard/` 後回傳位元組；讀不到時回傳原因（未在前景或沒有圖片）。
  - 複製：`track_details_copy_cover` 把內嵌封面寫到 cache `clipboard-share/`，再以 FileProvider（`${applicationId}.moe.updates`）的 content URI 加上確切的圖片 MIME 放上剪貼簿，其他 App 也能貼上。
  - 原生端的 log tag 是 `MoeMediaIndex`；面板以 portal 掛到 `.app-shell`（帶有 `--quick-settings-opacity`／`-blur` 等外觀變數，與快速設定同一 `.floating-surface` 樣式並即時跟隨設定；z-index 1000+），避免被播放列 dock 的疊層蓋住；面板為單一捲動區，由上而下：路徑列（不固定；頻譜圖、複製路徑、在資料夾中開啟）、格式｜標籤、時長｜位元率、取樣率｜位元深度、聲道｜修改時間、檔案大小｜建立時間、封面區（五個按鈕固定一列：選擇／匯出／複製／貼上／移除封面，Windows 剪貼簿沒有圖片時停用貼上，Android 一律可按）、標題…類型、錄音日期｜音軌號｜碟號、歌詞；欄位為標題／演出者／專輯／專輯演出者／作曲／類型各一列，錄音日期＋音軌號＋碟號同一列三等寬（minmax(0) 欄寬，不重疊），不顯示音軌總數／碟片總數（存檔時沿用檔案原值）；屬性區兩欄依序為 格式｜標籤、時長｜位元率、取樣率｜位元深度、聲道｜修改時間、檔案大小｜建立時間，時間顯示為 `YYYY-MM-DD HH:mm (UTC+8)`，Android 修改時間取自 provider、無建立時間；標籤格式用易讀名稱（Vorbis Comments、MP4 (iTunes ilst)、APEv2…）；DSF 唯讀）。存檔後呼叫增量 `syncLibrary` 更新投影。「頻譜圖」按鈕開獨立 Tauri 視窗（label `spectrogram`，同一 bundle 以 `?view=spectrogram&track=` 掛載 `SpectrogramView`，重複開啟時以 `spectrogram-track` 事件換曲並聚焦）；頻譜圖視窗不換播放曲目，但可「開啟 A／B」任意音訊檔（`spectrogram_pick_file`／`spectrogram_render_file`）或拖放開檔（拖兩檔載入 A、B；比較模式可拖到 A 或 B 區），A/B 比較模式有左右／上下並排、疊加（A／B 切換、空白鍵、B 透明度、差異圖）與可拖曳分割線，A/B 共用座標軸與設定、縮放平移同步、游標顯示 A／B dB 與差值，比較偏好存 `moe.spectrogramCompare`（`src/lib/spectrogram-compare.js`）；介面色彩用 app 主題變數（`--page`／`--panel`／`--accent`…，頻譜圖本身仍為黑底），可選配色（Spek／Magma／Inferno／Viridis／灰階）、FFT 大小、窗函數、線性／對數頻率軸、dB 範圍、聲道（混合／單聲道／分開）與最高頻率，設定存 localStorage `moe.spectrogramSettings`（`src/lib/spectrogram-settings.js`）；Android 改全螢幕頁。Android 後端：曲目為實體路徑（所有檔案存取權，ADR 0014），讀取／頻譜／`write_edit` 與 Windows 相同直接作用於檔案；殘留的 content:// 曲目經 `cacheContentUriNamed` 複製到 app cache `track-details/` 工作檔編輯，再以 `resolveDirectPath` 取得的實體路徑原地寫回、重讀逐位元組比對，失敗則推回原檔；不再有 createWriteRequest／RecoverableSecurityException 逐檔授權，取不到實體路徑時提示開啟「所有檔案存取權」；封面選擇／開啟音訊用 `pickOpenDocument`，封面匯出用 `pickCreateDocument`；Android 的路徑列按鈕統一顯示「在資料夾中開啟」：`track_details_reveal` 以 ExternalStorageProvider 的資料夾 document URI 加上 MIME_TYPE_DIR 用 ACTION_VIEW 開啟 DocumentsUI，依序嘗試：不指定 App、Google DocumentsUI、AOSP DocumentsUI、resource/folder、無類型。不能加 FLAG_GRANT_READ_URI_PERMISSION：App 對該 provider 沒有權限，加了 startActivity 會丟 SecurityException。每種形式都直接 startActivity 並 catch，不先用 resolveActivity 檢查；都沒有 App 接手時回報錯誤，提示改用「複製路徑」。Android WebView 停用頁面縮放（viewport `user-scalable=no` 加 `MainActivity` 的 `setSupportZoom(false)`）。Opus／DSF 無頻譜（symphonia 不支援）。Release GUI 驗收待做。

Android 與 Windows 對齊（2026-10-17）：
- 本機歌詞候選：`lyrics_service` 的 `android_find_all_lyrics` 依序讀同名 `.lrc` 與內嵌歌詞，與 Windows `find_all_lyrics` 相同；播放查找只取第一個結果（sidecar 優先），候選清單與「套用本機歌詞」列出全部。
- 排除資料夾：Android 用 `pickDirectoryPath` 選擇實際路徑，存進 `settings.json` 的 `excluded_folders`（與 Windows 同一欄位）。`fs_scan::scan_directory` 掃描時略過排除路徑及其子資料夾（`with_adapter_excluding`），完整掃描會移除其中的曲目。排除資料夾不能包含來源根目錄，來源也不能加在排除路徑內。
- 更新：Android arm64 從同一個 GitHub Release 讀取 `moemusicplayer-android-update.json` 與 APK，驗證 SHA-256 後交給系統安裝程式（ADR 0015）。發布腳本為 `npm run release:android`，只有加上 `-Upload -ReleaseTag` 才會上傳。
  - Android 封面剪貼簿讀取與複製已由使用者確認可運作；「在資料夾中開啟」、排除資料夾、本機歌詞候選及 APK 更新仍待裝置驗收（目前 Release 尚無 Android 資產）。

正在播放封面縮放（`src/lib/cover-zoom.js`，action `coverZoom` 掛在 `.cover-stage`，`tests/cover-zoom.test.mjs`）：桌面游標在封面上用滾輪縮放（以游標為錨點），Android 雙指縮放；範圍 0.4–4 倍（1 為原大小，小於 1 時封面原地縮小，放大才可平移），雙擊（觸控為 300 ms 內雙點）恢復原大小，放大後可平移（滑鼠拖曳；觸控用雙指一起移動，單指留給滑動手勢；位移夾在放大後溢出量內）。縮放只對封面本身套 `transform: translate scale`（原點置中），歌詞區域不跟著放大；佈局以 `--np-cover-width`／`--np-cover-height` × `--np-cover-zoom`（掛在 `.now-playing-view`）預留放大後封面的欄寬（窄版為列高），所以歌詞區域被封面擠壓、範圍變小，封面溢出的部分由卡片裁切。縮放中的封面取消 `max-width/max-height: 100%`（否則欄位／列縮小時只夾住一邊，封面被裁成長方形）。縮放期間封面基準尺寸（`coverFrame`）凍結，回到 1 倍才重算（否則欄位被縮放改變大小後封面又依欄位重算，縮小會縮到消失、放大會無限增長）。縮小時封面欄位的 fr 份額（`--np-art-col-a`／`--np-art-col-b`／`--np-art-row`，由 `coverShrinkShare` 在 0.4–1 之間線性淡出到 0）收斂成封面大小，歌詞份額（`--np-lyr-col-a`／`--np-lyr-col-b`／`--np-lyr-row`）同量增加（兩軌 fr 總和固定為 2，避免總和小於 1 時格線只用部分空間），歌詞區域隨之擴展；垂直堆疊（窄版）時歌詞列同樣受封面擠壓／展開，歌詞視圖高度改為填滿該列。縮放狀態下封面的左右滑切歌／下滑關閉照常可用（單指觸控永遠是滑動手勢），第二指按下時以合成 pointercancel 取消第一指的滑動（縮放自己的 up 以 `dispatchingCancel` 忽略這個合成事件，否則雙指狀態會被清掉而無法縮放）；關閉正在播放頁或切換曲目／封面（`resetKey`）時立即還原為 1 倍並讓基準尺寸重算。Release 與手機實測尚待使用者回報。

接續播放（Windows／Android）：`settings.json` 的 `resumePlayback`（預設 false）。正常關閉或更新重啟時若音訊正在播放，`set_resume_playback(true)` 寫入（更新流程在暫停前就記錄，失敗中止時改回 false）；下次啟動成功還原播放 session 後（狀態 `ready`）`take_resume_playback` 重設為 false 並 `request_play`。播放中被強制結束或當機不會寫入，因此不會自動播放。Android：播放 actor（`android_playback.rs`）在播放／暫停狀態改變時透過 `ResumeHook.sink` 即時寫入同一欄位（行程被殺也保留），啟動時 `restore_session` 若欄位為 true 就以 `play_when_ready` 載入並續播；`AppState.settings` 為 `Arc<SettingsStore>` 以便共享。Release 與裝置實測待使用者確認。

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
- 修改程式碼、資源或建置設定時，先完成測試並提交來源，再執行一次 `npm run release:windows`，避免執行檔內嵌舊 commit；純文件修改不建置。固定 Windows 執行檔為 `release/moemusicplayer.exe`，helper 與之同層。播放器執行中也照樣執行 `npm run release:windows`：腳本會在映像被占用時先把舊 exe 改名再放入新 exe（背景替換，含回滾與 SHA-256 驗證），不需要也不得關閉播放器；新版在下次啟動生效。
- Bug／功能驗收一律啟動固定路徑的 Release 執行檔，以該程序的 WebView2 CDP 檢查實際 DOM、IPC 或錯誤；不得改用 Chrome、Playwright 或無資料隔離副本。完成後關閉本次診斷程序並確認偵錯埠已關閉。CDP 啟動方式：先執行 `netsh int ipv4 show excludedportrange protocol=tcp` 確認埠不在 Windows 保留範圍（例如 9326–9425 會讓 9337 無法監聽），預設使用 9222；設定 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` 後啟動 `release/moemusicplayer.exe`，再以 `netstat -ano | findstr :9222` 確認 LISTENING，並以 `http://127.0.0.1:9222/json/version` 確認有回應後才開始驗收。建置／單元測試不等於實際 App 驗收。Android 必須在 Android 目標環境驗收。
- 不得強制終止使用中的播放器；Windows 新版按既定背景替換程序與備份規則更新，絕不觸碰 `UserData/`。不得未經同意重新啟動裝置。
- 不以 Python sqlite3 讀寫使用中的正式 SQLite；先確認播放器關閉，或使用已驗證的唯讀／備份流程。
- 依變更範圍執行 `npm run check`、`node --test`、相關 Rust tests、workspace Clippy 與 rustfmt；完成工作後更新本文件「目前工作狀態」，只保留仍有效的狀態與下一步，不記錄歷史流水帳。

## 按需閱讀的領域規則

- UI、列表互動、播放頁、歌詞、背景及視覺 token：[`src/lib/AGENTS.md`](src/lib/AGENTS.md)
- Tauri IPC、使用者資料、播放生命週期及更新流程：[`src-tauri/AGENTS.md`](src-tauri/AGENTS.md)
- Rust Core、SQLite、排序／分組、跨平台資料與音訊邊界：[`crates/AGENTS.md`](crates/AGENTS.md)
- 產品詞彙：[`CONTEXT.md`](CONTEXT.md)；架構決策：[`docs/adr/`](docs/adr/)
