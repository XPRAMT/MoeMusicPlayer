# src/lib/AGENTS.md

適用於 `src/lib/` 的 Svelte／TypeScript 前端工作；同時遵守根目錄 `AGENTS.md`。本文件只保存穩定行為規則，不記錄各版本已完成事項或每輪測試流水帳。

## UI 共用規則

- **設定頁與快速設定抽屜必須完全一樣**：App.svelte 的 `settingsPanels` snippet 同時渲染在兩處，所以 App 層的狀態天然共用；但被兩處各掛載一份的子元件（`OutputDevicesSettings`、`AboutPage` 等）不得持有自己的實例狀態——凡是畫面顯示或編輯的資料、開關、載入結果、提示訊息都放在 `<script module>` 的共用 `$state`，只做一次的工作（讀取、輪詢、回應 Windows 事件）由最先掛載的那一份執行。新增子元件到 `settingsPanels` 時照做。需要「隨時生效」的邏輯（例如音量同步）一律放後端，不得依賴設定介面是否開著。
- **所有使用者看得到的文字都用 `tr('繁體中文原文')`**（`src/lib/i18n.js`，ADR 0019）：繁體原文就是 key，帶值用 `{0}`、`{1}` 位置佔位符（`tr('共 {0} 首', [n])`），不要用字串相加或在 `tr` 外組句子。新增或修改文字後執行 `node scripts/i18n-catalog.mjs` 重新產生 `src/lib/i18n/zh-CN.json`，再執行 `node scripts/i18n-catalog.mjs --missing` 看 `ja.json`、`en.json` 缺哪些句子並手寫補上（`tests/i18n-catalog.test.mjs` 會檢查三份字典都涵蓋、佔位符一致）；不要把句子拆成「文字 + 值 + 文字」，要整句一個 key 加 `{0}`，各語言語序才能不同；簡體用詞不對時改 `zh-CN.overrides.json`（逐句）或 `zh-CN.terms.json`（全域用詞），不要手改 `zh-CN.json`。語言在頁面載入時決定，切換語言會重新載入視窗，所以模組層級常數也可以呼叫 `tr()`。使用者資料（曲名、歌詞、播放清單名稱）與後端回傳的訊息不翻譯。讀原始碼比對中文字面值的測試要先 `import './helpers/untr-fs.mjs'`。
- 重用既有元件、CSS token 與共用樣式，不另造相同用途的按鈕／浮板／列表。字級使用既有 token；不得任意新增數值字級。
- 所有 App 內提示用 `appTooltip` action 與 `.app-tooltip.floating-surface`；不要新增原生 HTML `title`。保留 hover／focus 語意與 ARIA 描述。
- 浮出控件與彈窗沿用快速設定面板 `.floating-surface` 的背景、模糊、線色和陰影；其遮罩、焦點圈限、捲動及關閉行為依各元件需求。
- 鍵盤焦點框：沒有自訂樣式的元素一律為 1px、主色 70% 不透明度的外框（`app.css` 的 `:where(:focus-visible)`，零特異性，元件自己的焦點樣式優先）；`button`／`input` 維持 2px `--accent-text`、offset 3px；列表列（`.paged-virtual-row`）同樣 1px 70% 但 `outline-offset: -1px` 畫在列內。滑鼠／觸控不顯示（`:focus:not(:focus-visible)`）（`tests/focus-outline.test.mjs`）。
- 內容分隔線統一 1 CSS px 的 `--divider-width` 與 `--divider-color`；元件／按鈕邊框用 `--line`。不要重複繪製線條或在共用 token 外再疊透明度。
- 清單標題與資料列使用相同 grid template，em 欄寬容器固定 `--font-body`。數值欄標題／內容置中，文字欄靠左；音訊格式欄標題與內容整組置中。標題排序命中區涵蓋整個欄位。
- 清單共用虛擬列表與分組列 surface。分組列高度為設定曲目列高的 0.7 倍、至少 30 CSS px；只在下方畫 1px 分隔線，不可遮住上一列的線。列表頁面切換重建資料後，以穩定 list ID 還原位置；排序、分組及資料頁狀態由後端查詢結果提供，前端不可全量掃描建立分類。
- 全寬度變更要同步檢查標題、資料、佔位列與命中區；不可只改 header 或只改內容欄。

## 列表與導覽

- 曲庫、播放清單、佇列、演出者／專輯明細共用曲目欄與列表行為。欄位偏好由共用設定管理；播放佇列重排操作 traversal，不改播放清單原始 position。
- 所有排序、搜尋、過濾、快速首字索引與分組邊界從後端 SQLite 取得。前端頁面每次固定取 1,000 筆，虛擬化只控制 DOM 數量；禁止依視窗高度改頁大小或逐頁掃描推導索引。
- 浮動「定位到目前播放曲目」按鈕（`.locate-fab`，樣式在 `app.css`）有三處：播放佇列（用佇列自己的游標位置）、播放清單頁（`PlaylistEntryList`：後端 `playlist_get_track_position` 以與分頁相同的排序和項目解析用 `ROW_NUMBER()` 算出正在播放曲目的位置，曲目重複出現時取第一筆；只有清單包含該曲目時按鈕才出現，曲目、清單、排序或內容變動時重新檢查）與曲庫頁（`TrackList` 的 `locateTrackId` 屬性，只有曲庫傳；後端 `library_get_track_position` 以與分頁相同的篩選與排序用 `ROW_NUMBER()` 算出該曲目在目前列表的位置，Windows 開啟了外部曲目時改在合併後的列表中找；該曲目不在列表（被搜尋／篩選隱藏、不在曲庫）時顯示提示而不捲動）。載入該位置前後各 250 筆頁面後以 `scrollToIndexRequest` 捲動。
- 只有分組的排序才顯示快速索引：名稱（曲目／演出者／專輯）依首字，「發行時間」「新增時間」依年份，「音訊格式」依格式名稱（DSD 各取樣率合為 `DSD`、`WAVE` 併入 `WAV`、`M4A`／`AAC` 併入 `MP4`；長度與播放次數不分組）；曲庫、播放清單、播放佇列都一樣（`grouped-sorts.js` 的 `GROUPED_SORT_FIELDS`）。年份與時間沒有資料的曲目歸入「未知」組，視為最小值：舊到新（正序）排最前、新到舊（倒序）排最後（例：正序 未知 1999 2025 2026，倒序 2026 2025 1999 未知；音訊格式的未知是最低等級，同樣在正序最前）。快速查找面板的格子最小 2 em 見方，標籤較長（「2024」「FLAC」）時該欄隨最寬的標籤加寬、不限一個字元（`getQuickIndexCellWidthEm`，欄數以最寬格子計算）。分類順序、語系與排序方向跟隨同一個後端 ICU4X 結果；前端只呈現標籤與定位結果。
- 演出者首頁可保留目前明細返回；專輯首頁亦保留明細返回狀態。從左側導覽切換時列表依各自穩定 ID 還原位置，切換播放清單須重新查詢該清單。
- 專輯以正規化後名稱分組，無標籤組顯示「其它／未標記」，不可與名稱剛好相同的真實專輯合併。卡片封面和詳情頁代表封面使用同一組已掃描結果；封面快取可重建，不保存原始大型圖片進資料庫。
- 專輯明細 Disc 分組只有存在 Disc 2 以上時啟用；啟用後無資料、0、1 各自成組，並依數字排序，未知依方向置於端點。`#` 只顯示曲目編號。

## 播放與內容呈現

- 底欄封面在所有頁面固定為直角；不得以目前頁面或背景模式條件切換底欄封面圓角。

- 播放佇列排序會更新實際 traversal 並保存；拖曳重排保留目前曲目、播放位置、播放狀態及隨機狀態。只有點擊欄位標題排序會關閉隨機；開啟隨機時隱藏方向符號但保留欄名。
- 上一首在播放位置嚴格大於 3000 ms 時 seek 至目前曲目起點，否則前往上一個 traversal entry。不可用的上一首／下一首按鈕為 50% 透明度。
- 缺少曲名時顯示檔名主體；演出者只讀 `ARTIST`，不以 `PERFORMER` 代替。演出者分割只使用明確分隔符 `|`、反斜線、`/`、`;`、`,`、`&`，一般空白保留。
- 年份只由 `RecordingDate` 來源欄位讀取；接受有效 `YYYY`、`YYYY-MM`、`YYYY-MM-DD` 等支援格式並顯示可用精度，不以檔案修改時間替代。音質格式使用「．」分隔；無損清單可隱藏碼率，Hi-Res 條件為取樣率至少 48 kHz 且位元深度至少 24 bit。
- 歌詞候選依「逐字歌詞／逐行歌詞／純歌詞」標示時間精度，僅列實際存在的譯文與羅馬拼音。自動採用（`player-core` `auto_lyric_candidate`）的候選必須同時標題正規化相似度 ≥ 0.88（`AUTO_MATCH_TITLE_MIN_SIMILARITY`）、時長差 ≤ 2 秒（`AUTO_MATCH_MAX_DURATION_DIFF_MS`，任一方時長未知即不合格）且有時間戳；加權分數只用來在合格者中排序。手動搜尋永不自動採用。線上歌詞搜尋一律只用簡體（`lyrics_search` 自行把查詢與曲目欄位轉簡，前端不送 `queryAlternates`），含假名的日文欄位不轉換。
- 已移除歌詞只在使用者明確移除並保存 cleared 狀態時隱藏；關閉候選面板不代表移除。逐行繁簡轉換按行處理，含平假名／片假名的行保持原樣。

## 背景與播放頁

- 背景來源切換先載入並解碼新圖，成功套用後才撤銷舊 Object URL；讀取中或失敗時保留舊圖，避免閃黑／閃底色。主介面和播放頁封面背景都依 blur 的 1.5σ 延伸，不附加固定放大倍率。
- Android 背景（ADR 0008）：單圖以 Photo Picker 挑選（不可用時退回文件選擇器）並複製到 `UserData/background`；幻燈片資料夾以系統選擇器指向位置後轉成實體路徑（不保留 SAF 授權，ADR 0014）遞迴列圖，與桌面共用順序／隨機及切換時間邏輯，資料夾以 `formatSlideshowFolderLabel` 顯示成「內部儲存空間/…」。系統桌布不讀像素（Android 13/14+ 需 MANAGE_EXTERNAL_STORAGE，已否決），改由 `appearance_set_android_wallpaper_backdrop` 打開 `FLAG_SHOW_WALLPAPER`，並讓視窗和 WebView 透明；網頁端在 `:root.has-android-wallpaper-backdrop` 下讓 html/body 透明，`.main-backdrop.is-system-wallpaper::before` 以 `1 − 不透明度` 疊上 `--page`。模糊只在裝置支援跨視窗模糊時才生效。該模式不做縮放，也不做自動對比取樣：自動背景透明度模式以最壞情況估算透明度（`systemWallpaperOpacityPercent` = 100 − 清晰度），不得讓覆蓋層變成不透明。若已授權「所有檔案存取權」（ADR 0014），就改為讀取靜態桌布圖片（`androidWallpaperHasImage`），與 Windows 相同，每 60 s 檢查一次；這時不啟用原生 backdrop。
- 設定 → 音樂來源：只有在未取得所有檔案存取權時才顯示授權卡片；Android 與 Windows 一樣只有「選擇資料夾」（`android_pick_folder_source`）與「選擇播放清單」（`playlist_import_m3u`），需先取得存取權。沒有 MediaStore 或 SAF 區塊。權限由未授權變成已授權時會自動 `library_sync`。舊版 MediaStore／SAF 來源不再自動遷移，會保持不可用並保留原有曲庫對應；請重新加入同一資料夾並完成同步後再移除舊來源。
- 歌曲資訊按「儲存」（或 Ctrl+S）時，先在 `.app-shell` 內顯示 `.floating-surface` 確認面板「將會覆寫檔案原有資訊，是否繼續？」（確認／取消）。按確認才寫檔；取消、Esc、Android 返回鍵或點遮罩都只關閉確認面板，暫存的編輯保留。
- 自動背景透明度的估算先縮成 32×32。一般自動文字顏色模式以 alpha 加權的算術 RGB 平均；自動背景透明度模式先與黑／白底色合成，再依文字顏色取最亮／最暗一半像素平均。避免線性化或逐級試探透明度。
- 播放頁是覆蓋層，底層頁面與播放列狀態不因開關重建；播放頁本身不捲動，歌詞區獨立捲動。無封面時移除封面欄、歌詞置中並使用主介面背景影像層，不讓主介面控件透出。
- 專輯詳情封面收合規則：曲目捲動視窗固定從 Y0（完全收合的標題＋欄標題）開始，捲動中不得改變其位置、大小或捲軸比例；內容前置等於收合行程的空白。向下捲時前段 scrollTop 以 1 CSS px 對 1 CSS px 縮封面至最小 80 px（展開最大 200 px），之後才捲曲目；向上先回到清單頂端再展開封面。原生捲軸、滾輪與鍵盤都使用瀏覽器原生捲動，不得攔截滾輪、不得做原生／邏輯捲動映射或逐列 clip-path。封面、標題與欄標題位置由 CSS scroll timeline 依捲動位置驅動，不得改成時間補間動畫。標題外框與欄位格線寬度扣除原生捲動槽並與列項對齊，標題外框不得覆蓋捲軸命中區。右側演出者與統計維持顯示。
- 底欄音量按鈕的喇叭圖示依顯示中的音量（含拖曳中）換：0 % `IconVolumeOff`、1–33 % `IconVolume4`（只有喇叭）、34–66 % `IconVolume2`（一道聲波）、67 % 以上 `IconVolume`（兩道聲波），門檻與換算在 `volume-icon.js`（`getVolumeIconLevel`，依四捨五入後的百分比）。
- 播放控制工具提示由 `playback-control-tooltips.js` 根據當前快照生成；進度、音量、收藏等浮板與共用面板樣式一致。

## 測試

優先測試狀態計算與列表映射的純函式／元件測試。不要執行 Chrome、Playwright 或瀏覽器測試；正式 UI 驗收遵守根目錄規定，啟動 `release/moemusicplayer.exe` 並使用該程序的 WebView2 CDP。
