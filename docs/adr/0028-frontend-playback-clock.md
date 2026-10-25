# Front-end playback clock instead of 10 Hz position polling

Status: Accepted and implemented on 2026-10-22 (front end only; backend push not done yet)

Decision date: 2026-10-22, Asia/Taipei (UTC+08:00)

## Context

The main window called `playback_get_snapshot` every 100 ms just to move the progress bar and the lyrics, and the
taskbar lyrics window ran its own 100 ms poll (20 IPC calls per second with both open). The backend pushes no
playback-state events, so the poll did two jobs at once: animating the position and noticing state changes.

## Decision

- The main window samples every 500 ms (`PLAYBACK_POLL_MS`), also while minimized, and takes a sample at once after
  any playback command, when the page becomes visible, on window focus, on `audio-devices-changed` and when the clock
  reaches the end of the track.
- `src/lib/playback-clock.ts` estimates the position between samples: base position + elapsed `performance.now()`
  time × rate (1 while playing, 0 otherwise), clamped to 0..duration. Corrections: deviation < 30 ms ignored;
  30–250 ms absorbed by a rate change over 750 ms (rate stays above 0, so never backwards); anything larger, a new
  track, a play/pause change or a new duration jumps and bumps `jumpId`.
- The progress bar is a linear Web Animation (`src/lib/progress-run.js`, `use:progressRun`) from the current ratio to 1
  over the remaining time, and it animates `transform` only, so it runs on the compositor and costs no style, layout or
  paint work per frame. The glow track is a fixed gradient band 3 × the track wide (`.progress-band`, translated by
  ratio / 3 of its own width) and the thumb sits in a mover as wide as its travel (`.progress-thumb-mover`, translated
  by ratio × 100%), so both are percentage based and width changes need no recalculation (no ResizeObserver). It
  restarts only when `jumpId` or the play state changes or a drag ends, so smooth corrections never make it jump.
  Why not an animated `@property --progress-ratio` (the first version): that custom property fed `left` and the
  gradient stops, which forced one style recalculation and one layout every frame (measured on the Release player at
  120 Hz while playing: 244 layouts and 245 style recalculations in 2 s, 196 ms of main-thread task time, against 4,
  31 and 40 ms with animations paused).
- Lyrics (Now Playing and taskbar) read the same clock and are woken by a timer at the next line start instead of
  recomputing per sample. The taskbar window no longer polls: the main window forwards each sample as
  `taskbar-lyrics-playback`, and the taskbar window asks with `taskbar-lyrics-playback-request` when it opens.

## Consequences

- About 10 → 2 snapshot calls per second (20 → 2 with taskbar lyrics); no per-frame JS work for the bar.
- Without backend events, a change the front end did not cause (end of track, SMTC/headset controls, buffering on a
  remote source, output reopen) shows up within 500 ms; the bar may run up to 500 ms ahead while a remote stream
  stalls, then jumps back.
- Later: a backend `playback-state-changed` event (position, rate, sequence) emitted on play/pause/seek/track change/
  buffering/output reopen for both Windows and Android would let the poll drop to a slow backstop.

## 目前設計細節（自 AGENTS.md 移入）

以下是目前仍有效的實作細節；AGENTS.md 只保留摘要與指向本節的連結。

播放進度時鐘（ADR 0028）：主視窗每 500 ms 取播放快照（`App.svelte` 的 `PLAYBACK_POLL_MS`，最小化時照跑），之間由 `src/lib/playback-clock.ts` 推算（基準＋`performance.now()` 經過時間 × rate，限制在 0..曲長）；偏差 < 30 ms 忽略、30–250 ms 以 750 ms 微調速率（恆 > 0）、> 250 ms／換歌／狀態或曲長改變直接跳（`jumpId` +1）；送指令後、`visibilitychange`、focus、`audio-devices-changed`、推算到曲長時立即再取。進度條：`PlaybackProgress.svelte` 的 `use:progressRun`（`src/lib/progress-run.js`）只在 `jumpId`／狀態改變或拖動結束時以 linear Web Animation 走到 1（`run.name` 交替作為重啟鍵），暫停或拖動時固定在該比例；只動畫 `transform`（`.progress-band` 寬 300% 的固定光暈漸層平移 ratio/3、`.progress-thumb-mover` 平移 ratio×100%），不動畫 `left`／`width`／漸層停點／自訂屬性，所以每幀不產生樣式重算或排版；Web Animation 不受 `prefers-reduced-motion` 影響，維持真實時間；光暈為固定形狀（±32% 後 34% 漸弱到底色 alpha），與舊版在軌道兩端夾限的停點僅在兩端約 32% 內有 ≤ 0.1 alpha 差異；經過時間文字每整秒重讀。`LyricsView` 只在下一句開始時由計時器喚醒（`nextCuePosition`＋`msUntilPlaybackPosition`）。上一首 3 秒判斷與鍵盤／手把快轉用 `currentPlaybackPositionMs()`。後端事件推送尚未做。測試 `tests/playback-clock.test.mjs`。
