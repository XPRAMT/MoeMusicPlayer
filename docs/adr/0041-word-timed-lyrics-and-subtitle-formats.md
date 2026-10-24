# 逐字歌詞與 ASS／SRT／VTT／TTML 字幕格式支援

狀態：提案，未採納

日期：2026-10-24，Asia/Taipei (UTC+08:00)

相關：[ADR 0029](0029-lyric-cue-lead.md)（換行提前）、[ADR 0030](0030-rust-chinese-script-conversion.md)（簡繁轉換）、[ADR 0018](0018-taskbar-lyrics.md)／[0031](0031-multi-monitor-taskbar-lyrics.md)（工作列歌詞）、[ADR 0032](0032-floating-lyrics.md)（浮動歌詞）、[ADR 0034](0034-native-lyrics-rendering-research.md)（原生繪製）、[ADR 0040](0040-library-sync-and-local-lyrics-data.md)（本機歌詞資料）

## 背景與現況

使用者要求研究逐字（卡拉 OK 式、每字／每音節有時間）歌詞與 ASS、SRT 等字幕格式。現況（讀程式碼所得）：

- 模型 `player_core::LyricLine { start_ms, text, translation, romanization }` 只有行級時間；`ParsedLyrics` 另有 `raw_karaoke: Option<String>`。
- 網易雲 provider（`src-tauri/src/lyrics_provider.rs`）**已經**以 `lv=-1&kv=-1&tv=-1&yv=-1&rv=-1` 一次取回 `lrc`、`yrc`、`tlyric`、`romalrc`；`parse_yrc` 把逐字時間摺成行級，原文保存在 `raw_yrc`／`raw_karaoke`，註解寫明「留給未來的逐字高亮」。
- QQ provider 請求帶 `qrc=1`，回傳的 `qrc` 欄位以 base64 原樣保存（`qrc_base64`），`preserve_qrc` 明確**不解密**，播放只用同時回傳的行級 LRC。
- 本機：同名 `.lrc`（`track_lyric_sidecars`）與內嵌歌詞文字（`track_embedded_lyrics`，ID3 為 USLT）都只當純文字 LRC 解析；增強型 LRC 的 `<mm:ss.xx>` 會留在文字裡或被當成雜訊。
- 繪製：播放頁 `LyricsView.svelte`（CSS），Windows 工作列／浮動歌詞已改原生 DirectComposition（ADR 0034，`crates/player-lyrics-native` 的 `plan.rs`／`sampling.rs`／`win.rs`，規則在 `player-lyrics-rules`），Android 浮動窗 `LyricLineView.kt`（Canvas 自繪）。三者都只知道「行」。

- `raw_karaoke` 的來歷（`git log -S raw_karaoke`）：2026-09-28 `914dccc`（Add lyrics parser and persistent cache）建立，同日 `f9ea866`（provider adapters）、之後 `806f3a6`（Integrate lyrics service）接上。設計意圖如 `parse_yrc` 註解：「保留完整 YRC／QRC 原文，未來繪製器加逐字高亮時不必重抓」。它**不是**永遠 None：網易回傳 yrc 時存 yrc 原文，QQ 回傳 qrc 時存 `{qrc_base64, trans, roma}` JSON；`has_word_synced_lyrics` 依它判斷候選是否「有逐字」（所以 QQ 候選目前會被標為有逐字，但實際無法使用）。本機 LRC／內嵌永遠是 None。
- **純文字內嵌歌詞會擋住自動線上搜尋**：`local_lyrics_for_track` 找到任何本機歌詞（含 `synced: false` 的 Plain）就回 `Found` → `LyricsTrackResultDto::ready`；前端 `lyrics-controller.js` 只有在 `!result.lyrics` 時才呼叫自動 `lyrics_search`。因此 USLT 裡放 SRT／ASS 原文時：播放頁顯示 `1`、`00:00:01,000 --> …`、`Dialogue:`、`{\k20}` 等原始行，工作列／浮動歌詞（只顯示同步行）空白，而且線上同步歌詞也不會自動補上。

所以「資料」其實有一半已在手上（yrc 已抓、已存），缺的是：模型、解析、轉換、選擇規則與三個繪製面。

## 1. 格式調查

| 格式 | 時間粒度 | 譯文／羅馬音 | 編碼 | 普及度 | 建議 |
|---|---|---|---|---|---|
| LRC（基本） | 行 | 同時間戳多行（已支援） | UTF-8 為主，舊檔 GBK／Shift-JIS | 極高 | 已支援 |
| 增強型 LRC（A2 extension） | 字：`[00:12.00]<00:12.00>這<00:12.40>是<00:12.90>` | 同上；譯文通常無逐字 | 同 LRC | 中（foobar ESLyric、MusicBee、Poweramp 匯出） | **P1** |
| 網易 yrc | 字：行 `[起ms,長ms]`，字 `(起ms,長ms,0)字`（絕對毫秒） | 由 `tlyric`／`romalrc` 行級提供 | JSON 內 UTF-8 | 網易 API 大多數熱門歌曲有 | **P1**（已抓取） |
| QQ qrc | 字：`[起,長]字(起,長)`，與 yrc 類似 | 另有 trans／roma | 回傳內容為加密（3DES 變體＋zlib） | QQ 曲庫大 | 見「法律」，建議不做 |
| Apple TTML（含 iTunes 擴充） | 字：`<p begin end><span begin end>字</span>`；`ttm:role="x-translation"`／`x-roman` | 有（同 `<p>` 內 role span 或 `<iTunesMetadata>`） | XML UTF-8 | Apple Music 取得需帳號，本機檔少見；AMLL 社群使用 | P3（只做匯入） |
| Spotify JSON | 行（`syllables` 欄位通常空） | 無 | JSON | 需登入的私有 API | 不做 |
| ASS／SSA | 行（Dialogue 事件）＋字：`{\k50}`（百分秒）、`\kf`／`\K`（掃色）、`\ko`（外框） | 以不同 Style／Layer 的事件表示，需啟發式配對 | UTF-8／UTF-16 有 BOM，舊檔 GBK | 動畫字幕、卡拉 OK 字幕組常見 | P3，只取子集 |
| SRT | 行（cue 有 start 與 end） | 常在同一 cue 第二行 | UTF-8／GBK | 極高（影片） | P3 |
| WebVTT | 行；cue 內 `<00:01.000>` 時間戳可做逐字 | 同 cue 多行 | 規定 UTF-8 | 中 | P3 |
| ID3v2 SYLT | 字或行：每段文字配絕對時間（ms 或 MPEG 幀），時間戳格式 2＝ms | 單一語言欄；多個 SYLT 可分別存語言 | 文字編碼欄位（Latin-1／UTF-16／UTF-8） | 少（部分卡拉 OK 軟體寫入） | P1 讀取（lofty 可讀 frame），寫入後議 |
| Vorbis `LYRICS`／`UNSYNCEDLYRICS`、MP4 `©lyr`、APEv2 `Lyrics` | 純文字；可放 LRC 或增強型 LRC | 同 LRC | UTF-8 | 常見 | 內容走 LRC 解析器即自動得到逐字 |

ASS 值得支援的子集：`[Script Info]`（`PlayResX` 忽略）、`[V4+ Styles]` 只讀名稱、`[Events]` 的 `Format` 欄序與 `Dialogue`；文字中 `\k` `\K` `\kf` `\ko`（全視為「該字段時長」）、`\N`／`\n` 換行，其餘 `{...}` 覆寫標籤（位置、顏色、字型、`\t` 動畫、繪圖 `\p`）一律剝除。`Comment` 事件忽略。譯文配對：同起訖時間（±50 ms）的不同 Style 事件，第一個為原文、其餘依序為譯文／羅馬音（與現行同時間戳 LRC 合併規則一致）。不做樣式還原：播放器以自己的歌詞外觀設定繪製。

旁檔名與優先序（建議）：同資料夾、同主檔名（不分大小寫）的 `.lrc` > `.ttml` > `.ass`/`.ssa` > `.vtt` > `.srt`；同時存在時以「有逐字的優先，否則依上列」。內嵌（SYLT > USLT/LYRICS 內含增強型 LRC > 純 LRC）排在旁檔之後，維持現行「LRC 旁檔優先」使用者規則。`.ass` 等需加入 `track_lyric_sidecars` 收集（ADR 0040 的掃描已在列資料夾，只是多認幾個副檔名，指紋機制不變）。

## 1a. 真實樣本分析（2026-10-24）

只用現有 provider 的端點與公開的 musicu 端點，對少數歌曲各請求一次；樣本只存在本機暫存資料夾（scratchpad，不在儲存庫），此處只列結構性片段。原型解析器 `proto_yrc.py` 也在 scratchpad，未提交。

**網易 yrc**（`/api/song/lyric?…&yv=-1` 與 `/api/song/lyric/v1?…&yv=0&ytv=0&yrv=0`，未登入）：

- 未登入時 yrc **只有部分歌曲有**：試了 10 首，4 首有 yrc（中文 3、英文 1），日文熱門曲（YOASOBI 等）都只有 lrc＋tlyric＋romalrc，沒有 yrc。所以「逐字＋羅馬音」的日文組合實際上很少拿得到；設計不能假設 yrc 普及。
- 結構：行 `[行起ms,行長ms]`，接著重複的 `(字起ms,字長ms,0)字`，例：`[4890,6250](4890,270,0)曲(5160,270,0)…`。**字時間是絕對毫秒**（不是相對行首）；第三欄 4 首全為 0（意義不明，忽略）。
- 開頭有 JSON 行 `{"t":0,"c":[{"tx":"作词: "},…]}`（製作資訊，10–20 行），需略過或另顯示為 credits。
- 間隙與重疊：字間有間隙（1 首 24 處）、極少重疊（1 處，<1 字長）、行與行偶有重疊（1 處）；字長最小 0 ms、中位數 270–480 ms、最大 3–6 s（拖長音）。零長字存在 → 進度計算需防除以零（視為瞬間亮）。
- 英文：空格附在字尾（`(7260,390,0)Will `），1 首有 14 個「空字」（只有時間沒有字，推測是空白分隔或呼吸）；`text = concat(words)` 不變式成立，但要保留空白不 trim 每字。
- 中文：字幾乎一字一段；`字未落在行範圍外`：4 首皆 0 例（行長涵蓋所有字）。
- 譯文：`tlyric` 是行級 LRC，時間與 **lrc** 對齊而不是與 yrc 行對齊（同曲 tlyric 第一行 `00:06.838` vs yrc 第一行 6840 ms，差 2 ms）；另有 **`ytlrc`**（v1 端點，「對齊 yrc 的譯文」），時間與 yrc 行起點相同，應優先使用。`yromalrc` 同理（本次樣本沒有）。合併規則：優先 ytlrc／yromalrc 依行起點精確匹配；否則 tlyric／romalrc 以 ±300 ms 最近行匹配——4 首樣本全部成立。
- 編碼：JSON 字串為 UTF-8，無 BOM、無 CR；內容是簡體（需 player-zh 轉換）。

**QQ qrc**（現有 `fcg_query_lyric_new.fcg?…&qrc=1` 以 curl 直接請求回 `retcode 1101`，需確認 App 實際送的參數；改用公開的 `musicu.fcg` `music.musichallSong.PlayLyricInfo / GetPlayLyricInfo`，`qrc:1, trans:1, roma:1`）：

- 回傳 `lyric`、`trans`、`roma` 皆為**大寫十六進位字串**（長度為 16 的倍數，即 8 位元組區塊），`crypt` 欄位為 0 但內容仍是密文。日文曲以 songID 查詢未拿到資料（ID 對應問題，未深追，避免多次請求）。
- 解密方式（依公開資料描述，未在本次執行、未複製任何程式碼）：hex 解碼 → QQ 自訂的 Triple-DES（三組 DES，金鑰為客戶端內嵌的固定 24 位元組字串，流傳於網路；其 DES 實作與標準 DES 不相容，標準函式庫無法直接解）→ zlib inflate → UTF-8 XML `<QrcInfos><LyricInfo><Lyric_1 LyricContent="…"/>`，`LyricContent` 內為 `[行起,行長]字(字起,字長)` 格式（與 yrc 幾乎相同，但字標記放在字**後面**）。
- 公開實作多為 GPL-3.0（例如 LDDC）或未標授權；不得複製。自行依演算法描述重寫在技術上可行（約 300 行），但見第 6 節法律疑慮。

**結論**：ADR 原先的 yrc 規則成立（絕對時間、行內字串接＝行文字），需補：JSON credits 行、零長字、空字、ytlrc 優先。QQ 逐字在不解密下不可用。

## 1b. 依內容判斷格式（嵌入、旁檔、貼上、匯入共用）

新增 `player_core::lyrics::detect_format(text) -> LyricFormat` 與 `parse_any(text)`，所有歌詞文字入口（內嵌 USLT/LYRICS/©lyr/APE、`.lrc` 等旁檔、手動貼上、檔案匯入）都經它，副檔名只作提示不作依據。判斷順序（去 BOM、統一換行後看前 64 個非空行）：

1. 第一個非空字元為 `<` 且含 `<tt` 或 `xmlns="http://www.w3.org/ns/ttml"` → TTML。
2. 第一行為 `WEBVTT` → WebVTT。
3. 含 `[Script Info]` 或 ≥1 行以 `Dialogue:` 開頭且前面出現 `[Events]` → ASS/SSA。
4. 出現「整數行，下一行為 `hh:mm:ss,mmm --> hh:mm:ss,mmm`」至少一次 → SRT（也接受 `.` 小數點）。
5. 行首 `[數字,數字]` 且內含 `(數字,數字,數字)` → yrc；`[數字,數字]` 且字後 `(數字,數字)` → qrc 明文。
6. 行首 `[mm:ss.xx]` → LRC；行內另有 ≥2 個 `<mm:ss.xx>` → 增強型 LRC（逐行判斷，混合檔允許）。
7. 其餘 → Plain。

規則：**看起來像字幕但解析失敗**（例如有 `-->` 但時間壞掉、ASS 沒有可用 Dialogue）時，結果標為 `Unusable`，**不當成 Plain**：不顯示原始格式行，且視同「沒有本機歌詞」讓自動線上搜尋照常進行。理由：顯示 `Dialogue:` 原文對使用者沒有價值，又會擋住線上同步歌詞。成功解析的字幕一律 `synced: true`，工作列與浮動歌詞自動可用。

另外（待決策 D8）：現行「純文字本機歌詞會擋住自動線上搜尋」本身是否保留。建議改成：本機只有 Plain 時仍顯示它，但**照常背景自動搜尋**，找到通過兩道門檻的同步歌詞就以「自動結果」優先顯示（手動選擇仍最優先；使用者在選擇器選回「內嵌歌詞」即固定）。

## 2. 資料模型

```rust
pub struct LyricWord { pub start_ms: u64, pub end_ms: u64, pub text: String }   // text 含其後空白
pub struct LyricLine {
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,               // 新：SRT/VTT/ASS/yrc/TTML 有；LRC 由下一行推得
    pub text: String,                      // 恆等於 words 的 text 串接（不變式）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<LyricWord>,             // 新：空＝只有行級
    pub translation: Option<String>,
    pub romanization: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub romanization_words: Vec<LyricWord>,// 選用：羅馬音逐字（yrc 無，TTML/ASS 可能有）
}
```

- 譯文維持行級（幾乎沒有來源有逐字譯文）。
- `raw_karaoke` 的處置：它原本就是「逐字原文暫存」，有了 `words` 之後語意改為「provider 原始逐字負載（只供重新解析）」：網易 yrc 解析成功後 `words` 為權威、`raw_karaoke` 可保留供日後解析器修正時重算；QQ 的密文 `qrc_base64` 保留但 `has_word_synced_lyrics` 改為只看 `words` 非空（避免把無法使用的 QQ 候選標成「逐字」）。不另開欄位。
- 相容性：新欄位皆 `serde(default)`，`track_lyrics` 既有 JSON 原樣可讀；不需 schema 遷移。舊的網易自動結果只存了 `raw_karaoke`，讀取時若 `words` 為空而 `raw_karaoke` 是 yrc，就即時以新 `parse_yrc` 補上（不改寫資料庫，下次重新取得或手動選擇時才存新格式）。手動選的本機來源本來就只存參照（每次重讀），自動得到逐字。
- `LyricFormat` 新增 `EnhancedLrc`、`Ass`、`Srt`、`WebVtt`、`Ttml`、`Sylt`；`synced` 意義不變，另加 `word_synced: bool` 供選擇規則與 UI 標示。
- 行級格式（SRT/VTT/無 `\k` 的 ASS）直接對應為行，`end_ms` 保留；`end_ms` 讓「行尾到下一行開始之間的空白」可以淡出，但第一版只用於逐字最後一字的結束。
- DTO：`LyricLineDto` 加 `words`（`[{s,e,t}]` 短鍵以減少 IPC 量）、`wordsTraditional` 不另存——見下。原生端 `player-lyrics-native::LyricLine` 加 `words: Vec<(f64 start, f64 end, Range<usize> 位元組範圍)>`。

### 簡繁轉換與音節切分

player-zh 的 OpenCC 轉換是**詞組最長匹配**，逐字分別轉換會失去詞組（例：「头发」逐字轉成「頭發」而非「頭髮」）。做法：對整行 `text` 轉換一次，再把字界對回去。OpenCC 的字表與 TWVariants 都是一字對一字（簡繁字數相同；STPhrases 的值與鍵等長，已在 ADR 0030 的資料中可驗證），所以以 **Unicode 純量（char）索引**對齊即可：轉換結果的第 i 個 char 屬於原文第 i 個 char 所屬的字段。player-zh 新增 `to_traditional_aligned(&str) -> Option<String>`：轉換後 char 數不同就回 `None`，此時退回逐字段個別轉換（極少數）。日文假名整行不轉的規則照舊。單元測試：以整份 STPhrases 檢查長度相等的前提。

## 3. 來源與流程

- **本機**：增強型 LRC 由同一個 `parse_lrc` 辨識 `<mm:ss.xx>`（行內出現兩個以上即視為逐字行），無逐字的行照舊。SYLT 由 lofty 讀出原始 frame（`player-track-metadata` 掃描時與 USLT 同次讀取，存進 `track_embedded_lyrics` 的新欄位或以序列化的增強型 LRC 文字存放——建議後者，免 schema 變更）。
- **網易**：已經同時取得 yrc 與 lrc，不需新請求。新規則：yrc 能解析出 ≥80% 的 lrc 行數且首尾時間差 < 2 s 時，以 yrc 為主體、`tlyric`／`romalrc` 依行起點（±300 ms 最近）合併；否則照舊用 lrc。
- **QQ**：見下節，建議繼續只用行級 LRC；`qrc_base64` 照舊保存。
- **自動套用**（`auto_lyric_candidate`）：兩道門檻（標題 ≥0.88、時長 ≤2 s、須同步）不變；排序分數相同或相差 <0.02 時偏好 `word_synced`。手動永遠優先。
- **手動選擇 UI**：候選清單每列加「逐字」標記；預覽可顯示前幾行的逐字分段。
- **歌曲資訊面板**：「導入使用中歌詞」若有逐字，寫入 USLT 時輸出增強型 LRC（相容：不支援的播放器會顯示 `<..>` 雜訊——需使用者決定，見決策 D4）；寫 SYLT 列為後續。歌詞欄位編輯器保持純文字。
- **匯入字幕**：P3 在歌詞選擇加「從檔案匯入（.lrc/.ass/.srt/.vtt/.ttml）」，解析後存為手動列（存文字快照，因檔案不在歌曲旁）。

## 4. 繪製

共同規則：每字狀態只取決於播放時間 t——`t < start` 未唱、`start ≤ t < end` 唱中（進度 `(t-start)/(end-start)`）、`t ≥ end` 已唱。ADR 0029 的 200 ms 換行提前只作用於「行」（捲動、目前行切換），**逐字填色不提前**（提前會讓字比歌聲早亮）。使用者偏移照舊疊加於兩者。行內無逐字時退回現行行級高亮（整行 lit），三個面行為一致。

### 播放頁（CSS）

每字一個 `<span>`，以兩層文字（底層暗色、上層亮色）搭配上層 `clip-path: inset(0 calc(100% - var(--p)) 0 0)` 或 `background-clip: text` 的線性漸層位置。不在每幀寫樣式：目前行的每個字在進入時設定一個 CSS `animation`（`@property --p` 0→1，duration＝字長，delay＝start−t，可為負），與 ADR 0028 進度條同一手法；跳轉、暫停、繼續時只重排目前行。非目前行不加動畫（全暗或全亮靜態）。成本：一行約 10–30 個 span、同時只有一行在動畫，`clip-path` 可在合成器上跑；不可用 `filter`。

### Windows 原生（工作列／浮動）

遵守 ADR 0034 的常規：點陣化只在載入時，動畫交給 DComp／Composition，每幀頂多提交改變的值。方案比較：

| 方案 | 做法 | 每幀成本 | 評估 |
|---|---|---|---|
| A. 雙表面＋動畫裁切 | 每行已畫好的暗版（現有）外再畫一份亮版，亮版 visual 疊在上面，`SetClip` 的 right 邊綁 `IDCompositionAnimation`（分段線性，每字一段，斜率＝字寬/字長） | 0（DWM 計算）；記憶體×2 | **建議**。一行只需一條分段線性曲線，字間停頓即平段 |
| B. 遮罩筆刷位移 | Windows.UI.Composition `CompositionMaskBrush`＋漸層遮罩 offset 動畫 | 0 | 可做柔邊掃色，但需 WinRT 遮罩管線；作為 A 的柔邊升級 |
| C. 每字 sprite | 每字獨立 visual，切 opacity | 0，但 visual 數多 | 只能整字跳亮，不能掃色；不建議 |

字的 x 範圍在點陣化時由 DirectWrite `HitTestTextRange` 取得（每字一次，載入時），存進 plan。與連續滑動（continuous strip）的互動：亮版 visual 是該行格子的子 visual，跟著同一個父位移動畫走，裁切以格子本地座標表示，所以兩個動畫互不影響；邊緣淡出（edge fade）套在父層，亮暗兩層一起被淡出。換行時的非目前透明度權重同樣套在格子層。陰影／描邊：亮版與暗版各自帶自己的陰影一起畫進表面（陰影顏色不變），裁切邊的陰影會被切成直線，可接受；若不行改用方案 B 柔邊。跳轉、暫停：重建目前行的裁切動畫（`sampling.rs` 已有暫停重排路徑）。

### Android 浮動窗

`LyricLineView` 在載入行時把亮版文字畫進一張快取 `Bitmap`（或直接在 `onDraw` 用 `canvas.clipRect` 後 `drawText` 亮色——文字少，兩次 drawText 便宜），每幀（現有 Choreographer 迴圈）只算 clip 右緣：`canvas.save(); clipRect(0,0,x(t),h); draw(亮); restore()`。x(t) 由每字 x 範圍（`Paint.measureText` 前綴寬，載入時算一次）內插。`FloatingLyricsRules` 新增純函式 `wordWipeX(words, t)`，與 Rust 的同名函式用同一組 fixture 測。

## 5. 函式庫

| crate | 用途 | 授權 | 評估 |
|---|---|---|---|
| `subparse` | SRT／ASS／VTT（部分）／IDX | 本次未能核實；印象為 LGPL-3.0 | 若為 LGPL 只能動態連結，Rust 靜態連結不符合 AGENTS.md，**不採用**；且多年未維護 |
| `srtlib`／`srtparse` | SRT | 印象為 MIT／Apache-2.0，未核實 | SRT 解析不到 100 行，不值得引入 |
| `ass_parser`／`substation` 系列 | ASS | 未核實；多為小型個人專案 | 不採用 |
| `quick-xml` | TTML 的 XML 解析 | MIT（本次未重新核對版本） | 若做 TTML 可採用；純 Rust，Android 可建置 |
| lofty（既有） | SYLT／USLT frame | MIT／Apache-2.0（既有相依） | 已在用 |

建議：**全部自寫小型解析器**放在 `player-core::lyrics`（或新 crate `player-lyrics-formats`）：增強型 LRC、yrc、SRT、VTT、ASS 子集各約 100–250 行，零新相依、Android 無建置風險、可完全受測。TTML 才考慮 `quick-xml`（引入前依 crates/AGENTS.md 重新核對版本與授權）。AGENTS.md 禁止 GPL／AGPL，libass（ISC）不需要（不做樣式還原），也不得參考 GPL 的歌詞外掛程式碼。

## 6. 法律與授權：QQ qrc

QRC 內容以 QQ 音樂自訂的金鑰加密（公開於多個 GitHub 專案的 3DES 變體＋zlib）。解密本身技術上簡單，但：(1) 金鑰屬服務方的技術保護措施，繞過可能違反服務條款，在部分法域可能觸及反規避條款（台灣著作權法第 80-2 條、美國 DMCA §1201）；(2) 現有公開實作多數無授權或為 GPL，不可複製；本專案原始碼公開，內嵌金鑰與解密程式等於公開散佈繞過方法；(3) 歌詞本身仍是有著作權的內容，與現在使用的行級 LRC 同樣性質，但多了「繞過保護」這一層。現有程式碼刻意不解密（`preserve_qrc`）。**建議維持不解密**，QQ 只用行級；逐字來源以網易 yrc、本機檔案為主。是否接受風險由使用者決定（D2）。

## 7. 分期與工作量（粗估，單人含測試）

- **P1（約 3–4 天）**：模型擴充與 DTO；增強型 LRC 解析（含內嵌文字自動受惠）；yrc 完整解析與 tlyric／romalrc 合併；舊 `raw_karaoke` 即時補解析；SYLT 讀取；player-zh 對齊轉換；自動套用偏好；播放頁 CSS 逐字掃色。測試：各格式 fixture（含 BOM、CRLF、GBK 拒絕／提示、空字、零長字、重疊時間）、`text == concat(words)` 不變式、簡繁對齊（STPhrases 全表長度檢查）、`tests/lyrics-view.playwright.js` 擴充逐字狀態。
- **P2（約 3–4 天）**：Windows 原生方案 A（亮版表面、分段裁切動畫、與 strip／淡出／透明度權重整合、暫停跳轉重排）；Android `LyricLineView` 裁切掃色與 `FloatingLyricsRules.wordWipeX`。測試：Rust／Kotlin 共用 fixture 的 wipe 位置純函式；原型式截圖檢查；量測沿用 ADR 0034 的 `measure.ps1`（確認 anim 模式 CPU 不增加）。實機：Windows 120 Hz＋平板。
- **P3（約 3–5 天）**：SRT、VTT（含 cue 內時間戳）、ASS 子集、TTML（quick-xml）解析；旁檔收集擴充副檔名；歌詞選擇「從檔案匯入」；候選清單逐字標記。測試：每格式真實樣本 fixture（自製，避免夾帶受版權保護的歌詞，用自撰文字）。
- 後續：USLT 寫增強型 LRC 或寫 SYLT（D4）、柔邊掃色（方案 B）。

## 8. 待使用者決定（與建議分開）

- **D1** 是否採用 P1→P2→P3 的順序與範圍（是否需要 TTML、ASS）。
- **D2** QQ qrc：維持不解密，或接受風險解密。
- **D3** 旁檔優先序：`.lrc` 永遠優先，或「有逐字的優先」。
- **D4** 歌曲資訊面板導入有逐字的歌詞時：寫增強型 LRC 進 USLT（相容性風險：其他播放器顯示 `<..>`）、另寫 SYLT、或只寫行級。
- **D5** 掃色樣式：硬邊（方案 A）即可，或要柔邊漸層（方案 B，多一層 WinRT 遮罩）。
- **D6** 唱過的字：保持亮色，或回到暗色只亮「正在唱」的字。
- **D7** 自動套用時同分偏好逐字是否接受。
- **D8** 本機只有純文字歌詞時，是否仍自動搜尋線上同步歌詞並優先顯示（現行：不搜尋）。

測試計畫補充：儲存庫內只放**自撰文字的合成 fixture**（每種格式、偵測邊界、壞掉的字幕→Unusable）；真實樣本以 ignored 測試讀環境變數 `MOE_LYRICS_SAMPLES=<資料夾>`（比照 `MOE_SMB_*`），斷言結構性質（字絕對時間遞增或少量重疊、`text==concat(words)`、譯文匹配率），不斷言歌詞內容，樣本不提交。

### 建議

D1 照分期、TTML 放最後；D2 不解密；D3 有逐字的旁檔優先、同級時 `.lrc` 優先；D4 先只寫行級，提供「另存增強型 LRC 旁檔」選項；D5 先硬邊；D6 唱過保持亮色（主流做法）；D7 接受；D8 改為照常搜尋。P1 加入 1b 的內容偵測（約 +1 天）與 ytlrc 優先。

## 參考資料

- 增強型 LRC：Wikipedia「LRC (file format)」Enhanced format 一節，https://en.wikipedia.org/wiki/LRC_(file_format)
- ID3v2.4 frames（SYLT/USLT）：https://id3.org/id3v2.4.0-frames
- ASS 規格（Aegisub 文件 ASS tags，`\k` 系列）：https://aegisub.org/docs/latest/ass_tags/
- WebVTT（W3C）：https://www.w3.org/TR/webvtt1/
- TTML2（W3C）：https://www.w3.org/TR/ttml2/
- SRT：Wikipedia「SubRip」，https://en.wikipedia.org/wiki/SubRip
- 網易 yrc、QQ qrc：無官方規格，依本專案 `lyrics_provider.rs` 既有測試資料與實際回應格式描述。
- 本次撰寫未連網核對 crate 授權；第 5 節標「未核實」者採用前必須重新查證。
