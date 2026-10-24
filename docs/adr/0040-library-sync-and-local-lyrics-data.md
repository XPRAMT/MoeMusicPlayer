# 曲庫同步流程與本機歌詞資料

Status: 已採納

## Context

曲庫同步的三段流程、取消與續掃、共用進度流程，以及 `.lrc`／內嵌歌詞隨同步存入資料庫的設計原本只記在 AGENTS.md。此 ADR 記錄其目前設計。

## 目前設計

曲庫同步（`crates/player-core/src/sync.rs`）：三段——（1）只查資料庫分出未變、舊版中繼資料、新增／變更；（2）需解析的曲目平行讀標籤（`METADATA_READ_THREADS` 4，轉接器提供 `MediaIndex::parallel_metadata_reader()` 才平行，遠端以 `metadata_read_threads` 提示 16；單檔 panic 只算該檔失敗）；（3）依原順序寫入。關閉視窗或更新重啟時 `cancel_library_sync`（`AppState.active_sync` 的 `SyncCancellation`）：已讀進度以「不完整」寫入（只新增／更新、不對帳、不移除），下次從中斷處續掃；一首都沒處理則不寫入。資料夾與播放清單共用進度流程：`MediaIndex::scan_with_progress` 回報 `MediaScanProgress`，播放清單（本機 Windows、本機 Android、伺服器）共用 `src-tauri/src/playlist_scan_index.rs` 的 `PlaylistScanIndex`，三種查詢（`WindowsMediaIndex::scan_playlist_paths_with_progress`、`fs_scan::scan_paths_with_progress`、`RemoteMediaIndex::scan_paths_with_progress`）回報「已查／總數」；新增來源種類只需提供查詢、`.lrc` 查找與標籤讀取三個函式。標籤讀取在 `player-track-metadata`（`ByteSource`，本機與遠端共用；`scan_parse_options` 一律 `read_cover_art(false)`）。

`.lrc` 與內嵌歌詞隨同步存入資料庫（schema 18–19，本機與遠端共用）：掃描器記下同資料夾同主檔名（不分大小寫）的旁檔（`.lrc`、`.qrc`、`.ass`、`.srt`，同一首有多個時取 `.lrc` > `.qrc` > `.ass` > `.srt`，`LYRIC_SIDECAR_EXTENSIONS`，ADR 0041）（`MediaIndex::take_lyric_sidecars`／`scans_lyric_sidecars`；播放清單來源對每個歌曲所在資料夾各列一次，`RemoteMediaIndex::lyric_sidecars_beside`、`lyric_sidecars_beside`），`SyncEngine` 的 `sync_lyric_sidecars` 以檔名＋大小＋修改時間為指紋比對 `track_lyric_sidecars`，變動才讀內容（上限 2 MiB、去 BOM、過大記 oversized）；每次未取消的掃描都在 `lyric_sidecar_sources` 標記已掃描，只有 `Complete` 才刪除沒看到的旁檔。內嵌歌詞於讀標籤時一併取得（`embedded_lyrics_text`，`TrackMetadata.embedded_lyrics` → `track_embedded_lyrics`，有列＝已讀過；`TRACK_METADATA_VERSION` 7）。播放時 `lyrics_service::local_lyrics_for_track` 依「內嵌 → 旁檔 → 線上」（ADR 0041）：先用資料庫的內嵌歌詞（`track_embedded_lyrics`），沒有可用內容才問 `Database::track_lyric_sidecar`（`Present` 直接用資料庫文字，都不開檔）；兩者都已知為空就回「沒有」，`NotScanned` 或尚無內嵌列時才走開檔查找（`raw_local_lyrics`，同一順序），遠端曲目的查找結果另存 `remote_lyric_cache`（schema 17，以 `remote_file_fingerprint` 失效；「沒有歌詞」只在伺服器連得上時記住；`clear_remote_lyric_cache` 清單曲）。Matroska 與 Android 備援解析不帶歌詞。
