# crates/AGENTS.md

適用於 workspace Rust crates；同時遵守根目錄 `AGENTS.md`。架構決策的理由及遷移歷史以 `docs/adr/` 為準，本文件只列現在必須維持的行為。

## SQLite 與列表查詢

- SQLite 負責歌曲、artist／album 快照、搜尋、欄位過濾、計數、排序、分組與快速索引邊界。大型快照在資料庫端完成聚合、正規化和排序，前端以固定 1,000 筆 offset/limit 讀頁；不得把全量原始 facet 傳到 renderer，也不得以多頁迴圈重做全量聚合。
- 排序、分組、快速索引要使用同一個完整 SQL 排序順序；文字 collation 共用 `MUSIC_LOCALE_NATURAL` ICU4X。Windows locale 來自 `GetUserDefaultLocaleName`，Android 來自 `Locale.getDefault()`，台灣採 Zhuyin。符號、數字、英文字母先於地區文字；未知是獨立組，正反序依整體順序反轉。平台原生索引 API 不參與分類。
- 演出者／專輯的 identity 正規化、代表顯示名稱、統計和 aliases 必須由 SQLite 函數與 SQL snapshot 一起產生。未標籤項目以獨立 unknown identity 保存，不與真正叫「未知」的標籤合併；未知依所選排序值與方向排序。
- 播放清單重複項按原始 position 保留；播放佇列保留重複 queue entry、traversal 次序與目前 entry identity。對佇列排序回傳 entry index，由播放服務更新 traversal，不以 track ID 去重。
- 正式曲庫與 Windows 外部佇列暫存曲庫可能同時查詢；兩庫各自 SQL 篩選，locator 批次去重，跨庫排序／分組在 SQLite 中完成。暫存曲庫以記憶體 SQLite 運作，生命週期屬於當次外部佇列。

## 持久化與掃描

- schema 遷移需可重入、交易化且保留可恢復資料。來源同步只變更該來源 mapping；部分失敗／離線來源不得刪除舊投影、播放歷史、session 或 unmatched playlist locator。
- `settings.json` 保存設定和 source registry；SQLite 保存媒體索引投影、穩定 Track ID、playlist、歌詞／候選、統計與播放 session。資料來源更新不能覆寫使用者設定或人工歌詞選擇。
- Play statistics 僅在 Playing 且自然位置前進時計入；暫停、載入、seek 跳轉不計時。shuffle ratio 為實際累積播放時間／曲目時長，低比值在新 traversal 提升抽中權重，未知時長採中性權重。
- Windows 檔案路徑在內部保留 Unicode／長路徑資訊，locator 以明確平台來源保存；Android Content URI 不可轉成假本機路徑。時間持久化使用明確時區或 UTC epoch。

## 音訊與測試

- 共用 playback policy 不應強迫共用平台 decoder/output；backend 是 position、duration、播放狀態的權威。metadata 的來源位元深度不可用解碼後 PCM 位元深度代替。
- 修改 parser、scanner、schema、query 或 audio path 時新增可重現回歸測試；用 fixture／暫存資料庫，避免直接修改使用者音樂或正式 UserData。
- 索引與資料庫測試至少涵蓋正反序、空／未知欄位、重複曲目、穩定順序、locale collation、來源停用／恢復及 migration。音訊測試分開驗證 metadata、decode、duration、seek 與輸出；單一 codec fixture 不代表所有檔案都可播放。
- 使用 ECHO、Any Listen 或 MusicPlayer2 只作行為與設計參考，不複製其程式碼；採用依賴前重新核對版本與授權。專案既有 ADR 記錄外部參考與限制。