# 現況：歌曲資訊面板

本文件是此領域「目前狀態」的快照，不是日誌；維護規則同根目錄 `AGENTS.md`「本文件維護規則」：就地改寫既有條目、不寫日期／量測數字／除錯經過，設計細節放 `docs/adr/`。

### 歌曲資訊面板（ADR 0013）

面板：播放頁頂欄資訊按鈕開啟 `TrackDetailsDialog`（portal 掛到 `.app-shell`）。欄位暫存，「儲存」（或 Ctrl+S）才經 `track_details_save` 寫檔（`crates/player-tags`：同目錄 `.moetmp-<檔名>` 暫存→寫入→重讀驗證→rename；Windows rename 前把暫存檔的建立時間設回原檔的（`replace_original`，盡力而為、失敗只記錄），mtime 照常更新；MP3 一律 ID3v2.4；歌詞一律寫內嵌、不建 `.lrc`）；歌詞欄有改（`lyricsChanged`）時同一次儲存把手動列設為內嵌參照（`link_embedded_lyrics`）。本次面板以非 0 延遲「導入使用中歌詞」後，下一次寫入歌詞的儲存把該曲延遲歸零（之後編輯過也一樣；「清空」或延遲 0 再匯入則取消，`nextBakedDelayState`，見 ADR 0013）。位元率全 App 只有一套 `formatBitrateKbps`（`track-details-format.js`）。修改／建立時間以本機時區顯示到秒，只有後端 `timesAbsolute` 為 false（FAT／exFAT、SD 卡／USB、content://、未知）時才加 `(UTC+8)` 類時區；判斷在 `player_tags::file_times`（見 ADR 0013「時間顯示」）。

封面：`player_tags::cover` 的 `plan_cover`／`prepare_cover`（各容器相同）：一律 JPG，真的有透明像素時改 PNG；「儲存時：…」由 `track_details_cover_plan` 顯示同一份計畫。複製→貼上保留原檔（`src/lib/cover-clipboard.js`）。Android 剪貼簿讀寫都走原生 ClipboardManager（`track_details_clipboard_has_image`／`track_details_read_clipboard_image`／`track_details_copy_cover`），WebView 的 clipboard API 不可用。

儲存後：`reloadAfterTrackDetailsSaved(trackId)` 只重讀這個檔案的標籤（`Database::refresh_track_metadata`），只清該專輯的封面選擇（`album-artwork-invalidated`），再 `playback_refresh_current_track` 並廣播 `current-track-refreshed`；不跑整個曲庫同步。Android 通知欄不跟著更新（下次切歌才更新）。

平台：Android 曲目為實體路徑（ADR 0014），讀寫與 Windows 相同；「在資料夾中開啟」為 `track_details_reveal`（不可加 FLAG_GRANT_READ_URI_PERMISSION）。SMB 曲目的時間取自伺服器 `stat`，寫回時保留原建立時間。遠端曲目的讀取與儲存見「遠端來源」。
