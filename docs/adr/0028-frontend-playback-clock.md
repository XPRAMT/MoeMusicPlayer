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
- The progress bar is a linear CSS animation of the registered custom property `--progress-ratio` (`@property`,
  `<number>`; Chromium 85+, so WebView2 and Android System WebView) from the current ratio to 1 over the remaining time.
  It restarts only when `jumpId` or the play state changes or a drag ends, so smooth corrections never make it jump.
  Gradient and thumb are percentage based, so width changes need no recalculation (no ResizeObserver).
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

播放進度時鐘（ADR 0028）：主視窗每 500 ms 取播放快照（`App.svelte` 的 `PLAYBACK_POLL_MS`，最小化時照跑），之間由 `src/lib/playback-clock.ts` 推算（基準＋`performance.now()` 經過時間 × rate，限制在 0..曲長）；偏差 < 30 ms 忽略、30–250 ms 以 750 ms 微調速率（恆 > 0）、> 250 ms／換歌／狀態或曲長改變直接跳（`jumpId` +1）；送指令後、`visibilitychange`、focus、`audio-devices-changed`、推算到曲長時立即再取。進度條：`app.css` 以 `@property --progress-ratio` 註冊，`PlaybackProgress.svelte` 只在 `jumpId`／狀態改變或拖動結束時以 linear CSS 動畫走到 1（交替 `playback-progress-run-a/b`），暫停或拖動時固定；`prefers-reduced-motion` 下仍保留動畫時間（`--progress-run-ms`）；經過時間文字每整秒重讀。`LyricsView` 只在下一句開始時由計時器喚醒（`nextCuePosition`＋`msUntilPlaybackPosition`）。上一首 3 秒判斷與鍵盤／手把快轉用 `currentPlaybackPositionMs()`。後端事件推送尚未做。測試 `tests/playback-clock.test.mjs`。
