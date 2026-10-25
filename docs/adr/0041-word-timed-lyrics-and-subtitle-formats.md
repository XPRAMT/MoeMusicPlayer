# 逐字歌詞與 SRT／ASS 字幕格式

狀態：已採納並實作。資料模型、yrc 逐字、AMLL QRC 純文字解析與查詢（D6）、SRT、ASS 子集、增強型 LRC 讀寫、內容偵測、本機歌詞順序、播放頁逐字高亮（D8），以及 Windows 原生工作列／浮動歌詞與 Android 浮動歌詞的逐字顯示（D10，第 5.3 節；5.4、5.5 為其後的透明度與字重修訂）都已實作。尚未實作：TTML、WebVTT 檔案格式（SRT 解析器只接受 WebVTT 的 cue 版面）、ID3 SYLT，以及連續模式加卡拉 OK 變色（見「待使用者決定」）。第 5.1、5.2 節是當時的研究，保留為歷史。

相關：[ADR 0029](0029-lyric-cue-lead.md)（換行提前）、[ADR 0030](0030-rust-chinese-script-conversion.md)（簡繁轉換）、[ADR 0032](0032-floating-lyrics.md)（浮動歌詞）、[ADR 0034](0034-native-lyrics-rendering-research.md)（原生繪製）、[ADR 0040](0040-library-sync-and-local-lyrics-data.md)（本機歌詞資料）

## 背景

模型原本只有行級時間。網易雲 provider 已取回 yrc（逐字），但只摺成行；QQ 的 qrc 是密文，以 base64 原樣保存。本機旁檔只認 `.lrc`，內嵌歌詞一律當 LRC 解析，所以 USLT 裡放 SRT／ASS 原文時，播放頁會顯示 `00:00:01,000 --> …`、`Dialogue:` 等原始行，而且這些文字還會擋住自動線上搜尋。

## 決策

### D1 本機歌詞順序（沒有手動選擇時）

1. 內嵌歌詞（音訊檔標籤：ID3 USLT、Vorbis `LYRICS`、MP4 `©lyr`、APEv2 `Lyrics`）。
2. 歌曲旁的同名旁檔（同資料夾、主檔名不分大小寫）：`.lrc` > `.qrc` > `.ass` > `.srt`。
3. 都沒有才自動線上搜尋。

- 內容可解析就算有，純文字也算（純文字會贏過旁檔）；看起來像字幕卻解析不出任何行（`LyricsContent::Unusable`）算沒有，往下一個來源找。
- 手動選擇照舊固定。本機手動選擇只存來源參照，每次重讀。
- 歌詞選擇清單的本機候選也依這個順序排：內嵌在前，旁檔在後。
- 所有平台同一個順序：資料庫路徑（`local_lyrics_for_track`：`track_embedded_lyrics` → `track_lyric_sidecars`）、開檔備援（Windows `find_lyrics`／`find_all_lyrics`、Android `android_find_all_lyrics`、遠端 `remote_lyric_texts`）、遠端查找快取。

旁檔之間取 `.lrc` > `.qrc` > `.ass` > `.srt`，理由：
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

現況補充：實際偵測順序是（加密的十六進位／Base64 整段內容 → `DetectedLyricFormat::Encrypted`，視為 `Unusable`）→ ASS → SRT → yrc → QRC（D6）→ LRC → 純文字。LRC 內的增強型字標記（`<mm:ss.xx>字`，A2 擴充）由 `parse_lrc` 讀成逐字（`enhanced_lrc_words`；沒有收尾時間戳的最後一個字在下一行起點結束，`close_open_enhanced_words`）；寫出由前端 `src/lib/lyrics-import.js` 的 `lyricsToText`（歌曲資訊面板「導入使用中歌詞」）負責，有逐字的行寫成增強型 LRC，只有行級的維持原樣。

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

不解密，QQ 官方端點維持行級。理由：繞過服務方的技術保護措施有法律疑慮（台灣著作權法第 80-2 條、DMCA §1201）；公開實作多為 GPL 或沒有授權；本專案原始碼公開。

App 實際送出的請求仍可用：`fcg_query_lyric_new.fcg?songmid=…&songid=…&qrc=1&trans=1&roma=1`，加上 Referer `https://y.qq.com/portal/player.html`。實測對搜尋得到的歌曲回 `retcode 0`，內含 LRC，`trans` 為空字串、沒有 `qrc` 欄位。不帶 Referer 時回 `-1310`。研究時 curl 得到的 `1101`，是未帶 App 的參數與 Referer 所致。

逐字來源是 AMLL TTML DB（`amll-dev/amll-ttml-db`，CC0）的 `qq-lyrics/<songmid>.qrc`，內容是明文 QRC（不是 QQ 的加密 payload），本專案自寫解析器（`player-core` 的 `parse_qrc`，不複製 LDDC、echo、myune_music 等 GPL/AGPL 專案的程式碼）：

- 格式：每行 `[行起,行長]` 後接 `文字(字起,字長)`；字時間為絕對毫秒，只符合「相對行起點」讀法時才換算；`(0,0)` 是空白佔位，其前文字併入前一個字（開頭的併入下一個字）；容忍 `<Lyric_1 LyricContent="…">` XML 外殼與實體；加密的十六進位/Base64 整段內容視為 `Unusable`，不解密。
- 偵測：`detect_lyric_format`/`parse_any_lyrics` 依內容辨識 QRC（在 yrc 之後、LRC 之前），旁檔 `.qrc` 排在 `.lrc` 之後、`.ass` 之前。
- 線上：使用者執行線上歌詞搜尋時，每批取得的 QQ 命中前 3 筆（依既有排序）各請求一次 `raw.githubusercontent.com/…/qq-lyrics/<mid>.qrc`（2 MiB 上限、5 秒逾時、mid 只允許英數與 `_-`）；404、錯誤、逾時、過大、無逐字行都靜默視為沒有，不影響搜尋。命中則新增候選 `qqmusic-amll:<mid>`（provider 仍為 QQ，`has_word_synced_lyrics`），譯文與羅馬音從同一首的一般 QQ 候選以行起點 ±300 ms 借用（不解析 TTML）。同分時逐字候選排在行級候選之前（`rank_lyric_candidates`），因此自動套用門檻不變、同 mid 時優先逐字。
- 授權理由：該資料庫為 CC0，但歌詞本身的權利屬權利人，所以不內建任何歌詞，也不在曲庫同步時抓取，只在使用者主動搜尋歌詞時才連線。

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
- 行起點是硬節點：該行第一個字形的中心在行起點時剛好落在歌詞區中心（行級歌詞也一樣，`StripLayout.anchors`／JS `layoutLyricsStrip` 第 4 參數／Kotlin `firstGlyphCenters`，取代原本的左緣）。連續模式完全忽略個別字的時間（極短的字會讓捲動速度忽快忽慢）。每一行各自判斷有沒有逐字時間，不是整首歌切換模式（使用者核准）：有逐字時間的行只有兩個硬節點，行起點，與行結束（最後一個字的結束時間，該行最後一個字形的中心落在歌詞區中心）；所有行的節點依時間排序，用一條單調 C1 曲線串起來（`word_strip_knots`＋`knot_cubic`，切線為 Fritsch–Butland 加權調和平均，第一個與最後一個節點為 0，與行級的 `strip_cubic` 同一族），所以行內與「行結束到下一行起點」的空檔速度都平滑變化，沒有停停走走，也沒有 0.5–1.5 倍速度帶與字導引。沒有逐字時間的行只有起點節點；單一字形的行（首尾字形中心相同）或結束不晚於起點的行沒有結束節點；從行結束到下一個起點節點的空檔速度（(下一行起點值 − 結束值) ÷ (下一行起點 − 結束時間)）大於該行自己的速度（(最後一個字形中心 − 第一個字形中心) ÷ (結束 − 起點)）時也不放結束節點，曲線從本行起點直接滑到下一行起點，相等時保留（各語言以交叉相乘比較，結果一致），最後一行（沒有下一行）保留結束節點；理由：最後一個字結束得太接近下一行時（例如 `[03:32.70]` 拿出下一颗软糖 結束於 215180、下一行起點 215190，20 px 字、40 px 行距），結束節點會讓捲動條在 10 ms 內移動 60 px（120 Hz 下單幀 16 px 再 44 px，前後約 1 px／幀），略過後整段約 0.6 px／幀；同一首歌另有 10 ms、30 ms 的空檔；重疊時（下一行起點不晚於本行結束）後一行的起點取代前面的節點，值維持非遞減。最後一行：有逐字時間時停在最後一個字的結束（不用估計）；沒有逐字時間時加上與行級相同的終點節點（`final_arrival_ms`：行起點加下述末句終點估計，不超過曲目結尾；該行最後一個字形的中心），所以整首只有少數逐字行的歌，最後一行也照樣從第一個字形滑到最後一個字形，不會停在第一個字形（例：`[03:51.30]` 啦啦啦啦啦啦啦啦 沒有逐字、前面的 拿出下一颗软糖 有逐字，終點節點為 231300 → 235300）。`word_strip_knots(lines, final_arrival_ms)`／JS `wordStripKnots(lines, finalArrivalMs)`／Kotlin `wordStripKnots(lines, finalArrivalMs)`；Android 的終點取決於曲目長度，所以 `StripModel.wordLines` 只存每行的節點輸入，`LyricsFramePlanner` 依（捲動條、終點）建一次節點，不在每幀建。整首都沒有逐字時間時，節點曲線與行級的 `strip_cubic` 完全相同（時間嚴格遞增時逐係數相等，Rust `without_word_times_the_knot_glide_is_the_line_glide`、JS 同名測試），Windows 與 Android 仍走原本的行級路徑，以免同時間行等邊界情況有任何差異；只要有一行逐字就走節點路徑。行級歌詞仍用原本的 PCHIP 行曲線（只改錨點）。沿用音訊時鐘的漂移修正（`plan::StripInput.word_glide` 為節點，下次重排在下一個節點）。
- 末句終點估計（使用者核准）：`terminal_line_estimate_ms`（JS `lyricsTerminalLineEstimateMs`、Kotlin `FloatingLyricsRules.terminalLineEstimateMs`）逐個字形叢集相加（Rust `unicode-segmentation`、JS `Intl.Segmenter` grapheme、Kotlin `java.text.BreakIterator.getCharacterInstance`），每個叢集只看第一個碼位的 East Asian Width：W（Wide）或 F（Fullwidth）＝500 ms（`WIDE_GRAPHEME_MS`：漢字、假名、諺文、全形英數與標點、U+3000、寬 emoji），其他（A、N、Na、H：拉丁字母、半形空格、半形片假名、模稜兩可的標點）＝250 ms（`NARROW_GRAPHEME_MS`）；空字串算一個窄叢集 250 ms；仍不超過曲目結尾（`final_arrival_ms`）。取代原本的「每個字形 500 ms」，因為拉丁字母一個字形遠比一個漢字短。W／F 表只有一份：`scripts/generate-east-asian-width.mjs` 由 Unicode 17.0.0 `EastAsianWidth.txt`（另加檔頭註明的預設 W：未指派的 CJK 統一／相容表意文字區塊與第 2、3 平面）產生 Rust `crates/player-lyrics-rules/src/east_asian_width.rs`、JS `src/lib/east-asian-width.js`、Kotlin `EastAsianWidth.kt`，檔頭記版本與 SHA-256，`tests/east-asian-width.test.mjs` 檢查三份相同；換 Unicode 版本時重跑該腳本。例：啦啦啦啦啦啦啦啦＝4000 ms；Hello world＝11×250＝2750 ms；愛してる love you＝4×500＋9×250＝4250 ms；ｱｲｳｴｵ＝1250 ms；ＡＢＣ！＝2000 ms；e＋U+0301＝250 ms（一個叢集）；か＋U+3099＝500 ms；❤️（U+2764 為 N，加 VS16 仍是 N）＝250 ms；國旗（區域指示符為 N）＝250 ms（共用向量 `finalArrival`）。平台差異：寬窄表三方相同，只有分段器隨平台的 Unicode 版本（Android 系統 ICU、JVM 單元測試的 JDK、瀏覽器 ICU、`unicode-segmentation`），最新的 emoji 序列或 Indic 子音連結（GB9c）在較舊的分段器可能切成不同數量的叢集；共用向量避開這些情況。
- 遮罩（實驗性，使用者核准；若不滿意可整筆還原提交 a457599；下列以「非目前歌詞透明度」a 為參數的描述已被 5.4 取代，現為主要／次要兩個值，`centre_mask_stops(W, em, main, secondary)`）：固定在歌詞區、不隨文字移動的靜態遮罩（`centre_mask_stops(W, em, a)`，Rust／JS／Kotlin 共用向量 `centreMask`）。W＝歌詞區寬、e＝主行字級 1 em、a＝「非目前歌詞透明度」÷100：x 在 [0, e] 由 0 直線升到 a，[e, W/2 − e/2] 由 a 直線升到 1，中心 1 em 為 1，右側對稱。歌詞區窄於 4 em 時 e 改用 W/4，各段長度仍為正、往兩端單調遞減、不超過 1；a＝1 時中間全平，只剩兩端 1 em 淡出。取代先前的置中遮罩（中心 1 em 不透明、直線降到兩端 0、不套用非目前透明度）。
- 行級與逐字歌詞都用這張遮罩，取代邊緣淡出與行淡化：連續模式下沒有每行的非目前透明度權重與 200 ms 過渡，所有格的透明度係數為 1，透明度只由位置決定。
- 譯文列沒有自己的節點：它是同一條捲動條的格，跟著主列的位移，並用同一張遮罩（以主列的 em 計算）。
- 羅馬音列只有在 `romanizationWords` 存在時才有逐字效果；原生視窗目前不顯示羅馬音列。
- Windows 遮罩是 `CompositionLinearGradientBrush` 的停駐點（`Mask::Centre`，與邊緣淡出共用 `FadeHost`）；Android 是每列一個 `LinearGradient`，以 `saveLayer`＋`DST_IN` 套用（連續模式時關閉 `EdgeFadeLayout` 的邊緣淡出），只在寬度、字級或主要／次要透明度改變時重建；Windows 的 `FadeHost` 鍵也含這兩個值（5.4）。

**「無」模式：卡拉 OK（方案 A）。**
- 套用「文字排版」對齊。
- 已唱的部分用主題主色，還沒唱的用文字顏色；一整行只有一條邊界，沿一條平滑曲線（`karaoke_glide`）移動：第一個字起點時在第一個字的左緣、最後一個字結束時在最後一個字的右緣（整行亮）是硬節點，中間以每個字的起點（左緣）與結束（右緣）為軟導引，速度維持在整行平均掃描速度的 0.5–1.5 倍；開始前停在第一個字左緣，結束後整行主色。與播放頁共用同一條曲線（播放頁以字寬比例為座標）。
- Windows：每行快取白色與主色兩張表面（版面相同，陰影／描邊的顏色都取自文字顏色，所以兩張的陰影一樣），兩個同位置 sprite 加互補 `InsetClip`：主色那張從行首切到邊界，白色那張從邊界到行尾（`WordClip`）。陰影沒有接縫；不在每幀點陣化；邊界移動至少 1/8 裝置像素才提交。下一行的主色副本會預先點陣化。主色由主視窗以 `lyrics-accent-color` 廣播。
- Android：主色副本另存一份 `LineRasterCache`，以 `clipRect` 分兩段畫；邊界移動至少 1 個裝置像素才重畫。主色經 `floatingLyricsUpdate { accentColor }` 送達；行資料帶 `words`（`{s, e, t}`）與 `lead`（前導空白的 UTF-16 長度）。
- 每一行各自判斷：只有自己帶逐字時間的行才掃色，同一首歌裡沒有逐字的行照一般行顯示（Windows `word_ranges()`、Android 每行的 `karaoke`、播放頁 `karaokeLineAnimation` 都以行為單位）。
- 譯文列不變色；長句橫向捲動照常（裁切掛在 sprite／畫布上，跟著捲動位移）。

**量測**（`--example demo`，3 個視窗：1 個浮動、2 個工作列樣式，都在 120 Hz 主螢幕（3840×2160）；release 建置，每段 32 秒、取中間 20 秒；`MOE_DEMO_WORDS=1` 為每行 80% 逐字）：

| 情境 | 本程序 CPU | 點陣快取（20 秒後） | 每視窗提交 | 超過 1.5 幀的取樣間隔（每視窗） |
|---|---|---|---|---|
| 無，行級 | 0.70% | 12.2 MiB | 只有長句捲動 | 6 |
| 無，卡拉 OK | 1.56% | 23.0 MiB | 唱字時約每幀 1 次（約 95/s） | 6 |
| 連續，行級 | 1.40% | — | 每幀 | 4–6 |
| 連續，逐字（方案 C） | 3.12% | — | 每幀 | 7–9 |

- （本表與下列數字量於當時的設計；之後連續逐字的曲線改為每行兩個節點、遮罩改為主要／次要兩個值〔5.4〕，這些數字沒有重測。）
- 卡拉 OK 的點陣快取約加倍（每行兩張）；本程序 CPU 增加約 0.9 個百分點，來自唱字期間每幀的裁切提交。換字時沒有額外掉幀：唱字期間邊界連續移動，換字不會讓取樣間隔變長。
- dwm 的 CPU 量不到：一般權限讀不到 dwm 的處理器時間。只在 120 Hz 主螢幕量過；60 Hz 虛擬螢幕沒有量。
- 連續模式加卡拉 OK（方案 C＋A）的估計：看得到的每行都要主色副本，點陣快取約加倍（每視窗再加約 10 MiB）；位移本來就每幀提交，正在唱的那格多一個裁切值，CPU 估計再增加不到 1 個百分點。尚未實作，由使用者決定。

**平滑曲線**（`crates/player-lyrics-rules/src/glide.rs` 的 `smooth_glide`，現在只用於卡拉 OK 邊界；JS 參考實作 `src/lib/lyrics-glide.js`、Kotlin `WordLyricsRules.smoothGlide`，共用向量 `glide`／`karaokeGlide`；連續模式的節點曲線向量為 `stripGlide`）：
- 輸入是硬節點 `(t, v)`（時間嚴格遞增，同時或更早的後者丟掉，值非遞減）、每段之間的軟導引，以及兩端是否緩起緩停（捲動條：兩端速度 0，條在曲線外靜止；卡拉 OK：兩端用該段平均速度）。
- 每段平均速度 a，速度帶 [0.5a, 1.5a]（`GLIDE_MIN_SPEED`／`GLIDE_MAX_SPEED`）。每個硬節點有切線 m：兩鄰段的調和平均夾進兩段速度帶的交集；兩段相差超過 3 倍、交集為空時取較慢段的上限 1.5·min。硬節點兩側各有一段斜率 m 的直線，長度為較短鄰段的 1/8（`GLIDE_KNOT_WINDOW`）。
- 折線：從硬節點窗口的端點出發，每個導引的值夾進「從前一點以速度帶能到達」與「還能以速度帶到達段尾窗口」兩個範圍的交集（必不為空），所以每一小段斜率都在速度帶內，且單調。
- 除硬節點外每個轉角以對稱二次曲線圓角，半徑為兩鄰小段較短者的一半：速度在兩斜率間線性變化，不離開速度帶、連續（C1）；硬節點不圓角，值剛好命中。導引點會被圓角偏離最多 r·Δs/4。
- 輸出為分段多項式 `GlidePiece { t, v, s, c }`（值 `v + s·u + c·u²`），`glide_at` 給值、速度與半加速度；Windows 計畫把每段轉成 `AddCubic` 鍵（c3＝0），重排在 `next_glide_break`（已被取代：Windows 不再使用合成器動畫，而是每幀依音訊時鐘在曲線上取樣並只提交有變的值，ADR 0034）；Android 每幀讀 `glideValue`。
- 例外（測試明列）：緩起緩停的兩端窗口內速度從 0 升到帶內；相鄰兩段平均速度差超過 3 倍時，交界窗口（約 1.5 個窗口長）內較快那段低於 0.5 倍。
- 測試：Rust `glide::tests`（120 Hz 取樣：單調、C0／C1、硬節點、速度帶；密集字、長間奏、同時節點）、`tests/lyrics-rules-vectors.test.mjs`、`LyricsRulesVectorsTest`。

### 待使用者決定

- 連續模式是否也加卡拉 OK 變色（方案 C＋A，成本見 5.3）：仍未決定，也未實作（卡拉 OK 只在動畫「無」）。
- 已決定並實作：歌曲資訊面板「導入使用中歌詞」遇到逐字歌詞時寫入增強型 LRC（`lyricsToText`；原先「只寫行級」的狀態已不適用）。增強型 LRC 的讀取也已實作（`parse_lrc`）。
- 尚未實作：WebVTT 檔案格式（SRT 解析器接受其 cue 版面）、TTML、SYLT。

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
- 原生與 Android 規則：`crates/player-lyrics-rules` 的 `words.rs` 單元測試與共用向量（`stripGlide`、`karaokeGlide`、`glide`、`centreMask`、`sung`），JS `tests/lyrics-rules-vectors.test.mjs`、Kotlin `LyricsRulesVectorsTest`；三邊都有卡拉 OK 邊界的逐幀連續性測試；`player-lyrics-native` 的 `plan.rs` 測試逐字計畫（`word_timed_plans_follow_the_glide_and_keep_every_cell_whole`：節點曲線、譯文跟著同一位移、沒有行淡化；`a_last_line_without_words_glides_to_the_terminal_knot_in_a_word_timed_song`）；共用向量 `stripGlide` 每例帶 `finalArrivalMs`（含上述歌曲、終點被曲目結尾截斷、逐字最後一行忽略估計、純行級）；Kotlin `aLastLineWithoutWordTimesGlidesToItsLastGlyphInAWordTimedSong`。
- 前端：`tests/lyrics-word-highlight.test.mjs` 檢查時間計算、簡繁、掃色 keyframes 與 `@property`、沒有 fill／filter、減少動態時改用換色、羅馬音逐字與顯示順序。
- `player-core` 的 `lyric_roles` 測試：日文＋羅馬字＋中文兩種順序、長音、韓文＋羅馬字＋英文、英文原文＋中文、中文＋拼音（含組合聲調）、Style 提示、無法判斷時的位置規則；`lyric_formats` 與 `lyrics` 另測 SRT 多行與成對 cue、ASS Style、LRC 兩種順序與寫入格式讀回。

## 參考資料

- ASS 覆寫標籤（`\k` 系列）：https://aegisub.org/docs/latest/ass_tags/
- SubRip：https://en.wikipedia.org/wiki/SubRip
- WebVTT：https://www.w3.org/TR/webvtt1/
- 網易 yrc、QQ qrc：沒有官方規格，依實際回應整理。

### 5.4 修訂：主要／次要透明度取代視窗透明度與共用的非目前透明度（使用者核准）

- 設定：播放頁（`lyricsPreferences`）、工作列（`taskbarLyrics`）、浮動（`floatingLyrics`）各自儲存 `mainOpacityPercent`（1–100，預設 100）與 `secondaryOpacityPercent`（1–100，預設 50）。數值是絕對百分比，互不夾限（次要可大於主要），介面不灰化任何一個。舊鍵 `opacityPercent`（工作列、浮動）與 `inactiveOpacityPercent`（共用）不遷移，載入時忽略；缺少新欄位時取預設。「外觀」群組改名「文字效果」（群組內的選擇器不再顯示同名小標，只留 aria-label），分頁「一般」改名「播放頁」。
- 播放頁：目前行 `mainOpacityPercent`，其餘行 `secondaryOpacityPercent`（`getLyricLineOpacity`）；沒有目前行時（尚未到第一句）全部以主要透明度顯示；換行的 CSS 淡化過渡不變。
- 視窗，動畫「無」：顯示中的那一行以主要透明度繪製，次要透明度無作用（滑桿照常可用）。動畫「連續」：遮罩 `centre_mask_stops(W, em, main, secondary)`：0 → 次要（兩端 1 em）→ 主要（到中心 1 em 的平台），Rust／JS／Kotlin 共用向量 `centreMask`（含次要大於主要、主要很小、窄寬度）；所有格與容器的不透明度為 1。
- 資料流：移除 `lyrics-inactive-opacity` 廣播與 Android `floatingLyricsUpdate` 的獨立 `inactiveOpacityPercent`；兩個值跟著各視窗自己的設定進入 Windows 的 `Look`（`main_opacity_percent`／`secondary_opacity_percent`）與 Android 的 settings。`lyricsLineOpacityFactor`／`line_opacity_factor` 與 Kotlin 的 `inactiveFactor`／`cellOpacity` 已無用途而移除。
- Android：浮動視窗的視窗 alpha 恆為 1；鎖定（`FLAG_NOT_TOUCHABLE`）且 Android 12+ 時為 min(1, `maximumObscuringOpacityForTouch`)，繪製值不補償，所以鎖定時整體亮度被系統上限（通常 0.8）壓住，這是刻意的取捨。動畫「無」時兩個列視圖以主要透明度作為自己的 alpha（背景與鎖頭不受影響）。

### 5.5 修訂：每個歌詞分頁各自的字體粗細（使用者核准）

- 設定：播放頁（`lyricsPreferences`）、工作列（`taskbarLyrics`）、浮動（`floatingLyrics`）各自儲存 `fontWeight`（整數，100–1000，滑桿步進 100，預設 500）。缺欄位取 500，載入時夾在範圍內（Rust `LYRICS_FONT_WEIGHT_RANGE`／`default_lyrics_font_weight`，JS `LYRICS_FONT_WEIGHT`／`normalizeLyricsFontWeight`，Kotlin `FloatingLyricsRules.fontWeight`），不遷移。滑桿「字體粗細」在「字體與大小」群組、緊接「譯文大小（相對原文）」之後，三個分頁與播放頁快速設定共用 `LyricsLookSettings.svelte`。
- 意義：`fontWeight` 是原文的字重；譯文與羅馬拼音恆為 `fontWeight − 50`（`lyricsAuxiliaryWeight`／`translation_weight`／`auxiliaryFontWeight`，至少 1，所以字重 100 時為 50）。
- 播放頁：`LyricsView.svelte` 以 CSS 變數 `--lyric-primary-weight`／`--lyric-auxiliary-weight` 取代寫死的字重（原文 500；譯文與羅馬拼音原本沿用預設字重）；行內墨水行高以同一字重量測（canvas 字型字串帶字重），字重改變時重測；列高仍由 DOM 觀察器量測，所以版面會跟著重排。
- Windows 原生：`Look.font_weight`；DirectWrite 的 `DWRITE_FONT_WEIGHT` 只收 1..=999，設定值 1000 以 999 繪製（`dwrite_weight`）。點陣列改為 `(文字, 字級 px, 字重)`，主列用 `fontWeight`、譯文列用 −50，所以點陣鍵（`plan::raster_key`）含每一列的字重：字重不同的視窗不共用點陣。字重會改變文字寬度、第一與最後字形中心，所以連續模式的行寬量測鍵（`Win.measured`）含字重；設定改變時整份快取與量測清除並重新規劃（`Cmd::Look`），不是每幀。卡拉 OK 的白色與主色兩張表面用同一字重。行框以 Segoe UI Medium 的 ascent／descent 計算，沒有隨字重改變（同一字型家族各字重的垂直度量相同）。
- Android：`LyricLineView.setLook` 帶字重，API 28 以上 `Typeface.create(Typeface.DEFAULT, weight, false)`；minSdk 24，更舊的系統只有一般與粗體，字重 ≥ 600 用粗體、否則一般（`FloatingLyricsRules.typefaceChoice`）。`LineLook`（`LineRasterCache` 的鍵）含字重，改變時所有行重新點陣化並重新量測；`stripFor` 的鍵含字重。字重隨 `floatingLyricsUpdate` 的 settings 傳入，不需新通道。
- 非可變字型（Segoe UI、Roboto 等系統字型視版本）無法呈現所有字重，系統會對應到最近的可用字重，不特別處理。
