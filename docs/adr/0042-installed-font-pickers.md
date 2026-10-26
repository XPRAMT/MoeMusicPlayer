# 字型選擇（已安裝字型的掃描、快取、排序與套用）

Status: 已採納

## Context

使用者要能為主介面、播放頁歌詞、工作列歌詞（僅 Windows）與浮動歌詞各自選擇系統已安裝的字型。字型清單要以作業系統地區的語言顯示名稱、依地區排序，打開選單時不能每次重讀數百個字型檔，也不能在啟動時掃描。設計參考使用者的注音字型工具（只參考讀登錄與字型名稱表的做法，未複製程式碼）。

## 決策

**設定**：四個位置各自儲存「字型家族名稱」，`null`／空字串＝「預設字型」（即原本的字型，所以預設什麼都不變）：`settings.json` 的 `interfaceFontFamily`（`settings_get/set_interface_font`）、`lyricsPreferences.fontFamily`、`taskbarLyrics.fontFamily`、`floatingLyrics.fontFamily`。存的是 `canonical_name`（美式英文名稱，沒有就任一英文、Unicode 平台名稱、第一個名稱），與地區無關；CSS 與 DirectWrite 都比對所有語系的名稱。正規化（Rust `normalize_font_family`、JS `normalizeFontFamily`）只去頭尾空白、空字串或超過 256 字元視為 `null`；沒有安裝的字型照樣保留，套用時退回預設字型。字重仍由各頁既有的「字體粗細」決定。

**讀字型（`crates/player-fonts`，ttf-parser 0.25.1，MIT OR Apache-2.0）**：自己讀 sfnt／TTC 目錄（`sfnt.rs`），每個 face 只讀 `name`、`cmap`、`OS/2` 三張表（以 seek 讀，大型 CJK 檔也只讀幾百 KB），交給 ttf-parser 的表解析器。名稱：家族＝ID 16，沒有才用 ID 1；樣式＝ID 17，沒有才用 ID 2；完整名稱＝ID 4。Windows 平台的 Unicode 名稱以 LCID 記；Mac Roman 英文（純 ASCII）記為 0x0409；Unicode 平台名稱記為 0；其他編碼（Big5、GB）略過。`OS/2` 的 usWeightClass 與斜體位元；沒有 `OS/2` 時為 400、正體。`@` 開頭的直書家族不列。同家族（`canonical_name` 不分大小寫）的各樣式合成一個項目，face 依「正體、最接近 400」排序。

**書寫系統判斷（`scripts.rs`）**：除注音外每項都是「一組字元中至少有幾個」，缺幾個字的舊字型仍算數。常用字：繁＝的一是不了人我在有他這們中大來上國個到說時、簡＝同一組的簡體字形（这们来国个说时），各 21 字至少 18 個（判斷字型能否顯示一般中文）。區分字形是 200 組依實際頻率選出的繁簡對（`han_forms.rs`，見下）。繁體＝繁常用字＋200 個繁體字形至少 180 個（90 %）＋注音 ㄅ（U+3105）（不要求輕聲符 ˙：Zpix、仓耳今楷等繁體完整的字型沒有它；JIS X 0213 的日文字型如 Meiryo、Yu Gothic 也有大部分舊字形，但沒有注音；Big5／HKSCS 字型都有）。簡體＝簡常用字＋200 個簡體字形至少 180 個。兩者都成立的字型（GBK、方正「简繁」字型）照地區順序歸到最前面的一組（台灣＝繁中）；沒有另外「簡體字形全都有就要繁體字形也全都有」的例外。日文＝假名 あいうえおのをアイウエオンヴ至少 12 個＋漢字 日本人一大年中出時行会見至少 10 個。三者都沒有＝「只有英文」。

**繁簡字形表（`crates/player-fonts/src/han_forms.rs`）**：由 `node scripts/generate-han-script-sets.mjs <解開的 Unihan 資料夾> <Unihan.zip>` 產生並提交（Unihan 檔不提交），檔頭記 Unihan 版本與 zip 的 SHA-256；目前為 Unihan 18.0.0（Unicode License v3）。規則：簡體字依 `kHanyuPinlu`（各讀音次數加總）由高到低，只留不在 Big5（無 `kBigFive`）且有不同於自身的 `kTraditionalVariant` 者；其繁體取候選中在 Big5、不在 GB2312（無 `kGB0`）且最常用的一個：有 `kGradeLevel` 的優先（年級低者先），否則須在 Big5 常用字區 A440–C67E，都不符合就捨棄該對（例：当→當而非噹）；已被更常用的一對用掉的繁體字、以及雖在常用字區卻罕用的 吶（呐 的次數含語氣詞用法，繁體寫「呢」）也捨棄；取前 200 對。測試檢查 200 對、彼此不重複並含 這／这、說／说、當／当。改規則或換 Unihan 版本時重產並提高 `PARSE_VERSION`。

只有在 cmap 沒有任何 Unicode 子表（平台 0，或 Windows 編碼 1／10）時才看舊編碼子表：Windows ShiftJIS（3,2）與 Mac 日文（1,1）＝日文、Windows PRC／GB（3,3）與 Mac 簡中（1,25）＝簡中、Windows Big5（3,4）與 Mac 繁中（1,2）＝繁中；韓文編碼（Wansung 3,5、Johab 3,6、Mac 韓文 1,3）不標記，排在「只有英文」組。這類字型（例如长城粗圆体＝簡中、文鼎新藝體＝繁中）一律列出。

**不列出的字型**：有 Unicode cmap、含至少 16 個漢字（URO、擴充 A、擴充 B 以後，不含相容漢字區，因為圖示字型會用到）、注音或假名字母（不含 ・ ー 與濁點），但繁、簡常用字都不到 18 個、日文不成立、也沒有至少 12 個韓文音節（가나다…하）的字型，無法顯示一般文字：只有擴充 B 的字型（細明體-ExtB、SimSun-ExtB）、外字集（EUDC 補字）、只有注音的精靈文、只有假名的裝飾字型。不比對名稱。只有英文字母、圖示或私用區字形的字型照常列出；有漢字的韓文字型以常用字或韓文音節通過。判斷在 face 層級（`FaceInfo.hidden`），一個家族所有 face 都隱藏才隱藏（`FontFamily.hidden`，不序列化），`fonts_list` 與 `fonts-list-updated` 只送 `visible_families`；完整清單（含隱藏）仍用來解析已存的設定，選過的隱藏字型照常套用與繪製，只是選單不再提供（目前值顯示儲存的名稱）。

**地區（`region.rs`）**：用作業系統地區，不用 App 介面語言。Windows：`GetUserDefaultGeoName`，取不到再用 `GetUserDefaultLocaleName` 的地區；Android：外掛 `listSystemFonts` 回傳的 `Locale.getDefault().getCountry()`。分組順序：TW／HK／MO＝繁中 → 簡中 → 日文 → 只有英文；CN／SG＝簡中 → 繁中 → 日文 → 只有英文；JP＝日文 → 繁中 → 簡中 → 只有英文；其他＝只有英文 → 繁中 → 簡中 → 日文。一個字型支援多種時歸到該順序中最前面的一組（例：微軟雅黑在台灣歸繁中）。組內以專案的 ICU4X 自然排序（`Database::compare_music_text`）比較顯示名稱。顯示名稱的語言優先序：TW＝zh-TW 1028、zh-HK 3076、zh-MO 5124、zh-CN 2052、zh-SG 4100、en；HK／MO＝zh-HK、zh-MO、zh-TW、zh-CN、zh-SG、en；CN／SG＝zh-CN、zh-SG、zh-TW、zh-HK、zh-MO、en；JP＝ja 1041、en；其他＝en；之後任一英文、Unicode 平台名稱、第一個名稱。

**列出字型檔（`enumerate.rs`）**：Windows 讀 HKCU 與 HKLM 的 `Software\Microsoft\Windows NT\CurrentVersion\Fonts`，相對值分別對 `%LOCALAPPDATA%\Microsoft\Windows\Fonts` 與 `%WINDIR%\Fonts` 解析，只收 ttf／otf／ttc／otc，路徑不分大小寫去重。Android：掃 `/system/fonts`、`/product/fonts`、`/system_ext/fonts`、`/vendor/fonts`、`/data/fonts/files`（兩層），再加上外掛 `SystemFonts.getAvailableFonts()`（API 29 以上）的檔案。只取名稱與 stat，不開檔。

**快取（player-db schema v22–v24）**：`font_files`（路徑、大小、修改時間 UTC ms、`parse_version`）與 `font_faces`（路徑＋face index、名稱 JSON＝`FaceNames`、字重、斜體、書寫系統位元、`hidden`）。只存名稱與數字，不存字型內容；檔案是事實，快取只是索引。v22 為 `CREATE TABLE IF NOT EXISTS`，v23 只在欄位不存在時 `ADD COLUMN`，各為單一交易。`parse_version` 記錄解析該檔的 `player_fonts::PARSE_VERSION`（目前 2；v22 留下的列為 0）：版本不同的檔案在比對時視為已改變（修改時間當成不可能的值），下次開選單的背景更新就重新解析，消失時照樣刪除；之後改判斷規則只要遞增 `PARSE_VERSION`，不必改 schema。v24 的 `font_subsets` 見下方「名稱子集」。隱藏的 face 照樣存入快取，下次掃描不必重讀。`font_cache_apply` 在一個交易內刪除消失的檔案並取代重新解析的檔案的 face；沒有可用 face 的檔案仍記錄，直到它改變才再讀。

**流程（`src-tauri/src/fonts.rs`）**：只有打開選單時才呼叫 `fonts_list`，啟動時不掃描。**第一次開啟**（快取沒有任何可列出的字型，或沒有任何一項有子集：`needs_full_build`）不先回傳空清單：在命令內完成全部工作——列出目前的字型檔、解析、寫入快取、產生每個家族的名稱子集——才回傳完整清單（`refreshing` 為假），期間頁面顯示「讀取字型中…」（本機約 10 秒）；建置中不送任何部分清單，完成後另送一次 `fonts-list-updated` 給其他等待中的選單。**之後的開啟**立即回傳快取組成的清單（含子集），並在背景列出目前的字型檔、`diff_files` 比對大小與時間、只解析新增或改變的檔案、刪除消失的，送 `fonts-list-updated`（`changed` 為真時帶新清單；沒變時不帶清單，讓頁面結束讀取狀態），接著為新增或改變的家族產生名稱子集（見下）。首次建置與背景更新由 `FONT_WORK`（tokio Mutex）一次只跑一個：首次開啟等待執行中的工作結束後重新判斷；背景更新已在跑時，新的開啟只回傳快取並等它送結果。資料庫工作都很短（`run_database_work`），讀字型與寫子集在 blocking pool，不在資料庫工作內進行。流程寫成泛型的 `FontHost`（資料庫、blocking、目前的字型檔、子集資料夾、送清單），App 與測試（記憶體資料庫＋fixture 字型，`player-fonts` 的 `testing` feature）共用同一段程式。清單的每一項另帶 `subset`（子集檔路徑，有才帶）；Android 另帶 `systemDefault`。頁面就地替換清單，保留捲動位置與搜尋字。`fonts_resolve` 只讀快取，供 Android 啟動時套用；最近一次組好的清單留在記憶體供浮動歌詞解析。

**選單（`src/lib/FontPicker.svelte`，純函式在 `font-picker.js`、`font-row-faces.js`）**：四處共用一個元件（主介面在設定 → 主介面；三個歌詞分頁在「字體與大小」的第一項，字體大小之前）；清單、讀取狀態、更新與已註冊的列字型放在 `<script module>`，設定頁與快速設定的每個選單看到同一份。上方搜尋框（不分大小寫、全半形與空白，比對顯示名稱與儲存名稱），第一列永遠是預設字型。選了立即套用（即時預覽），選單保持開啟。

清單直接用曲庫的 `PagedVirtualList`（以記憶體陣列餵 `itemAt`，不另寫虛擬清單）：列高 36 px；分組列「繁體中文／簡體中文／日文／英文」放在每組第一個字型之前，順序就是後端的地區順序（`fontPickerGroups`），搜尋時沒有結果的組沒有分組列；分組列與曲庫相同（0.7 倍列高、至少 30 px，只在下方畫 1 px 線），點它開曲庫的快速查找面板（只列出目前有的組）跳到該組；方向鍵移動、Enter 選擇；捲動位置以每個選單自己的 list ID 保存，搜尋字改變時回到頂端，背景更新替換清單時以錨點保持位置。快速查找面板放在瀏覽器 top layer（`top-layer-popover.js` 的 `topLayerPopover`，`popover="manual"`）：快速設定抽屜有 backdrop-filter，會成為 fixed 元素的包含區塊、裁切與 backdrop root，曾讓面板位置偏移、巢狀模糊把整個抽屜洗白；top layer 不受這些影響，面板以視窗座標置於清單中央，外觀與曲庫相同（規則見 `src/lib/AGENTS.md`）。列沿用曲庫的 `track-row virtual-track-row` 樣式。

**列的字型**：每列以自己的字型繪製名稱，但有子集時不把列的 `font-family` 設成真正的家族：WebView 會在排版時同步載入整個字型檔（CJK 常為 10–40 MB），Release 實測以 400 px 一步捲動時大多 45 ms 一步，有些步卡住主執行緒 833–975 ms。列改用該家族的名稱子集：列第一次渲染時（`font-row-faces.js` 的 `createFontRowFaces().faceFor`）同步以 `new FontFace(專用名稱, url(convertFileSrc(subset)), { display: 'block' })` 加入 `document.fonts` 並開始載入，該列立即以它為 `font-family`；同一個子集（以路徑為鍵，跨選單與清單更新共用）只註冊一次。沒有延遲、沒有同時載入上限、沒有先用預設字型的階段：子集是本機約 5 KB 的檔案，載入在 block 期間內完成，列直接以自己的字型出現（載入失敗時退回預設字型）。選擇在列渲染時才註冊而不是清單一載入就全部註冊（約 650 個、共約 3 MB）：全部註冊並載入會在開啟時一次發出數百個 asset 請求與字型檢查，而大多數列從未捲到；渲染時註冊只花建立一個 FontFace 物件的時間，之前在 Chromium 以本機 687 個家族、每步 400 px 停 250 ms 捲完整份清單、共載入約 520 個子集時，沒有任何超過 50 ms 的長工作、每步最多 11 ms。沒有子集的列（舊編碼與符號字型，本機約 34 個；或新裝而背景尚未產生子集的字型）直接以真正的家族繪製（`"家族", 預設字型串`，使用者接受可能的短暫卡頓），子集產生後隨清單更新換成子集。目前值的標籤與套用到介面／歌詞的字型仍是真正的家族（Android 為下述 `@font-face`）。

**名稱子集**：`player_fonts::read_standalone_face` 以 seek 只讀該 face 的 head、hhea、hmtx、maxp、loca、glyf、CFF、cvt、fpgm、prep、name、post、cmap、OS/2，組成獨立字型（TTC 的其他 face 與 GSUB／GPOS、點陣、直書表留在磁碟）；`name_subset` 以 Unicode cmap 找出顯示名稱各字元的字形，交給 subsetter 0.2.6（MIT OR Apache-2.0，typst；不開 `variable-fonts`，所以不帶 skrifa／write-fonts，CFF2 不產生子集）留下這些字形（TrueType 含複合字形的元件），再補上 subsetter 為 PDF 而省略、瀏覽器需要的表：只含這些字元的 cmap（format 4，有 BMP 以外的字元時加 format 12）、原本的 OS/2、只留 Windows 平台 ID 0–6 的小 name 表；並清除簡單字形點旗標的保留位元 7（細明體等舊系統字型有設，瀏覽器的字型檢查〔OTS〕會拒絕，系統安裝的字型則不經檢查）。字型缺的字元不放進 cmap，由字型串的下一個字型繪製；沒有任何字元對得上（舊編碼字型、符號字型）、沒有 OS/2 或 name、或無法切割時不產生子集。檔案在 UserData `font-subsets/`，檔名是來源、face、大小、時間、顯示名稱與 `SUBSET_VERSION` 的雜湊（重做後網址不同，WebView 不會用到舊快取），只有這個資料夾加入 asset protocol scope（`allow_directory`，不含子資料夾）。schema v24 `font_subsets`（家族鍵、來源路徑與 face、來源大小與時間、顯示名稱雜湊、`subset_version`、子集檔名；無法切割時檔名為 NULL，來源或名稱沒變就不再試）。流程：只在選單開啟時（第一次開啟在回傳清單之前、之後的開啟在背景更新之後；從不在啟動時），`plan_subsets` 比對列出的家族（正體最接近 400 的 face、快取的檔案時間戳、目前地區的顯示名稱）與紀錄：新增、來源改變、顯示名稱改變（例如換地區）、版本不同或檔案不見的才產生，依清單順序每批 32 個；背景更新時每批寫入後重送 `fonts-list-updated`，新列就換成子集（第一次開啟不送，全部完成才回傳）；家族消失就刪紀錄，最後刪掉沒有紀錄指向的檔案。子集仍只是字型檔的快取：檔案是事實。本機量測（Release 範例 `cargo run --release -p player-fonts --example subset_fonts -- OUT_DIR`）：687 個家族產生 653 個子集，共約 3.1 MB（平均約 4.9 KB，最大 43 KB 的標楷體，因保留 fpgm／prep 等 hinting 表），34 個沒有（符號字型與舊編碼字型），從讀檔到寫完約 9 秒；全部子集都通過 Edge 的 FontFace 載入（OTS）。

**「預設（…）」**：「預設字型」列與未選擇時的目前值顯示預設實際繪製該目標 CJK 文字的家族（`defaultFontFamily`，以清單的顯示名稱，`tr('預設（{0}）')`；判斷不出時只顯示「預設字型」）：主介面＝預設字型串 `'Segoe UI', 'Microsoft JhengHei', sans-serif` 中第一個已安裝且有 CJK 字形的家族（Segoe UI 沒有 CJK 字形，Chromium 對中文與假名逐字往後找，落在微軟正黑體），沒有就第一個已安裝的，走到通用的 `sans-serif` 時用系統預設；播放頁歌詞跟隨主介面字型（已選且已安裝時就是它，`setInterfaceFontForPickers`），否則同主介面；Windows 工作列與浮動歌詞＝原生繪製的 fallback 中 CJK 對應的家族 Microsoft JhengHei（win.rs 的 `AddMapping`，Latin 為 Segoe UI；測試鎖定兩邊一致）；Android（WebView 與浮動歌詞）＝`fonts_list` 的 `systemDefault`：讀 `/system/etc/font_fallback.xml`（Android 15 起）或 `fonts.xml`，台灣／香港取 `zh-Hant`、中國取 `zh-Hans`、日本取 `ja` 的 fallback 家族（略過 `fallbackFor` 的襯線替身），其他地區取第一個 `sans-serif` 家族，再以檔名與 face index 對到清單（`player_fonts::android_default_font`、`family_of_system_font`）。

**套用**：
- 主介面：`<html>` 的 `--app-font-family`＝`"選擇的家族", 預設字型串`；`app.css` 的 `--app-font-stack: var(--app-font-family, var(--app-default-font))`，`:root` 以它為 `font-family`。字型不存在時瀏覽器自動往後退。
- 播放頁歌詞：`LyricsView` 根元素的 `--lyric-font-family`＝`"家族", var(--app-font-stack)`；未選時跟隨主介面字型。量測墨水行高的 canvas 也跟著字型重算。
- Windows 工作列與浮動歌詞（ADR 0034）：`Look.font_family` 交給 DirectWrite；渲染執行緒對每個家族只解析一次（系統字型集合的 `FindFamilyName`，含使用者安裝的字型；新家族時 `checkForUpdates`），並取該字型自己的 ascent／descent 作為 CSS 行框；找不到時以 Segoe UI 繪製。點陣快取鍵（跨視窗共用）與量測鍵都包含家族。CJK 缺字仍依既有 fallback（微軟正黑體、系統 fallback）。
- Android WebView：WebView 不能以名稱取用系統字型，所以以 FontFace API 為選用的家族註冊 `@font-face`（別名 `MoeFont <家族>`，每個 face 一條，帶字重與樣式），來源是 `convertFileSrc` 的 asset 網址。asset protocol 已啟用但設定中的 scope 為空，後端只把列出的字型檔與名稱子集資料夾加入 scope（`allow_file`、`allow_directory`）。選單的列與其他平台一樣用名稱子集（上述），只有目前值的標籤為選用的家族註冊；WebView 只在文字用到某個 face 時才讀檔。主介面與播放頁的字型在啟動時以 `fonts_resolve` 從快取取得檔案（不掃描）。
- Android 浮動歌詞：`floating_lyrics_android` 在轉送 `floatingLyricsUpdate` 前把 `settings.fontFamily` 解析成該家族正體 face 的檔案（`fontFaces`：path、faceIndex、weight；空＝預設字型）。Kotlin `LyricFonts` 取最接近字重的 face，以 `Typeface.Builder(File).setTtcIndex()`（API 26 以上；API 24–25 只能讀 TTC 的第一個 face）建立，再照原本方式套用字重；讀不到檔案時用預設字型。`fontFaces` 是 `LineLook` 與模型鍵的一部分，換字型會重新點陣化與量測。

## 限制與未驗證

- CSS `@font-face` 無法指定 TTC 的 face index：Android WebView 讀 TTC 檔時一律用第一個 face（例如 Noto Sans CJK 的 TTC 會用到第一個地區的字形，靠 `locl` 與頁面語言調整）。
- 家族名稱以 ID 16 為準；若某字型的 DirectWrite／WWS 家族名稱與 ID 16 不同（光學尺寸家族如 Sitka、Segoe UI Variable），Windows 上 CSS 與原生歌詞可能找不到而退回預設字型。
- 只讀 Unicode 編碼的名稱；只有 Big5／GB 編碼名稱的舊字型會以英文名稱顯示（长城粗圆体、文鼎新藝體的名稱表另有 Unicode 名稱，不受影響）。
- 沒有 Unicode cmap 的舊編碼字型照樣列出，但瀏覽器與 DirectWrite 是否能以它繪製中文未驗證。
- 沒有子集的列以真正的家族繪製，WebView 會在排版時同步載入整個字型檔（只影響舊編碼／符號字型與背景尚未產生子集的新字型）。
- 第一次開啟要等全部掃描與子集完成（本機約 10 秒）才顯示清單。
- 名稱子集依目前地區的顯示名稱產生；換地區後第一次開啟前，列仍用舊名稱的子集（缺的字由預設字型繪製），背景重做後更新。字型檔本身缺字形（例如某些字型的空白字形）時子集同樣是空白。
- 列與快速查找面板的行為、子集載入與捲動時間只在 Chromium（Playwright，`tests/font-picker.playwright.js` 的 harness，舊的延遲載入版本）驗證；改為渲染時立即註冊後未重量，也未在 WebView2 Release 與 Android WebView 實測；快速設定抽屜內的面板位置未實測。
- `systemDefault` 只在 Android 讀系統設定檔，未在裝置上驗證；Windows 的「預設（…）」以 win.rs 與 app.css 的固定字型串推得，系統未安裝微軟正黑體時退回下一個可判斷的家族或「預設字型」。
- 未實機驗收：Windows Release GUI 選單與即時套用、原生歌詞換字型；Android WebView 的子集與 `@font-face` 載入（asset scope）、浮動歌詞的 Typeface 建立。
