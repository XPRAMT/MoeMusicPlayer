# 曲庫同步流程與本機歌詞資料

Status: 已採納

## Context

曲庫同步的三段流程、取消與續掃、共用進度流程，以及 `.lrc`／內嵌歌詞隨同步存入資料庫的設計原本只記在 AGENTS.md。此 ADR 記錄其目前設計。

## 目前設計

曲庫同步（`crates/player-core/src/sync.rs`）：三段——（1）只查資料庫分出未變、舊版中繼資料、新增／變更；（2）需解析的曲目平行讀標籤（`METADATA_READ_THREADS` 4，轉接器提供 `MediaIndex::parallel_metadata_reader()` 才平行，遠端以 `metadata_read_threads` 提示 16；單檔 panic 只算該檔失敗）；（3）依原順序寫入。關閉視窗或更新重啟時 `cancel_library_sync`（`AppState.active_sync` 的 `SyncCancellation`）：已讀進度以「不完整」寫入（只新增／更新、不對帳、不移除），下次從中斷處續掃；一首都沒處理則不寫入。資料夾與播放清單共用進度流程：`MediaIndex::scan_with_progress` 回報 `MediaScanProgress`，播放清單（本機 Windows、本機 Android、伺服器）共用 `src-tauri/src/playlist_scan_index.rs` 的 `PlaylistScanIndex`，三種查詢（`WindowsMediaIndex::scan_playlist_paths_with_progress`、`fs_scan::scan_paths_with_progress`、`RemoteMediaIndex::scan_paths_with_progress`）回報「已查／總數」；新增來源種類只需提供查詢、`.lrc` 查找與標籤讀取三個函式。標籤讀取在 `player-track-metadata`（`ByteSource`，本機與遠端共用；`scan_parse_options` 一律 `read_cover_art(false)`）。

設計規則（使用者定案）：檔案本身的內容是最新事實，資料庫只是索引與快取。標籤、內嵌歌詞、旁檔、封面等以檔案當下內容為準，資料庫內容只在讀不到檔案時才使用；任何檔案（本機與遠端）開始播放時都在背景檢查是否為最新（大小／修改時間），有變就立即更新不影響播放的資料，不得卡住播放。

本機歌詞不存內容（schema 21，本機與遠端共用）：曲庫同步不存任何歌詞文字。內嵌歌詞什麼都不記（連「有沒有」都不記；`TrackMetadata` 沒有歌詞欄位，掃描不讀）。旁檔（`.lrc` > `.qrc` > `.ass` > `.srt`，`LYRIC_SIDECAR_EXTENSIONS`，ADR 0041）只記位置：`track_lyric_sidecars`（`source_id`、`source_item_id`、`sidecar_name`），即該 mapping 的音訊檔旁的檔名；掃描器從資料夾清單取得（`MediaIndex::take_lyric_sidecars`／`scans_lyric_sidecars`；播放清單來源對每個歌曲所在資料夾各列一次，`RemoteMediaIndex::lyric_sidecars_beside`、`lyric_sidecars_beside`），`SyncEngine::sync_lyric_sidecars` 只寫新增或改名者、不讀檔；每次未取消的掃描都在 `lyric_sidecar_sources` 標記已掃描，只有 `Complete` 才刪除沒看到的旁檔。`Database::track_lyric_sidecar` 回 `player_core::LyricSidecarLookup`：`Present { audio, name }`（`sidecar_locator()` 算出旁檔位置）、`Absent`（來源掃描過且沒有）、`NotScanned`。播放時 `lyrics_service::local_lyrics_for_track`（自動順序「內嵌 → 旁檔 → 線上」，每首各自決定）每次讀檔案當下內容：內嵌歌詞讀該曲標籤（本機直接開檔；遠端與同一首的封面查找共用一次標籤讀取：封面查找把同一份標籤裡的歌詞文字以 `player_core::remember_remote_tag_lyrics` 依來源、路徑與大小暫留 60 秒，歌詞查找先取 `remembered_remote_tag_lyrics`，沒有才自己讀並暫留；不存入任何地方），再讀 `Present` 指向的旁檔（只有 `NotScanned` 才在旁邊依序找，`Absent` 不讀）；只讀這一首的檔案，不讀清單其他曲目。遠端旁檔經 `RemoteContent::open_cached`（`RemoteFiles::cached_file`，與音訊同一個磁碟快取 `DiskCache`／`CachedFile`）讀取，讀過整個檔案就算已快取，伺服器關閉時與已快取的歌一樣可讀；沒有其他歌詞儲存（舊的 `remote_lyric_cache` 已刪除）。連不上的伺服器由 `GuardedFs` 退避立即失敗，第一個網路錯誤後不再試其他檔案；本機曲目不受影響。手動選本機來源（`local:embedded`／`local:sidecar`）與候選清單的本機項目也讀當下內容；線上歌詞的手動選擇仍存文字（那是唯一一份）。遷移（v20 → v21，單一交易）：刪除 `track_embedded_lyrics`、`remote_lyric_cache`，`track_lyric_sidecars` 以複製改名只留三欄，其餘資料不動。Matroska 與 Android 備援解析不帶歌詞。

播放時的檔案檢查：頁面在播放中的曲目換成另一首時呼叫 `playback_check_track_file`（立即返回），背景執行緒 `moe-track-check` 執行 `track_freshness::check_track_file`：取該曲第一個啟用的 mapping（`Database::track_file_mapping`，檔案系統優先）記錄的大小與修改時間，本機以 `fs::metadata`、遠端以 `RemoteContent::stat` 取當下值（連不上或退避中就放棄、不通知）；大小不同，或兩邊都有修改時間而不同，才算變了。變了就：遠端先 `RemoteContent::file_changed`（`RemoteFiles` 更新磁碟快取記住的大小與時間，下次開檔改用新版，正在播放的串流續讀它開的版本）並 `forget_remote_tag_lyrics`；以同步用的讀取器重讀標籤（Windows `WindowsMediaIndex`、Android `media_index().read_metadata`、遠端 `RemoteMediaIndex`），`refresh_track_metadata` 寫入並 `update_mapping_fingerprint` 記下新的大小與時間（下次同步不再重讀）；清掉前後專輯的封面選擇並送 `album-artwork-invalidated`；`refresh_current_track` 更新播放快照的曲目摘要（Windows 另刷新 SMTC、Android `RefreshCurrentTrack`）並送 `current-track-refreshed`；最後送 `track-file-refreshed`，頁面遞增清單版本並重載目前曲目的封面與歌詞（歌詞本來就讀當下檔案）。不開檔播放、不重開、不 seek。限制：觸發點在頁面，Android WebView 暫停期間換的歌在頁面恢復、看到新曲目時才檢查；外部開啟（不在曲庫）的檔案不檢查。
