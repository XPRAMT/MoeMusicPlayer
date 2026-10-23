# 播放頁互動：封面縮放、手勢、睡眠定時與播放鍵狀態

Status: 已採納

## Context

播放頁的封面縮放、觸控／滑鼠手勢、睡眠定時與 seek 期間播放鍵的行為原本只記在 AGENTS.md，沒有對應的 ADR。此 ADR 記錄其目前設計。

## 目前設計

播放鍵與 seek 期間：`PlaybackService.kt` 的 `playbackStateName` 在 `playWhenReady && STATE_BUFFERING && 有項目` 時回 `"buffering"`（快照節流看 `playWhenReady`）；Rust `android_playback.rs` 的 `counts_as_playing`（playing／buffering）決定 `is_playing`、`resumePlayback` 與換曲 `play_when_ready`，收聽計時只算 `"playing"`；前端 `PlaybackState` 有 `buffering`，播放時鐘與現場播放次數只在 `playing` 前進。seek 走 `sendPlaybackCommand(..., { seek: true })`（設 `isSeeking`）；點擊處理在 `isSendingPlaybackCommand || isSeeking` 時忽略但外觀不變；上一首／下一首 `disabled` 只看 `playbackNavigationReady` 與有無上／下一首；播放、隨機、循環在其他指令忙碌時 disabled。測試：`tests/seek-transport-lock.test.mjs`、`PlaybackProtocolStateTest.aSeekWhilePlayingIsBufferingNotAPause`（`src-tauri/gen/android` 下 `gradlew :tauri-plugin-media-index:testDebugUnitTest`）、`android_playback::integration_tests::a_seek_buffering_snapshot_keeps_playing_without_flipping_resume_or_counting_time`。

正在播放封面縮放（`src/lib/cover-zoom.js`，action `coverZoom` 掛在 `.cover-stage`，`tests/cover-zoom.test.mjs`）：滾輪（以游標為錨點）或雙指縮放，0.5–10 倍，雙擊恢復，放大後可平移（滑鼠拖曳、觸控雙指，夾在溢出量內）；只對封面套 `transform: translate scale`；佈局以 `--np-cover-width`／`--np-cover-height` × `--np-cover-zoom` 預留欄寬（窄版列高），最多佔卡片 65%；縮小時封面欄 fr 份額由 `coverShrinkShare` 收斂（`--np-art-col-a/-b`／`--np-art-row`），歌詞份額（`--np-lyr-*`）同量增加，兩軌 fr 總和固定為 2；倍率 1 不預留（`--np-cover-reserve` 0）。縮放中的封面以 `data-cover-zoomed` 屬性標記（不用 class）；基準尺寸用 1 倍時的 `coverBaseAvailable`；每步縮放後在 rAF 以 `settleAnchor` 補償；第二指以合成 pointercancel 取消第一指的滑動（`dispatchingCancel`）。直排／橫排只看長寬比例（`@media (orientation: portrait)` 直排，封面 1.1fr、歌詞 0.9fr）。狀態保存在 `coverZoomPersisted`（action 的 `initial`／`syncScale`）：縮小的封面切歌後維持，放大的切歌即還原；跨重啟只記縮小（`settings.json` 的 `coverZoom.scale` 0.5–1，`settings_get/set_cover_zoom`，300 ms 去抖；`coverBaseReady` 前忽略）。Release 與手機實測待回報。

手勢：`src/lib/swipe-dismiss.js`（`use:swipeDismiss`，測試 `tests/swipe-dismiss.test.mjs`）。觸控／筆超過 10 px 鎖定主軸，非關閉軸交還瀏覽器；從 input／select／textarea／`[role="slider"]`／`[data-swipe-ignore]` 開始的手勢忽略；跟手用 CSS `translate`，放開時位移 ≥ 30% 或速度 ≥ 0.5 px/ms 即觸發，否則 180 ms 彈回；滑動後吞一次 click；reduced-motion 不播動畫。綁定：窄版導覽面板左滑 → `closeNavPanel`，快速設定右滑 → `closeQuickSettings`；正在播放封面 `follow: false, damped: true`（橡皮筋約 24 px）：左滑（deltaX < 0）→ 下一首、右滑 → 上一首（使用者實測確認，勿對調）、下滑 → `closeNowPlaying`；歌詞區（`.now-playing-lyrics`，`touch-action: pan-y`）同樣左右切歌，回饋經 `feedbackTarget` 播在封面。滑鼠（`mouseActions`，僅主鍵）：整個 `.now-playing-view` 水平拖曳切歌，`.cover-stage` 向下拖曳收合；`data-cover-zoomed` 時滑鼠拖曳是平移；拖曳鎖定時清除文字選取（`getSwipeActionsFor`／`isSwipeMouseStart`）。

睡眠定時：播放頁頂欄「羅」與歌曲資訊（ⓘ）之間的鬧鐘按鈕（aria-label／tooltip「睡眠定時」，啟用時 `.is-active` 並在 tooltip 顯示剩餘時間）開啟向下的 `.floating-surface` 面板（定位共用 `getPopoverHorizontalShiftPx`），點外部或 Esc 關閉。hh:mm 顯示剩餘（向上取整、每秒更新）；小時、分鐘為兩個 `role="slider"` 欄位，上下拖曳（每 14 px 一格，`touch-action: none`）、滾輪（像素模式每 50 px）或方向鍵調整，各自夾住不進位（`adjustSleepTimerField`），上限 23:59（Rust `MAX_SLEEP_TIMER_MS`）；以手勢為單位凍結快照（`beginSleepTimerEdit`／`stepSleepTimerEdit`，滾輪／方向鍵 800 ms 無動作結束）；`sleep_timer_set` 回應以序號丟棄過期結果；> 0 啟動、0 或「取消定時」關閉。期限由 `src-tauri/src/sleep_timer.rs` 持有（epoch 毫秒截止＋世代號，每秒重讀系統時鐘），到期送 `sleep-timer-changed` 並 `playback_pause`；前端 `src/lib/sleep-timer.js` 只做顯示與換算。
