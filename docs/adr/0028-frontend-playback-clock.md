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
- Stalls never move the clock back. When a remote stream waits for data the backend still reports "playing" while its
  position stands still; plain extrapolation ran ~500 ms ahead, the next sample jumped it back, and the bar and the
  lyrics showed a sawtooth with the poll period. The clock keeps the previous sample (`lastSample`, a class instance
  so Svelte's deep `$state` does not proxy it; ignored samples update it in place and still return the same clock, so
  nothing reruns). A playing sample is stalled when at least `STALL_MIN_WALL_MS` (250 ms, half a poll) passed since
  the previous one and the position advanced by less than `STALL_MAX_RATE` (0.5) of that time without going back by
  more than 30 ms. A stalled sample 30 ms–`STALL_HOLD_MAX_MS` (1000 ms, two polls) behind the estimate freezes the
  clock where it is shown (`frozen`, rate 0, `playing` stays true). A frozen clock stays put until the real position
  is within 30 ms of it, or is moving again (not stalled) and at most 250 ms behind; then it resumes from the frozen
  position with the usual convergence (rate > 0). So it only waits, at most about the time it had shown ahead. Still
  jumping at once: a new track, state or duration, `force` (user seeks), a sample ahead by more than 250 ms, and a
  sample behind by more than 1000 ms (e.g. an external seek back, or a stall after a long gap between samples). An
  external seek back by less than about 250 ms looks like a stall and is waited out instead of jumped.
- Freeze and resume do not bump `jumpId` (they are not seeks); they bump `runKey`. While frozen
  `playbackClockRunning` is false: `msUntilPlaybackPosition` returns null (no cue or second timers), the estimate is
  the frozen position, `playbackProgressRun` parks the bar, and the Now Playing karaoke sweep stops; on resume each
  restarts from the frozen position.
- The progress bar is a linear Web Animation (`src/lib/progress-run.js`, `use:progressRun`) from the current ratio to 1
  over the remaining time, and it animates `transform` only, so it runs on the compositor and costs no style, layout or
  paint work per frame. The glow track is a fixed gradient band 3 × the track wide (`.progress-band`, translated by
  ratio / 3 of its own width) and the thumb sits in a mover as wide as its travel (`.progress-thumb-mover`, translated
  by ratio × 100%), so both are percentage based and width changes need no recalculation (no ResizeObserver). It
  restarts only when `jumpId`, the play state or `runKey` (stall freeze/resume) changes or a drag ends, so smooth
  corrections never make it jump.
- Buffering cue (user-approved): while the snapshot state is `buffering` (today only Android reports it;
  `bufferingCueVisible`, `src/lib/buffering-cue.js`), `.progress-row[data-buffering]` makes the thumb
  (`.progress-thumb`, not its mover) pulse `filter: brightness()` 1 → 0.9 → 1 over 1000 ms, ease-in-out, infinite. It
  starts as soon as the state is seen and stops the moment it changes, with no delay or timer: the pulse is subtle
  enough that the short buffering after an ordinary seek may show it, and the user wanted it to react at once. This is
  the only buffering cue: no text, label or tooltip changes, and the play button keeps its pause icon. The 1 s period
  is a user-approved exception to the 200 ms animation rule; `prefers-reduced-motion: reduce` turns the pulse off. The
  mover's transform run is untouched. Test `tests/buffering-cue.test.mjs`.
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
  remote source, output reopen) shows up within 500 ms; when a remote stream stalls the bar and the lyrics freeze up to
  one poll ahead of the real position and wait for it instead of jumping back.
- The native lyric windows (ADR 0034) do not use this clock: they follow the audio engine's own reading
  (`position_ms` at `position_at`, which only changes when the position does), so during a stall they keep
  extrapolating with no new reading and correct once (slew or jump back) when the audio moves again.
- Later: a backend `playback-state-changed` event (position, rate, sequence) emitted on play/pause/seek/track change/
  buffering/output reopen for both Windows and Android would let the poll drop to a slow backstop.

## 目前設計細節（自 AGENTS.md 移入）

以下是目前仍有效的實作細節；AGENTS.md 只保留摘要與指向本節的連結。

播放進度時鐘（ADR 0028）：主視窗每 500 ms 取播放快照（`App.svelte` 的 `PLAYBACK_POLL_MS`，最小化時照跑），之間由 `src/lib/playback-clock.ts` 推算（基準＋`performance.now()` 經過時間 × rate，限制在 0..曲長）；偏差 < 30 ms 忽略、30–250 ms 以 750 ms 微調速率（恆 > 0）、> 250 ms／換歌／狀態或曲長改變直接跳（`jumpId` +1）；卡住（播放中但位置不動，判定與門檻見上方 Decision）時凍結在畫面位置（`frozen`、`runKey` +1、不動 `jumpId`），不往回跳，真實位置追上或恢復前進且落後 ≤ 250 ms 才從凍結位置繼續（`playbackClockRunning`）；送指令後、`visibilitychange`、focus、`audio-devices-changed`、推算到曲長時立即再取。進度條：`PlaybackProgress.svelte` 的 `use:progressRun`（`src/lib/progress-run.js`）只在 `jumpId`／狀態／`runKey` 改變或拖動結束時以 linear Web Animation 走到 1（`run.name` 交替作為重啟鍵），暫停或拖動時固定在該比例；只動畫 `transform`（`.progress-band` 寬 300% 的固定光暈漸層平移 ratio/3、`.progress-thumb-mover` 平移 ratio×100%），不動畫 `left`／`width`／漸層停點／自訂屬性，所以每幀不產生樣式重算或排版；Web Animation 不受 `prefers-reduced-motion` 影響，維持真實時間；光暈為固定形狀（±32% 後 34% 漸弱到底色 alpha），與舊版在軌道兩端夾限的停點僅在兩端約 32% 內有 ≤ 0.1 alpha 差異；經過時間文字每整秒重讀。`LyricsView` 只在下一句開始時由計時器喚醒（`nextCuePosition`＋`msUntilPlaybackPosition`）。上一首 3 秒判斷與鍵盤／手把快轉用 `currentPlaybackPositionMs()`。後端事件推送尚未做。測試 `tests/playback-clock.test.mjs`。
