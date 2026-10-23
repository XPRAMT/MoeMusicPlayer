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

- 本文件是「目前狀態」快照，不是日誌，全文最多 50,000 字元（`tests/agents-md-size.test.mjs` 檢查）。
- 更新時就地改寫該功能既有的條目，刪掉被取代的內容；不得另起新段落追加。
- 不寫日期、量測數字或除錯經過；這些放在 commit message 或 ADR。
- 每個功能只有一個位置。
- 設計細節（演算法、參數、門檻、理由、替代方案、量測）放 `docs/adr/`，這裡只留目前行為、硬性規則、關鍵名稱、待辦／未驗證項目與「見 ADR NNNN」。
- 每次發布前整理本文件（見「工作與驗收規則」）。

## 目前工作狀態

### 歌詞視窗（工作列歌詞、浮動歌詞）

原生繪製（ADR 0034）：Windows 工作列歌詞與浮動歌詞由 `crates/player-lyrics-native` 繪製（Windows.UI.Composition＋D2D／DWrite，單一 `moe-lyrics-native` 執行緒；`plan.rs`、`sampling.rs`、`win.rs`、`shared_cache.rs`），沒有 WebView 歌詞視窗；需要 Windows 10 1809 以上，更舊的系統不建立歌詞視窗。每行點陣化一次並跨視窗共用，每個合成幀只提交有變的值，閒置時不輪詢；播放時鐘取自音訊引擎（`position_at`）。規則在 `crates/player-lyrics-rules`，與 JS（`lyrics-strip.js` 等，參考實作）、Kotlin（`FloatingLyricsRules.kt`）以 `tests/fixtures/lyrics-rules-vectors.json` 共用向量（`node scripts/generate-lyrics-rules-vectors.mjs` 重產）。資料流：主視窗廣播 `taskbar-lyrics-data`／`taskbar-lyrics-playback`／`lyrics-inactive-opacity`／`lyrics-native-delay`／`floating-lyrics-tips`，`src-tauri/src/native_lyrics.rs` 接收，需要時以 `taskbar-lyrics-request`／`taskbar-lyrics-playback-request` 要求重送；WM_CLOSE 轉成主視窗正常關閉。診斷：`MOE_LYRICS_NATIVE_LOG`、`MOE_LYRICS_FRAME_LOG`、`MOE_LYRICS_NO_FADE`，`--example measure`／`--example demo`。待辦：使用者實機驗收（外觀、鎖頭 tooltip、固定模式穿透、拖曳存檔、自動隱藏／全螢幕／Explorer 重啟、熱插拔、DPI、裝置遺失重建）；歌詞開啟時其他執行緒額外 CPU 的來源待查。

工作列歌詞（ADR 0018、0031，僅 Windows，預設啟用）：設定 `settings.json` 的 `taskbarLyrics`（`settings_get/set_taskbar_lyrics`，Rust `TaskbarLyricsSettings`，前端 `taskbar-lyrics-settings.js`，變更經 `taskbar-lyrics-settings` 事件廣播）。`src-tauri/src/taskbar_lyrics.rs` 的監看執行緒為每條工作列（`taskbar::list_taskbars`）各建一個視窗、擁有者為該工作列，以 UI Automation 算空白處擺位（純函式 `free_stretches`／`pick_free_area`／`lyrics_area`）；自動隱藏、直式工作列、空白太窄、全螢幕／簡報時隱藏（`foreground_covers_monitor_of`）。`displayMode`：`all`（預設）／`playerMonitor`／`custom`（`customMonitors`，按鈕來自 `taskbar_lyrics_monitors`）。只顯示同步歌詞，過寬的行依播放位置水平捲動（`getLyricScrollProgress`）。診斷 `cargo run -p player-platform-windows --example probe_taskbar`。待驗收：第二螢幕位置與字級、「識別」編號是否與按鈕一致、熱插拔、Explorer 重啟、playerMonitor 切換、Win10 工作列。

浮動歌詞（ADR 0032；Windows 為原生置頂透明視窗，Android 為系統懸浮窗）：`settings.floatingLyrics`（`FloatingLyricsSettings`，外觀欄位同 `taskbarLyrics`，另有 `enabled` 預設關、`locked`、`position`），可與工作列歌詞同時開啟。Windows `src-tauri/src/floating_lyrics.rs`（`settings_get/set_floating_lyrics`、`floating_lyrics_set_locked`、`floating_lyrics_reset_position`、`floating_lyrics_fit`、`floating_lyrics_nudge`、`floating_lyrics_centre_axis`）；只有歌詞區必須留在工作區內；固定模式整窗穿透，只有游標在解鎖按鈕上時接收滑鼠。`widthPx` 是**歌詞區**寬，面板＝左右透明邊＋歌詞區＋右側鎖頭欄；鎖頭圖示表示目前狀態（開鎖＝可移動、閉鎖＝鎖定）。設定頁「面板」：Android 獨有「移動到狀態列」是一次性移動（`centredOnBar`，依方向儲存），不是模式；十字鍵、水平置中／垂直置中／重設位置、寬度滑桿。Android：`FloatingLyricsOverlay.kt`，頁面經 `floating_lyrics_android` 送 `floatingLyricsUpdate`，鎖定與位置以 SharedPreferences `moe_floating_lyrics` 為準；懸浮窗權限只能由使用者在系統設定頁開啟（禁止 adb 授權）。限制：只顯示有時間軸的歌詞；WebView 暫停時換歌，新歌詞要等頁面恢復才送到。待使用者實機驗收。

連續動畫與透明度（兩種歌詞視窗共用，`animation`：`none` 預設／`continuous`，工作列與浮動分開儲存）：所有歌詞接成一條，目前行左緣在該行開始時對齊歌詞區中心並以單調 C1 曲線滑向下一行（`lyricsStripCubic`／Rust `strip_cubic`／Kotlin `stripOffset`）；只繪製會出現的行（`lyricsStripWindow`／`FloatingLyricsRules.stripWindow`），不可把整首歌放進捲動條。其餘可見行乘「非目前歌詞透明度」（`lyricsPreferences.inactiveOpacityPercent`；動畫「無」不套用），換行前 200 ms 過渡（`lyricsStripFocus`／`lyricsLineWeight`／`lyricsLineOpacityFactor`）；歌詞區左右邊緣淡出（`edge_fade_width`／`edge_fade_opacity`）。Android 連續模式的陰影待平板實機驗收。工作列與浮動歌詞同時開連續動畫有性能負擔，建議一次只開一種。

動畫設計規則（使用者定案，所有動畫適用）：（1）歌詞動畫必須跑滿螢幕更新率（120 Hz＝120 fps、60 Hz＝60 fps），所有歌詞視窗與主視窗同時動畫時也一樣，這是驗收標準；不得改成 60 fps 滑動。（2）CSS 與 Android 的連續位置動畫只在移動至少 1 個實體像素時才更新，落後不到 1 px、每個目標精確到達；Windows 原生歌詞例外，以 1/8 px 次像素位移（見上）；播放進度條與底欄／聲道矩陣跑馬燈（`dock-marquee.ts`）以 `linear` 次像素移動（測試 `tests/linear-position-animations.test.mjs`）。透明度淡化不受此限。（3）每幀 CPU 越低越好：每幀只依時鐘算位置並提交有變的值，不在每幀點陣化、排版或模糊，能快取就快取，閒置時睡眠。時間規則（滑動對齊、200 ms 淡化、換行提前）不變。

### 播放頁與歌詞

歌詞設定三分頁（一般／工作列歌詞〔僅 Windows〕／浮動歌詞，最後分頁存 localStorage `moe.settings.lyricsTab`）都經 `src/lib/LyricsLookSettings.svelte` 畫出同樣分組：「顯示」（啟用 → 顯示譯文 → 該頁專屬開關 → 動畫）、「字體與大小」（字體大小 → 譯文大小（相對原文） → 句間距）、「外觀」（文字效果 → 自訂顏色 → 透明度 → 非目前歌詞透明度 → 對齊／文字排版）、「位置與面板」；專屬設定以 snippet 放入對應分組，標題用 `.quick-settings-group h3`；前端純函式在 `src/lib/floating-lyrics-settings.js`。一般頁譯文與羅馬拼音大小是相對原文百分比 `lyricsPreferences.auxiliaryScalePercent`（1–100，預設 90），舊檔的 `auxiliaryFontSizePx` 由 Rust（`RawLyricsPreferences`）與 `normalizeLyricsPreferences` 換算為 round(譯文 px ÷ 原文 px × 100) 並不再寫回；播放頁以 `lyricsAuxiliaryFontSizePx` 繪製。字體大小 1–50 px，舊設定檔的 0 載入為 1。

歌詞只有一份：`App.svelte` 的歌詞控制器載入一次（播放頁開過或歌詞視窗啟用時才跑），播放頁 `LyricsView` 以 `sharedController`／`sharedState` 顯示，同一份（已套用後端繁體行、只含同步行、附偏移）經 `taskbar-lyrics-data`（`src/lib/taskbar-lyrics-data.js`）推給歌詞視窗；歌詞視窗不搜尋、不載入、不轉換。偏移計算只有 `lyrics-timing-offset.js` 的 `lyricsDisplayOffsetMs(lyrics.offsetMs, 每首延遲秒)`；每首延遲存 localStorage，調整時 `emit('lyrics-timing-offset-changed', { trackId, delaySec })`，App 轉成 `lyrics-native-delay`（Android 為 `floatingLyricsUpdate`）。歌詞視窗沒有換行提前，在 `startMs - offset` 換行。

播放頁同步歌詞（ADR 0029）：虛擬視窗依捲動位置繪製（`lyrics-window.js` 的 `getTimedLyricWindow`、`sameLyricWindow`）。換行動畫提前開始、在該句時間點剛好結束，提前量只定義在 `src/lib/lyrics-cue-timing.ts` 的 `LYRIC_CUE_LEAD_MS`（不開放設定）；`activeTimedIndex`（`data-current-cue-index`）是真正的當前句，畫面用 `displayTimedIndex`（`data-active-cue-index`）。所有捲動變化經 `lyrics-scroll-glide.ts` 的 `planTimedScroll`；跳轉、換歌、版面改變與 `prefers-reduced-motion` 一律 instant。播放次數、上一首 3 秒判斷用原始時間。測試：`tests/lyrics-cue-timing.test.mjs`、`tests/lyrics-scroll-glide.test.mjs`、`tests/lyrics-controller.test.mjs`、`tests/lyrics-view.playwright.js`（需另開測試伺服器）。Release 實機未驗收。

播放進度時鐘（ADR 0028）：主視窗每 500 ms 取播放快照（`App.svelte` 的 `PLAYBACK_POLL_MS`，最小化時照跑），之間由 `src/lib/playback-clock.ts` 推算並平滑校正，大偏差、換歌或狀態改變時跳轉（`jumpId` +1）。進度條以 `@property --progress-ratio` 的 linear CSS 動畫走動，只在 `jumpId`／狀態改變或拖動結束時重設（`PlaybackProgress.svelte`），不逐幀更新。需要即時位置處用 `currentPlaybackPositionMs()`。後端事件推送尚未做。測試 `tests/playback-clock.test.mjs`。

跳轉與焦點：前端所有 `playback_seek` 只能經 `src/lib/ipc.ts` 的 `seekPlayback(positionMs, durationMs?)`（`seek-position.js` 的 `normalizeSeekPositionMs` 四捨五入為整數毫秒並夾在 0..曲長；Rust 維持 `u64` 契約；`tests/seek-position.test.mjs` 檢查無其他直接呼叫）。`shortcutEventShouldRun` 對 `shortcutTargetIsSeekBar` 一律放行；未綁定的方向鍵／PageUp／PageDown 由 `handleSeekKeydown` 依 `playbackSeekKeyStepMs`（±5 s／±30 s）從播放時鐘跳轉，按住重複只 `preventDefault`；Home／End 保持原生；音量條等其他滑桿照原生。點擊不留焦點（全 App 規則）：`src/main.ts` 呼叫一次 `installPointerFocusRelease(document)`（`src/lib/pointer-focus.js`）：滑鼠／觸控按下按鈕、range、checkbox／radio（含對應 role）後，pointerup 之後的下一個 task 移除焦點；`select` 於 `change` 後移除；文字欄位不套用；點擊處理已移走焦點就不動；modal 內焦點交給 modal 容器；鍵盤取得的焦點不處理，外框用 `:focus:not(:focus-visible)`。新控制項不需另外處理。

播放鍵與 seek 期間（ADR 0036）：Android 緩衝中回報 `"buffering"`（`PlaybackService.kt` 的 `playbackStateName`、Rust `android_playback.rs` 的 `counts_as_playing`），播放鍵不閃成暫停，收聽計時與播放時鐘只在 `playing` 前進。seek 走 `sendPlaybackCommand(..., { seek: true })`（`isSeeking`），忙碌時忽略點擊但外觀不變；上一首／下一首不因指令忙碌反灰。測試：`tests/seek-transport-lock.test.mjs`、`PlaybackProtocolStateTest`（`gradlew :tauri-plugin-media-index:testDebugUnitTest`）、`android_playback::integration_tests`。

播放頁切歌封面：大封面用 `nowPlayingArtworkObjectUrl`（`src/lib/active-track-artwork.ts`）：`ready`，或新封面 `loading` 而控制器保留上一首 object URL 時繼續顯示舊圖並維持有封面版面；新封面 `decode()`（`createDecodedImageObjectUrl`）後才替換；`missing`、過大或失敗才隱藏、歌詞置中；generation 序號丟棄舊結果。底欄小封面不受影響。測試 `tests/active-track-artwork.test.mjs`。專輯首頁卡片封面、專輯詳情封面及底欄封面皆直角；底欄封面在所有頁面使用同一個全域樣式。

正在播放封面縮放（ADR 0036，`src/lib/cover-zoom.js` 的 `coverZoom`，`tests/cover-zoom.test.mjs`）：滾輪或雙指縮放 0.5–10 倍、雙擊還原、放大後可平移；只對封面套 transform，版面依倍率預留欄寬並擠壓歌詞區。縮放中的封面以 `data-cover-zoomed` 屬性標記（不用 class）。直排／橫排只看長寬比例（`@media (orientation: portrait)`）。縮小的封面切歌後維持、放大的切歌即還原（`coverZoomPersisted`）；跨重啟只記縮小（`settings.json` 的 `coverZoom.scale`，`settings_get/set_cover_zoom`）。Release 與手機實測待回報。

手勢（ADR 0036）：`src/lib/swipe-dismiss.js`（`use:swipeDismiss`，`tests/swipe-dismiss.test.mjs`）。窄版導覽面板左滑 → `closeNavPanel`，快速設定右滑 → `closeQuickSettings`；正在播放封面與歌詞區左滑（deltaX < 0）→ 下一首、右滑 → 上一首（使用者實測確認，勿對調），封面下滑 → `closeNowPlaying`；滑鼠（`mouseActions`）在 `.now-playing-view` 水平拖曳切歌、`.cover-stage` 向下拖曳收合；`data-cover-zoomed` 時滑鼠拖曳是平移。從 input／select／textarea／`[role="slider"]`／`[data-swipe-ignore]` 開始的手勢一律忽略。

線上歌詞自動套用（`player-core` `auto_lyric_candidate`）：候選須同時標題正規化相似度 ≥ 0.88（`AUTO_MATCH_TITLE_MIN_SIMILARITY`）、時長差 ≤ 2 秒（`AUTO_MATCH_MAX_DURATION_DIFF_MS`，任一方未知即不合格）且為同步歌詞；加權分數（標題 0.55／演出者 0.25／時長 0.15／專輯 0.05）只用於在合格者中排序。手動選擇永遠優先。

本機歌詞選擇：選「同名 LRC」`local:sidecar` 或「內嵌歌詞」`local:embedded` 時，`track_lyrics` 手動列只存來源參照，每次讀取取當下內容（同步存的 `track_embedded_lyrics`／旁檔，沒有才讀檔），該來源為空就照 LRC → 內嵌 → 自動快取往下找；舊版本機手動列的文字快照一律忽略（`lyrics_service.rs` 的 `manual_lyrics_dto`／`live_local_pick`）；線上（網易雲／QQ）手動選擇仍用存下的文字。Windows `find_all_lyrics` 與 Android `android_find_all_lyrics` 都依序讀同名 `.lrc` 與內嵌歌詞（播放取第一個，候選清單列出全部）。

簡繁轉換（ADR 0030）全專案只有一份：`crates/player-zh`，OpenCC 上游 ver.1.4.2 原檔放在 `crates/player-zh/data/opencc-1.4.2/`（`SOURCE.md` 記版本與 SHA-256，`.gitattributes` 設 `-text`），`include_str!` 內嵌。只換字形不換用詞：`to_traditional`＝STPhrases＋STCharacters（含 keep-只 覆寫）→ TWVariantsPhrases＋TWVariants；`to_simplified`＝TWVariantsRevPhrases＋TWVariantsRev → TSPhrases＋TSCharacters；含日文假名的整行或欄位不轉。使用者規則：簡體「只」一律不轉「隻」（由 STPhrases 推導覆寫表還原）。用途：歌詞顯示（`LyricLineDto` 的 `textTraditional`／`translationTraditional`，前端 `lyricsForDisplay` 同步挑選）；線上歌詞一律只用簡體搜（`lyrics_search` 自行轉簡，前端不送 `queryAlternates`）；曲庫搜尋由 player-db `search_terms_json` 加 `search_alternates`；身份鍵 `music_identity_key` 用 `to_traditional`（只在每連線的 TEMP 表）；候選評分 `fold_to_simplified`。`opencc-js` 只剩 devDependency（`scripts/i18n-catalog.mjs` 的 twp→cn、`scripts/generate-zh-convert-fixture.mjs` 重產 `tests/fixtures/zh-convert-opencc-js.json`；差異只能是 `tests/fixtures/zh-convert-differences.json` 記錄的項目）。前端不得再引入任何簡繁轉換。

### 介面

介面語言（ADR 0019）：設定 → 主介面最下面「語言」（跟隨系統／繁體中文／简体中文／日本語／English，`settings.json` 的 `language`，`settings_get/set_language`，切換後存 localStorage 並重新載入，其他視窗收 `app-language-changed`）。前端文字寫成 `tr('繁體原文')`；`src/lib/i18n/zh-CN.json` 由 `node scripts/i18n-catalog.mjs`（OpenCC twp→cn＋`zh-CN.terms.json`＋`zh-CN.overrides.json`）產生；`ja.json`、`en.json` 手寫（`tests/i18n-catalog.test.mjs`，`--missing` 列出缺句）。跟隨系統取 `navigator.languages` 第一個支援的語言，否則英文。Rust 後端訊息仍是繁體（使用者決定先不改）。分組標籤 `漢字` 不翻譯；後端「未知」與「本地」經 `displayUnknown()`（`src/lib/unknown-label.js`）顯示。

左側導覽：品牌與搜尋列固定在上，導覽項目在 `.sidebar-scroll` 內捲動，「設定」固定在底部；播放清單群組恆為展開。視窗寬 ≤ 820 CSS px（`src/lib/nav-panel.js` 的 `NAV_PANEL_MEDIA_QUERY`，Android 手機都在內）時工作區佔滿寬度，頂欄左側 MOE 標誌按鈕（`.nav-panel-toggle`，左右間距 `--nav-toggle-gap` 14 px，自圖示可見像素起算）切換導覽面板：同一個 `<aside class="sidebar">` 加 `.floating-surface`，fixed 疊在內容上，上緣貼標題列、左緣貼視窗、下緣停在播放列時間軸線條頂端（`--nav-panel-dock-overhang`＝`--dock-edge-progress-track`；點擊帶 `--dock-edge-progress-hit` 20 px，排版用 `--dock-edge-progress-row` 10 px），四角直角、保留陰影，160 ms 淡入（遵循 reduced-motion）；點外部、Esc、選任何項目後關閉，放寬視窗或開啟播放頁時自動關閉，關閉時 inert。同一斷點下頂欄「重新整理」只顯示圖示。Release CDP 寬／窄驗收尚待補做。

浮動面板外觀：快速設定、窄版導覽面板（`.sidebar.floating-surface`）、睡眠定時等面板只透過 `.floating-surface` 取得背景（`--text-inverse` 依 `--quick-settings-opacity` 混合）、`--quick-settings-blur`、陰影與圓角（變數掛在 `.app-shell`）；以 `.sidebar` 為主體設定 background／backdrop-filter 的規則必須加 `:not(.floating-surface)`，各面板自身不可再設背景（`tests/floating-surface.test.mjs`）。

睡眠定時（ADR 0036）：播放頁頂欄「羅」與歌曲資訊（ⓘ）之間的鬧鐘按鈕開啟 `.floating-surface` 面板；小時、分鐘為兩個 `role="slider"` 欄位，各自夾住不進位（`adjustSleepTimerField`，上限 23:59，與 Rust `MAX_SLEEP_TIMER_MS` 同步），以手勢為單位凍結快照（`beginSleepTimerEdit`／`stepSleepTimerEdit`）。期限由 `src-tauri/src/sleep_timer.rs` 持有，到期送 `sleep-timer-changed` 並暫停（不是停止）；前端 `src/lib/sleep-timer.js` 只做顯示與換算。

專輯詳情捲動（ADR 0012）：曲目捲動視窗固定從「封面收合後的標題高度＋欄標題高度」開始，位置與大小不變；內容前有等於收合行程（桌面 120、窄版 80 CSS px）的空白，收合量恆為 `min(scrollTop, 收合行程)`（封面桌面 200、窄版 160 → 80 CSS px）。收合是純合成器 transform，由列表原生 scroll timeline 驅動（封面以左上角 `scale()`，摘要與欄標題 `translate()`，timeline 動畫明確指定 `1s`，`animation-duration:auto` 在 WebView2 會停在最小封面）；不支援 scroll timeline 時才由 scroll 事件寫 `--album-detail-collapse-progress`。規則：封面／標題／排序表頭的收合只能動 `transform`，不得動畫 `width`、`height`、`top`、`flex-basis`，也不得每次 scroll 寫 inline style；不做逐列 clip-path、不攔截滾輪。改 transform 後尚未在真實 WebView2 搭配連續歌詞重驗。

專輯詳情資料（ADR 0037）：`AlbumDetail.svelte` 的 `loadDetailData` 讓 facets 與 artists 各自取得、不等封面；封面先用卡片的第一張，再背景換成拼貼（`album_artwork_selection_ensure` 的 `representative`）。「依專輯曲目編號排序」：有碟號 ≥ 2 才分碟（`list_album_tracks_page_with_index_sort` 與 `album_detail_facets` 同一規則）。專輯頁點歌用 `PlaybackQueueSource` 的 `album` 來源，佇列與畫面只有一個排序來源（Rust `queue_for_track`、`Database::list_album_track_ids`，Windows 與 Android 相同）。測試 `album_queue_*`。

專輯首頁（ADR 0037）：卡片依 `trackListColumns.albumCardMaxWidthPx` 填滿整列（`album-index-layout.js` 的 `getAlbumCardGrid`）。封面預載目前視窗前後各一頁（`album-cover-prefetch.js` 的 `getAlbumPrefetchIndices`、`album-card-cover.ts` 的 `createAlbumCoverPrefetcher`），與卡片共用 `loadAlbumCardCover`；限流器 `withAlbumCoverLookupSlot`／`withAlbumArtworkSlot` 讓可見卡片永遠先於預載，避免大量第一次封面查詢佔住唯一的資料庫連線鎖。測試 `tests/album-cover-prefetch.test.mjs`、`tests/album-cover-lookup-slot.test.mjs`。診斷：命令 `db_slow_locks`（`player-db` 的 `TimedConnection`）。Release／手機捲動與多視窗寬度驗收待做。

列表（ADR 0037）：`PagedVirtualList` 列寬為 `max(100%, --track-list-min-width)`（`src/lib/grid-min-width.js`）；捲動視窗與 `AlbumIndex` 卡片格一律 `overscroll-behavior: none`，不可改回 `contain`。長按重排（播放佇列、設定「曲目欄位」）共用 `src/lib/long-press-reorder.js`（`longPressDragGuard`）。快速查找面板欄數由 `quick-index.js` 計算（寬度優先）。欄位標題首次點擊：時間類、長度、播放次數、音訊格式先降冪，其餘先升冪（`track-columns.js` 的 `getFirstSortDirection`）。未來最佳化：快速拖曳捲軸時虛擬列表可能短暫空白（候選做法見 ADR 0037）。

曲目欄位（ADR 0037）：十欄（標題、演出者、專輯、發行時間、新增時間、修改時間、長度、音訊格式、播放次數、來源）。預設可見度依平台（Windows 勾「新增時間」、Android 勾「修改時間」）：Rust `default_created_column_visible`／`default_modified_column_visible` 與前端 `defaultColumnVisible` 必須一致。「來源」本機為「本地」、遠端為伺服器名稱（`source_labels` 表、`TRACK_SOURCE_NAME_SQL`），預設隱藏。所有 `TrackSummary` 由 `attach_file_names` 一併補上檔名、來源名稱與修改時間。曲庫預設依新增時間排序。

歌曲來源篩選開關（ADR 0037）：頂欄三段開關（`SourceScopeSwitch.svelte`、`source-scope.js`），只有同時有已啟用本地與遠端來源時顯示，選擇存 `librarySourceScope`。`player-db` 的 SQL 函式 `moe_in_scope(source_kind, locator_kind)` 套在所有可見性判斷，播放解析、歌詞與指紋查詢不套用；播放佇列保留完整順序、只篩頁面（`build_playback_queue_page`、`queue_rows_to_positions`）。測試 `the_source_scope_*`、`tests/source-scope.test.mjs`。

同步結果顯示（設定 → 音樂來源頂部橫幅）：每個來源一行「來源名　同步完成；新增 n 個項目，移除 n 個項目。」，有項目錯誤才加「，n 個項目錯誤」（`sync-result-line.js` 的 `formatSyncResultLine`，n 取自 `library-sync-finished` 的 `addedOrUpdated`／`removedMappings`）；進度條只在有已知總數的階段出現；只有失敗或沒有可同步來源時才顯示說明。

頻譜圖（ADR 0038）：獨立 Tauri 視窗（label `spectrogram`，`SpectrogramView`；Android 為全螢幕頁），可 A／B 比較（`src/lib/spectrogram-compare.js`），設定存 `moe.spectrogramSettings.v2`（`src/lib/spectrogram-settings.js`）。「Spek」配色是依其色階重新實作（`spek_color`），不得複製 Spek 的 GPL 程式碼；色階上限固定 0 dB。DSF／WVDSD 經 `player-tags` 的 `dsd_input` 轉 352.8 kHz 分析；DFF 與 Android 的 Opus 不支援。Release GUI 驗收待做。

### 歌曲資訊面板（ADR 0013）

面板：播放頁頂欄資訊按鈕開啟 `TrackDetailsDialog`（portal 掛到 `.app-shell`）。欄位暫存，「儲存」（或 Ctrl+S）才經 `track_details_save` 寫檔（`crates/player-tags`：同目錄 `.moetmp-<檔名>` 暫存→寫入→重讀驗證→rename；MP3 一律 ID3v2.4；歌詞一律寫內嵌、不建 `.lrc`）；歌詞欄有改（`lyricsChanged`）時同一次儲存把手動列設為內嵌參照（`link_embedded_lyrics`）。位元率全 App 只有一套 `formatBitrateKbps`（`track-details-format.js`）。

封面：`player_tags::cover` 的 `plan_cover`／`prepare_cover`（各容器相同）：一律 JPG，真的有透明像素時改 PNG；「儲存時：…」由 `track_details_cover_plan` 顯示同一份計畫。複製→貼上保留原檔（`src/lib/cover-clipboard.js`）。Android 剪貼簿讀寫都走原生 ClipboardManager（`track_details_clipboard_has_image`／`track_details_read_clipboard_image`／`track_details_copy_cover`），WebView 的 clipboard API 不可用。

儲存後：`reloadAfterTrackDetailsSaved(trackId)` 只重讀這個檔案的標籤（`Database::refresh_track_metadata`），只清該專輯的封面選擇（`album-artwork-invalidated`），再 `playback_refresh_current_track` 並廣播 `current-track-refreshed`；不跑整個曲庫同步。Android 通知欄不跟著更新（下次切歌才更新）。

平台：Android 曲目為實體路徑（ADR 0014），讀寫與 Windows 相同；「在資料夾中開啟」為 `track_details_reveal`（不可加 FLAG_GRANT_READ_URI_PERMISSION）。SMB 曲目的時間取自伺服器 `stat`，寫回時保留原建立時間。遠端曲目的讀取與儲存見「遠端來源」。

### 格式與解碼

格式能力表（`crates/player-formats`，零相依）：所有副檔名判斷都查這張表——掃描（`is_windows_library_audio`／`is_android_library_audio`、`windows_library_extensions`）、檔案關聯、開檔對話框（`open_dialog_extensions`）、標籤可寫（`tag_write_supported`，DSF 可寫、DFF 唯讀）、頻譜圖（`spectrogram_supported`）、MIME→副檔名（`extension_for_mime`）、解碼引擎（`DecodeEngine`：`Dsd`、`Opus`、`WavPack`，其餘 `Rodio`；Ogg 內容偵測在 `open_decoder`）。新增格式只改 `FORMATS`／`EXTENSIONS`；`plugins/tauri-plugin-media-index/tests/kotlin_extension_list.rs` 把關 Kotlin `AUDIO_EXTENSIONS`。Windows 掃描集合不含 dff／alac／wave。

DSD（ADR 0016，Windows 與 Android）：`crates/player-dsd` 轉成 PCM（`reader::DsdPcmReader` 為共用轉換，Windows `DsdPcmSource`）。設定 `settings.json` 的 `dsd`（`gainDb`、`outputRateHz`、`dop`；`settings_get_dsd`／`settings_set_dsd`）。不做自動增益（使用者決定），濾波器不提供選項。DSF 標籤可讀寫（`write_dsf_edit`）。未實作：`dff`、多聲道、48 kHz 系列。Android：Media3 無 DSD 解碼器，`.dsf`／`.wv` 以 `moedsd:///`／`moewv:///` URI 經 `DsdDataSource` 與 JNI（`DsdBridge`）當 float WAV 讀；`PlaybackService` 的載入檢查與 `setMimeType("audio/wav")` 必須包含這兩個 scheme（漏掉時靜默失敗）；Android 只輸出 176.4 kHz。實際聽感待使用者確認。

DoP（ADR 0020 P2，Windows 獨佔模式，**尚無 DAC 可實機驗證**）：`dsd.dop`（預設 false；Android 不顯示）；`crates/player-dsd/src/dop.rs`、`exclusive/dop.rs` 的 `DopSource`（DSF 與 WVDSD 共用，條件 `is_dsd_path`）；只有雙聲道 DSD64／128／256；裝置不接受時該曲改走 DSD→PCM 並在 `ExclusiveInfo.dop_fallback` 說明。待實機：DAC 是否鎖定、進出 DoP 爆音、各 DAC 接受的容器。

Opus（ADR 0017，Windows；Android 由 Media3）：`crates/player-opus`（自己的 Ogg 頁面索引與封包讀取器＋libopus，`opusic-sys` 從原始碼靜態連結），48 kHz f32，套用標頭增益、去 pre-skip、依 granule 精確裁尾，seek 預滾 200 ms；只支援單／雙聲道；鏈接串流只播第一段。`player-audio-windows/src/opus.rs` 的 `OpusSource`；`.ogg`／`.oga` 以 `sniff_opus_file` 決定 Opus 或 Vorbis。標籤走 lofty（`crates/player-tags/tests/opus_edit.rs`）。`player-opus` 只在非 Android 目標編譯。Release 實機播放待確認。

WavPack（ADR 0025）：`crates/player-wavpack`（libwavpack 5.6.0 解碼端，BSD-3-Clause，`vendor/`）。編解碼短名（使用者定案）`WVLL`＝無損、`WVL`＝有損 hybrid、`WVDSD`＝DSD-in-WavPack，由 `player_wavpack::summarize` 決定；品質排序（`audio_format_order_by`，由高到低）：DSD、WVDSD、WAV、WVLL、FLAC、WVL、Vorbis、Opus、MP4、MP3。WVDSD 走專案的 DSD 轉換器並支援 DoP。WavPack／Monkey's Audio 的標籤、封面與歌詞直接用檔案的 `ApeTag`（`player-tags/src/ape_cover.rs`、`write_ape_edit`）。位元深度一律以官方編碼器的檔案驗證。待確認：Android 手機上的 WavPack 歌詞、Release GUI 實機播放。

Android PCM：Media3 1.11.1 官方 `ToFloatPcmAudioProcessor` 接在自訂 processor chain 的速度／靜音處理後，把 PCM16 等轉成 float32；`DefaultAudioSink` 的原生 float 路徑處理 float 與 24／32-bit PCM，不再二次轉換；`player.volume` 由 Media3 輸出端套用。聽感待驗收。

### Windows 音訊輸出（ADR 0020）

多裝置輸出（ADR 0020 P0）：`settings.json` 的 `outputDevices`；系統預設與自選裝置互斥（`active_devices`、`setRowEnabled`）；音訊端 `output_devices.rs`、`mirror.rs`，`RodioBackend::apply_devices` 比對差異；斷線以靜音心跳 `StallWatch` 偵測並重試。介面 `OutputDevicesSettings.svelte`＋`output-devices.js`（設定頁與快速設定共用），聲道映射是所有啟用裝置共用的一張矩陣，5 個方案槽（`outputPresets`）。音量是 Windows 端點真實音量（`crates/player-platform-windows/src/endpoint_volume.rs`），不是對音訊乘增益；音量同步（`outputVolumeSync`）由後端 `src-tauri/src/volume_sync.rs` 隨時執行。實機測試 `real_two_devices_play_together`（`--ignored`，靜音）。

取樣率轉換一律 rubato FFT（使用者決定移除選項；設定頁該區只剩「DSEE HX」）；`settings.json` 的 `resamplingMode` 保留相容但不讀；引擎的 `ResamplingMode::WindowsBuiltin` 已無入口。時鐘量測顯示各端點無持續漂移，不做漂移補償（`measure_real_device_clocks`，`MOE_CLOCK_DEVICES`／`MOE_CLOCK_SECONDS`）。

獨佔模式（ADR 0020 P1）：`outputExclusive`（`settings_get/set_output_exclusive`），只播第一台啟用裝置；實作 `crates/player-audio-windows/src/exclusive/`。開不起來時該曲以共享備援播放並說明原因（`exclusiveFallback`），下一首重試；勾「系統預設」時跟隨預設裝置，指定裝置時不跟隨；暫停立即釋放裝置，閒置 10 秒（`IDLE_RELEASE`）釋放。裝置通知 `device_notify.rs` → `audio-devices-changed`。實機測試（`--ignored`，靜音）：`real_device_opens_exclusive_and_plays`、`real_device_in_use_is_reported_with_its_reason`、`real_default_device_is_followed`。沒有 DAC，無聽感驗證；ASIO 未做。

### Android 平台

建置與安裝：`npm run android:build:arm64`（debug、aarch64、`--apk --ci`，輸出 `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`，複製為 `release/moemusicplayer-android-arm64-debug.apk`）；日常小修改用 `npm run android:build:arm64:fast`（同一把金鑰，不做 LTO、16 個編譯單元、不產 AAB，輸出 `release/moemusicplayer-android-arm64-fast.apk`，可直接覆蓋安裝）；使用者說「上傳 github」時才跑完整最佳化版 `npm run android:build:arm64:release`（`scripts/build-android-release.ps1`：fat LTO、單一編譯單元、strip、`opt-level=s`，同時產 APK 與 AAB，以 release 金鑰 `C:\APP\@Android\Play KEY.jks`〔別名 `anime theme key`，密碼由腳本從 `Play KEY.txt` 讀入建置程序環境變數〕簽章，找不到退回 debug 金鑰；`MOE_ANDROID_DEBUG_KEY=1` 改用舊 debug 金鑰並加 `-debugkey`；版本碼為日期數字 26.10.19 → 261019；ADR 0023；換金鑰後舊 debug 簽章安裝須先解除安裝；release 版不可除錯，WebView CDP 驗收用 debug 版）。**每次安裝到有連線的裝置：手機 NX809J（`adb connect 192.168.1.10:5555`）與平板 23046RP50C（192.168.1.92，無線偵錯埠每次不同，先 `adb mdns services` 找埠再 `adb connect`／`adb -s <序號> install -r <apk>`）；連得上的都安裝並在上面測試驗收，連不上的就略過，結案報告註明哪台已安裝與測試、哪台略過**；任何裝置都不得重新啟動。Android 專屬程式碼只在 aarch64-linux-android 目標編譯，變更共用 Rust 後須實際跑一次 APK 建置。

本機來源（ADR 0014、0039）：資料夾來源用 `AndroidFilesystem`／`fs_scan`；已有所有檔案存取權時改用 App 自己的資料夾瀏覽器（`android_folder_browser::pick_directory`、`FolderBrowserDialog.svelte`），因為系統選擇器不允許選儲存空間根目錄。排除資料夾存在 `excluded_folders`（與 Windows 同欄）。舊 `AndroidMediaStore`／`AndroidSaf` 來源不自動轉換。手機實測待做：資料夾瀏覽器、「在資料夾中開啟」、排除資料夾、本機歌詞候選、APK 更新。

播放服務與通知（ADR 0006、0039）：播放在外掛的 Media3 `PlaybackService`，佇列在 Rust；音訊生命週期不得依賴 Activity 存活，`MainActivity.onStart` 每次回前景 `ensurePlaybackService`；`onCreate` 必須 `addSession(session)` 才有 MediaStyle 通知。回前景畫面只剩底色的修正在 `MainActivity.onWindowFocusChanged(true)`（WebView `onPause`＋`onResume`）；再出現時的追查方式見 ADR 0039。

權限說明面板（ADR 0039）：所有檔案存取權、系統桌布需要的 `READ_MEDIA_IMAGES`、Android 17 的 `ACCESS_LOCAL_NETWORK`（targetSdk 37）都先顯示說明面板，按「前往授權」才請求；區域網路被拒時不呼叫遠端 IPC。API 37 系統提示的實機驗收待具備該版本的裝置。多個 SMB App 同時連不上時，NAS 445 埠、防火牆、Wi-Fi／VLAN 隔離也要列入調查。

### 曲庫同步與資料

曲庫同步（ADR 0040，`crates/player-core/src/sync.rs`）：查資料庫分類 → 平行讀標籤 → 依原順序寫入；`cancel_library_sync` 取消時已讀進度以「不完整」寫入（不對帳、不移除），下次續掃。資料夾與播放清單共用進度流程（`src-tauri/src/playlist_scan_index.rs` 的 `PlaylistScanIndex`）；新增來源種類只需提供查詢、`.lrc` 查找與標籤讀取三個函式，不要另寫索引與進度。標籤讀取在 `player-track-metadata`（`scan_parse_options` 一律不讀封面）。

`.lrc` 與內嵌歌詞隨同步存入資料庫（ADR 0040）：`track_lyric_sidecars`、`lyric_sidecar_sources`、`track_embedded_lyrics`；只有 `Complete` 掃描才刪除沒看到的 `.lrc`。播放時 `lyrics_service::local_lyrics_for_track` 先問 `Database::track_lyric_sidecar`：`Present`／`Absent` 不開檔，`NotScanned` 或尚無內嵌列時才開檔查找；遠端開檔結果另存 `remote_lyric_cache`。

冷建庫效能（ADR 0027）：設 `MOE_LIBRARY_PROFILE=<檔案.jsonl>` 寫出各階段事件；無 GUI 基準 `scripts\profile-cold-library.ps1 -MusicRoot D:\Music -Runs 5`（不碰播放器與 UserData）；冷建庫批次模式 `player-db` 的 `BulkBuild`。下一步：GUI 冷建庫量測、依磁碟實體位置排序讀取、冷狀態量 HDD 執行緒數。

遺失檔案隱藏（ADR 0033，schema v20）：`source_mappings.missing_since_utc_ms`；所有清單查詢加 `m.missing_since_utc_ms IS NULL`，記錄、覆寫與播放紀錄保留，再找到就自動恢復（`persist_mapping` 清為 NULL）。播放清單來源只在 `MediaSourceErrorKind::MissingFile`（NotFound 且 `volume_root` 仍在）時標記；資料夾來源（各平台與遠端）在 Complete 掃描把沒看到的對應標為遺失；遠端播放清單只有帳號根目錄（`list("")`）可連時才算遺失。設定「管理音樂來源」最近一次同步摘要的遺失檔案錯誤（`SourceSyncError.missingFile`、`itemKey`）旁有「從播放清單移除」→ `sync_remove_missing_playlist_entry`（伺服器清單先確認檔案不存在，再走 `remote_favorites` 取消收藏流程；本機與播放頁取消收藏共用 `rewrite_local_playlist_membership`），完成後重新同步。限制：搬移或改名的資料夾檔留下隱藏舊對應；新排除子資料夾內的檔案變成隱藏。

### 遠端來源（ADR 0026、0035）

狀態：SMB 已完成並以真實伺服器驗證（帳密在使用者的 `Downloads\smb.txt`，**不得寫進程式、文件或日誌**）；WebDAV 進行中（ADR 0035 P1）；SFTP／FTPS 延後、明文 FTP 不支援；SMB Kerberos 未支援。待驗收：Release GUI（CDP）、裝置實機、Android release 簽章版遠端播放、介面的「緩衝中／重新連線中」狀態（第二階段）。

- **帳號與密碼**：`crates/player-remote/src/secret.rs`（XChaCha20-Poly1305）、`account.rs`、`src-tauri/src/remote_accounts.rs`。`settings.json` 的 `remoteAccounts[].password` 只存裝置加密文字 `v1.…`，金鑰是 UserData 旁的 `remote-credentials.key`；頁面拿不到密碼。帳號只隨備份匯出輸出。刪除仍被來源使用的帳號會被拒絕。不認得的協定載入為 `RemoteProtocol::Unsupported`（保留、不能連線）。
- **來源與身分**：`MediaLocator::Remote { source_id, path }`，設定與 M3U 中以 `moeremote://<來源 id>/<路徑>` 表示（`player_core::remote_locator_uri`／`parse_remote_locator_uri`）。`locator_key` 為帳號層級 `remote-acct:{帳號 id}:{路徑}`（`remote_locator_key`），同一帳號下同一檔案只有一個 TrackId，每個來源保有自己的 mapping；`player-db` 的 `unify_remote_tracks`／`merge_tracks` 在掃描交易最後合併重複曲目。實機資料庫合併未驗證。
- **根目錄**：SMB 選擇器（`remote_browse_draft`）只列共用資料夾；WebDAV 從 `/` 進入並以「使用這個資料夾」選目前那層；資料夾、播放清單、排除資料夾相對於根目錄。SMB 根目錄＝整台伺服器的規劃使用者決定先不做，既有帳號如何處理尚未決定。
- **SMB 用戶端**：`smb.rs`（vendored `smb-rs` 0.12.1 MIT fork，見 `vendor/smb-rs/UPSTREAM.md`）；`smb_transport.rs` 的 `NoDelayTransport` 必須設 `TCP_NODELAY` 並一次寫出標頭與訊息；SRVSVC 支援 NDR32／NDR64 與續頁；每次伺服器呼叫由 `bounded_on` 從外部限時，並以 `LinkHealth` 區分慢與斷（`STALL_DETECT`、`ECHO_WAIT`、`Link::ping`）。
- **WebDAV 用戶端**：`webdav.rs`（Basic／Digest、憑證指紋釘選 `TlsMode::Pinned`、`remote_certificate_probe`），`src/lib/remote-protocols.js` 的 `WEBDAV_ENTRY_VISIBLE = true`。真實伺服器測試 `crates/player-remote/tests/real_webdav.rs`（`MOE_WEBDAV_*`，帳密只在 repo 外）。尚待：Release GUI、平板實機、區網裝置、其他 WebDAV 伺服器。
- **讀取層**：`fs.rs`（`RemoteFs`、`RangeReader`，標籤用 `RemoteByteSource`）、`prefetch.rs`（`PrefetchReader`：網路錯誤退避重抓，`RETRY_GIVE_UP` 後放棄）、`ReopeningFile`。放棄時錯誤以 `player_core::REMOTE_LINK_LOST` 開頭：Windows 在該位置暫停（`AudioError::RemoteLinkLost`，不跳下一首），Android 丟 `RemoteLinkLostException`。真實伺服器斷線下尚未驗證。
- **磁碟快取**：`cache.rs` 的 `CachedFile`／`DiskCache`（UserData `remote-cache/`，先進先出，`meta/` 記大小與時間讓已快取的歌開檔不連網）；標籤、封面、歌詞的小讀取優先讀快取但不存入。設定「快取」：`remoteCacheLimitMb`、`coverCacheLimitMb`（`remote_cache_*`、`album_artwork_cache_*` 命令）。遠端播放卡住時不要重裝或重開 App，先取 `adb logcat -d | grep "\[remote\]"` 與 `/proc/net/tcp6`。
- **掃描**：`index.rs` 的 `RemoteMediaIndex`；連不上（`Unavailable`）或帳密被拒（`PermissionRevoked`）都不移除曲目；建立／修改時間取伺服器值。量測測試皆 `--ignored`（`scan_transfer.rs`、`scan_heavy.rs`、`smb_latency.rs`、`scan_created_timing.rs`）。真實 SMB 上的時間列表顯示未驗證。
- **App 端**：`src-tauri/src/remote_connections.rs`（每帳號一條共用連線 `RemoteConnections`；`RemoteFiles` 實作 `player_core::RemoteContent`，啟動時 `set_remote_content`；開啟中的串流登記為 `OpenStream`，可由 `RemoteContent::release` 經 `ReleasableFile`／`FileRelease` 關閉）、`remote_sources.rs`（`remote_account_test`／`remote_smb_shares`／`remote_browse`／`remote_source_add`）。`library_sync` 把 Remote 根交給 `RemoteMediaIndex`。介面 `src/lib/RemoteSourcesSettings.svelte`；「本地」與每個伺服器分頁共用 `src/lib/SourceSections.svelte`（資料夾、播放清單、排除資料夾三區，新增來源種類也走此元件）。
- **播放**：Windows `resolve_playable_filesystem_locator` 無本機檔時回傳偽路徑 `moeremote://<來源 id>/<路徑>`，`player-audio-windows/src/remote_input.rs` 讓每個開檔處改用 `open_streaming`；遠端不套 DSEE HX。Android：`moeremote://` URI 交給 `DsdDataSource`（`REMOTE_URI_SCHEME`、`isConvertedUri`），JNI `android_dsd.rs` 的 `open_remote` 對 DSF／WavPack 轉 float WAV、其他格式原樣傳給 Media3；遠端曲目載入指令不帶封面，封面由 `fetchRemoteArtwork` 背景經 `DsdBridge.nativeRemoteCover` 取得後 `replaceMediaItem` 補上。
- **封面與歌詞**：Windows `find_artwork`／`find_lyrics`／`find_all_lyrics` 處理 Remote，Android 用 `src-tauri/src/remote_media.rs`；`remote_cover_bytes` 全域限流；可能只是讀取失敗時不得記為「沒有封面」（`remote_cover_lookup_may_have_failed`、`album_artwork_cache_mark_missing_for_track` 的 `definitive`）。
- **歌曲資訊與編輯**：`track_details.rs` 的 `fetch_remote`（頭尾暫存副本）與 `save_remote`（`release` 串流 → 下載並 `write_edit` → `RemoteFs::replace_file`；伺服器檔案已變則 `RemoteError::Changed` 拒絕）。真實 SMB 寫入測試 `MOE_SMB_WRITE_TEST=1`。實機在真實歌曲上的儲存驗收待做。
- **遠端播放清單**：`remote_browse_playlists`／`remote_playlist_add`；同步 `remote_playlist_sync.rs` 的 `sync_remote_playlist_source`（Windows 與 Android 共用，必須把進度交給 `on_progress`）；路徑樣式讀寫統一由 `crates/player-remote/src/playlist_style.rs` 的 `PlaylistStyle` 處理（往返相同），不得另寫轉換。
- **收藏到伺服器清單**：`playlist_edit.rs` 的 `edit_playlist_file`（寫入前才從伺服器讀，保留編碼、換行與未動到的行）；插入位置與本機收藏共用 `player_core::playlist::favorite_insertion_index`；應用層 `src-tauri/src/remote_favorites.rs`（曲目須屬同一帳號，否則 `reason` 為 `notOnServer`／`noAccount`）。遠端清單不算「我的最愛」。真實伺服器寫入、Android 裝置實測未驗證。
- **單一備份（本機與遠端一致）**：每次改寫播放清單檔只保留一份「原檔名＋`.bak`」，順序為暫存檔 → 刪舊 `.bak` → 原檔改名 → 暫存改名，失敗還原（遠端 `RemoteFs::replace_file_keeping_backup`、本機 `favorites.rs` 的 `write_keeping_backup`）；讀取後被改動就拒絕且不產生備份；`playlist_export` 不產生備份。SMB／WebDAV 上的順序未在真實伺服器驗證。
- **測試伺服器**：區網 SMB 測試分享——Ubuntu 24.04 WSL 隔離設定 `/etc/samba/moemusic-test.conf`，唯讀分享 `moemusic-test`（`/srv/moemusic-test`），帳號 `moe_test`、TCP 1445、只允許 `192.168.1.0/24`、停用 guest；密碼只交付使用者。下一步：以 Ubuntu／TrueNAS 測試帳號重跑目前程式碼的 NDR32 live 測試（`MOE_NDR32_SMB_*`）；真實 SMB 測試需設 `MOE_SMB_*`，未設時略過。

### 更新、接續播放與備份

自動更新（ADR 0022 版本辨識、ADR 0021 Windows 安裝、ADR 0015 Android）：只用版本號辨識（版本＝建置日期 `src-tauri/build-version.txt`，Release tag `v<版本>`）；Release 只放 `moemusicplayer-windows-x86_64.zip` 與 `moemusicplayer-android-arm64.apk`，沒有 JSON 清單、沒有單獨的更新器 exe，完整性靠 GitHub 資產 SHA-256 digest（沒有 digest 就不自動更新）。`UpdateCenter`：「忽略」（`ignoredUpdateVersion`）、「自動安裝」、「自動安裝並重啟」。Windows `install_in_place` 改名替換，只留一份 `.moemusicplayer.previous.<id>.bak`。發布：`npm run release:windows`、`npm run release:android`，加 `-Upload -ReleaseTag v<版本>` 才上傳。端到端更新要等有更新的 Release 才能驗證。

接續播放：`settings.json` 的 `resumePlayback`（預設 false）。正常關閉或更新重啟時若正在播放，`set_resume_playback(true)`（更新流程在暫停前記錄，中止時改回 false）；下次啟動還原 session 成功（`ready`）後 `take_resume_playback` 重設並 `request_play`；強制結束或當機不寫入。Android 播放 actor 在狀態改變時經 `ResumeHook.sink` 即時寫入，啟動時 `restore_session` 依欄位以 `play_when_ready` 續播（`AppState.settings` 為 `Arc<SettingsStore>`）。Release 與裝置實測待確認。

備份匯出／匯入（ADR 0024，設定 → 關於 → 備份與還原）：勾選面板（`AboutPage.svelte`、`src/lib/backup-export.js`）匯出只含勾選項目的 zip；帳號只以備份密碼封存（`remote-accounts.sealed`），任何項目都不得含明文密碼；不含封面與背景圖。匯入只替換給的部分，先放 `import-pending`，下次啟動、開設定與資料庫之前才換入（舊檔移到 `before-import`）。測試 `src-tauri/src/backup.rs`。Release 實機與手機實測待確認。

## 程式預設值

設定的預設值取自維護者當時的 `settings.json`，涵蓋主介面（`theme`：主色 `#A9C3FE`、邊框／分隔線 10%、自動背景透明度模式、背景清晰度 65／封面 50、快速設定透明度 20%／模糊 5 px、主背景模糊 5 px／透明度 60%、幻燈片間隔 2 分鐘）、曲目欄位（欄位順序「長度」在「音訊格式」前、欄寬 184／144／257、字尾省略、專輯卡片最大寬度 200、曲庫預設依新增時間排序）、播放頁（封面背景模糊 10 px／透明度 80%）、歌詞（顯示譯文、簡轉繁開、非目前行 50%、原文 20 px／譯文與羅馬拼音 90%（相對原文，`auxiliaryScalePercent`）、行距 10 px、工作列歌詞預設啟用且置中、20 px、譯文 100%、自訂色 `#00e1ff`、陰影）、輸出（音量同步預設開）與快捷鍵（鍵盤加手把按鍵／搖桿，沒有滑鼠綁定）；個別播放裝置設定、音樂來源、排除資料夾、視窗位置與語言不屬於預設值。預設值有兩份且必須一致：Rust `src-tauri/src/settings.rs`（`Default` 與 `#[serde(default = …)]` 函式，舊檔缺欄位時也套用）與前端 `theme.js`、`lyrics-preferences.js`、`now-playing-appearance.js`、`taskbar-lyrics-settings.js`、`shortcuts.js`、`track-columns.js`、`album-index-layout.js`、`App.svelte`（文字省略模式）、`OutputDevicesSettings.svelte`（音量同步）與 `app.css` 的主色變數；測試鎖定這些值。

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
- 建置、替換 Windows 執行檔、重新啟動播放器與安裝 APK 由主工作階段執行；子代理只修改程式碼與執行測試。
- 修改程式碼、資源或建置設定時，先完成測試並提交來源，再執行一次 Windows 建置：日常小修改用 `npm run release:windows:fast`（opt-level 1、256 個編譯單元、增量編譯，獨立的 `target/fast`，不產更新 zip），使用者說「上傳 github」時才跑完整最佳化的 `npm run release:windows`，避免執行檔內嵌舊 commit；純文件修改不建置。固定 Windows 執行檔為 `release/moemusicplayer.exe`。播放器執行中也照樣背景替換新版（含回滾與 SHA-256 驗證），替換完成後須正常關閉舊程序並立即啟動固定路徑的新 EXE，核對新版視窗及 build identity；不得強制終止播放器。**建置前先備份目前的 exe**：一律放在 `release/backups/`，檔名 `backup-<yyyyMMdd-HHmmss>-<舊檔 SHA-256 前 8 碼（大寫）>.exe`（時間為台北時間的備份當下），例如 `backup-20261022-163948-6D0314E9.exe`；只保留最新 5 個（依檔名時間排序），每次備份後自動刪除較舊的，不需詢問使用者。腳本自己留下的 `.moemusicplayer.previous.<id>.bak` 不屬於這個規則，不得改名或搬動。
- Bug／功能驗收一律啟動固定路徑的 Release 執行檔，以該程序的 WebView2 CDP 檢查實際 DOM、IPC 或錯誤；不得改用 Chrome、Playwright 或無資料隔離副本。開發期間主播放器一律以偵錯埠 9222 啟動並保持開啟，驗收後不必關閉。CDP 啟動方式：先執行 `netsh int ipv4 show excludedportrange protocol=tcp` 確認埠不在 Windows 保留範圍（例如 9326–9425 會讓 9337 無法監聽），預設使用 9222；設定 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` 後啟動 `release/moemusicplayer.exe`，再以 `netstat -ano | findstr :9222` 確認 LISTENING，並以 `http://127.0.0.1:9222/json/version` 確認有回應後才開始驗收。建置／單元測試不等於實際 App 驗收。Android 必須在 Android 目標環境驗收，並在所有有連線的裝置上測試（見「Android 平台」）。
- 不得強制終止使用中的播放器；Windows 新版按既定背景替換程序與備份規則更新，絕不觸碰 `UserData/`。不得未經同意重新啟動裝置。
- 工作結束時必須提供完整結案報告：實際修改內容、執行的建置命令與版本、安裝／啟動在哪些裝置、無法安裝或未實測項目及原因；不得把建置成功說成已安裝，也不得把單元測試說成實機驗收。
- 不以 Python sqlite3 讀寫使用中的正式 SQLite；先確認播放器關閉，或使用已驗證的唯讀／備份流程。
- 依變更範圍執行 `npm run check`、`node --test`、相關 Rust tests、workspace Clippy 與 rustfmt；完成工作後依「本文件維護規則」就地更新「目前工作狀態」。
- 發布前整理 AGENTS.md：使用者說「上傳 github」或執行 `npm run release:windows`／`npm run release:android` 上傳前，先依「本文件維護規則」整理本文件（刪除已完成或被取代的內容、把設計細節移到 ADR），並確認 `node --test tests/agents-md-size.test.mjs` 通過。

## 按需閱讀的領域規則

- UI、列表互動、播放頁、歌詞、背景及視覺 token：[`src/lib/AGENTS.md`](src/lib/AGENTS.md)
- Tauri IPC、使用者資料、播放生命週期及更新流程：[`src-tauri/AGENTS.md`](src-tauri/AGENTS.md)
- Rust Core、SQLite、排序／分組、跨平台資料與音訊邊界：[`crates/AGENTS.md`](crates/AGENTS.md)
- 產品詞彙：[`CONTEXT.md`](CONTEXT.md)；架構決策：[`docs/adr/`](docs/adr/)
