# 逐字歌詞與 SRT／ASS 字幕格式

狀態：部分採納。資料模型、yrc 逐字、SRT、ASS 子集、內容偵測、本機歌詞順序與播放頁逐字高亮已實作；Windows 原生與 Android 的逐字顯示只做研究（第 5 節），待使用者決定。

相關：[ADR 0029](0029-lyric-cue-lead.md)（換行提前）、[ADR 0030](0030-rust-chinese-script-conversion.md)（簡繁轉換）、[ADR 0032](0032-floating-lyrics.md)（浮動歌詞）、[ADR 0034](0034-native-lyrics-rendering-research.md)（原生繪製）、[ADR 0040](0040-library-sync-and-local-lyrics-data.md)（本機歌詞資料）

## 背景

模型原本只有行級時間。網易雲 provider 已取回 yrc（逐字），但只摺成行；QQ 的 qrc 是密文，以 base64 原樣保存。本機旁檔只認 `.lrc`，內嵌歌詞一律當 LRC 解析，所以 USLT 裡放 SRT／ASS 原文時，播放頁會顯示 `00:00:01,000 --> …`、`Dialogue:` 等原始行，而且這些文字還會擋住自動線上搜尋。

## 決策

### D1 本機歌詞順序（沒有手動選擇時）

1. 內嵌歌詞（音訊檔標籤：ID3 USLT、Vorbis `LYRICS`、MP4 `©lyr`、APEv2 `Lyrics`）。
2. 歌曲旁的同名旁檔（同資料夾、主檔名不分大小寫）：`.lrc` > `.ass` > `.srt`。
3. 都沒有才自動線上搜尋。

- 內容可解析就算有，純文字也算（純文字會贏過旁檔）；看起來像字幕卻解析不出任何行（`LyricsContent::Unusable`）算沒有，往下一個來源找。
- 手動選擇照舊固定。本機手動選擇只存來源參照，每次重讀。
- 歌詞選擇清單的本機候選也依這個順序排：內嵌在前，旁檔在後。
- 所有平台同一個順序：資料庫路徑（`local_lyrics_for_track`：`track_embedded_lyrics` → `track_lyric_sidecars`）、開檔備援（Windows `find_lyrics`／`find_all_lyrics`、Android `android_find_all_lyrics`、遠端 `remote_lyric_texts`）、遠端查找快取。

旁檔之間取 `.lrc` > `.ass` > `.srt`，理由：
- `.lrc` 是歌詞專用格式，最可能就是使用者為這首歌準備的；
- ASS 有 `\k` 逐字，又常帶雙語樣式；
- SRT 只有行級。

同一首歌有多個旁檔時，掃描器只記下排序最前的一個（`player_core::lyric_sidecar_rank`、`LYRIC_SIDECAR_EXTENSIONS`）。`track_lyric_sidecars` 一首一列、指紋含檔名，不需 schema 變更。這個排序不看內容：排序最前的旁檔即使解析不出歌詞，資料庫路徑也不會改用下一個；開檔備援路徑則會依序試下去。

### D2 內容偵測

`player_core::detect_lyric_format` 與 `parse_any_lyrics` 用於內嵌文字、旁檔與貼上／匯入，副檔名不參與判斷。偵測時先去 BOM、統一換行，再依序判斷：

1. 有 `[Script Info]`／`[Events]`／`Dialogue:` → ASS。
2. 有 `-->` → SRT（也接受 WebVTT 的 cue 版面）。
3. 行首 `[數字,數字]` 後接 `(` → yrc。
4. LRC 時間戳 → LRC。
5. 其餘 → 純文字。

ASS、SRT、yrc 解析不出任何行時回 `Unusable`。

### D3 SRT

- 每個 cue 是一行，保留起訖時間。
- 多行 cue 以空白接成一行；但某一行的文字系統與第一行明顯不同時（見 D9 的 `contrast_role`），從那一行起是額外行，依 D9 分類。
- 起點相同的不同 cue（成對 cue）合成一行，額外的 cue 依 D9 分類。
- 去掉 `<i>` 一類標記與 `{\an8}` 一類覆寫區塊，解碼 `&amp;` 等實體。
- 接受 BOM、CRLF、`.` 小數點、省略小時，也接受 cue 之間沒有空行。

### D4 ASS 子集（自寫）

- 只讀 `[Events]` 的 `Dialogue`，依 `Format:` 欄序取 `Start`／`End`／`Style`／`Text`；沒有 `Format:` 時用 v4+ 標準欄序。`Comment` 事件忽略。
- `\k`、`\K`、`\kf`、`\ko` 的時長（百分秒）成為逐字時間，以事件起點累加；第一個 `\k` 前的文字成為零長字。
- `\N`、`\n`、`\h` 變成空白（與 SRT 多行相同，以一行顯示）。
- 繪圖模式（`\p1`…）的內容丟棄，其餘覆寫標籤一律去掉。
- 起訖時間相差都在 ±50 ms 內、但 Style 不同的事件，是第一個事件的額外行，依 D9 分類（明確的 Style 名稱優先）；羅馬音的逐字存在 `romanization_words`。
- 不還原樣式。

函式庫查核：查 crates.io（只看中繼資料，未讀各 crate 的 LICENSE 檔，也未逐一確認 `\k` 支援）：

| crate | 版本 | 授權 | 備註 |
|---|---|---|---|
| `ass_parser` | 0.2.3 | MIT OR Apache-2.0 | 最後更新 2025-04 |
| `rassa-parse` | 0.7.1 | MIT | 2026-08 |
| `aspasia` | 0.2.1 | 0BSD | 2024-06 |
| `subtitler` | 2.9.0 | Apache-2.0 | 197 KB |

授權都可接受，但需求只是約 250 行的子集加逐字擷取。因此自寫（`crates/player-core/src/lyric_formats.rs`），不新增相依、不增加 Android 建置風險。

### D5 網易 yrc

- 端點改為 `/api/song/lyric/v1?…&lv=-1&kv=-1&tv=-1&rv=-1&yv=-1&ytv=-1&yrv=-1`，一次取回 `lrc`、`yrc`、`tlyric`、`romalrc`，以及對齊 yrc 的 `ytlrc`、`yromalrc`。
- `parse_yrc` 的規則：
  - 字時間是絕對毫秒；
  - 略過 JSON credit 行；
  - 字尾空白跟著字保留，所以行文字＝各字串接；
  - 零長字與只有時間的空字都保留；
  - 行尾＝max（行起＋行長，最後一字的結束）。
- yrc 取代 LRC 的條件（`yrc_matches_lrc`）：yrc 有逐字，行數至少是 LRC 的 80%（只計 yrc 第一行之後的 LRC 行），且最後一行起點相差小於 2 s。開頭的 credit 行時間在兩份資料中可以差數秒，所以不比對第一行。
- 譯文與羅馬音：先用 `ytlrc`／`yromalrc` 以行起點精確合併；仍空著的行，再以 `tlyric`／`romalrc` 找 ±300 ms 內最近的一行（`merge_lrc_auxiliary_nearest`）。
- `raw_karaoke` 保存原始 yrc（服務層包成 `{"yrc": …}` JSON）。讀取舊資料時，`restore_words_from_raw_karaoke` 依行起點與行文字把逐字補回，不改寫資料庫。

### D6 QQ qrc

不解密，QQ 維持行級。理由：繞過服務方的技術保護措施有法律疑慮（台灣著作權法第 80-2 條、DMCA §1201）；公開實作多為 GPL 或沒有授權；本專案原始碼公開。

App 實際送出的請求仍可用：`fcg_query_lyric_new.fcg?songmid=…&songid=…&qrc=1&trans=1&roma=1`，加上 Referer `https://y.qq.com/portal/player.html`。實測對搜尋得到的歌曲回 `retcode 0`，內含 LRC，`trans` 為空字串、沒有 `qrc` 欄位。不帶 Referer 時回 `-1310`。研究時 curl 得到的 `1101`，是未帶 App 的參數與 Referer 所致。

### D7 資料模型

- 新增 `LyricWord { start_ms, end_ms, text }`。
- `LyricLine` 新增 `end_ms`、`words`、`romanization_words`，都是 `serde(default)`，空值不序列化。舊的 `track_lyrics` JSON 原樣可讀，不需 schema 遷移。
- `has_word_synced_lyrics` 只看 `words`：QQ 候選不再被標為逐字。
- DTO：`LyricLineDto` 新增 `endMs`、`words: [{s,e,t}]`（短鍵）、`wordsTraditional`、`romanizationWords`。
- 簡繁轉換（`traditional_word_texts`）：整行轉一次再依 char 索引切回各字，保留詞組轉換（头发 → 頭髮）；字數改變時退回逐字轉換。有逐字時，`textTraditional` 等於轉換後各字串接。

### D8 播放頁逐字高亮

- 目前行維持白色（`--text`）。正在唱的那一個字以主要顏色 `var(--accent)` 由左到右掃過（柔邊各 12%），主題變更即時生效。已唱過的字維持主色直到換行（卡拉 OK；曾經是字結束就回白色，使用者認為閃得太快而改掉），尚未唱的字維持白色。`prefers-reduced-motion` 時不掃，字一開始唱整個字就是主色並維持。換行後舊行回到一般非目前行樣式。
- 沒有逐字的行維持整行高亮；其他行的顏色與透明度規則不變。
- 做法：目前行的每個字一個 `<span>`，各帶一個 CSS 動畫 `lyric-word-wipe`：
  - span 的背景是 `linear-gradient(主色 → --text)`，以 `background-clip:text` 裁到字形；漸層位置是註冊過的 `@property --lyric-word-fill`（app.css），keyframes 從 −12% 推到 112%，同時把 color 設成透明；
  - fill mode `forwards`：開始前文字是不透明的 `--text`、蓋住背景；結束後停在 112%（整字主色）。跳轉、暫停、偏移重建時，已過的字 delay ≤ −duration，一建好就是主色；
  - delay＝字起點−目前位置（可為負），duration＝字長；
  - 整段沒有逐幀寫樣式、span 上沒有 filter、沒有 `will-change`。陰影模式的 drop-shadow 仍在行上；描邊（`-webkit-text-stroke`）照常畫在填色上；`box-decoration-break: clone` 讓跨行的字每個片段各自掃。
- 譯文行永遠沒有逐字效果；羅馬音行只有在該行帶 `romanization_words`（DTO `romanizationWords`）時才有，規則與原文相同。
- 播放頁顯示順序為 原文 → 羅馬音 → 譯文。
- 只在換行、跳轉（時鐘 `jumpId`）、播放／暫停、偏移改變時重新計算並重建 span；暫停時 `animation-play-state: paused`。
- 使用者偏移與行相同；逐字不套用 200 ms 換行提前（提前只作用於行，ADR 0029）。
- 純函式在 `src/lib/lyrics-word-highlight.js`，測試在 `tests/lyrics-word-highlight.test.mjs`。
- 工作列、浮動歌詞與 Android 仍以行級顯示 SRT、ASS、yrc（它們讀同一份行資料）。

### D9 額外行的內容分類

LRC 同時間戳的行、SRT 多行 cue 與成對 cue、ASS 同時間不同 Style 的事件，第一行（檔案順序）是原文，其餘是額外行，依內容分成譯文或羅馬音（`crates/player-core/src/lyric_roles.rs`）。網易、QQ 另外帶來的譯文與羅馬音有明確標籤，不經這裡。

1. ASS 的 Style 名稱明確時優先（不分大小寫）：含 `roma`、`ローマ字`、`羅馬`、`罗马` 為羅馬音；含 `trans`、`中文`、`翻譯`、`翻译`、`譯文`、`译文` 為譯文。
2. 原文含漢字、假名或諺文，且額外行幾乎全是拉丁字（字母含長音與聲調符號、組合附加符號、空白、撇號、連字號、數字、基本標點）：羅馬音（拼音也算）；但其中出現兩個以上不同的英文功能詞（the、you、and、my、is…，刻意排除 to、me、in、no 等羅馬字也會用的詞）時是英文譯文。
3. 原文含漢字、假名或諺文，額外行也含：譯文。
4. 原文是拉丁字：額外行是譯文。
5. 內容無法判斷（只有標點、數字，或原文沒有文字）：沿用舊的位置規則（第一個額外行譯文、第二個羅馬音）；已被佔用時改放另一個空位。已有的角色保留，多出的行捨棄。
6. SRT 單一 cue 的多行，同文字系統的行是換行而非譯文；只有文字系統明顯不同（拉丁對亞洲字、亞洲字對拉丁、日文下沒有假名的漢字、韓文下沒有諺文的漢字）時，才從那一行起當額外行（`contrast_role`）。

增強型 LRC 寫入維持檔案順序（原文、譯文、羅馬音），讀回時依內容分類，結果相同。

## 第 5 節　研究：漸層字與原生逐字

### 5.1 播放頁的漸層掃色（CSS）

| 做法 | 能否純 CSS 動畫加 delay | CJK／換行 | 陰影／描邊 | 每幀成本 |
|---|---|---|---|---|
| `background-clip:text`＋`@property --p` 動畫漸層位置 | 可以 | 單一 span 的漸層以 inline box 計算，跨行斷開時每個片段各自從 0 開始，需改用 `box-decoration-break: clone` 且仍不連續 | 文字變成透明填色，`text-shadow` 會畫在文字下方而透出；描邊（`-webkit-text-stroke`）與背景裁切可並存 | 動畫期間該 span 每幀重繪（主執行緒 paint＋raster），一次一個字，面積小 |
| 亮色疊層＋`mask-image` 漸層位移 | 可以（`mask-position` 動畫） | 疊層須與底層同排版：重複一份文字並絕對定位，換行時兩份須完全一致 | 疊層與底層各自帶效果，陰影會疊兩次 | mask 動畫在 Chromium 多半走 paint，不在合成器上 |
| `clip-path: inset()` 動畫＋柔邊 | 可以，而且 `clip-path` 動畫可在合成器上執行 | 只能是矩形，跨行的字無法正確裁切 | `clip-path` 沒有柔邊，柔邊要再疊 mask，又回到 paint | 硬邊可在合成器上跑，近乎 0；柔邊則同 mask |

已採用第一列（見 D8）。陰影維持行上的 drop-shadow，span 不帶陰影。

量測（Chromium 測試頁：Edge 156、120 Hz、一行 24 個 CJK 字、34 px、行上兩層 drop-shadow，每字 350 ms 連續播放，每種模式 8 秒、三輪取一致值）：

| 指標 | 舊：逐字換色 | 漸層掃色 |
|---|---|---|
| 幀間隔 p50／p95 | 8.3／8.4 ms | 8.3／8.5 ms |
| 每幀 Paint 次數 | 0.05 | 1.8 |
| 每幀 Paint 時間 | 0.002 ms | 0.09 ms |
| 每幀 Raster 時間 | 0.004 ms | 0.10 ms |
| 每幀樣式重算 | 0.017 ms | 0.06 ms |
| 渲染主執行緒佔用 | 1.8% | 3.9% |

舊做法只在字的開始與結束各畫一次；掃色在唱中每幀重畫該字（含行上的 drop-shadow 區域），主執行緒多約 2 個百分點，沒有掉幀。Release 播放器（WebView2）未量測：量測當時播放器沒有開偵錯埠，依規則不重啟播放器；搭配連續歌詞視窗的實測仍待做。

### 5.2 Windows 原生與 Android 逐字

ADR 0034 的繪製器是 Windows.UI.Composition，以音訊時鐘每幀提交，不使用合成器關鍵影格動畫。以下方案都依這個規則：每幀由音訊時鐘算值，變化至少 1/8 裝置像素才提交。

**方案 A：每行一張亮色表面加移動裁切。**
- 每行已有的暗色表面外，再點陣化一張主色版本，疊在暗色表面上方。
- 每幀依音訊時鐘與各字 x 範圍（DirectWrite `HitTestTextRange`，載入時取得）算出裁切邊，變化 ≥ 1/8 裝置像素才 commit `InsetClip`，不用合成器關鍵影格。
- 只亮目前一個字時，裁切改成左右兩邊（目前字的 x 範圍）。
- 成本：記憶體加倍（每行兩張，共用快取也加倍）；每幀至多兩個值。陰影各自畫在兩張表面內；裁切邊會把陰影切成直線。

**方案 B：每字快取 sprite。**
- 每個字在載入時依整行版面點陣化白、主兩份，目前字只切 opacity；漸層時在主色 sprite 上放靜態漸層遮罩，每幀位移（≥ 1/8 px 才提交）。
- 數量：一行 10–30 字，每個視窗通常同時看得到 3–9 行，連續模式約 30–270 字 × 2 個 visual；記憶體約等於方案 A 再加每字邊距。
- 每幀 CPU：一個 opacity 或 offset；DWM：visual 多時合成成本上升。
- 缺點：
  - 陰影與描邊在字界要有邊距，否則相鄰 sprite 疊合時出現接縫或重複陰影；
  - 拉丁字母的字距與 kerning 要以整行版面的位置放置，不能各自排版；
  - 連續滑動與邊緣淡出套在父層，不受影響，但 visual 數目會讓共用快取的效益下降。

**方案 C：以字為節點的置中滑動加靜態置中遮罩（使用者提出）。**
- 連續滑動的節點從「行起點」改為「字起點」，讓正在唱的字恰好停在面板中心；曲線沿用單調 C1 與漂移修正。
- 不畫逐字、不需要主色副本，每行表面維持原狀；每幀仍只提交一個位移。
- 遮罩完全靜態，只在字級、面板尺寸、DPI 或設定改變時重建：
  - 中心 1 em（主行字級）為 100% 不透明；
  - 左右各 1 em 為「非目前歌詞透明度」（`inactiveOpacityPercent`，預設 50%）；這個寬度是主 session 的假設，待使用者確認；
  - 其外線性淡到面板兩端為 0。
- 頻帶交界建議做 0.1 em 的短柔邊，避免硬切在字形中間出現鋸齒。
- 每字調整漸層停駐點的設計已放棄：遮罩寬度不跟著目前字寬（CJK 一字與英文單字寬度差很多）。
- 譯文列建議用同一張遮罩，但以自己的字級換算 1 em，並跟著主列的位移（譯文同寬格，與現行連續模式一致）。

方案 C 的影響：
- 可讀性：之後的字在右側淡出，越遠越看不清；淡出寬度太窄會讀不到下一個詞。
- 速度變化：字密集或時間不均時滑動速度跳動大，需要限制最大速度與加速度（例如最大速度＝2 倍行平均速度），或對節點做移動平均。
- 沒有逐字的行退回行節點，與現在相同。
- 「文字排版」對齊設定在此模式下無意義（永遠置中），應停用或忽略。
- 工作列寬度窄時，1 em＋兩側 1 em 已佔不小比例，淡出區變短。

**Android**：每行快取 Bitmap，亮色以 `canvas.clipRect` 或平移的 `LinearGradient` shader 繪製；方案 C 的遮罩是一個 `LinearGradient`（同樣的頻帶），只在尺寸改變時重建。純函式與 Rust 共用 fixture。

**比較與建議**：
- 方案 C 每幀成本最低，也不增加記憶體，但改變閱讀方式；
- 方案 A 保留現有版面，成本是記憶體加倍；
- 方案 B 的 visual 數與接縫問題最多，不建議。

建議先決定是否要原生逐字變色。若只要「目前字在中心、兩側淡出」，採方案 C；若要逐字變色，採方案 A。

### 待使用者決定

- 原生要採方案 A 或 C；方案 C 另需決定：
  - 淡出寬度與兩側 1 em 頻帶是否做成設定；
  - 置中遮罩要取代還是疊加現有的邊緣淡出；
  - 原生是否也改變目前字的顏色。
- 歌曲資訊面板「導入使用中歌詞」遇到逐字歌詞時寫入什麼：增強型 LRC、SYLT，或只寫行級（目前只寫行級）。
- 增強型 LRC、WebVTT、TTML、SYLT 尚未實作。

## 測試

- `player-core`：
  - `lyric_formats` 單元測試：SRT、ASS、偵測、Unusable、旁檔排序；
  - `lyrics` 單元測試：yrc 逐字、空白、零長字、ytlrc 與 tlyric 合併、舊 JSON 相容、簡繁對齊；
  - 忽略測試 `tests/lyrics_samples.rs`：以 `MOE_LYRICS_SAMPLES` 讀網易回應，只檢查結構性質，樣本不提交。
- `player-platform-windows`：內嵌優先、旁檔排序、壞掉的內嵌字幕改用旁檔、遠端順序。
- `src-tauri`：
  - 服務層的內嵌字幕與 Unusable 改用旁檔；
  - yrc 取代 LRC；
  - 逐字旗標。
- 前端：`tests/lyrics-word-highlight.test.mjs` 檢查時間計算、簡繁、掃色 keyframes 與 `@property`、沒有 fill／filter、減少動態時改用換色、羅馬音逐字與顯示順序。
- `player-core` 的 `lyric_roles` 測試：日文＋羅馬字＋中文兩種順序、長音、韓文＋羅馬字＋英文、英文原文＋中文、中文＋拼音（含組合聲調）、Style 提示、無法判斷時的位置規則；`lyric_formats` 與 `lyrics` 另測 SRT 多行與成對 cue、ASS Style、LRC 兩種順序與寫入格式讀回。

## 參考資料

- ASS 覆寫標籤（`\k` 系列）：https://aegisub.org/docs/latest/ass_tags/
- SubRip：https://en.wikipedia.org/wiki/SubRip
- WebVTT：https://www.w3.org/TR/webvtt1/
- 網易 yrc、QQ qrc：沒有官方規格，依實際回應整理。
