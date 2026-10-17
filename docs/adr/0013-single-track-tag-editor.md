# ADR 0013（草案）：單曲詳細資訊／標籤編輯面板

## 狀態

Proposed（設計草案，尚未實作）。範圍限定**單曲**；多檔批次編輯不在此 ADR 範圍。D2／D3／D4／D7／D9、封面範圍與頻譜圖優先（D11）皆已由使用者確認。

## 背景

使用者要一個「比 Mp3tag 更強」的歌曲詳細資訊面板：顯示並編輯標題、演出者、專輯、專輯演出者、作曲、碟號／音軌號（含總數）、歌詞、錄音日期、封面，顯示完整路徑（唯讀＋「在資料夾中開啟」），編輯先暫存、按「儲存」才寫檔，並提供 FFT 頻譜圖。

現況：讀標籤用 `lofty 0.25.4`（`player-platform-windows`、`tauri-plugin-media-index` Android、`lyrics_service.rs`），解碼用 `symphonia 0.5.5`（`player-audio-windows`），重取樣用 `rubato`（已間接帶入 FFT）。SQLite（`player-db`）是檔案標籤的投影快取。ADR 0010 已確立「明確使用者動作才可寫回外部檔案，寫完只更新對應投影、不全庫重掃」的原則；本 ADR 沿用。

## 「比 Mp3tag 強」的定義（單曲範圍）

Mp3tag 強在批次、轉換、完整欄位；它沒有播放器整合與頻譜。單曲面板的差異化重點：

1. **完整原始欄位**：除常用欄位外，列出檔案內所有 frame／atom／Vorbis comment（含自訂 TXXX、`----:com.apple.iTunes:*`、未知項），可新增／刪除／編輯。
2. **音訊屬性**（唯讀）：容器／編碼、位元率（CBR/VBR）、取樣率、位元深度、聲道、時長、檔案大小、標籤版本（ID3v2.3/2.4、APEv2…）、ReplayGain 值。
3. **封面**：目前只處理**一張**（前封面），顯示尺寸／格式／大小，可**匯出成檔案**或**以新圖取代**；可調 JPG 品質與自動縮放（見 D9）。多張封面含類型（Front/Back/Artist…）延後。
4. **歌詞**：編輯一律寫入音訊檔的內嵌歌詞（USLT／LYRICS），不建立也不儲存 `.lrc`；若同資料夾已有 `.lrc`，只唯讀並列顯示並標示來源。支援同步 LRC 預覽並跟播放進度高亮。
5. **頻譜圖**：dB 色階、時間／頻率軸、截止頻率估計，作為「假無損」參考（只提示，不下定論）。
6. **與播放器整合**：從正在播放／任何列表開啟、儲存後列表即時更新、正在播放的檔案也能安全寫入。
7. **撤銷**：儲存前可逐欄還原；儲存後可「還原到上次儲存前」（見決策 D5）。

Picard／foobar2000 參考：foobar 的 Properties 分「Metadata／Details」兩分頁；Picard 顯示「原值 → 新值」差異。本設計採用：分頁＋暫存欄位以顏色標示已修改並可單欄還原。

## 決策（建議）

### 架構

- 新 Rust 模組 `track_tags`（放 `player-core` 或新 crate `player-tags`，Windows／Android 共用）：
  - `read_track_details(path) -> TrackDetails`：常用欄位（統一模型）＋ `raw_items[]`（原生鍵、值、型別）＋封面清單（不含 bytes，以 id 另取）＋音訊屬性＋歌詞來源。
  - `write_track_tags(path, TagEdit, expected_fingerprint) -> TrackDetails`：TagEdit 是**差異**（set/remove 的欄位、raw 項、封面操作），不是整份覆寫，避免抹掉未知欄位。
  - `expected_fingerprint`（大小＋mtime＋標籤區段雜湊）防止開啟面板後檔案被外部修改又被覆蓋。
- 前端：暫存狀態全在 Svelte（`draft` vs `original`），只有「儲存」才呼叫 IPC；關閉面板時有未儲存變更要確認。
- 介面設計保持「以單一路徑為參數」，不為批次預留抽象；未來若做批次，只需在外層迴圈呼叫同一個 `write_track_tags`。

### 寫入安全

1. 複製原檔到同目錄暫存檔 `.<name>.moetmp`，在暫存檔上由 lofty 寫入，`fsync` 後以 `ReplaceFileW`（Windows）／rename 原子取代；失敗則刪暫存檔、原檔不動。
2. 寫入後檔案 mtime **照常更新**為寫入時間，不保留舊 mtime、也不提供保留選項（已確認）。
3. 正在播放的檔案：Windows 播放端若以共用讀取開檔，ReplaceFileW 可能因無 `FILE_SHARE_DELETE` 失敗。建議播放端改以 `FILE_SHARE_READ|FILE_SHARE_DELETE` 開檔（已開的 handle 繼續讀舊資料）；若仍失敗，記住位置→釋放 handle→寫入→從原位置續播，不中斷使用者操作。
4. 備份：寫入前把原檔的**標籤區段**（非整檔）存到 UserData `tag-backups/<hash>.json`（含封面），保留最近 N 筆，供「還原上次儲存」。
5. 寫完：重讀標籤 → 只更新該曲 SQLite 投影與相關專輯／演出者聚合 → 發事件讓列表、正在播放、Windows SMTC／Android Media3 metadata 刷新；不全庫重掃。
6. Android：經 SAF 需要寫入授權（同 ADR 0010），沒有授權時欄位唯讀並提示重新加入來源；SAF 無法原子 rename 時改為「寫到 cache 暫存→驗證→以 `openFileDescriptor("rwt")` 覆寫」，先備份標籤。

### 格式支援（lofty 0.25）

| 格式 | lofty 寫入 | 備註 |
|---|---|---|
| MP3 ID3v2.4 | 可 | |
| MP3 ID3v2.3 | 不寫 | 已確認：MP3 一律升為 ID3v2.4 寫入（D3），讀取仍支援 2.3 |
| FLAC | 可（Vorbis comment＋PICTURE） | |
| M4A/ALAC | 可（ilst，含 freeform `----`） | |
| Ogg Vorbis / Opus | 可 | 封面為 METADATA_BLOCK_PICTURE |
| WAV | 可（RIFF INFO＋ID3v2 chunk） | INFO 欄位少，建議寫 ID3 chunk |
| APE | 可（APEv2） | |
| DSF | **可讀寫（2026-10-17 起）** | lofty 無 DSF；改由 `player-dsd` 解析檔頭與檔尾 ID3v2（`id3` feature），寫入時重寫檔尾 ID3v2.4 並更新檔頭的檔案總大小與標籤指標，音訊資料逐位元組不變 |
| DFF | 唯讀 | 尚無讀取器 |

替代方案：`id3` crate、`metaflac`、`mp4ameta`。建議 MVP 只用 lofty；若驗證 ID3v2.4 寫入或特定 frame 有問題再針對 MP3 換 `id3`。

### 頻譜圖

- **不嵌在詳細資訊面板內**（使用者確認）：面板上有「頻譜圖」按鈕，按下後在**獨立視窗**開啟。桌面：開新的 Tauri WebviewWindow（label `spectrogram`，重複開啟時聚焦既有視窗並切換曲目）；Android 沒有多視窗，改為全螢幕的獨立頁面（返回鍵關閉）。
- Phase 1 實作驗證（2026-10-16，`crates/player-tags/examples/verify_lofty_write.rs`）：MP3/FLAC/M4A/WAV 約 0.1–0.4 秒產生 1600×512 PNG。symphonia 沒有 Opus 解碼器、DSF 無讀取器，這兩種格式暫不提供頻譜圖。

- Rust 端以 symphonia 解碼、混為單聲道（或選聲道）、`realfft`（實數 FFT，比 rustfft 省一半）計算 STFT：FFT 4096、Hann 窗。
- 長檔：不處理每個樣本——把整首切成固定欄數（如 1600 欄），每欄只取一個窗口（或少量平均），時間複雜度與時長幾乎無關；一首 10 分鐘 FLAC 預估主要成本是解碼（可用 seek 跳讀）。
- 在背景執行緒（`spawn_blocking`）計算，可取消（關閉面板即中止），先回傳低解析度預覽再細化。
- 輸出：Rust 直接產生 PNG（dB −120…0 映射到 magma 類色階），前端以 `<img>` 顯示，軸線與刻度由前端 SVG 疊加；滑鼠懸停顯示時間／頻率／dB。
- 快取：UserData `spectrogram-cache/<路徑+大小+mtime 雜湊>.png`，有大小上限 LRU。
- 截止頻率估計：找高頻能量持續低於門檻的頻帶，顯示「約 16 kHz 截止，可能源自有損編碼」作參考。

### 歌詞「導入」按鈕（Phase 1）

- 歌詞欄位旁有「導入」按鈕：按下後以本曲**目前使用中的線上歌詞**取代歌詞欄位內容。
- 只是暫存編輯：算作未儲存變更（欄位標示已修改），儲存前可還原；按「儲存」才依 D7 寫入內嵌歌詞，不產生 `.lrc`。
- 本曲沒有使用線上歌詞時，按鈕停用（灰掉並以提示說明原因）。
- 現有程式中線上歌詞的存放方式：SQLite `player-db` 的 `track_lyrics` 表（`database.rs` 的 `get_track_lyrics`／`save_automatic_track_lyrics`／`save_manual_track_lyrics`），候選快取在 `lyric_candidates` 表。前端經 IPC `lyrics_get_track`（`src-tauri/src/lyrics_service.rs`）取得 `TrackLyrics`（`src/lib/ipc.ts`），其 `source` 為 `'local' | 'embedded' | 'netease' | 'qqmusic' | 'manual'`。判斷規則：`source` 是 `netease`／`qqmusic`／`manual`（手動選的線上候選）時才算線上歌詞、按鈕可用；`local`／`embedded` 或已清除（`cleared:` 候選）時停用。實作時再確認 `manual` 的實際來源。
- 格式：`TrackLyrics` 存的是解析後的 `lines`（含 `offsetMs`、`synced`），導入時需序列化成 LRC 文字（同步歌詞輸出 `[mm:ss.xx]`，套用 offset；非同步則輸出純文字）。若原始 LRC 文字仍保留在 DB，優先使用原文。

### 頻譜圖視窗功能（使用者確認）

- **不在視窗內換曲**（使用者更正：「切換音軌」指聲道）：沒有曲庫搜尋、佇列上／下一首或跟隨播放；視窗顯示從面板開啟的曲目，從面板再按「頻譜圖」會以 `spectrogram-track` 事件換成該曲。聲道選單依解碼串流的實際聲道數動態列出（支援 5.1、7.1 等任意聲道數），標籤取自 symphonia 聲道配置（L、R、C、LFE、SL、SR、RL、RR…，未知時為 `Ch n`），另有「混合（全部聲道）」與「各聲道上下分開」。
- **配色**：Spek（預設）、Magma、Inferno、Viridis、灰階；Rust 端 256 階 LUT，前端圖例以相同色標的 CSS 漸層顯示。
- **分析設定**：FFT 大小 1024–16384、窗函數（Hann／Hamming／Blackman／Blackman-Harris／矩形，以窗的相干增益正規化，滿幅正弦約 0 dBFS）、頻率軸線性／對數（對數下限 20 Hz）、dB 下限／上限、聲道（混合／單一聲道／各聲道上下分開）、最高頻率（空白＝Nyquist）。輸出解析度依繪圖區實際大小（×DPR，最多 2 倍）決定，視窗縮放後自動重繪；另有「重新繪製」。
- 設定存於 localStorage `moe.spectrogramSettings`（每裝置），經 `toRenderOptions` 轉成 Rust `SpectrogramOptions`（serde camelCase；聲道為 `{ mode, index }`）。
- 截止頻率提示改為「中位數高於 −100 dB 的最高頻率」，低於 Nyquist 93% 才顯示。

### 頻譜圖：開啟任意檔案與 A／B 比較（使用者確認）

- **開啟檔案**：「開啟 A」「開啟 B」以檔案對話框選取任意音訊檔（不必在曲庫中）。**拖放為必要功能**：拖進視窗即開啟；一次拖兩個檔案時分別載入 A 與 B；比較模式下可拖到 A 或 B 區域（並排時落在哪張圖就放哪個；疊圖／分割時左半＝A、右半＝B）；單張模式拖一個檔案取代 A。後端 `spectrogram_render_file(path, options)` 與曲庫曲目的 `track_spectrogram` 共用渲染器；從面板開啟時 A 為該曲目。
- **比較模式**（像圖片比較工具）：單張（只看 A）、並排（左右／上下）、疊圖（A／B 快速切換鍵 Space、B 透明度混合、差異圖）、滑桿拖曳分割（wipe）。
- **共用**：A／B 用同一組設定（配色、FFT、窗函數、軸、dB、聲道、最高頻率）與相同輸出尺寸；時間軸以兩者較長的時長為準，較短者依比例只佔部分寬度；頻率軸取兩者最高頻率的較大值。縮放／平移同步（滾輪縮放時間軸、Shift+滾輪縮放頻率軸、拖曳平移、雙擊重置）；縮放只放大既有影像，需要更細時按「重新繪製」。
- **讀值**：後端除 PNG 外另傳每像素 0–255 的量化強度（`levelsBase64`），前端換算 dB；滑鼠讀值同時顯示 A、B 的時間、頻率、dB 與差值。差異圖在前端以 canvas 由兩份強度計算（B − A，藍－白－紅發散色階，dB 範圍一半為滿色）。
- **檔案資訊**：A、B 各顯示檔名、取樣率、聲道數、時長、截止頻率提示。

### UI 位置

- 開啟（使用者確認）：**主要入口**是播放頁頂欄的資訊圖示按鈕（ⓘ，aria-label「歌曲資訊」），位於「羅」（羅馬拼音）切換與「快速設定」按鈕之間，開啟目前播放曲目的面板；曲目列右鍵／長按選單等其他入口為次要、日後再加。
- 桌面：沿用 `.floating-surface` 樣式的大型對話框（置中、可捲動），頂部為封面縮圖＋標題＋路徑（唯讀、「在資料夾中開啟」、「複製路徑」），分頁：**標籤**／**歌詞**／**封面**／**全部欄位**／**屬性**（含「頻譜圖」按鈕，開獨立視窗）。底部固定「還原」「取消」「儲存」，未儲存欄位以強調色標示。
- Android：全螢幕頁（返回鍵＝取消並確認），分頁改為頂部可橫向捲動的標籤列；頻譜圖可橫向捲動放大。

## 分期

- **Phase 1（MVP）**：**第一項實作：FFT 頻譜圖**（含快取、截止頻率提示，使用者確認為最高優先）；其後為常用欄位讀寫（標題、演出者、專輯、專輯演出者、作曲、碟號/總數、音軌號/總數、日期、內嵌歌詞（含「導入」目前線上歌詞按鈕）、單張封面：匯出／取代、JPG 品質、自動縮放）、暫存＋儲存、路徑與「在資料夾中開啟」、音訊屬性唯讀、原子寫入（mtime 照常更新）＋正在播放處理、DB 投影同步。格式：MP3（一律 ID3v2.4）/FLAC/M4A/Ogg/Opus/WAV/APE；DSF 唯讀。桌面＋Android。
- **Phase 2**：全部原始欄位檢視/編輯、多封面含類型、既有 `.lrc` 唯讀並列與同步預覽。
- **Phase 3**：儲存後撤銷（標籤備份）、ReplayGain 顯示/計算、檔名 ↔ 標籤（單曲依樣板重新命名，需同步播放清單路徑）、DSF 寫入。
- 未來（不在範圍）：多檔批次編輯。

## 待決事項（附建議預設）

- D1 模組位置：新 crate `player-tags` vs 放 `player-core`。建議新 crate。
- D2 mtime：**已確認**——寫入後 mtime 照常更新，不保留舊值，無相關設定。
- D3 ID3 版本：**已確認**——MP3 一律升為 ID3v2.4（含原本是 v2.3 的檔案）。
- D4 演出者等欄位：**已確認**——純文字輸入、原樣寫入單一字串，不做 `; ` 分隔或多值格式化；播放時由程式自動格式化。
- D5 撤銷：只做儲存前還原 vs 也做儲存後還原。建議 MVP 只做儲存前，Phase 3 加標籤備份。
- D6 正在播放的檔案：改播放端 share mode vs 寫入時暫停釋放。建議先改 share mode，失敗時退回「記位置→釋放→寫→續播」。
- D7 歌詞儲存位置：**已確認**——一律寫入內嵌歌詞，永不建立或儲存 `.lrc`；既有 `.lrc` 僅唯讀顯示。
- D8 頻譜圖格式：Rust 產 PNG vs 傳原始資料到 canvas。建議 PNG（簡單、可快取、Android 也省事）。
- D9 封面寫入：**已確認**——目前只處理一張封面，可匯出或取代。設定：JPG 品質可調；自動縮放開關，開啟時可設定短邊 px，一律保持長寬比。
- D10 DSF：MVP 唯讀 vs 自寫 ID3 尾端寫入。MVP 唯讀；**2026-10-17 修訂：DSF 改為可寫**（理由：ADR 0016 之後 DSF 可播放、可掃描，`player-dsd` 已能解析 DSF 並讀出檔尾 ID3v2；寫入沿用暫存檔 → 重讀驗證 → 改名取代流程，驗證包含音訊資訊與標題）。DFF 仍唯讀。
- D11 頻譜圖分期：**已確認**——頻譜圖移到 Phase 1，且為第一個實作項目。

## Phase 1 實作現況（2026-10-16）

- Rust：`crates/player-tags`（`read_details`／`write_edit`／`prepare_cover`／`render_spectrogram`），`src-tauri/src/track_details.rs` 提供 IPC 與頻譜圖視窗（label `spectrogram`，載入 `index.html?view=spectrogram&track=<id>`，已加入 capability `default.json`）。
- 前端：`src/lib/TrackDetailsDialog.svelte`、`src/lib/SpectrogramView.svelte`（`main.ts` 依 `view=spectrogram` 掛載）、`lyrics-import.js`（導入判斷與 LRC 序列化）、`cover-edit-options.js`。
- lofty 驗證（樣本複本）：MP3（含 v2.3→v2.4 升級）、FLAC、M4A、Opus、WAV（寫 ID3v2 chunk）皆可往返；ID3v2 不接受 `ItemKey::Lyrics`，改寫 USLT（`UnsyncLyrics`）。該 Ogg Vorbis 樣本 lofty 讀取失敗（與既有掃描一致），未處理。
- 偏離／暫定：封面設定（JPG 品質、自動縮放短邊）暫存於前端 localStorage（每裝置），尚未進 `settings.json`；儲存後以既有增量 `library_sync` 更新曲庫投影（會走訪來源目錄，尚非單曲投影更新）；JPEG 且不需縮放時原樣寫入以免重壓縮，其他格式一律轉 JPEG。Android 標籤讀寫與頻譜圖尚未接上（`track_path` 只支援 Windows 檔案路徑），Android 全螢幕頻譜頁的 UI 已就緒。

## 「導入使用中歌詞」（2026-10-17 使用者修訂）

- 按鈕改名「導入使用中歌詞」，取代先前只限線上來源的「導入」：播放器目前為此曲使用的歌詞，不論來源（網易雲／QQ 線上、同名 .lrc、手動選擇的歌詞檔、內嵌）都可匯入。
- 同步歌詞輸出為 LRC 時間戳；翻譯與羅馬音以相同時間戳的額外行跟在原文後。
- 沒有使用中歌詞，或使用中歌詞與欄位內容相同時停用；只暫存，按儲存才寫檔。

## Android 後端（2026-10-16）

- 曲庫曲目在 Android 是 `content://`（MediaStore 或 SAF 文件），不直接當檔案路徑用。讀取、封面、頻譜都先複製成 app cache 的私有工作檔（副檔名取自顯示名稱，沒有時看 MIME），再沿用桌面同一套 lofty／symphonia 程式。
- 存檔：工作檔上照桌面語意寫入（同目錄暫存→寫入→重讀驗證→rename），另留一份原檔還原點；再經 ContentResolver 寫回：先用 "rw" 開啟，拿到描述元之後才寫入，最後 truncate 到新長度，所以權限失敗時原檔不會被截斷。寫回後重新讀出並逐位元組比對，寫回或驗證失敗就推回原檔。無法做到真正的原子 rename，這是 Android 文件存取的限制。
- 權限：MediaStore 項目在 Android 11+ 以 `MediaStore.createWriteRequest` 請使用者同意，Android 10 用 `RecoverableSecurityException` 的授權畫面；使用者拒絕，或 SAF 資料夾只授權讀取，就回報需要重新授權（可寫入）。
- 封面選擇、頻譜圖「開啟 A／B」用系統文件選擇器（ACTION_OPEN_DOCUMENT），封面匯出用 ACTION_CREATE_DOCUMENT；Android 沒有拖放；「在資料夾中開啟」會開啟曲目所在資料夾。
- 頻譜圖在 Android 是全螢幕頁。單檔上限 512 MiB（快取租約上限）。

## 風險

- 寫入損壞音訊檔：以暫存檔＋原子取代＋重讀驗證緩解；仍須用各格式樣本做回歸測試。
- lofty 對罕見 frame 的往返可能遺失或改寫格式（例如 ID3v2.3 ↔ 2.4 日期 TYER/TDRC 轉換、COMM 語言碼）；需樣本檔測試，差異寫入而非全覆寫。
- Windows 檔案鎖定（播放端、防毒、雲端同步）導致取代失敗：給清楚錯誤並保留原檔。
- Android SAF 無原子 rename，失敗時可能留下半寫檔；以先備份與寫後驗證緩解。
- 長檔／高取樣率頻譜計算的 CPU 與電量（Android）：抽樣欄位、可取消、快取。
- DB 投影與檔案不一致（寫成功但更新 DB 失敗）：以重讀檔案為準，下次掃描修正。
