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

- 目前行維持白色（`--text`）。一整行只有**一條**主色 `var(--accent)` 的邊界由左到右掃過（柔邊各 0.15 em），沿 D10 的卡拉 OK 平滑曲線移動，主題變更即時生效。已唱過的部分維持主色直到換行（卡拉 OK；曾經是字結束就回白色，使用者認為閃得太快而改掉），尚未唱的維持白色。`prefers-reduced-motion` 時不掃，字一開始唱整個字就是主色並維持（硬邊）。換行後舊行回到一般非目前行樣式。
- 沒有逐字的行維持整行高亮；其他行的顏色與透明度規則不變。
- 做法（`lyrics-word-highlight.js` 的 `karaokeLineAnimation`，`LyricsView.svelte` 的 `karaokeLine`）：目前行的文字放在一個行內 `<span class="lyric-karaoke">`，裡面每個字一個只用來量寬度的 `<span>`：
  - 外層 span 的背景是 `linear-gradient(主色 → --text)`，以 `background-clip:text` 裁到字形；漸層位置是註冊過的 `@property --lyric-line-fill`（app.css），keyframes `lyric-line-wipe` 從 0% 推到 100%，同時把 color 設成透明；`box-decoration-break: slice` 讓換行的各片段共用一條背景，所以邊界依閱讀順序跨行移動，座標是「該處之前的字寬占整行字寬的比例」；
  - 時間函數是 `linear(…)`：建行時量一次各字寬（`getClientRects` 加總），算出 D10 的 `karaoke_glide`，二次段每段取 4 點、直線段取端點，寫成 `linear()` 的點；減少動態時改成在每個字起點跳一階；
  - delay＝第一個字起點−目前位置（可為負），duration＝到最後一個字結束；fill mode `forwards`：開始前文字是不透明的 `--text`、蓋住背景；結束後停在 100%（整行主色）。跳轉、暫停、偏移重建時直接從曲線上的位置接著走；
  - 整段沒有逐幀寫樣式、沒有逐字動畫、span 上沒有 filter、沒有 `will-change`。陰影模式的 drop-shadow 仍在行上；描邊（`-webkit-text-stroke`）照常畫在填色上。
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

### 5.3 定案與實作（D10）

**連續模式：方案 C。**（Windows 原生工作列與浮動歌詞、Android 浮動歌詞）
- 行起點是硬節點：該行第一個字形的中心在行起點時剛好落在歌詞區中心（行級歌詞也一樣，`StripLayout.anchors`／JS `layoutLyricsStrip` 第 4 參數／Kotlin `firstGlyphCenters`，取代原本的左緣）。逐字行之內的字起點（字中心）只是軟導引：兩個行起點之間的曲線跟著字走，但速度維持在該行平均速度（本行起點位置到下一行起點位置的距離 ÷ 兩者時間差）的 0.5–1.5 倍，不會突然衝刺也不會停住，正在唱的字可以稍微偏離中心（`word_strip_glide`，見下方「平滑曲線」；選項 B，取代原本的字起點節點＋2 倍速度上限＋最多提前 600 ms）。沿用音訊時鐘的漂移修正（`plan::StripInput.word_glide`，下次重排在下一個曲線段）。最後一行有字時停在最後一個字起點；行級歌詞仍用原本的 PCHIP 行曲線（只改錨點）。
- 遮罩（實驗性，使用者核准；若不滿意可整筆還原提交 a457599）：固定在歌詞區、不隨文字移動的靜態遮罩（`centre_mask_stops(W, em, a)`，Rust／JS／Kotlin 共用向量 `centreMask`）。W＝歌詞區寬、e＝主行字級 1 em、a＝「非目前歌詞透明度」÷100：x 在 [0, e] 由 0 直線升到 a，[e, W/2 − e/2] 由 a 直線升到 1，中心 1 em 為 1，右側對稱。歌詞區窄於 4 em 時 e 改用 W/4，各段長度仍為正、往兩端單調遞減、不超過 1；a＝1 時中間全平，只剩兩端 1 em 淡出。取代先前的置中遮罩（中心 1 em 不透明、直線降到兩端 0、不套用非目前透明度）。
- 行級與逐字歌詞都用這張遮罩，取代邊緣淡出與行淡化：連續模式下沒有每行的非目前透明度權重與 200 ms 過渡，所有格的透明度係數為 1，透明度只由位置決定。
- 譯文列沒有自己的節點：它是同一條捲動條的格，跟著主列的位移，並用同一張遮罩（以主列的 em 計算）。
- 羅馬音列只有在 `romanizationWords` 存在時才有逐字效果；原生視窗目前不顯示羅馬音列。
- Windows 遮罩是 `CompositionLinearGradientBrush` 的停駐點（`Mask::Centre`，與邊緣淡出共用 `FadeHost`）；Android 是每列一個 `LinearGradient`，以 `saveLayer`＋`DST_IN` 套用（連續模式時關閉 `EdgeFadeLayout` 的邊緣淡出），只在寬度、字級或非目前透明度改變時重建；Windows 的 `FadeHost` 鍵也含非目前透明度。

**「無」模式：卡拉 OK（方案 A）。**
- 套用「文字排版」對齊。
- 已唱的部分用主題主色，還沒唱的用文字顏色；一整行只有一條邊界，沿一條平滑曲線（`karaoke_glide`）移動：第一個字起點時在第一個字的左緣、最後一個字結束時在最後一個字的右緣（整行亮）是硬節點，中間以每個字的起點（左緣）與結束（右緣）為軟導引，速度維持在整行平均掃描速度的 0.5–1.5 倍；開始前停在第一個字左緣，結束後整行主色。與播放頁共用同一條曲線（播放頁以字寬比例為座標）。
- Windows：每行快取白色與主色兩張表面（版面相同，陰影／描邊的顏色都取自文字顏色，所以兩張的陰影一樣），兩個同位置 sprite 加互補 `InsetClip`：主色那張從行首切到邊界，白色那張從邊界到行尾（`WordClip`）。陰影沒有接縫；不在每幀點陣化；邊界移動至少 1/8 裝置像素才提交。下一行的主色副本會預先點陣化。主色由主視窗以 `lyrics-accent-color` 廣播。
- Android：主色副本另存一份 `LineRasterCache`，以 `clipRect` 分兩段畫；邊界移動至少 1 個裝置像素才重畫。主色經 `floatingLyricsUpdate { accentColor }` 送達；行資料帶 `words`（`{s, e, t}`）與 `lead`（前導空白的 UTF-16 長度）。
- 譯文列不變色；沒有逐字的行整行顯示；長句橫向捲動照常（裁切掛在 sprite／畫布上，跟著捲動位移）。

**量測**（`--example demo`，3 個視窗：1 個浮動、2 個工作列樣式，都在 120 Hz 主螢幕（3840×2160）；release 建置，每段 32 秒、取中間 20 秒；`MOE_DEMO_WORDS=1` 為每行 80% 逐字）：

| 情境 | 本程序 CPU | 點陣快取（20 秒後） | 每視窗提交 | 超過 1.5 幀的取樣間隔（每視窗） |
|---|---|---|---|---|
| 無，行級 | 0.70% | 12.2 MiB | 只有長句捲動 | 6 |
| 無，卡拉 OK | 1.56% | 23.0 MiB | 唱字時約每幀 1 次（約 95/s） | 6 |
| 連續，行級 | 1.40% | — | 每幀 | 4–6 |
| 連續，逐字（方案 C） | 3.12% | — | 每幀 | 7–9 |

- 卡拉 OK 的點陣快取約加倍（每行兩張）；本程序 CPU 增加約 0.9 個百分點，來自唱字期間每幀的裁切提交。換字時沒有額外掉幀：唱字期間邊界連續移動，換字不會讓取樣間隔變長。
- dwm 的 CPU 量不到：一般權限讀不到 dwm 的處理器時間。只在 120 Hz 主螢幕量過；60 Hz 虛擬螢幕沒有量。
- 連續模式加卡拉 OK（方案 C＋A）的估計：看得到的每行都要主色副本，點陣快取約加倍（每視窗再加約 10 MiB）；位移本來就每幀提交，正在唱的那格多一個裁切值，CPU 估計再增加不到 1 個百分點。尚未實作，由使用者決定。

**平滑曲線**（`crates/player-lyrics-rules/src/glide.rs` 的 `smooth_glide`；JS 參考實作 `src/lib/lyrics-glide.js`、Kotlin `WordLyricsRules.smoothGlide`，共用向量 `glide`／`karaokeGlide`／`stripGlide`）：
- 輸入是硬節點 `(t, v)`（時間嚴格遞增，同時或更早的後者丟掉，值非遞減）、每段之間的軟導引，以及兩端是否緩起緩停（捲動條：兩端速度 0，條在曲線外靜止；卡拉 OK：兩端用該段平均速度）。
- 每段平均速度 a，速度帶 [0.5a, 1.5a]（`GLIDE_MIN_SPEED`／`GLIDE_MAX_SPEED`）。每個硬節點有切線 m：兩鄰段的調和平均夾進兩段速度帶的交集；兩段相差超過 3 倍、交集為空時取較慢段的上限 1.5·min。硬節點兩側各有一段斜率 m 的直線，長度為較短鄰段的 1/8（`GLIDE_KNOT_WINDOW`）。
- 折線：從硬節點窗口的端點出發，每個導引的值夾進「從前一點以速度帶能到達」與「還能以速度帶到達段尾窗口」兩個範圍的交集（必不為空），所以每一小段斜率都在速度帶內，且單調。
- 除硬節點外每個轉角以對稱二次曲線圓角，半徑為兩鄰小段較短者的一半：速度在兩斜率間線性變化，不離開速度帶、連續（C1）；硬節點不圓角，值剛好命中。導引點會被圓角偏離最多 r·Δs/4。
- 輸出為分段多項式 `GlidePiece { t, v, s, c }`（值 `v + s·u + c·u²`），`glide_at` 給值、速度與半加速度；Windows 計畫把每段轉成 `AddCubic` 鍵（c3＝0），重排在 `next_glide_break`；Android 每幀讀 `glideValue`。
- 例外（測試明列）：緩起緩停的兩端窗口內速度從 0 升到帶內；相鄰兩段平均速度差超過 3 倍時，交界窗口（約 1.5 個窗口長）內較快那段低於 0.5 倍。
- 測試：Rust `glide::tests`（120 Hz 取樣：單調、C0／C1、硬節點、速度帶；密集字、長間奏、同時節點）、`tests/lyrics-rules-vectors.test.mjs`、`LyricsRulesVectorsTest`。

### 待使用者決定

- 連續模式是否也加卡拉 OK 變色（方案 C＋A，成本見 5.3）。
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
- 原生與 Android 規則：`crates/player-lyrics-rules` 的 `words.rs` 單元測試與共用向量（`wordGlide`、`centreMask`、`sung`、`karaoke`），JS `tests/lyrics-rules-vectors.test.mjs`、Kotlin `LyricsRulesVectorsTest`；三邊都有卡拉 OK 邊界的逐幀連續性測試；`player-lyrics-native` 的 `plan.rs` 測試逐字計畫（每個字起點置中、譯文跟著同一位移、沒有行淡化、逐幀單調）。
- 前端：`tests/lyrics-word-highlight.test.mjs` 檢查時間計算、簡繁、掃色 keyframes 與 `@property`、沒有 fill／filter、減少動態時改用換色、羅馬音逐字與顯示順序。
- `player-core` 的 `lyric_roles` 測試：日文＋羅馬字＋中文兩種順序、長音、韓文＋羅馬字＋英文、英文原文＋中文、中文＋拼音（含組合聲調）、Style 提示、無法判斷時的位置規則；`lyric_formats` 與 `lyrics` 另測 SRT 多行與成對 cue、ASS Style、LRC 兩種順序與寫入格式讀回。

## 參考資料

- ASS 覆寫標籤（`\k` 系列）：https://aegisub.org/docs/latest/ass_tags/
- SubRip：https://en.wikipedia.org/wiki/SubRip
- WebVTT：https://www.w3.org/TR/webvtt1/
- 網易 yrc、QQ qrc：沒有官方規格，依實際回應整理。
