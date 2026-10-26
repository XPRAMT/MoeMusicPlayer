# 字型選擇（已安裝字型的掃描、快取、排序與套用）

Status: 已採納

## Context

使用者要能為主介面、播放頁歌詞、工作列歌詞（僅 Windows）與浮動歌詞各自選擇系統已安裝的字型。字型清單要以作業系統地區的語言顯示名稱、依地區排序，打開選單時不能每次重讀數百個字型檔，也不能在啟動時掃描。設計參考使用者的注音字型工具（只參考讀登錄與字型名稱表的做法，未複製程式碼）。

## 決策

**設定**：四個位置各自儲存「字型家族名稱」，`null`／空字串＝「預設字型」（即原本的字型，所以預設什麼都不變）：`settings.json` 的 `interfaceFontFamily`（`settings_get/set_interface_font`）、`lyricsPreferences.fontFamily`、`taskbarLyrics.fontFamily`、`floatingLyrics.fontFamily`。存的是 `canonical_name`（美式英文名稱，沒有就任一英文、Unicode 平台名稱、第一個名稱），與地區無關；CSS 與 DirectWrite 都比對所有語系的名稱。正規化（Rust `normalize_font_family`、JS `normalizeFontFamily`）只去頭尾空白、空字串或超過 256 字元視為 `null`；沒有安裝的字型照樣保留，套用時退回預設字型。字重仍由各頁既有的「字體粗細」決定。

**讀字型（`crates/player-fonts`，ttf-parser 0.25.1，MIT OR Apache-2.0）**：自己讀 sfnt／TTC 目錄（`sfnt.rs`），每個 face 只讀 `name`、`cmap`、`OS/2` 三張表（以 seek 讀，大型 CJK 檔也只讀幾百 KB），交給 ttf-parser 的表解析器。名稱：家族＝ID 16，沒有才用 ID 1；樣式＝ID 17，沒有才用 ID 2；完整名稱＝ID 4。Windows 平台的 Unicode 名稱以 LCID 記；Mac Roman 英文（純 ASCII）記為 0x0409；Unicode 平台名稱記為 0；其他編碼（Big5、GB）略過。`OS/2` 的 usWeightClass 與斜體位元；沒有 `OS/2` 時為 400、正體。`@` 開頭的直書家族不列。同家族（`canonical_name` 不分大小寫）的各樣式合成一個項目，face 依「正體、最接近 400」排序。

**書寫系統判斷（`scripts.rs`）**：每項都是「一組字元中至少有幾個」，缺一兩個字的舊字型仍算數。常用字：繁＝的一是不了人我在有他這們中大來上國個到說時、簡＝同一組的簡體字形（这们来国个说时），各 21 字至少 18 個。繁體＝繁常用字＋繁體字形 體邊說們這嗎妳戶至少 7 個＋注音 ㄅㄆㄇㄈㄉㄊ至少 4 個（JIS X 0213 的日文字型如 Meiryo、Yu Gothic 也有這些舊字形，但沒有注音；Big5／HKSCS 字型都有注音）；簡體字形 体边说们这吗书东**全都有**的字型要繁體字形也全都有才算繁體（GBK 字型有注音與大部分繁體字形，方正字型缺說或戶，仍只算簡體）。簡體＝簡常用字＋簡體字形至少 7 個。日文＝假名 あいうえおのをアイウエオンヴ至少 12 個＋漢字 日本人一大年中出時行会見至少 10 個。三者都沒有＝「只有英文」。

只有在 cmap 沒有任何 Unicode 子表（平台 0，或 Windows 編碼 1／10）時才看舊編碼子表：Windows ShiftJIS（3,2）與 Mac 日文（1,1）＝日文、Windows PRC／GB（3,3）與 Mac 簡中（1,25）＝簡中、Windows Big5（3,4）與 Mac 繁中（1,2）＝繁中；韓文編碼（Wansung 3,5、Johab 3,6、Mac 韓文 1,3）不標記，排在「只有英文」組。這類字型（例如长城粗圆体＝簡中、文鼎新藝體＝繁中）一律列出。

**不列出的字型**：有 Unicode cmap、含至少 16 個漢字（URO、擴充 A、擴充 B 以後，不含相容漢字區，因為圖示字型會用到）、注音或假名字母（不含 ・ ー 與濁點），但繁、簡常用字都不到 18 個、日文不成立、也沒有至少 12 個韓文音節（가나다…하）的字型，無法顯示一般文字：只有擴充 B 的字型（細明體-ExtB、SimSun-ExtB）、外字集（EUDC 補字）、只有注音的精靈文、只有假名的裝飾字型。不比對名稱。只有英文字母、圖示或私用區字形的字型照常列出；有漢字的韓文字型以常用字或韓文音節通過。判斷在 face 層級（`FaceInfo.hidden`），一個家族所有 face 都隱藏才隱藏（`FontFamily.hidden`，不序列化），`fonts_list` 與 `fonts-list-updated` 只送 `visible_families`；完整清單（含隱藏）仍用來解析已存的設定，選過的隱藏字型照常套用與繪製，只是選單不再提供（目前值顯示儲存的名稱）。

**地區（`region.rs`）**：用作業系統地區，不用 App 介面語言。Windows：`GetUserDefaultGeoName`，取不到再用 `GetUserDefaultLocaleName` 的地區；Android：外掛 `listSystemFonts` 回傳的 `Locale.getDefault().getCountry()`。分組順序：TW／HK／MO＝繁中 → 簡中 → 日文 → 只有英文；CN／SG＝簡中 → 繁中 → 日文 → 只有英文；JP＝日文 → 繁中 → 簡中 → 只有英文；其他＝只有英文 → 繁中 → 簡中 → 日文。一個字型支援多種時歸到該順序中最前面的一組（例：微軟雅黑在台灣歸繁中）。組內以專案的 ICU4X 自然排序（`Database::compare_music_text`）比較顯示名稱。顯示名稱的語言優先序：TW＝zh-TW 1028、zh-HK 3076、zh-MO 5124、zh-CN 2052、zh-SG 4100、en；HK／MO＝zh-HK、zh-MO、zh-TW、zh-CN、zh-SG、en；CN／SG＝zh-CN、zh-SG、zh-TW、zh-HK、zh-MO、en；JP＝ja 1041、en；其他＝en；之後任一英文、Unicode 平台名稱、第一個名稱。

**列出字型檔（`enumerate.rs`）**：Windows 讀 HKCU 與 HKLM 的 `Software\Microsoft\Windows NT\CurrentVersion\Fonts`，相對值分別對 `%LOCALAPPDATA%\Microsoft\Windows\Fonts` 與 `%WINDIR%\Fonts` 解析，只收 ttf／otf／ttc／otc，路徑不分大小寫去重。Android：掃 `/system/fonts`、`/product/fonts`、`/system_ext/fonts`、`/vendor/fonts`、`/data/fonts/files`（兩層），再加上外掛 `SystemFonts.getAvailableFonts()`（API 29 以上）的檔案。只取名稱與 stat，不開檔。

**快取（player-db schema v22、v23）**：`font_files`（路徑、大小、修改時間 UTC ms、`parse_version`）與 `font_faces`（路徑＋face index、名稱 JSON＝`FaceNames`、字重、斜體、書寫系統位元、`hidden`）。只存名稱與數字，不存字型內容；檔案是事實，快取只是索引。v22 為 `CREATE TABLE IF NOT EXISTS`，v23 只在欄位不存在時 `ADD COLUMN`，各為單一交易。`parse_version` 記錄解析該檔的 `player_fonts::PARSE_VERSION`（目前 2；v22 留下的列為 0）：版本不同的檔案在比對時視為已改變（修改時間當成不可能的值），下次開選單的背景更新就重新解析，消失時照樣刪除；之後改判斷規則只要遞增 `PARSE_VERSION`，不必改 schema。隱藏的 face 照樣存入快取，下次掃描不必重讀。`font_cache_apply` 在一個交易內刪除消失的檔案並取代重新解析的檔案的 face；沒有可用 face 的檔案仍記錄，直到它改變才再讀。

**流程（`src-tauri/src/fonts.rs`）**：只有打開選單時才呼叫 `fonts_list`，啟動時不掃描。它立即回傳快取組成的清單（第一次為空，頁面顯示「讀取字型中…」），並在背景（同時只有一個）列出目前的字型檔、`diff_files` 比對大小與時間、只解析新增或改變的檔案、刪除消失的，最後送 `fonts-list-updated`（`changed` 為真時帶新清單；沒變時不帶清單，讓頁面結束讀取狀態）。頁面就地替換清單，保留捲動位置與搜尋字。`fonts_resolve` 只讀快取，供 Android 啟動時套用；最近一次組好的清單留在記憶體供浮動歌詞解析。

**選單（`src/lib/FontPicker.svelte`，純函式在 `font-picker.js`）**：四處共用一個元件；清單、讀取狀態與更新放在 `<script module>`，設定頁與快速設定的每個選單看到同一份。上方搜尋框（不分大小寫、全半形與空白，比對顯示名稱與儲存名稱），第一列永遠是「預設字型」，每列以該字型繪製自己的名稱；只繪製看得見的列（固定列高 36 px 的視窗化清單）。選了立即套用（即時預覽），選單保持開啟。

**套用**：
- 主介面：`<html>` 的 `--app-font-family`＝`"選擇的家族", 預設字型串`；`app.css` 的 `--app-font-stack: var(--app-font-family, var(--app-default-font))`，`:root` 以它為 `font-family`。字型不存在時瀏覽器自動往後退。
- 播放頁歌詞：`LyricsView` 根元素的 `--lyric-font-family`＝`"家族", var(--app-font-stack)`；未選時跟隨主介面字型。量測墨水行高的 canvas 也跟著字型重算。
- Windows 工作列與浮動歌詞（ADR 0034）：`Look.font_family` 交給 DirectWrite；渲染執行緒對每個家族只解析一次（系統字型集合的 `FindFamilyName`，含使用者安裝的字型；新家族時 `checkForUpdates`），並取該字型自己的 ascent／descent 作為 CSS 行框；找不到時以 Segoe UI 繪製。點陣快取鍵（跨視窗共用）與量測鍵都包含家族。CJK 缺字仍依既有 fallback（微軟正黑體、系統 fallback）。
- Android WebView：WebView 不能以名稱取用系統字型，所以以 FontFace API 註冊 `@font-face`（別名 `MoeFont <家族>`，每個 face 一條，帶字重與樣式），來源是 `convertFileSrc` 的 asset 網址。asset protocol 已啟用但設定中的 scope 為空，後端只把列出的字型檔加入 scope（`asset_protocol_scope().allow_file`）。選單只為看得見的列註冊；WebView 只在文字用到某個 face 時才讀檔。主介面與播放頁的字型在啟動時以 `fonts_resolve` 從快取取得檔案（不掃描）。
- Android 浮動歌詞：`floating_lyrics_android` 在轉送 `floatingLyricsUpdate` 前把 `settings.fontFamily` 解析成該家族正體 face 的檔案（`fontFaces`：path、faceIndex、weight；空＝預設字型）。Kotlin `LyricFonts` 取最接近字重的 face，以 `Typeface.Builder(File).setTtcIndex()`（API 26 以上；API 24–25 只能讀 TTC 的第一個 face）建立，再照原本方式套用字重；讀不到檔案時用預設字型。`fontFaces` 是 `LineLook` 與模型鍵的一部分，換字型會重新點陣化與量測。

## 限制與未驗證

- CSS `@font-face` 無法指定 TTC 的 face index：Android WebView 讀 TTC 檔時一律用第一個 face（例如 Noto Sans CJK 的 TTC 會用到第一個地區的字形，靠 `locl` 與頁面語言調整）。
- 家族名稱以 ID 16 為準；若某字型的 DirectWrite／WWS 家族名稱與 ID 16 不同（光學尺寸家族如 Sitka、Segoe UI Variable），Windows 上 CSS 與原生歌詞可能找不到而退回預設字型。
- 只讀 Unicode 編碼的名稱；只有 Big5／GB 編碼名稱的舊字型會以英文名稱顯示（长城粗圆体、文鼎新藝體的名稱表另有 Unicode 名稱，不受影響）。
- 沒有 Unicode cmap 的舊編碼字型照樣列出，但瀏覽器與 DirectWrite 是否能以它繪製中文未驗證。
- 未實機驗收：Windows Release GUI 選單與即時套用、原生歌詞換字型；Android WebView 的 `@font-face` 載入（asset scope）、浮動歌詞的 Typeface 建立。
