# AGENTS.md

## 專案現況與目標

目前只有本規格文件，尚無程式碼、測試、README 或 ADR。下一步是建立最小可執行骨架，先驗證資料來源、持久化與播放路徑；本文件描述目標，不代表功能已完成。

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
- **搜尋：**100,000 首仍可快速查詢，使用分頁或虛擬列表；不得先把全庫載入前端再篩選。SQLite FTS5 是預設候選，需由實測確認。
- **播放：**播放/暫停、前後首、跳轉、音量、循環、隨機與佇列。架構需容納無縫播放。Windows 整合系統媒體控制；Android 使用 Media3/MediaSessionService，支援背景、通知列與藍牙控制，Activity 關閉後仍可依平台規範播放。
- **歌詞：**支援本地 LRC、可取得的內嵌歌詞、網易雲與 QQ Provider、同步顯示、候選手動搜尋/指定及持久快取。Provider 與 UI、Library Core 分離，需有逾時、可取消與受控重試；網路失敗不得阻礙啟動、曲庫瀏覽、本地歌詞或播放。自動匹配須綜合標題、演出者、長度、專輯、版本與 feat. 資訊；低信心不自動套用。使用者指定與本地歌詞優先於網路自動結果。
- 歌詞匹配要有可解釋分數與信心門檻。標題、演出者權重高；長度差異有實質影響；專輯為低權重。正規化大小寫、全半形、空白、常見括號及 feat./featuring/ft.，但須保留 Live、Cover、Acoustic、Instrumental、TV Size、Remaster 等版本差異；手動指定須持久保存並優先於後續自動搜尋。播放期間不得批量觸發網路補全。
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
- **Android 媒體來源**一般共享媒體使用 MediaStore；未涵蓋或需授權的來源使用 SAF，並在平台允許時保存持久 URI 權限。MediaStore 欄位不足時才針對檔案補解析。
- 系統索引失效或回退掃描時，不能要求使用者重建整個 App SQLite；平台 adapter 需將 SystemIndex/MediaStore/SAF 結果正規化為共同的曲目模型，再由同步層與 App 資料合併。
- **App SQLite**保存內部 ID、啟動/離線顯示所需的曲目投影、來源映射、使用者覆寫、播放清單、歌詞/封面快取、設定與同步狀態。系統媒體索引用於確認來源現況，但不得覆蓋使用者資料。使用 migration、批次交易、索引與必要的 FTS；不得逐曲提交交易。快取有上限、可清除、可觀察。
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
- [ECHO NEXT 授權檔](https://github.com/Moekotori/ECHO/blob/main/LICENSE)標示為 source-available，並非一般開源授權；不得直接複製其原始碼，亦不得以其 fork 作為本專案可公開分發的基底，除非取得作者明確許可。

### [Any Listen](https://github.com/any-listen/any-listen)

- 可研究 Provider／擴充套件分層、本地列表同步的操作體驗、現代 Web 播放器介面，以及跨模組共用資料模型；須自行驗證對本地優先與低資源用量的適用性。
- [Any Listen 授權檔](https://github.com/any-listen/any-listen/blob/main/LICENSE)以 AGPLv3 為基礎並加入商業使用限制；不要直接複製程式碼，除非已確認授權範圍及本專案分發方式可接受。

### [MusicPlayer2](https://github.com/zhongyang219/MusicPlayer2)

- 可研究 M3U/M3U8 匯入匯出行為、本地播放器功能完整度、網易雲／QQ 歌詞來源的使用體驗，以及 Windows 桌面播放控制細節；以行為測試重新實作，不移植其程式碼。
- [MusicPlayer2 授權檔](https://github.com/zhongyang219/MusicPlayer2/blob/master/LICENSE)為 GPLv3；如考慮使用其程式碼，須先評估 GPL 義務與本專案的授權策略。

## 工作方式與下一步

1. 開工先閱讀本文件、相關程式碼與 ADR；以最新需求、程式和測試結果修正過時敘述。較大修改先說明目標、落點層級與硬限制，區分平台硬限制、工程限制和慣例假設；比較至少兩種方案，必要時用原型或 benchmark 找瓶頸。重大架構變更寫入 `docs/adr/`，優先修正根因，不以 UI 權宜作法掩蓋後端問題。
2. 變更保持聚焦，不自行改變硬性需求或更換核心框架。引入依賴前確認必要性、重量、維護狀況與授權；參考外部專案時遵守上節邊界。臨時做法要標記 temporary、建立 issue/TODO 並避免成為長期依賴。
3. 每次修改後提交 Git，包含程式碼、`AGENTS.md`、雙語 README 與必要 ADR；執行暫存資料和使用者設定加入 `.gitignore`。新文字檔優先 UTF-8 with BOM，除非工具鏈要求其他編碼。README 使用英文 `README.md` 與繁體中文 `README_TW.md`，頂端皆放 `[English](README.md) | [繁體中文](README_TW.md)`，內容保持同步且只寫使用者需要的用途、功能、安裝與基本用法。技術文件的時間紀錄必須帶時區。
4. 每次收尾更新本文件的**目前狀態、未解問題與下一步**；不累積版本歷史。需要重啟任何裝置時，先說明原因並取得使用者明確同意，或由使用者親自操作。

階段順序保留原規劃的範圍：

1. Tauri/Svelte 骨架、媒體資料抽象、Windows SystemIndex 與 Android MediaStore 適配、App SQLite、來源管理、Windows fallback、metadata、分頁列表與基本播放。
2. M3U/M3U8、搜尋/FTS、專輯與演出者分組、封面快取、佇列。
3. 本地與內嵌歌詞、窗口渲染、網易雲與 QQ Provider、可解釋匹配及手動覆寫。
4. Windows 系統整合、Android Media3/MediaSession 背景播放與 SAF 來源。
5. 核心穩定後評估無縫播放、進階音訊裝置、DSP/EQ、遠端曲庫/同步與插件；Phase 1 不先投入大量視覺特效。

目前下一步：建立最小可執行骨架與雙語 README；先做 Windows 的來源同步、SQLite 持久化、分頁列表和基本播放端到端驗證，再完成同階段的 Android MediaStore 適配。已知未解：SystemIndex 的覆蓋率與 metadata 品質、100,000 首效能、Windows 音訊選型及 Android 背景服務均尚無原型或實測；不要將文件中的預設方案視為已驗證。每一步以可運作功能與測量結果更新本文件。
