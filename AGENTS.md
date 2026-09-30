# AGENTS.md

## 專案現況與目標

專案已有可建置的 Tauri 2、Svelte 5、TypeScript 與 Vite 骨架，並包含 Rust Core、SQLite 曲庫、Windows 與 Android 媒體來源 adapter，以及雙語 README。

目前已完成：

- Now Playing 使用延遲掛載且保持已掛載狀態的全內容區覆蓋層，跨過側欄與主工作區但避開固定播放列；底層路由、頁面 DOM、曲庫列表捲動位置、播放核心、佇列與歌詞不因開關頁面而重設。左上返回鈕與覆蓋期間底部封面都返回進入前的曲庫／播放清單／佇列／設定頁；覆蓋期間底層側欄和主工作區 inert，固定播放列仍可操作。覆蓋頁本身不捲動，歌詞區可獨立捲動；不另呈現同步摘要或原生播放狀態提示。A/B 排列只在「設定 → 正在播放」選擇；播放頁封面下依序顯示音訊格式、曲名、演出者、專輯，四行資訊與歌詞文字置中；Hi-Res 標誌依來源音訊取樣率至少 48 kHz 且來源位元深度至少 24-bit 判斷。Tabler Svelte 5 圖示與中文無障礙名稱保持一致。`npm run check`、Node 68/68 與 headless Microsoft Edge Playwright 通過；真實 Tauri WebView 尚待人工確認。
- 首頁頂部宣傳 Banner 已移除，包含宣傳標題、說明、功能口號、唱片插圖與假輪播頁碼；主內容直接呈現各頁實際功能區，相關 CSS、響應式覆寫與裝飾動畫一併刪除。`npm run check`、`npm run build`、Node 64/64 與 Windows Tauri Release 建置通過；正式 Tauri 視窗仍待人工確認頁面間距。
- Windows 可新增來源資料夾，透過 Core 增量同步與 Lofty 讀取標籤；SQLite 保存曲目、來源映射與同步狀態，前端只取分頁結果。
- Windows 來源設定使用原生資料夾選擇器；程式在取消時不新增來源。使用者已在隔離 app-data 實際選取資料夾並完成掃描；Cancel 不新增來源尚未單獨確認。Tauri picker 及 M3U/M3U8 檔案對話框都以呼叫端 WebviewWindow 設為 owner，主視窗 config 與啟動時也明確關閉 always-on-top。
- 啟動同步由 renderer 在註冊 `library-sync-progress` 與 `library-sync-finished` 後觸發，避免空來源或快速掃描在 listener 建立前完成；不定總數的列舉只顯示已處理數。前端 call-site regression 與來源事件彙整測試通過。
- 對 `WindowsSystemIndex` root 會探測 WSearch 服務、範圍規則與 catalog 狀態供診斷；目前不把 Search 查詢結果當完整清單，曲目與刪除對帳只依完整檔案系統走訪。
- Android MediaStore/SAF 外掛、Rust IPC 與來源管理畫面已接線；Android ARM64 Debug APK 曾成功建置。最新版 insets 修正尚未在裝置複驗。
- Windows 音訊 crate 使用 Rodio/CPAL；Rodio 0.22.2 的 Windows feature set 使用 Symphonia MP3/FLAC/Vorbis/WAV decoder，因原 minimp3、claxon、lewton 路徑對常見格式明確回報 `NotSupported` seek。隔離合成 MP3/FLAC/Ogg-Vorbis/WAV 測試驗證 duration、seek 與 position；離線 96 kHz mixer 在 debug/release 都以 96,000 output frames 推進約 1 秒音訊 position。使用者最新回報在隔離 Tauri debug 視窗中已確認進度條點擊與拖曳 seek 正常，且以已知曲目時長對照後播放速度正常。前次「從開始就加速」回報在此 build 不再重現，但程式差異中未找到速度根因；`7c423d6` 仍只是依舊版 executable 修改時間推定的候選正常版，無 embedded build hash 證明。該候選至 seek 修復前的 HEAD 無音訊 crate、Rodio feature、CPAL config 或播放命令差異；只有 snapshot metadata duration 顯示 fallback。不可宣稱加速根因已定位，也不可把 GUI 驗收推廣到未測的個別音訊格式。最新人工驗收的執行檔 build hash 與 app-data identifier 未記錄。Rodio player 原始預設 speed=1.0，專案沒有 `set_speed` 呼叫。Tauri 已將 `TrackId` 解析為 SQLite 中啟用來源的本機路徑，再呼叫 `PlayerHandle`。actor 現提供非阻塞命令 ticket，Play/Pause/Seek/Volume ACK 於 worker 更新後快照；等待逾時會回明確錯誤，Tauri IPC 使用 `spawn_blocking` 等待，避免阻塞 async runtime。`PlaybackProgress` 以本地草稿呈現拖曳位置；worker snapshot 是外部唯一權威，seek ACK 前的輪詢由 fence 丟棄。失敗時立即清除草稿並顯示錯誤；metadata duration 可補顯示長度，但不代表播放器已具 seek 能力。Headless DOM harness 覆蓋失敗、鍵盤、切歌與拖曳生命週期。Rust session queue 已接上曲庫查詢與播放清單來源、自然播完、前後首、隨機及 off/all/one 循環；播放清單佇列保留重複項，並以 entry position 選取所點項目。SQLite v8 另保存正規化的播放 session queue traversal、游標、曲目 ID 與播放位置；播放位置每 5 秒至多更新一次，曲目／佇列變更、成功 seek、暫停及關閉時立即 checkpoint。啟動時依目前 TrackId/source mapping 解析曲目、恢復精確 shuffle traversal 與位置，載入後保持不播放（audio state 為 Ready），待使用者按 Play；離線／暫不可用時保留 queue/checkpoint，不保存路徑。播放清單 duplicate slot 由來源 entry position 保留，cursor 只在音訊 ACK 成功後提交。系統媒體控制的 Next/Previous 能力依目前 queue 狀態提供。人工 Tauri 驗收只覆蓋使用者實際測試的操作與曲目，不代表所有格式或 SMTC 已驗收。
- 點擊曲庫或播放清單歌曲只發出播放命令並留在目前頁面；不提供側欄「正在播放」直達入口。只有使用者手動點擊底部目前歌曲封面才切到「正在播放」頁；沒有目前歌曲時封面入口停用。Node 回歸測試涵蓋兩種點歌流程、側欄與底部入口；使用者已在最新隔離 Tauri debug 視窗人工確認點歌維持原頁、點底部封面進入正在播放頁。底部音量滑桿已移除與一般播放命令共用的 busy gate，拖曳以本地草稿即時跟隨，最多維持一個執行中命令和一個最新待送值，pointerup/change 立即送出最終值；音量 ACK 只更新音量欄位，不改播放狀態或位置。Headless Playwright 實際掛載 App 驗證多段 pointermove、ACK 期間草稿不回退、最終值、單一併發與鍵盤調整通過；可見 Tauri 視窗的音量拖曳尚待人工複驗。
- 曲庫 `src/lib/TrackList.svelte` 與播放清單 `src/lib/PlaylistEntryList.svelte` 共用 `PagedListController`、`PagedVirtualList.svelte` 的分頁快取、虛擬 viewport 與鍵盤導覽；每次只取後端 40 項頁面，最多六頁／240 項 LRU 快取。播放清單以全域 `position` 同時作穩定 row key 與 queue `entryPosition`，保留重複曲目與未匹配 locator 項目；切換清單或同步刷新會變更 scope/reset generation，丟棄舊回應。Node 64/64 通過；合成 100,000 項測試限制最多 28 個虛擬 row、240 個快取項。Headless Chrome 實跑 `tests/track-list-harness.html` 100 個大步距位置，最多 24 列、最後 16 列；`tests/playlist-list-harness.html` 102 個位置，最多 25 列、最後 16 列。播放清單 harness 亦確認未匹配項目可見，前兩筆同 TrackId 的列按 position 顯示，鍵盤播放第二筆回傳 `entryPosition=1`。可用 `tests/configurable-columns-harness.html` 搭配 `playwright-cli run-code --filename=tests/configurable-columns.playwright.js` 驗證共用欄位、兩種列表的 header/row/skeleton/aria-colcount、未匹配欄位佔位符與 Now Playing A/B；headless 360×800 實測 document width 為 360、列表只在自身 viewport 橫向捲動、六欄各自隱藏時格數同步、A/B DOM 節點切換保留且窄版順序正確。此測試不代表真實 Tauri WebView 影格效能，該環境仍待驗收。
- SQLite v8 保存播放清單與順序項目、曲目/來源投影、同步狀態、metadata version、legacy locator reconciliation 索引、以 TrackId 為鍵的歌詞與候選快取，以及正規化播放 session header/entry rows；新增的 session row 不依賴 Track table foreign key，避免來源短暫離線時清掉恢復資料，也不保存檔案路徑；新增 128 筆一批的背景 metadata 回填，取消或重啟後可續跑。播放清單頁面或 queue 查詢前會在單一交易內嘗試重對應整份清單，只填入唯一匹配且仍為 NULL 的 TrackId，不覆蓋已匹配項目或人工欄位。Windows 前端以分頁 IPC 瀏覽清單，透過 Rust 原生對話框匯入 M3U/M3U8、匯出 UTF-8 M3U/M3U8；未匹配曲目保留原 locator。相對匯出會拒絕逃出所選共同根目錄的路徑。Unicode M3U/M3U8 匯入、分頁及匯出 helper 整合測試通過；播放清單原生對話框仍未以 GUI 驗收。播放 session DB reopen、100,000 項含重複 ID/shuffle traversal、位置、損毀游標拒絕與離線保留測試通過；隔離 Tauri service 測試驗證 queue/duplicate entry position、解析失敗不移動 cursor、ACK seek checkpoint，以及啟動載入／seek 後保持 Ready 且不自動播放。正式 GUI 重啟恢復尚未人工驗收。
- M3U/M3U8 匯入可將外部檔案登記為音樂來源。Windows 啟動背景同步以檔案路徑、大小、mtime 與 SHA-256 判定播放清單變更；每次仍檢查唯一引用歌曲的 size/mtime，只重讀變更歌曲標籤。外部清單是投影權威且不自動回寫；同路徑重複匯入沿用 source/playlist ID；同名 legacy playlist 只在完整有序 locator 相符時移轉。不同來源引用同檔共用 TrackId，移除一個播放清單來源只清除自己的 mapping；播放清單讀取/解析失敗保留前次投影，單首缺檔保留 locator 並允許其他歌曲繼續同步。隔離測試涵蓋 Unicode/空白/相對路徑、重複項、playlist 內容同 size+mtime 編輯、音訊 metadata 增量更新、離線/恢復、同名不同內容、來源移除與非權威 registry 保護。
- 設定頁將來源管理收在「設定 → 音樂來源」，另有「設定 → 外觀」可自訂背景色與主色；預設純黑 `#000000`、水藍 `#55D9FF`。Theme、shuffle、repeat 與 folder/playlist path source registry 由版本化 JSON 持有，SQLite 僅保存可重建的音樂資料投影；成功遷移後舊 SQLite theme preference 已清除。Now Playing 使用目前曲目已載入的同一個封面 Blob URL 作全視窗模糊背景，並以單一主題色遮罩維持對比；設定 JSON schema v4 保存背景模糊程度（預設 20 px，範圍 0–40 px）與元件底色透明度（預設 35%，範圍 0–100%），只在播放覆蓋頁套用，背景關閉時移除渲染。主 JSON 與備份都損壞時標記 registry 非權威並暫停整次同步/清理；使用者重新登記並明確確認前不修改既有來源投影。十六進位色值驗證、對比、設定遷移、備份復原與降級保護測試通過。設定頁的實際 GUI 視覺、縮放及偏好重啟回讀尚未由使用者驗收。
- Now Playing 封面背景的 Playwright harness 為 `tests/now-playing-appearance.playwright.js`，mock 提供合法的本機同步歌詞 fixture，便於查看亮色封面下歌詞文字對比；測試涵蓋設定、慢 ACK 合併、背景共用封面 URL、圖片失敗 fallback、dock 命中及 12 種視窗尺寸。最新 `npm run check` 無錯誤或警告、Node 測試 65/65、Playwright 測試通過；截圖位於忽略的 `target/now-playing-appearance-light-cover.png`。真實 Tauri WebView 的歌詞對比、封面載入及控制項仍待驗收。
- Windows 正在播放曲目封面透過 `library_get_track_artwork` 單首二進位 IPC 延遲讀取，只使用啟用來源映射；優先 Lofty 內嵌圖片，再找同資料夾 `cover.jpg`、`folder.jpg`。保留原始編碼位元組與像素尺寸，不經 JSON/base64、不重編碼；單一 Blob URL 共用於「正在播放」與底部播放列，切歌、停止或元件卸載時釋放，過期回應會丟棄。上限 32 MiB、單邊 16,384、64 百萬像素；後端會以 `ARTWORK_TOO_LARGE` 區分超限與缺圖，前端顯示對應說明；有效 sidecar 仍可作為超限內嵌圖的 fallback。封面 Rust 測試 20 項、前端測試 7 項通過；真實 Tauri 畫面尚未驗收，Android Content URI artwork 尚未接線。
- 歌詞已接入 Now Playing 與 Tauri IPC：`lyrics_get_track`、`lyrics_search`、`lyrics_select_candidate`、`lyrics_cancel_search`。Windows 每次只依 TrackId 的啟用來源映射讀取本機 `.lrc` sidecar，再讀 Lofty `Lyrics`／`UnsyncLyrics` 內嵌內容；本機結果先於 SQLite 的 Provider 快取，掃描 `TrackMetadata` 不保存歌詞。Rust Core 解析 LRC 與 NetEase YRC 的逐行時間，YRC 逐字原文及 QQ QRC/trans/roma 原始資料保留。QRC-only 目前不解密、不宣稱同步，也沒有可顯示文字；候選明確標示原始 QRC 未解碼／目前不可預覽，仍可手動選取並保存，套用後顯示無可讀歌詞提示。NetEase/QQ 遠端查詢使用無 cookie/憑證 transport、單次 5 秒逾時與 512 KiB response cap；service 最多 20 個候選、抓取 8 份歌詞、3 個併行 fetch、單次搜尋 20 秒，低信心不自動套用，手動選取會擋住後續自動覆寫。Provider fixtures 不連 live endpoint。歌詞顯示偏好存於版本化 `settings.json` schema v4，預設不顯示譯文／羅馬拼音、非目前歌詞文字透明度 70%、原文字級 14 px、輔助字級 10 px；範圍分別為 10–100%、12–36 px、9–24 px。Now Playing 歌詞列的「譯／羅」和「設定 → 歌詞」共用偏好。非目前歌詞文字只在播放或暫停且存在可見的目前同步 cue 時套用透明度；目前列文字保持完全不透明。歌詞面板、目前與非目前歌詞列背景皆透明；空文字 cue 保留時間點但不渲染或高亮，也不會令下一句成為目前列；停止／無 cue 狀態及純歌詞文字保持完全不透明。虛擬歌詞列保留原文兩行 clamp；非空原文以兩行槽位計算，譯文／羅馬拼音只有偏好開啟且該句內容非空時才計入各自一行，空句高度為 0。句間距 lineGapPx 預設 24 px、範圍 0–64 px，只加在有內容句塊之間且末句無尾間距。依歌詞／偏好變更建立共享 prefix offset 與 visible-index 索引，供 DOM、spacer、plain/timed virtual window 與 auto-scroll 共用；scroll 以二分搜尋可見行，DOM 有界，播放位置更新不重建全本索引。Timed window 最多 13 列，plain window 依 viewport 加前後緩衝渲染。`cargo test --workspace --locked --offline`、嚴格 Clippy 與格式檢查通過，涵蓋 DB v6→v7 lyrics 與 v7→v8 playback session migration、TrackId locator→本機 sidecar、候選快取／手動選取、高信心自動保存、低信心保留候選、取消不寫 DB、DTO key 契約與同步輔助行對時。
- 使用者已在隔離 Tauri/app-data 中透過原生資料夾選擇器選取唯讀來源 `\\?\D:\Music`；隔離資料庫保存 1 個 root、5,362 個曲目與來源映射，掃描狀態為 complete，另有 3 項 WSearch 診斷。正式 App SQLite 未觸碰。原生對話框的取消路徑尚未單獨確認。
- Windows 系統媒體控制已從 Tauri 主視窗 HWND 接入 WinRT worker；Shell 以播放器快照每 750 ms 至少間隔更新標題、演出者、專輯、封面、播放狀態與時間軸，並把 Play/Pause/Stop/Seek/Next/Previous 事件送回音訊服務。Next/Previous 能力依 active queue 狀態更新。crate 的 18 項單元與 2 項隱藏 HWND 整合測試通過；隱藏視窗測試以有效 1×1 PNG 讀回 Thumbnail stream bytes，並確認切至缺圖曲目後清除 thumbnail、仍保留曲名等 metadata；Shell fake-event 測試涵蓋控制映射、無佇列能力時停用佇列事件及 attach/update 失敗與音訊隔離。實際 Tauri 視窗、媒體 flyout 與硬體按鍵仍未驗收。
- Svelte 型別檢查、Vite 正式版建置、Node 64/64、`cargo test --workspace --locked --offline`、`cargo clippy --workspace --all-targets --locked -- -D warnings` 與 `cargo fmt --all -- --check` 通過。Rust metadata 測試涵蓋 `RecordingDate` 年份正規化、可辨識無損來源的 bit depth、v5→v6 metadata migration、v6→v7 歌詞 migration、v7→v8 播放 session migration、metadata parser v1→v2 回填與可取消／重啟續跑的 128 筆背景批次；Android adapter 欄位轉換目標檢查通過，但 Android runtime 尚未驗收。Rodio pipeline 另有 1 項 ignored、只讀預設 endpoint config diagnostic。Windows `npm run release:windows` 執行 Tauri Release `build --no-bundle --ci`，建置時暫時固定 `CARGO_TARGET_DIR` 為 repo `target` 並在結束時還原呼叫端值；若指定 `CARGO_BUILD_TARGET`，來源路徑包含該 target triple。成功建置且來源 EXE 非空後，腳本以同目錄暫存副本核對大小與 SHA-256，再首次 Move 或以唯一 `.bak` 執行原子 Replace；只有固定輸出大小與 hash 驗證成功後才刪備份，建置或替換失敗不會預先刪除舊 EXE。固定輸出為 `release\moemusicplayer.exe`，已加入 `.gitignore`。`npm run check` 無錯誤或警告，Windows Release 已更新固定輸出 `release\moemusicplayer.exe`。若預設 `target/release` 來源遭正在執行的播放器鎖定，可指定與 `rustc` host 相同的 native target（`CARGO_BUILD_TARGET=x86_64-pc-windows-msvc`），改從 target triple 專屬目錄建置，無須操作使用者程序。正式 Tauri GUI 仍待人工驗收。Windows shell 整合測試涵蓋 Unicode 資料夾新增、合成 MP3 同步、SQLite 分頁/搜尋、關閉重開，以及 TrackId 到保存路徑再到播放 handle 的命令流程；另有 queue/lyrics IPC JSON 契約測試。

仍未完成或未驗證：

- Windows AAC/M4A 已修復：Rodio Windows feature 啟用 `mp4`，帶入 Symphonia ISO-MP4 demuxer 與 AAC decoder。合成 AAC-LC/M4A regression 在修正前失敗、修正後通過；另以 `MOEMUSICPLAYER_AAC_M4A_PATH` 對使用者指定的 `hanser - Cyberangel(純真夢歌).m4a` 完整解碼，讀取 167.018 秒、16,033,792 個非零樣本，未初始化音訊輸出裝置。`cargo test --workspace --locked --offline`、音訊 pipeline tests、格式檢查與嚴格 Clippy 通過。尚未在 Tauri app 實際發聲驗收，因此仍需確認真實播放器輸出。
- 本輪 Now Playing 全內容區覆蓋層與雙返回控制已通過五尺寸幾何測試，以及掛載真實 App.svelte 的曲庫／播放清單／佇列／設定往返與列表捲動保留測試；真實 Tauri WebView 尚待人工複驗。
- 頂部宣傳 Banner 已移除並通過 Svelte 型別檢查、正式前端建置與 Node 64/64；新版 Tauri 視窗尚未人工確認各頁內容起始位置與同步資訊布局。
- Windows 原生資料夾選擇器已由使用者在隔離 app-data 選取 `D:\Music` 並完成唯讀同步；正式 AppData 未觸碰。Cancel 不新增來源的路徑尚未單獨確認；播放清單原生對話框仍待使用者驗收。系統媒體控制尚未由實際 Tauri 視窗、Windows 媒體 flyout 或硬體媒體按鍵端到端驗收。
- Windows Tauri/音訊 IPC 的命令 ACK 與 queue/mode 已接線；`npm run check/build`、workspace Rust tests、嚴格 Tauri Clippy 及 Windows release build 通過。服務整合測試以臨時 Unicode 音樂資料夾與合成 MP3 驗證 TrackId 路徑、worker 播放、暫停與音量，並驗證重複 playlist TrackId 依 entry position 建 queue。Headless DOM harness 已覆蓋 seek failure、鍵盤操作、切歌及拖曳生命週期；worker snapshot 維持外部唯一權威，seek ACK 前的 poll 由 request fence 隔離，失敗會丟棄草稿並顯示錯誤。Rodio Symphonia paths 對 MP3/FLAC/Vorbis/WAV 的合成 duration/seek tests 通過，並不保證任意檔案格式均可 seek。使用者已在最新隔離 Tauri debug 視窗實際確認 slider 點擊／拖曳可正常 seek、啟動播放速度符合已知曲目時長、點歌維持目前頁面而底部封面可進正在播放頁；個別曲目格式、執行檔 hash 與 app-data identifier 未記錄。前次加速回報於最新驗收未再重現，但未找到可證明的程式碼根因；底部右側音量滑桿拖曳已通過 headless App DOM 回歸，但尚待 Windows Tauri 視窗人工複驗；SMTC flyout／硬體媒體鍵（包含最小化時 WebView timer 節流）仍待驗收。
- 設定頁與真實 Tauri 視窗的自訂色對比、偏好重啟回讀及縮放版面尚未人工驗收。欄位偏好與 Now Playing A/B 已接版本化設定 JSON；Svelte 型別檢查、純函式測試與 headless DOM harness 通過。
- 播放清單來源的程式級隔離測試已通過，但 Windows 真實 Tauri 啟動同步、原生匯入對話框與檔案外部修改後的畫面更新尚未 GUI 驗收。Android playlist-file sync 尚未接線；Windows sync 為目前實作範圍。
- Hanser `hanser.m3u8` 已以唯讀隔離來源資料庫透過 SQLite backup API 複製到 `target/` 新資料庫，未觸碰正式資料庫或音樂原檔。嚴格 UTF-8 驗證為 176 個絕對路徑且全數存在。來源舊清單 ID `8bf3f89c-04c0-418e-b29b-a33d867b15ab` 原有 176 項、0 匹配；觸發頁面/queue 查詢後全清單重對應為 176/176，queue 為 176 項。實際匯入另建立新清單 ID `ca39c101-c843-48a4-a100-9d72cf351736`，176/176 匹配；來源清單數 1→2，兩份清單項目順序 0..175，關閉重開後仍一致。Core 共用 Windows locator key 正規化涵蓋一般與 extended path、Unicode、空白及長路徑；Android 音訊尚未實作。
- Android 最新頂部/底部 system insets 修正及 MediaStore/SAF 真機掃描尚未驗證。先前 ARM64 APK 建置早於本輪共用 shell 變更；本輪直接 `cargo check --target aarch64-linux-android` 因目前 shell 找不到 `clang.exe` 而停在 SQLite native build，未得到 Android app 編譯結果。Android 播放未實作。
- 歌詞前端與 Rust IPC/Provider 及歌詞偏好儲存已通過 fixture、SQLite、取消、DTO 契約、偏好範圍／migration／重開保存測試；`npm run check/build`、Node 與 LyricsView Playwright 幾何／偏好測試通過，Windows Tauri `--no-bundle` release 編譯亦成功。尚未由真實 Windows Tauri 視窗確認 sidecar/內嵌歌詞、網路候選、手動套用、譯文／羅馬拼音切換、偏好實際回讀與切歌取消。NetEase/QQ 使用非官方 HTTP 端點，端點格式或來源檢查可能變動；沒有以 live endpoint 測試成功作為驗收。QRC-only 原始內容目前不能解碼為可讀行；候選明確標示原始 QRC 未解碼／目前不可預覽，仍可手動選取並保存，但套用後沒有可顯示歌詞；後續只需評估 QRC 解碼與呈現方式。LyricsView Edge harness 測過混合空／缺譯文與羅馬拼音、偏好切換不增高缺內容列、透明 pane/row、句間距 0/64、最大字級、variable prefix/spacer 對齊、遠距 seek/scroll、目前行自動置中及停止／paused；Node 68/68 含 100,000 列中僅 3 列可見的有界 window 測試；這些自動測試不代表真實 Tauri GUI 驗收。非 Windows local lyrics adapter 尚未接線，Android playback 仍不支援。`lyrics_search` 會在先前 `lyrics_get_track` 回空後重查本機來源，避免本機歌詞更新競態；同一首曲目可能因此再解析一次 Lofty 內嵌標籤。
- Windows 封面目前只讀取已啟用本機檔案來源；Android MediaStore/SAF 封面、使用者實際 Tauri 視窗圖片解碼／縮放與封面載入失敗畫面仍待平台接線及驗收。封面上限內仍可能遇到瀏覽器無法解碼的容器資料，該路徑會回到占位圖。
- 100,000 首真實曲庫端到端效能、真實 Tauri WebView 的捲動影格表現及完整 Windows SystemIndex 使用判準尚無實測結論；目前共用列表的 Node 與 headless Chrome harness 已驗證合成 100,000 項、虛擬 DOM 最多 25 列、LRU 最多 240 項，不能替代正式 Tauri 視窗效能量測。

下一步：請使用者在隔離 Windows Tauri 視窗驗收歌詞本機優先、Provider 候選顯示與手動保存，並確認譯文／羅馬拼音控制、字級／非目前歌詞文字透明度與偏好關閉重開回讀；網路端點未實測，先以 fixtures 維持測試。另於真實 Tauri 視窗確認新版頂部 Banner 移除後的頁面間距、共享欄位設定保存／重啟回讀，以及 Now Playing 覆蓋側欄、兩種返回均回到原頁、歌詞與列表狀態保留和窄視窗播放列可用；headless 幾何與真實 App DOM 測試已通過但不代表 Tauri WebView 人工驗收。保持已確認正常的 seek、音量拖曳及點歌／封面導覽行為。另需在隔離 Tauri app-data 中人工驗收關閉後重開會精確恢復 queue/shuffle 游標與播放位置、維持不自動播放，並檢查來源暫不可用後 queue/checkpoint 仍保留。其他待驗包括播放清單來源啟動同步與外部變更、原生匯入對話框、degraded registry 重新登記確認、舊 Hanser 清單播放與 queue 控制、SMTC flyout／硬體按鍵、真實封面畫面與應用音訊輸出；不得操作使用者正在使用的視窗或正式曲庫。`D:\Music` 已在隔離資料庫唯讀掃描完成，不重複掃描或寫入音樂檔。正式 Tauri WebView 的捲動與縮放效能仍待量測；Android 驗收需以 Tauri Android CLI/NDK 環境重建目前 shell，確認安全區與 MediaStore/SAF 真機掃描；Android playback 未實作。每首實際播放累計時間與依較低 played/duration 優先的 shuffle 尚未實作；不得描述為目前播放模式功能。

曲目欄位與 Now Playing 規則：

- 側欄「播放清單」樹可收合並選取播放清單；播放清單與「播放佇列」頁面都讓右側清單使用主要內容區全寬。「播放佇列」是顯示目前播放佇列的導覽按鈕，不會把整份播放清單加入佇列。Tauri `playback_get_queue_page` 以每頁最多 100 項取得目前 traversal 順序，`entryPosition` 保留重複曲目的個別佇列項，曲目中繼資料暫時不可用時仍保留項目。佇列 UI 共用分頁虛擬列表，游標變更只探測單筆目前位置。Node 64/64 與 Playwright queue/layout harness 通過；實際按下導覽按鈕會顯示佇列，合成 100,000 項最多渲染 28 列，1366/1024/800/412/360 寬度均無水平溢出，console 無錯誤或警告。真實 Tauri WebView 尚待人工驗收。
- 曲庫與播放清單共用六個資訊欄的順序與顯示偏好；序號與播放操作固定。設定 JSON 保存順序／可見性，兩列表格的標頭、資料列、骨架、欄數與網格模板使用相同可見欄位。此功能已接入，headless harness 覆蓋首末欄移動、六欄逐一隱藏、曲庫與播放清單同步及未匹配資料的佔位符。
- 「演出者」只讀音訊標籤 `ARTIST`，不以 `PERFORMER` 替代；曲目標題不再重複顯示演出者副標。使用者提供的 18 首 FLAC 在 `ARTIST` 都有 hanser。
- 年份只讀 Lofty `ItemKey::RecordingDate`；接受 `YYYY` 與有效的 `YYYY-MM-DD`，介面只顯示年份。不得回退獨立 `YEAR`、`ORIGINALDATE` 或檔案修改時間。Lofty 將 MP3 `TDRC`、M4A `©day` 與 FLAC Vorbis `DATE` 映射為 `RecordingDate`。metadata parser 版本由 1 升為 2，讓既有曲目在正常來源同步時以每批 128 首的背景回填更新 SQLite `year`；失敗或中斷的項目可於後續同步續跑。音訊格式摘要只由可取得的 codec、sample rate、bit depth、bitrate 組成；缺少欄位顯示 `—`，bit depth 只對可辨識的無損來源提供，不把有損解碼 PCM depth 當作來源深度。
- 對 `D:\Music\Loss\[hanser]` 的直層唯讀稽核涵蓋 84 個音訊檔：62 個 MP3 有 `TDRC`、14 個 M4A 有 `©day`、7 個 FLAC 有 Vorbis `DATE`；另 9 個 MP3 有與 `TDRC` 重複的 ID3v1 `YEAR`。這些日期欄位都由 Lofty 映射至 `RecordingDate`；程式變更已覆蓋前 83 首，唯一無日期音訊檔仍為空年份。正式 App SQLite 尚未執行這次 v1→v2 回填，會在新版後續來源同步時逐批更新。
- 「正在播放」預設 A（封面在前、歌詞在後），B 反轉兩區；偏好保存到設定 JSON，只有「設定 → 正在播放」可切換 A/B。切換只移動既有 DOM，不重建播放或歌詞內容；沒有歌詞時仍顯示提示。Now Playing 以延遲掛載的全內容區覆蓋層遮住但保留底層側欄與主頁 DOM，底部播放列保持可用；左上返回與底部封面都回到進入前的頁面，關閉時不重設播放／佇列／歌詞／列表狀態；主播放區限制在標題列與播放列之間，不接受外層捲動，歌詞保留獨立可捲動視窗。封面 `object-fit: contain` 依來源長寬比縮放，frame 貼合影像，只保留 1 px 邊框；以 A/B 當側可用寬高、資訊高度及 viewport 限制計算最大尺寸，不裁切、不拉伸、不留黑邊。封面下方順序為音訊格式（若來源音訊取樣率 ≥48 kHz 且來源位元深度 ≥24-bit，顯示原始 Hi-Res 圖示）、最多兩行曲名、演出者、專輯；四行資訊與歌詞文字置中，歌詞工具列及候選操作維持原有對齊；桌面格式／演出者／專輯實測 14 px、曲名 21–23 px，窄版輔助文字 13 px、曲名 20 px；長曲名可換行，資訊區不另建捲動視窗。Headless Playwright `tests/now-playing-layout.playwright.js` 使用 12 個規定視窗尺寸檢查 A/B 順序、重要控制邊界、頁面不捲動、方形封面最大填合及歌詞內部捲動，並在 1920×1080、1366×768、412×915 對方形／直向／橫向來源測試 A/B 下可視影像比例和貼邊。`tests/now-playing-overlay.playwright.js` 掛載實際 App，檢查設定所選 A/B、音訊欄位、置中 computed style、兩種返回、底層頁面及列表捲動保留；實際 App 方形封面 frame 為 697×697（1920×1080，A）及 404×404（1366×768，B）。真實 Tauri GUI 尚待使用者複驗。

下一輪待辦：由使用者人工驗收前述欄位偏好和 A/B 設定持久性。每首實際播放累計時間與依較低 `played_ms / track_duration_ms` 優先的 shuffle 仍待實作，並須明確定義 duration 為 null 或 0 的行為及補測。

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
- **搜尋：**100,000 首仍可快速查詢，使用分頁或虛擬列表；不得先把全庫載入前端再篩選。曲庫與每一份播放清單歌曲列表必須共用泛型分頁控制器、虛擬 viewport 與鍵盤導覽元件，不得複製分頁／虛擬化邏輯；每個 row renderer 可依資料特性不同。播放清單 entry `position` 是穩定 row key 與 queue `entryPosition`；未匹配項仍可見但不可播放。切換清單或外部同步刷新須隔離舊回應。SQLite FTS5 是預設候選，需由實測確認。
- **播放：**播放/暫停、前後首、跳轉、音量、循環、隨機與佇列。架構需容納無縫播放。Windows 整合系統媒體控制；Android 使用 Media3/MediaSessionService，支援背景、通知列與藍牙控制，Activity 關閉後仍可依平台規範播放。點擊曲庫或播放清單歌曲只開始播放並留在原頁；只有手動點擊底部目前歌曲封面才進入「正在播放」頁，不提供側欄直達入口。進度滑桿拖曳時以本地草稿呈現，pointerup/cancel（包含滑出滑桿後）只提交一次或還原草稿；切歌時丟棄舊草稿，過期播放快照不得蓋掉 seek ACK 權威快照。音量拖曳以本地值即時跟隨，pointerup/change 提交最終音量；音量命令需合併為有界佇列，與播放、seek 的 busy 狀態隔離，舊 ACK 或輪詢不得倒退較新的拖曳草稿。
- **播放 session：**App SQLite 持久保存目前播放佇列的精確來源順序、shuffle traversal、目前 entry 游標與播放位置；播放清單重複曲目以各自 entry position 保留。下次啟動還原相同佇列、目前曲目與位置，但不自動播放，必須由使用者按 Play 開始。來源暫時離線或曲目目前無法解析時，保留 session，恢復可用後再以穩定 TrackId 解析；native path 不得作為 session identity 或持久化權威。
- **歌詞：**支援本地 LRC、可取得的內嵌歌詞、網易雲與 QQ Provider、同步顯示、候選手動搜尋/指定及持久快取。Provider 與 UI、Library Core 分離，需有逾時、可取消與受控重試；網路失敗不得阻礙啟動、曲庫瀏覽、本地歌詞或播放。自動匹配須綜合標題、演出者、長度、專輯、版本與 feat. 資訊；低信心不自動套用。使用者指定與本地歌詞優先於網路自動結果。
- 歌詞匹配要有可解釋分數與信心門檻。標題、演出者權重高；長度差異有實質影響；專輯為低權重。正規化大小寫、全半形、空白、常見括號及 feat./featuring/ft.，但須保留 Live、Cover、Acoustic、Instrumental、TV Size、Remaster 等版本差異；手動指定須持久保存並優先於後續自動搜尋。播放期間不得批量觸發網路補全。
- **首頁文案：**不顯示無用途的宣傳式頂部標語、功能口號、裝飾唱片插圖或假輪播頁碼；主要頁面直接呈現實際曲庫、播放清單、播放或設定內容。
- **介面圖示：**Svelte 5 UI 的功能性圖示使用官方 `@tabler/icons-svelte-runes`；涵蓋播放、導覽、搜尋、排序、收藏、音量、來源與設定等語意圖示，保留既有操作、中文 `aria-label`／`title`、可讀尺寸與狀態差異。品牌標誌、歌曲封面、純裝飾波形及非圖示文字不替換。
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
- **App SQLite**保存內部 ID、啟動/離線顯示所需的曲目投影、來源映射、使用者覆寫、播放清單、歌詞/封面快取與同步狀態；播放 session（精確佇列 traversal、目前 entry 與位置）由 App SQLite 持久保存，偏好和媒體來源 registry 仍以版本化 `settings.json` 為權威。系統媒體索引用於確認來源現況，但不得覆蓋使用者資料。使用 migration、批次交易、索引與必要的 FTS；不得逐曲提交交易。快取有上限、可清除、可觀察。
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
- [ECHO NEXT 根目錄 LICENSE](https://github.com/Moekotori/ECHO/blob/main/LICENSE)標示 AGPL-3.0；本專案只參考其行為，不複製程式碼。若未來考慮採用其程式碼，須先評估 AGPL 義務與本專案分發方式。

### [Any Listen](https://github.com/any-listen/any-listen)

- 可研究 Provider／擴充套件分層、本地列表同步的操作體驗、現代 Web 播放器介面，以及跨模組共用資料模型；須自行驗證對本地優先與低資源用量的適用性。
- [Any Listen 授權檔](https://github.com/any-listen/any-listen/blob/main/LICENSE)以 AGPLv3 為基礎並加入商業使用限制；不要直接複製程式碼，除非已確認授權範圍及本專案分發方式可接受。

### [MusicPlayer2](https://github.com/zhongyang219/MusicPlayer2)

- 可研究 M3U/M3U8 匯入匯出行為、本地播放器功能完整度、網易雲／QQ 歌詞來源的使用體驗，以及 Windows 桌面播放控制細節；以行為測試重新實作，不移植其程式碼。
- [MusicPlayer2 授權檔](https://github.com/zhongyang219/MusicPlayer2/blob/master/LICENSE)為 GPLv3；如考慮使用其程式碼，須先評估 GPL 義務與本專案的授權策略。

## 工作方式與下一步

1. 開工先閱讀本文件、相關程式碼與 ADR；以最新需求、程式和測試結果修正過時敘述。較大修改先說明目標、落點層級與硬限制，區分平台硬限制、工程限制和慣例假設；比較至少兩種方案，必要時用原型或 benchmark 找瓶頸。重大架構變更寫入 `docs/adr/`，優先修正根因，不以 UI 權宜作法掩蓋後端問題。
2. 變更保持聚焦，不自行改變硬性需求或更換核心框架。引入依賴前確認必要性、重量、維護狀況與授權；參考外部專案時遵守上節邊界。臨時做法要標記 temporary、建立 issue/TODO 並避免成為長期依賴。
3. 每次修改後提交 Git，包含程式碼、`AGENTS.md`、雙語 README 與必要 ADR；執行暫存資料和使用者設定加入 `.gitignore`。每次專案修改完成後都必須自動執行 `npm run release:windows`，範圍包含程式碼、測試、設定與文件（含 `AGENTS.md`、README）。只有建置成功、來源 EXE 存在且固定輸出 `release\moemusicplayer.exe` 已更新並驗證成功，才能回報新版已產生；失敗時保留舊 EXE、如實回報失敗，不得宣稱已有新版。新文字檔優先 UTF-8 with BOM，除非工具鏈要求其他編碼或不支援 BOM。README 使用英文 `README.md` 與繁體中文 `README_TW.md`，頂端皆放 `[English](README.md) | [繁體中文](README_TW.md)`，內容保持同步且只寫使用者需要的用途、功能、安裝與基本用法。技術文件的時間紀錄必須帶時區。
4. 每次收尾更新本文件的**目前狀態、未解問題與下一步**；後續使用者新增的功能、限制與驗收需求必須同步納入本文件的當前需求與狀態，不得只留在對話，並移除已過時資訊；不累積版本歷史。需要重啟任何裝置時，先說明原因並取得使用者明確同意，或由使用者親自操作。

階段順序保留原規劃的範圍：

1. Tauri/Svelte 骨架、媒體資料抽象、Windows SystemIndex 與 Android MediaStore 適配、App SQLite、來源管理、Windows fallback、metadata、分頁列表與基本播放。
2. M3U/M3U8、搜尋/FTS、專輯與演出者分組、封面快取、佇列。
3. 本地與內嵌歌詞、窗口渲染、網易雲與 QQ Provider、可解釋匹配及手動覆寫。
4. Windows 系統整合、Android Media3/MediaSession 背景播放與 SAF 來源。
5. 核心穩定後評估無縫播放、進階音訊裝置、DSP/EQ、遠端曲庫/同步與插件；Phase 1 不先投入大量視覺特效。

目前可重現的驗證命令為 `npm run check`、`npm run build`、`node --test`、`node node_modules/vite/bin/vite.js build --config vite.now-playing-layout-harness.config.ts`（重建布局 harness）與 `node node_modules/vite/bin/vite.js build --config vite.volume-slider-harness.config.ts`（重建實際 App harness）、`playwright-cli -s=now-playing-layout open --browser msedge http://127.0.0.1:4174/tests/now-playing-layout-harness.html`、`playwright-cli -s=now-playing-layout run-code --filename=tests/now-playing-layout.playwright.js`、`playwright-cli -s=now-playing-overlay open --browser msedge http://127.0.0.1:4173/tests/volume-slider-harness.html`、`playwright-cli -s=now-playing-overlay run-code --filename=tests/now-playing-overlay.playwright.js`、`playwright-cli -s=now-playing-appearance run-code --filename=tests/now-playing-appearance.playwright.js`、`npm exec vite -- --host 127.0.0.1 --port 1450 --strictPort`（LyricsView harness 開發伺服器）、`playwright-cli -s=lyrics-view open --browser=msedge http://127.0.0.1:1450/tests/lyrics-view-harness.html` 與 `playwright-cli -s=lyrics-view run-code --filename=tests/lyrics-view.playwright.js`、`python -X utf8 tests/volume-slider-dom.py`、`cargo check -p moemusicplayer --locked`、`cargo test --workspace --locked --offline`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo fmt --all -- --check` 與 `npm run tauri -- build --ci`。本輪 `npm run check` 0 errors/warnings、Node 68/68 通過；後端歌詞設定驗證 settings 26/26 與嚴格 Tauri Clippy 通過；Headless Microsoft Edge Playwright 的 Now Playing A/B 12 尺寸幾何、三種來源比例各於三種 viewport 的 A/B 貼邊測試與實際 `App.svelte` overlay 四頁往返、四行曲目資訊、Hi-Res、兩種返回及曲庫列表捲動保留均通過；截圖為 `target/now-playing-layout-a-1920x1080.png`、`target/now-playing-layout-b-1366x768.png`、`target/lyrics-line-gap-settings.png` 與 `target/lyrics-transparent-row-gap.png`。真實 Tauri WebView 仍待人工驗收。最新 Rust workspace 驗證通過：Tauri 62、Core 44、DB 39（含 100,000 項 playback session reopen/traversal 測試）、Windows platform 27、Android media plugin 6、Windows audio 17 單元測試；另有 2 項 Windows audio system-media integration 與 4 項 Rodio pipeline integration 通過，1 項只讀 endpoint diagnostic ignored。嚴格 Clippy 與格式檢查通過。歌詞服務 fixture/IPC/SQLite 測試確認本機優先、手動與自動快取優先序、匹配門檻、逐時間戳輔助行、YRC/QRC 原始資料、取消及 response/query 數量界限；provider 測試不連真實端點。正式 Tauri WebView、實際 NetEase/QQ endpoint 行為、Windows SMTC 封面圖廣播已完成，隱藏 HWND 真 WinRT 測試已讀回有效 PNG stream 並確認缺圖清除；實際飛出面板呈現仍待人工驗收；SMTC flyout／硬體鍵與應用音訊輸出亦尚未驗收；Android playback 未實作。
