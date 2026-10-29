# 現況：曲庫同步與資料

本文件是此領域「目前狀態」的快照，不是日誌；維護規則同根目錄 `AGENTS.md`「本文件維護規則」：就地改寫既有條目、不寫日期／量測數字／除錯經過，設計細節放 `docs/adr/`。

### 曲庫同步與資料

曲庫同步（ADR 0040，`crates/player-core/src/sync.rs`）：查資料庫分類 → 平行讀標籤 → 依原順序寫入；`cancel_library_sync` 取消時已讀進度以「不完整」寫入（不對帳、不移除），下次續掃。資料夾與播放清單共用進度流程（`src-tauri/src/playlist_scan_index.rs` 的 `PlaylistScanIndex`）；新增來源種類只需提供查詢、`.lrc` 查找與標籤讀取三個函式，不要另寫索引與進度。所有平台與遠端共用 `player-track-metadata` 讀標籤（不讀封面；多值演出者以 ` | ` 連接）；內嵌歌詞只經 `player_tags::embedded::read_embedded_lyric_texts`（含 DSF、APE 家族）。

歌詞不存內容（ADR 0040，schema 21）：同步不存任何歌詞文字，內嵌歌詞什麼都不記；旁檔（`.lrc`／`.qrc`／`.ass`／`.srt`，一首只記排序最前者）只記位置（`track_lyric_sidecars` 的檔名、`lyric_sidecar_sources` 標記已掃描，`Database::track_lyric_sidecar` → `LyricSidecarLookup`），只有 `Complete` 掃描才刪除沒看到的旁檔。播放時 `local_lyrics_for_track` 讀該曲標籤（遠端與封面查找共用一次標籤讀取，`remember_remote_tag_lyrics`），再讀記下的旁檔（`NotScanned` 才在旁邊找，`Absent` 不讀）；只讀這一首。遠端旁檔經 `RemoteContent::open_cached` 走音訊的磁碟快取，沒有其他歌詞儲存。

播放時檢查檔案（ADR 0040）：目前曲目換成另一首時頁面呼叫 `playback_check_track_file`（立即返回），背景 `track_freshness::check_track_file` 以本機 stat／遠端 `RemoteContent::stat`（連不上就靜默放棄）比對 `track_file_mapping` 記錄的大小與修改時間；變了就重讀標籤（`refresh_track_metadata`＋`update_mapping_fingerprint`）、遠端 `file_changed`（下次開檔用新版）、清該專輯封面選擇、`refresh_current_track`（`current-track-refreshed`）並送 `track-file-refreshed`，頁面重載清單、封面與歌詞；不重開、不 seek 播放中的音訊。限制：Android WebView 暫停期間換的歌在頁面恢復後才檢查。測試 `track_freshness::tests`、`tests/track-details-refresh.test.mjs`。

冷建庫效能（ADR 0027）：設 `MOE_LIBRARY_PROFILE=<檔案.jsonl>` 寫出各階段事件；無 GUI 基準 `scripts\profile-cold-library.ps1 -MusicRoot D:\Music -Runs 5`（不碰播放器與 UserData）；冷建庫批次模式 `player-db` 的 `BulkBuild`。下一步：GUI 冷建庫量測、依磁碟實體位置排序讀取、冷狀態量 HDD 執行緒數。

遺失檔案隱藏（ADR 0033，schema v20）：`source_mappings.missing_since_utc_ms`；所有清單查詢加 `m.missing_since_utc_ms IS NULL`，記錄、覆寫與播放紀錄保留，再找到就自動恢復（`persist_mapping` 清為 NULL）。播放清單來源只在 `MediaSourceErrorKind::MissingFile`（NotFound 且 `volume_root` 仍在）時標記；資料夾來源（各平台與遠端）在 Complete 掃描把沒看到的對應標為遺失；遠端播放清單只有帳號根目錄（`list("")`）可連時才算遺失。設定「管理音樂來源」最近一次同步摘要的遺失檔案錯誤（`SourceSyncError.missingFile`、`itemKey`）旁有「從播放清單移除」→ `sync_remove_missing_playlist_entry`（伺服器清單先確認檔案不存在，再走 `remote_favorites` 取消收藏流程；本機與播放頁取消收藏共用 `rewrite_local_playlist_membership`），完成後重新同步。限制：搬移或改名的資料夾檔留下隱藏舊對應；新排除子資料夾內的檔案變成隱藏。

逐曲歌詞延遲（schema 25）：`track_lyrics_timing_offsets`（`track_id` 主鍵、`delay_tenths` 為 ±100 的非零整數，0.1 秒；沒有列即 0），與播放統計一樣以 Track ID 為鍵、不設外鍵，曲目移除後再加回仍保留；隨資料庫一起備份。`Database::lyrics_timing_offset_tenths`／`set_lyrics_timing_offset_tenths`（0 刪列）／`import_lyrics_timing_offsets`（單一交易、已有值不覆蓋）。舊版存在瀏覽器 localStorage 的 `moemusicplayer.lyricsTimingOffsetSec:<trackId>`，由 `lyrics-timing-offset.js` 的 `migrateLegacyLyricsTimingOffsets` 在啟動時（以及第一次讀寫前）讀一次交給 `lyrics_timing_offsets_import`，成功才刪除舊 key，失敗留待下次；不是 Track ID 的 key 丟棄。
