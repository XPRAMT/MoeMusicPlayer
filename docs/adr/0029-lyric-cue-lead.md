# Lyric line changes lead the line start

Status: Accepted and implemented on 2026-10-22

Decision date: 2026-10-22, Asia/Taipei (UTC+08:00)

## Context

On the Now Playing page a new line only started its change animation (smooth scroll, 140 ms colour/opacity
transitions) when the line started, so the page settled after the singing had already begun and the lyrics felt late.
The smooth scroll's length was chosen by the browser, so it could not be made to end at a given moment.

## Decision

- One constant, `LYRIC_CUE_LEAD_MS` = 200 ms (first set to 300 ms, shortened the same day) in `src/lib/lyrics-cue-timing.ts`, is both the lead and the animation
  length; CSS reads it through `--lyric-cue-ms`. Not user-adjustable.
- While playing, a line is shown `lead` before its start: lead = min(200 ms, gap to the previous line); the first line
  is never shown before position 0. The user's lyric delay still applies on top. The animation runs for the time left
  until the line really starts, so a shortened lead or resuming inside the lead animates for what remains.
- The scroll is a fixed-length ease-out rAF tween (only while it runs) instead of `scrollTo({ behavior: 'smooth' })`.
- `activeTimedIndex` stays the line really playing; `displayTimedIndex` drives what is drawn. Paused playback does not
  lead; seeks while playing, track changes and reduced motion stay instant. Play counts and the previous-button
  three-second rule use the real position.
- Taskbar lyrics: no fade; the line switches at its real start (no lead); long-line scrolling starts at the real start.

## Consequences

- The highlighted line on the Now Playing page changes up to 200 ms before the line's timestamp, by design.
- Playback-clock corrections can still move a change by up to the clock's convergence error (< 250 ms, ADR 0028).

## 目前設計細節（自 AGENTS.md 移入）

以下是目前仍有效的實作細節；AGENTS.md 只保留摘要與指向本節的連結。

播放頁同步歌詞（ADR 0029）：虛擬視窗依捲動位置繪製（`lyrics-window.js` 的 `getTimedLyricWindow(layout, scrollTop, 視窗高, 頭部留白, 額外 scrollTop[], overscan=6)`），滑動中範圍取目前視窗與起點／終點的聯集，只在列範圍或留白改變時（`sameLyricWindow`）更新。換行動畫提前開始、在該句時間點剛好結束：提前量與時長只定義在 `src/lib/lyrics-cue-timing.ts` 的 `LYRIC_CUE_LEAD_MS`（200 ms，不開放設定），兩句間距更短時縮短為間距，第一句不早於 0（`displayCuePositions`）；`activeTimedIndex`（`data-current-cue-index`）是真正的當前句，畫面用 `displayTimedIndex`（`data-active-cue-index`、`.active`、`aria-current`）。捲動用自訂 rAF 補間（`startCueScroll`，結束後 `correctTimedCenter`，使用者滾輪／觸控中止），color／opacity transition 用 `--lyric-cue-ms`。所有捲動變化經 `lyrics-scroll-glide.ts` 的 `planTimedScroll`：正常換行為 200 ms ease-out（`restingGlide`），調整每首延遲或 glide 進行中的換行以 `retargetGlide` 從目前位置與速度重設目標（`LYRIC_OFFSET_GLIDE_MS`＝`LYRIC_CUE_LEAD_MS`），位置取整到裝置像素（`snapToDevicePixel`）；`cuePositionMs` 依 `timedOffsetMs` 重算。暫停中不提前；繼續播放時離下一句不到提前量則以剩餘時間動畫；跳轉、換歌、版面改變與 `prefers-reduced-motion` 一律 instant。播放次數、上一首 3 秒判斷用原始時間。測試：`tests/lyrics-cue-timing.test.mjs`、`tests/lyrics-scroll-glide.test.mjs`、`tests/lyrics-controller.test.mjs`、`tests/lyrics-view.playwright.js`（需另開測試伺服器）。Release 實機未驗收。
